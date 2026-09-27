//! `Peripheral` — long-lived BLE peripheral library API for the daemon.
//!
//! S-003 ships the trait, the `PersistentPeripheral` production impl
//! over `bluer 0.17`, and the radio-free `FakePeripheral` test double.
//! See `specs/journeys/JOURNEY-S-003-peripheral-library-api.md` for
//! the design rationale.
//!
//! The trait splits the BLE peripheral role into four named operations
//! the daemon (`syauth-presenced`) consumes across many PAM calls:
//!
//! 1. [`Peripheral::add_peer`] — register a bonded peer's challenge +
//!    response characteristics with the long-lived GATT application.
//! 2. [`Peripheral::remove_peer`] — drop a peer's characteristics
//!    (used after a revoke or a `bonds.toml` diff).
//! 3. [`Peripheral::set_session_uuids`] — replace the advertised
//!    `service_uuids` set. Called by the daemon's per-minute rotation
//!    timer (S-004).
//! 4. [`Peripheral::notify_challenge`] — push challenge bytes on the
//!    per-peer challenge characteristic.
//!
//! `PersistentPeripheral` owns one `bluer::Adapter`, one
//! `bluer::adv::AdvertisementHandle` (replaceable via
//! `set_session_uuids`), one `bluer::gatt::local::ApplicationHandle`
//! (long-lived for the daemon's lifetime), and a
//! `Mutex<HashMap<peer_id, PeerCharSet>>` keyed by stable peer id. The
//! daemon's tokio orchestrator clones an `Arc<dyn Peripheral>` into
//! every per-peer task, so the trait requires `Send + Sync`.
//!
//! `BluerAdvertiser` (the per-PAM-call burst path used by `pam_syauth`
//! today, `crates/syauth-pam/src/auth.rs:575`) is intentionally NOT
//! refactored by S-003 — it remains byte-identical until S-009 deletes
//! it. The library API is a strict superset.

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use bluer::{
    Uuid,
    adv::Advertisement,
    agent::{Agent, AgentHandle, RequestConfirmation},
    gatt::{
        CharacteristicReader, CharacteristicWriter,
        local::{
            Application, ApplicationHandle, Characteristic, CharacteristicControlEvent, CharacteristicNotify, CharacteristicNotifyMethod,
            CharacteristicWrite, CharacteristicWriteMethod, Service, characteristic_control,
        },
    },
};
use futures::StreamExt;
use syauth_core::pair_transaction::{Message, Operation};
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    sync::{Mutex, mpsc, watch},
    task::JoinHandle,
};

use crate::{
    bluez::{
        BOND_KEY_BYTES, PAIR_PUBKEY_LEN, SYAUTH_CHALLENGE_CHAR_UUID, SYAUTH_RESPONSE_CHAR_UUID, map_adapter_open_error, session_uuid_for,
    },
    bluez_advertise::{ADVERTISE_DISCOVERABLE, ADVERTISE_LOCAL_NAME},
    error::TransportError,
    pair_engine::{PairCommitPhase, PairCommitRequest, PairServiceState, build_pair_service, host_name_payload, run_pair_session},
    pairing::PairingBroker,
};

/// Per-peer mpsc depth for incoming response frames. Sized to absorb
/// short bursts of malformed writes without back-pressuring the GATT
/// thread; one in-flight challenge per peer (SPEC §3 #7) makes 8 frames
/// generous headroom.
const RESPONSE_READ_BUF_BYTES: usize = 512;
const PRESENCE_HEARTBEAT: &[u8] = b"SYAUTH-PRESENCE-v1";

/// The peer id named by a day-2 `Revoke` frame, or `None` for everything else
/// the writable characteristic of a bonded session carries (presence heartbeat,
/// RSSI sample, challenge response).
///
/// Revocation has to be understood here, not only on the pair engine's
/// v2-control channel: a bonded session is served by `build_and_register_peer`,
/// whose GATT app exposes just the challenge/response pair, while the phone
/// writes its revoke frame to the response characteristic
/// (`PersistentGattClient.send`). Without this branch the frame lands in the
/// challenge-response channel and the bond survives (BUG-20260924: `sent=true`
/// on the phone, `bonds.toml` untouched on the computer).
fn revoke_peer_id(bytes: &[u8]) -> Option<[u8; 16]> {
    match Message::decode(bytes) {
        Ok(message) if message.operation == Operation::Revoke => Some(message.transaction),
        _ => None,
    }
}
const RSSI_TELEMETRY_PREFIX: &[u8] = b"SYAUTH-RSSI-v1:";
const RSSI_EWMA_ALPHA: f64 = 0.25;

fn rssi_state_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|runtime| std::path::PathBuf::from(runtime).join("syauth").join("rssi.last"))
}

fn parse_rssi(bytes: &[u8]) -> Option<i32> {
    let value = std::str::from_utf8(bytes.strip_prefix(RSSI_TELEMETRY_PREFIX)?)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if (-127..=0).contains(&value) { Some(value) } else { None }
}

fn record_rssi(raw: i32, previous_filtered: Option<f64>) -> f64 {
    previous_filtered.map_or(raw as f64, |previous| {
        RSSI_EWMA_ALPHA * raw as f64 + (1.0 - RSSI_EWMA_ALPHA) * previous
    })
}

fn write_rssi_state(raw: i32, filtered: f64) {
    let Some(path) = rssi_state_path() else { return };
    let Some(parent) = path.parent() else { return };
    let _ = std::fs::create_dir_all(parent);
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let contents = format!("raw={raw}\nfiltered={filtered:.2}\nsample_epoch_ms={timestamp_ms}\n");
    let temp = parent.join(format!(".rssi.last.{}.tmp", std::process::id()));
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).truncate(true).write(true).open(&temp)
        && std::io::Write::write_all(&mut file, contents.as_bytes()).is_ok()
        && file.sync_all().is_ok()
    {
        let _ = std::fs::rename(temp, path);
    }
}

fn challenge_ready_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(|runtime| std::path::PathBuf::from(runtime).join("syauth").join("challenge-ready.last"))
}

fn remove_challenge_ready_marker() {
    if let Some(path) = challenge_ready_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Stable service UUID per bond, derived from the bond key at minute=0.
/// The phone's GATT client discovers characteristics by UUID after the
/// connect — service UUID identity doesn't have to rotate.
fn peer_service_uuid(bond_key: &BondKey) -> Uuid {
    let bytes = session_uuid_for(bond_key, 0);
    Uuid::from_bytes(bytes)
}

// ---------------------------------------------------------------------------
// Public type aliases — keep the trait surface readable.
// ---------------------------------------------------------------------------

/// 32-byte bond key the daemon holds for one bonded peer. Mirrors the
/// width of `syauth_core::BOND_KEY_DERIVED_BYTES` /
/// [`BOND_KEY_BYTES`]. Re-exported here so callers do not have to dig
/// into the bluez module just to declare a parameter type.
pub type BondKey = [u8; BOND_KEY_BYTES];

// ---------------------------------------------------------------------------
// PeripheralError — typed surface for the trait.
// ---------------------------------------------------------------------------

/// Errors produced by [`Peripheral`] implementations.
///
/// Distinct from [`TransportError`] because the persistent-peripheral
/// surface has different failure modes than the per-PAM-call client:
/// `UnknownPeer` is a structural diff-time error the daemon must
/// surface to the operator, while `AdapterMissing` and `Backend` align
/// with their `TransportError` cousins so log lines read consistently.
#[derive(Debug, Error)]
pub enum PeripheralError {
    /// The named BlueZ adapter does not exist on this host. The
    /// operator's fix is well-defined: edit `/etc/syauth.conf` to
    /// name a real adapter (or plug one in).
    #[error("bluetooth adapter '{name}' not found")]
    AdapterMissing {
        /// The adapter id the caller asked for (e.g. `"hci0"`).
        name: String,
    },

    /// The daemon called `remove_peer` or `notify_challenge` with a
    /// `peer_id` that was not previously added via `add_peer`. Always
    /// a structural diff bug at the orchestrator layer.
    #[error("unknown peer: peer_id={peer_id}")]
    UnknownPeer {
        /// The peer_id that was not found.
        peer_id: String,
    },

    /// The daemon called `add_peer` with a `peer_id` that was already
    /// added. The orchestrator's diffing layer must reconcile against
    /// the live set; silent overwrite would leak GATT service handles.
    #[error("peer already added: peer_id={peer_id}")]
    PeerAlreadyAdded {
        /// The peer_id that collided.
        peer_id: String,
    },

    /// Opaque upstream failure from `bluer` or `dbus`. Wraps the
    /// rendered upstream `Display` so the upstream type never escapes
    /// this crate's public API.
    #[error("peripheral backend error: {reason}")]
    Backend {
        /// Human-readable description of the upstream failure.
        reason: String,
    },

    /// The PAM client cancelled the in-flight challenge.
    #[error("challenge cancelled")]
    Cancelled,

    /// `wait_for_response(peer_id, deadline)` reached its deadline
    /// without observing a write on the per-peer response
    /// characteristic. Distinct from `Backend` so the orchestrator's
    /// challenge state machine can map the timeout to the SPEC §6
    /// `TimedOut → AuthInfoUnavail(reason=response-timeout)`
    /// transition without parsing an error string.
    #[error("response timed out: peer_id={peer_id} deadline={deadline_ms}ms")]
    ResponseTimeout {
        /// The peer_id the orchestrator was waiting on.
        peer_id: String,
        /// The deadline that elapsed, in milliseconds, so the audit
        /// row carries the budget that was applied.
        deadline_ms: u64,
    },
}

impl From<TransportError> for PeripheralError {
    fn from(err: TransportError) -> Self {
        match err {
            TransportError::AdapterMissing { name } => PeripheralError::AdapterMissing { name },
            other => PeripheralError::Backend { reason: other.to_string() },
        }
    }
}

// ---------------------------------------------------------------------------
// Peripheral trait — the daemon's stable contract.
// ---------------------------------------------------------------------------

/// Long-lived BLE peripheral the daemon holds across many PAM calls.
///
/// All methods are `async`. Implementations must be `Send + Sync` so
/// the daemon's tokio orchestrator can share one instance behind
/// `Arc<dyn Peripheral>` across per-peer tasks. Object-safety is
/// load-bearing: the orchestrator stores a `Arc<dyn Peripheral>`
/// field, not a generic parameter.
///
/// See the journey doc for the four-phase CJM that motivates each
/// method.
#[async_trait]
pub trait Peripheral: Send + Sync {
    /// Register a bonded peer's challenge + response characteristics
    /// with the long-lived GATT application.
    ///
    /// Returns [`PeripheralError::PeerAlreadyAdded`] if `peer_id` was
    /// already added — silent re-add would leak handles across diff
    /// cycles in the orchestrator.
    async fn add_peer(&self, peer_id: &str, bond_key: &BondKey) -> Result<(), PeripheralError>;

    /// Drop a peer's characteristics from the GATT application.
    ///
    /// Returns [`PeripheralError::UnknownPeer`] if `peer_id` was never
    /// added — diff bugs surface loud, not silent.
    async fn remove_peer(&self, peer_id: &str) -> Result<(), PeripheralError>;

    /// Replace the advertised `service_uuids` set. The previous
    /// advertisement is torn down before the new one is registered so
    /// a passive observer never sees both UUID sets simultaneously.
    async fn set_session_uuids(&self, uuids: std::collections::HashSet<Uuid>) -> Result<(), PeripheralError>;

    /// Push challenge bytes on the per-peer challenge characteristic.
    /// Returns [`PeripheralError::UnknownPeer`] if `peer_id` was never
    /// added.
    async fn notify_challenge(&self, peer_id: &str, frame: &[u8]) -> Result<(), PeripheralError>;

    /// Push a challenge while allowing the owning PAM request to cancel it.
    async fn notify_challenge_cancellable(
        &self,
        peer_id: &str,
        frame: &[u8],
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<(), PeripheralError> {
        tokio::select! {
            _ = cancel.wait_for(|cancelled| *cancelled) => Err(PeripheralError::Cancelled),
            result = self.notify_challenge(peer_id, frame) => result,
        }
    }

    /// Await a single GATT-write on the per-peer response
    /// characteristic, returning the buffered bytes. Returns
    /// [`PeripheralError::ResponseTimeout`] if `deadline` elapses
    /// before a write arrives, or [`PeripheralError::UnknownPeer`]
    /// if `peer_id` was never added.
    ///
    /// S-006 contract: the production [`PersistentPeripheral`]
    /// subscribes once (in `add_peer`) to the response
    /// characteristic's GATT-WRITE events and buffers them in a
    /// per-peer `mpsc::Receiver<Vec<u8>>`. The fake exposes
    /// `inject_response(peer_id, bytes)` so tests queue a synthetic
    /// response without touching a radio.
    async fn wait_for_response(&self, peer_id: &str, deadline: Duration) -> Result<Vec<u8>, PeripheralError>;

    /// Await a response while allowing the owning PAM request to cancel it.
    async fn wait_for_response_cancellable(
        &self,
        peer_id: &str,
        deadline: Duration,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Vec<u8>, PeripheralError> {
        tokio::select! {
            _ = cancel.wait_for(|cancelled| *cancelled) => Err(PeripheralError::Cancelled),
            result = self.wait_for_response(peer_id, deadline) => result,
        }
    }
}

// ---------------------------------------------------------------------------
// PersistentPeripheral — bluer 0.17 production impl.
// ---------------------------------------------------------------------------

/// Per-peer characteristic state owned by [`PersistentPeripheral`].
///
/// S-006 adds the per-peer `tokio::sync::mpsc::Receiver<Vec<u8>>`
/// buffer that backs `wait_for_response(peer_id, deadline)`. The
/// production sender side is fed by the GATT WRITE callback on the
/// response characteristic, but the SPEC keeps the bluez-side
/// subscription wiring as a GAP — see the trait doc on
/// `wait_for_response`.
///
/// GAP: bluez-side GATT WRITE → mpsc::Sender bridge — closure plan
/// is the S-006 response-characteristic registration (this S-006 row
/// ships the trait method + buffer; the BlueZ subscription that
/// pushes onto `response_tx` lives behind the same field name and
/// closes in a follow-on row).
struct PeerCharSet {
    /// Receiver side of the per-peer response buffer. `Mutex` so the
    /// trait method can take exclusive access to a single shared
    /// receiver without requiring `&mut self`.
    response_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    /// Sender side. Held alongside the receiver so the peer's
    /// channel stays alive across the call lifecycle of the GATT
    /// WRITE callback.
    _response_tx: mpsc::Sender<Vec<u8>>,
    /// Per-peer challenge notifier — populated by the bluer control
    /// loop when the phone subscribes to the challenge characteristic
    /// (CCCD write). `notify_challenge` reads this slot and writes the
    /// frame bytes. `None` until the phone subscribes.
    notifier_slot: Arc<Mutex<Option<CharacteristicWriter>>>,
    /// JoinHandle for the per-peer control loop (challenge subscribe +
    /// response write reader). Aborted on `remove_peer`.
    task_handle: Mutex<Option<JoinHandle<()>>>,
    /// Bond key for this peer. Cached so we can rebuild the GATT
    /// application registration on dead-writer detection without
    /// having to plumb the bond store all the way down. The bond
    /// key is the input to `peer_service_uuid(bond_key)`, which the
    /// rebuild path needs to construct the service tree.
    bond_key: BondKey,
}

/// Per-peer response buffer depth for the `PersistentPeripheral`'s
/// `mpsc::channel`. Sized so a malformed phone that batches a burst
/// of WRITEs in 1 s does not back-pressure the GATT thread to a
/// halt; one in-flight challenge per peer is the SPEC §3 scope item
/// #7 contract, so a depth of 8 leaves headroom for transient
/// timing skew without unbounded growth.
const RESPONSE_BUFFER_DEPTH: usize = 8;

/// Bounded budget for one adapter-mode transition. BlueZ answers `Busy`
/// while a previous mode change or a discovery is still settling, so the
/// effective state is re-read and the transition retried instead of failing
/// the session outright.
const ADAPTER_MODE_RETRIES: usize = 10;
const ADAPTER_MODE_RETRY_DELAY: Duration = Duration::from_millis(150);

/// Adapter state a pair session changed, so release restores only that.
///
/// DeskUnlock never touches the adapter's global `Discoverable` flag. The pair
/// service is published as an LE advertisement, which BlueZ serves
/// independently of `Adapter1.Discoverable`: making the whole desktop
/// generically discoverable is not required to accept one GATT pairing
/// session, and it changes the desktop's Bluetooth role for every other
/// device. Only `Pairable` is DeskUnlock's to own, and only for the duration
/// of the session.
struct PairAgentRestore {
    /// Previous `Pairable` value, present only when this session changed it.
    previous_pairable: Option<bool>,
}

impl PairAgentRestore {
    /// Restore plan for a session that drove `Pairable` to the desired state.
    /// An unchanged flag is never touched on release.
    fn from_pairable(changed: bool, previous: bool) -> Self {
        Self {
            previous_pairable: changed.then_some(previous),
        }
    }
}

/// Held pair-session BlueZ agent plus the adapter state to restore. Generic
/// over the handle type so the bookkeeping is unit-testable without a radio.
struct PairAgentSession<H> {
    handle: Option<H>,
    restore: Option<PairAgentRestore>,
}

impl<H> Default for PairAgentSession<H> {
    fn default() -> Self {
        Self {
            handle: None,
            restore: None,
        }
    }
}

impl<H> PairAgentSession<H> {
    fn is_held(&self) -> bool {
        self.handle.is_some()
    }

    fn begin(&mut self, handle: H, restore: PairAgentRestore) {
        self.handle = Some(handle);
        self.restore = Some(restore);
    }

    /// Release the held session, returning its restore plan exactly once.
    /// Further calls are no-ops, so release can neither leak nor double-restore.
    fn end(&mut self) -> Option<PairAgentRestore> {
        self.handle.take()?;
        self.restore.take()
    }
}

/// Minimal seam over the `Pairable` adapter flag, so the acquisition state
/// machine is unit-testable without a radio. `Discoverable` is deliberately
/// absent: a GATT pair session has no reason to touch it.
#[async_trait]
trait PairableControl: Send + Sync {
    async fn pairable(&self) -> Result<bool, String>;
    async fn set_pairable(&self, value: bool) -> Result<(), String>;
}

#[async_trait]
impl PairableControl for bluer::Adapter {
    async fn pairable(&self) -> Result<bool, String> {
        self.is_pairable().await.map_err(|err| err.to_string())
    }

    async fn set_pairable(&self, value: bool) -> Result<(), String> {
        bluer::Adapter::set_pairable(self, value).await.map_err(|err| err.to_string())
    }
}

/// Drive the adapter to `Pairable = true` and verify the effective state.
///
/// Returns `Ok(true)` when this call changed the flag, `Ok(false)` when the
/// adapter was already pairable, and `Err` when the state could not be verified
/// within the bounded retry budget. A BlueZ `Busy` is not fatal on its own:
/// the state is re-read after every attempt.
async fn ensure_pairable_with(adapter: &impl PairableControl, retries: usize, delay: Duration) -> Result<bool, PeripheralError> {
    if adapter.pairable().await.unwrap_or(false) {
        return Ok(false);
    }
    for _ in 0..retries {
        let _ = adapter.set_pairable(true).await;
        if adapter.pairable().await.unwrap_or(false) {
            return Ok(true);
        }
        tokio::time::sleep(delay).await;
    }
    Err(PeripheralError::Backend {
        reason: "adapter Pairable could not be verified as true".to_owned(),
    })
}

/// Production bounds for [`ensure_pairable_with`].
async fn ensure_pairable(adapter: &impl PairableControl) -> Result<bool, PeripheralError> {
    ensure_pairable_with(adapter, ADAPTER_MODE_RETRIES, ADAPTER_MODE_RETRY_DELAY).await
}

/// Acquisition sequence shared by the real adapter and the unit tests: verify
/// `Pairable`, then register the agent, rolling the flag back when registration
/// fails. `Discoverable` is never read or written. A failed acquisition leaves
/// no agent held and no partially acquired adapter state behind.
async fn acquire_pair_agent_with<H, F, Fut>(
    adapter: &impl PairableControl,
    session: &mut PairAgentSession<H>,
    register: F,
) -> Result<(), PeripheralError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<H, String>>,
{
    if session.is_held() {
        return Ok(());
    }
    let previous_pairable = adapter.pairable().await.unwrap_or(false);
    let pairable_changed = ensure_pairable(adapter).await?;
    let handle = match register().await {
        Ok(handle) => handle,
        Err(err) => {
            if pairable_changed {
                let _ = adapter.set_pairable(previous_pairable).await;
            }
            return Err(PeripheralError::Backend {
                reason: format!("register_agent: {err}"),
            });
        }
    };
    session.begin(handle, PairAgentRestore::from_pairable(pairable_changed, previous_pairable));
    Ok(())
}

/// Release sequence shared by the real adapter and the unit tests: drop the
/// agent and restore `Pairable` only when the session changed it. `Discoverable`
/// is never read or written. Idempotent.
async fn release_pair_agent_with<H>(adapter: &impl PairableControl, session: &mut PairAgentSession<H>) {
    let Some(restore) = session.end() else {
        return;
    };
    if let Some(previous_pairable) = restore.previous_pairable {
        let _ = adapter.set_pairable(previous_pairable).await;
    }
}

/// Production peripheral backed by `bluer 0.17`.
///
/// Owns one `bluer::Session`, one `bluer::Adapter`, one long-lived
/// `ApplicationHandle`, and an in-memory map of `PeerCharSet`. The
/// `AdvertisementHandle` lives in a `Mutex` slot so
/// `set_session_uuids` can replace it without taking `&mut self`.
pub struct PersistentPeripheral {
    /// BlueZ session (a `bluer::Session` is the DBus client connection
    /// to `bluetoothd`). Held so the adapter and the application stay
    /// alive for the lifetime of the daemon.
    _session: bluer::Session,
    /// Adapter the peripheral operates on (e.g. `hci0`).
    adapter: bluer::Adapter,
    /// Live GATT application registration. Replaced on every
    /// `add_peer` / `remove_peer` because bluer's `Application` is a
    /// snapshot — services cannot be appended after registration.
    /// `None` until the first peer is added.
    app_handle: Mutex<Option<ApplicationHandle>>,
    /// Currently-published advertisement. Replaced by
    /// `set_session_uuids`. Optional because the daemon may construct
    /// the peripheral before any UUIDs are known (cold-start path).
    adv_slot: Mutex<Option<bluer::adv::AdvertisementHandle>>,
    /// Held pair-session BlueZ agent and the adapter state to restore.
    /// Empty outside an explicit DeskUnlock pair session, so no default agent
    /// is ever left registered.
    pair_agent: Mutex<PairAgentSession<AgentHandle>>,
    /// Broker that relays a real BlueZ confirmation request to the GUI.
    pairing_broker: PairingBroker,
    /// Shared GATT pair-service state and V2 transaction inbox (holds the
    /// host public key and authenticated host-name metadata).
    pair_state: Arc<PairServiceState>,
    /// Commit-boundary persistence channel. The daemon persists trust only
    /// after the V2 commit decision.
    pair_commit_tx: mpsc::Sender<PairCommitRequest>,
    /// JoinHandle for the pair engine task spawned by the most
    /// recent application registration. Aborted before re-spawning
    /// so we never have two engines competing on stale control
    /// streams.
    pair_watcher: Mutex<Option<JoinHandle<()>>>,
    /// Per-peer characteristic state. Keyed by stable peer_id.
    peers: Mutex<HashMap<String, PeerCharSet>>,
}

/// Resolve a peer's Bluetooth display name for a bond record.
///
/// The transport name is a *label*, never a trust identity: it exists so the
/// operator sees "Pixel 8" instead of "phone (paired via daemon)" in
/// `syauth list`. Returns `None` when the adapter or the device name is
/// unavailable, and the caller falls back to the transport label.
pub async fn peer_display_name(adapter_id: &str, address: &str) -> Option<String> {
    let session = bluer::Session::new().await.ok()?;
    let adapter = session.adapter(adapter_id).ok()?;
    let device = adapter.device(address.parse::<bluer::Address>().ok()?).ok()?;
    device.name().await.ok().flatten().filter(|name| !name.trim().is_empty())
}

impl PersistentPeripheral {
    /// Construct a `PersistentPeripheral` bound to `adapter_id`.
    ///
    /// Opens the BlueZ adapter, powers it on, and registers an empty
    /// long-lived GATT application. The advertisement slot is empty
    /// until the caller invokes [`Peripheral::set_session_uuids`].
    ///
    /// # Errors
    ///
    /// Returns [`PeripheralError::AdapterMissing`] when the named
    /// adapter is unknown to BlueZ, or [`PeripheralError::Backend`]
    /// for any other upstream failure.
    pub async fn new(
        adapter_id: &str,
        pairing_broker: PairingBroker,
        pair_commit_tx: mpsc::Sender<PairCommitRequest>,
    ) -> Result<Arc<Self>, PeripheralError> {
        let session = bluer::Session::new()
            .await
            .map_err(|err| PeripheralError::from(map_adapter_open_error(adapter_id, err)))?;
        let adapter = session
            .adapter(adapter_id)
            .map_err(|err| PeripheralError::from(map_adapter_open_error(adapter_id, err)))?;
        adapter.set_powered(true).await.map_err(|err| PeripheralError::Backend {
            reason: format!("adapter set_powered: {err}"),
        })?;
        // The general Bluetooth role (discoverable/pairable + default agent)
        // is NOT owned here. It is acquired only for an explicit DeskUnlock
        // pair session via `acquire_pair_agent`, so DMS stays the desktop's
        // general Bluetooth frontend.
        //
        // Mint an opaque 32-byte host pubkey used as HKDF input on the
        // pair-mode characteristic. The desktop only uses it as
        // pair-time HKDF input; persistence across daemon restarts is
        // not required because the derived bond_key is committed to
        // disk once at pair time.
        let mut host_pubkey = [0u8; PAIR_PUBKEY_LEN];
        getrandom::fill(&mut host_pubkey).map_err(|err| PeripheralError::Backend {
            reason: format!("host_pubkey rng: {err}"),
        })?;
        let host_name = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|name| host_name_payload(&name))
            .unwrap_or_else(|_| host_name_payload(""));
        let pair_state = Arc::new(PairServiceState::new(host_pubkey, host_name));
        let peripheral = Arc::new(Self {
            _session: session,
            adapter,
            app_handle: Mutex::new(None),
            adv_slot: Mutex::new(None),
            pair_agent: Mutex::new(PairAgentSession::default()),
            pairing_broker,
            pair_state,
            pair_commit_tx,
            pair_watcher: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
        });
        // Register an initial GATT application that contains only the
        // pair-mode service. Without this a fresh-install daemon (no
        // bonds yet) would be invisible to a phone trying to pair:
        // BlueZ rejects empty applications, so registration used to
        // be deferred until the first bond was added — but you cannot
        // add a bond without first pairing. The pair service is
        // always-present to break the chicken-and-egg.
        //
        // Advertising the pair-mode UUID is the orchestrator's job
        // (it includes the pair UUID in every minute's union); we
        // just register the GATT app here so the characteristic is
        // discoverable once the orchestrator publishes the UUID.
        peripheral.rebuild_application(vec![]).await?;
        Ok(peripheral)
    }

    /// Acquire the BlueZ default agent for an explicit DeskUnlock pair
    /// session. Idempotent. Verifies the adapter is pairable for the duration
    /// and registers the confirmation agent; [`Self::release_pair_agent`]
    /// restores the previous pairable state and drops the agent so DeskUnlock
    /// never remains the global Bluetooth pairing manager.
    ///
    /// The adapter's global `Discoverable` flag is deliberately left untouched:
    /// the pair service is published as an LE advertisement, which BlueZ serves
    /// regardless of that flag.
    ///
    /// # Errors
    ///
    /// Returns [`PeripheralError::Backend`] when the adapter or agent
    /// registration fails.
    pub async fn acquire_pair_agent(&self) -> Result<(), PeripheralError> {
        let mut session = self.pair_agent.lock().await;
        if session.is_held() {
            return Ok(());
        }
        // The mode is verified before the agent is registered. A mode
        // transition that answered Busy earlier must never skip that
        // registration.
        acquire_pair_agent_with(&self.adapter, &mut session, || async {
            let broker = self.pairing_broker.clone();
            let agent = Agent {
                request_default: true,
                request_confirmation: Some(Box::new(move |req: RequestConfirmation| {
                    let broker = broker.clone();
                    Box::pin(async move {
                        if broker.request_confirmation(req.device.to_string(), req.passkey).await {
                            Ok(())
                        } else {
                            Err(bluer::agent::ReqError::Rejected)
                        }
                    })
                })),
                ..Default::default()
            };
            self._session.register_agent(agent).await.map_err(|err| err.to_string())
        })
        .await?;
        tracing::info!(target: "syauth_transport", "DeskUnlock pair agent acquired for this session only");
        Ok(())
    }

    /// Release the pair-session agent and restore `Pairable` only when this
    /// session changed it. `Discoverable` is never touched. Idempotent.
    pub async fn release_pair_agent(&self) {
        let mut session = self.pair_agent.lock().await;
        if !session.is_held() {
            return;
        }
        release_pair_agent_with(&self.adapter, &mut session).await;
        tracing::info!(target: "syauth_transport", "DeskUnlock pair agent released; Pairable restored to the desktop's own value");
    }

    /// Disconnect every LE peer currently connected to our BlueZ
    /// adapter. Returns `Ok(())` when every disconnect call succeeds;
    /// surface-level failures (peer unknown, dbus error) are swallowed
    /// behind a `warn` so a single stuck device cannot block the
    /// caller. Called after every fresh `serve_gatt_application` so a
    /// phone whose CCCD subscription is bound to the previous
    /// Application registration is forced to re-handshake.
    async fn kick_connected_peers(&self) -> Result<(), PeripheralError> {
        let addrs = self.adapter.device_addresses().await.map_err(|err| PeripheralError::Backend {
            reason: format!("device_addresses: {err}"),
        })?;
        for addr in addrs {
            let device = match self.adapter.device(addr) {
                Ok(d) => d,
                Err(err) => {
                    tracing::warn!(
                        target: "syauth_transport",
                        addr = %addr,
                        error = %err,
                        "kick_connected_peers: device handle unavailable"
                    );
                    continue;
                }
            };
            let connected = device.is_connected().await.unwrap_or(false);
            if !connected {
                continue;
            }
            match device.disconnect().await {
                Ok(()) => {
                    tracing::info!(
                        target: "syauth_transport",
                        addr = %addr,
                        "kick_connected_peers: disconnected stale peer"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        target: "syauth_transport",
                        addr = %addr,
                        error = %err,
                        "kick_connected_peers: Device::disconnect failed"
                    );
                }
            }
        }
        Ok(())
    }

    /// Register a fresh GATT application. `peer_services` is the
    /// per-bonded-peer service list (one entry per active bond);
    /// the always-present pair-mode service is prepended here so
    /// the daemon stays discoverable for a phone trying to pair
    /// even with zero bonds.
    ///
    /// Replaces the live `app_handle` and (re)spawns the pair
    /// watcher task. The previous watcher is aborted before the
    /// new one starts so we never have two competing on stale
    /// `chal_control` streams.
    async fn rebuild_application(&self, peer_services: Vec<Service>) -> Result<(), PeripheralError> {
        // Abort the previous pair watcher first; its chal_control
        // stream will be invalidated when we drop the old app_handle.
        if let Some(prev) = self.pair_watcher.lock().await.take() {
            prev.abort();
        }
        let (pair_service, pair_phone_control, pair_v2_control) = build_pair_service(Arc::clone(&self.pair_state));
        let peer_count = peer_services.len();
        let mut services = Vec::with_capacity(1 + peer_count);
        services.push(pair_service);
        for s in peer_services {
            services.push(s);
        }
        let app = Application {
            services,
            ..Default::default()
        };
        // Drop the previous app_handle before registering the new
        // one — bluer rejects a second registration while the first
        // is live.
        {
            let mut slot = self.app_handle.lock().await;
            *slot = None;
        }
        tracing::info!(target: "syauth_transport", peers = peer_count, "registering GATT app with pair service");
        let new_handle = self
            .adapter
            .serve_gatt_application(app)
            .await
            .map_err(|err| PeripheralError::Backend {
                reason: format!("serve_gatt_application: {err}"),
            })?;
        tracing::info!(
            target: "syauth_transport",
            "GATT app registration accepted by BlueZ"
        );
        {
            let mut slot = self.app_handle.lock().await;
            *slot = Some(new_handle);
        }
        // Spawn the daemon-owned pair engine: the phone-pubkey write opens a
        // session, the engine drives the V2 transaction, and the daemon
        // persists trust only at the commit boundary.
        let task = tokio::spawn(run_pair_session(
            Arc::clone(&self.pair_state),
            self.pairing_broker.clone(),
            self.pair_commit_tx.clone(),
            pair_phone_control,
            pair_v2_control,
        ));
        *self.pair_watcher.lock().await = Some(task);
        Ok(())
    }

    /// Helper: build a `bluer` advertisement object from a UUID set.
    /// Pure synchronous factory so the unit tests in this module can
    /// inspect the constructed structure without an adapter.
    fn build_advertisement(uuids: std::collections::HashSet<Uuid>) -> Advertisement {
        // bluer's `Advertisement::service_uuids` is a `BTreeSet`, so
        // we collect once into the destination shape.
        let service_uuids: std::collections::BTreeSet<Uuid> = uuids.into_iter().collect();
        Advertisement {
            service_uuids,
            discoverable: Some(ADVERTISE_DISCOVERABLE),
            local_name: Some(ADVERTISE_LOCAL_NAME.to_owned()),
            ..Default::default()
        }
    }
}

// `Send + Sync` audit: every field is `Send + Sync` —
// `bluer::Session`, `bluer::Adapter`, `ApplicationHandle`,
// `Mutex<Option<AdvertisementHandle>>`, `Mutex<HashMap<..>>`.
// The auto-derived bounds suffice.

impl PersistentPeripheral {
    /// Build a fresh Service+Characteristic tree for one peer and
    /// register it. Returns the per-peer state (notifier slot,
    /// response channel, control-loop task) that `add_peer` stashes
    /// into `PeerCharSet`. Called fresh on every `add_peer` because
    /// bluer's `Application` is a snapshot — control_handles are
    /// consumed by registration and cannot be reused.
    async fn build_and_register_peer(
        &self,
        bond_key: &BondKey,
    ) -> Result<
        (
            Arc<Mutex<Option<CharacteristicWriter>>>,
            mpsc::Sender<Vec<u8>>,
            mpsc::Receiver<Vec<u8>>,
            JoinHandle<()>,
        ),
        PeripheralError,
    > {
        let (mut chal_control, chal_handle) = characteristic_control();
        let (mut resp_control, resp_handle) = characteristic_control();
        let service_uuid = peer_service_uuid(bond_key);
        let app = Application {
            services: vec![Service {
                uuid: service_uuid,
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: SYAUTH_CHALLENGE_CHAR_UUID,
                        notify: Some(CharacteristicNotify {
                            notify: true,
                            method: CharacteristicNotifyMethod::Io,
                            ..Default::default()
                        }),
                        control_handle: chal_handle,
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: SYAUTH_RESPONSE_CHAR_UUID,
                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Io,
                            ..Default::default()
                        }),
                        control_handle: resp_handle,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        tracing::info!(target: "syauth_transport", uuid=%service_uuid, "registering GATT app for peer");
        let new_handle = self
            .adapter
            .serve_gatt_application(app)
            .await
            .map_err(|err| PeripheralError::Backend {
                reason: format!("serve_gatt_application: {err}"),
            })?;
        tracing::info!(target: "syauth_transport", "GATT app registration accepted by BlueZ");
        // Replace the live application registration.
        let mut slot = self.app_handle.lock().await;
        *slot = Some(new_handle);
        drop(slot);

        // Fast path: do not force an Android BLE reconnect here.
        // If the notifier is genuinely stale, notify_challenge()
        // invokes rebuild_peer_registration(), which performs the
        // controlled disconnect and fresh GATT registration.

        let notifier_slot: Arc<Mutex<Option<CharacteristicWriter>>> = Arc::new(Mutex::new(None));
        let (response_tx, response_rx) = mpsc::channel::<Vec<u8>>(RESPONSE_BUFFER_DEPTH);
        let notifier_slot_for_task = notifier_slot.clone();
        let response_tx_for_task = response_tx.clone();
        // Day-2 revocation reaches the daemon through the same commit channel
        // the pair engine uses: a bonded session is served by this task, and its
        // GATT app has no v2-control characteristic to carry a revoke frame.
        let revoke_tx_for_task = self.pair_commit_tx.clone();
        let task = tokio::spawn(async move {
            let mut reader_opt: Option<CharacteristicReader> = None;
            let mut rssi_filtered: Option<f64> = None;
            loop {
                tokio::select! {
                    // Phone subscribes / unsubscribes to challenge notifications.
                    chal_evt = chal_control.next() => {
                        match chal_evt {
                            Some(CharacteristicControlEvent::Notify(writer)) => {
                                tracing::info!(target: "syauth_transport", "chal_control: Notify event — phone subscribed");
                                *notifier_slot_for_task.lock().await = Some(writer);

                                remove_challenge_ready_marker();
                                if let Some(path) = challenge_ready_path() {
                                    let mut token = [0u8; 16];
                                    if getrandom::fill(&mut token).is_ok() {
                                        let token = token.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
                                        let _ = std::fs::write(path, format!("{token}\n"));
                                    }
                                }
                            }
                            Some(CharacteristicControlEvent::Write(_)) => {
                                tracing::warn!(target: "syauth_transport", "chal_control: unexpected Write event");
                            }
                            None => {
                                *notifier_slot_for_task.lock().await = None;
                                remove_challenge_ready_marker();
                                tracing::warn!(target: "syauth_transport", "chal_control: stream ended, task exiting");
                                break;
                            }
                        }
                    }
                    // Phone writes a response frame; accept and drain.
                    resp_evt = resp_control.next() => {
                        match resp_evt {
                            Some(CharacteristicControlEvent::Write(req)) => {
                                tracing::info!(target: "syauth_transport", "resp_control: Write event — phone writing response");
                                match req.accept() {
                                    Ok(reader) => { reader_opt = Some(reader); }
                                    Err(err) => {
                                        tracing::warn!(target: "syauth_transport", error=%err, "resp_control: req.accept failed");
                                        continue;
                                    }
                                }
                            }
                            Some(CharacteristicControlEvent::Notify(_)) => {
                                tracing::warn!(target: "syauth_transport", "resp_control: unexpected Notify event");
                            }
                            None => {
                                *notifier_slot_for_task.lock().await = None;
                                remove_challenge_ready_marker();
                                tracing::warn!(target: "syauth_transport", "resp_control: stream ended, task exiting");
                                break;
                            }
                        }
                    }
                    // Drain bytes from the response reader, if active.
                    read_res = async {
                        match &mut reader_opt {
                            Some(reader) => {
                                let mut buf = vec![0u8; RESPONSE_READ_BUF_BYTES];
                                let n = reader.read(&mut buf).await?;
                                buf.truncate(n);
                                Ok::<Vec<u8>, std::io::Error>(buf)
                            }
                            None => std::future::pending().await,
                        }
                    } => {
                        match read_res {
                            Ok(bytes) if !bytes.is_empty() => {
                                if bytes.as_slice() == PRESENCE_HEARTBEAT {
                                    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
                                        let path = std::path::PathBuf::from(runtime)
                                            .join("syauth")
                                            .join("presence.last");
                                        let _ = std::fs::write(path, b"1
                ");
                                    }
                                    tracing::debug!(
                                        target: "syauth_transport",
                                        "presence heartbeat received"
                                    );
                                } else if let Some(peer_id) = revoke_peer_id(&bytes) {
                                    // Not part of any challenge: the phone is
                                    // telling us it dropped this bond, with the
                                    // peer id in the transaction field (16 raw
                                    // bytes = 32 hex).
                                    let (reply, _answer) = tokio::sync::oneshot::channel();
                                    let request = PairCommitRequest {
                                        phase: PairCommitPhase::Revoke,
                                        transaction: peer_id,
                                        phone_pubkey: [0; PAIR_PUBKEY_LEN],
                                        host_pubkey: [0; PAIR_PUBKEY_LEN],
                                        peer: String::new(),
                                        reply,
                                    };
                                    if let Err(err) = revoke_tx_for_task.try_send(request) {
                                        tracing::warn!(target: "syauth_transport", error = %err, "day-2 revoke request dropped");
                                    }
                                } else if let Some(raw) = parse_rssi(&bytes) {
                                    let filtered = record_rssi(raw, rssi_filtered);
                                    rssi_filtered = Some(filtered);
                                    write_rssi_state(raw, filtered);
                                    tracing::debug!(
                                        target: "syauth_transport",
                                        raw,
                                        filtered = format_args!("{filtered:.2}"),
                                        age_ms = 0,
                                        "RSSI telemetry sample"
                                    );
                                } else {
                                    let _ = response_tx_for_task.send(bytes).await;
                                }
                            }
                            Ok(_) | Err(_) => {
                                // Reader closed or errored; drop it, wait for next Write event.
                                reader_opt = None;
                            }
                        }
                    }
                }
            }
        });
        Ok((notifier_slot, response_tx, response_rx, task))
    }

    /// Rebuild the GATT application registration for a single peer.
    ///
    /// Why this is needed: a `Device::disconnect()` (our
    /// `kick_connected_peers`) drops the LE link but **does not** force
    /// BlueZ to emit a fresh `CharacteristicControlEvent::Notify` when
    /// the phone re-subscribes against the same `Application` object.
    /// BlueZ remembers the per-characteristic subscription state across
    /// link transitions, so a re-subscribe is silently merged into the
    /// existing one. The `CharacteristicWriter` cached in
    /// `notifier_slot` stays dead forever, and `notify_challenge`
    /// audits `transport-error` until the daemon is restarted.
    ///
    /// The only kick that reliably triggers a fresh Notify event is
    /// unregistering the application entirely and re-registering it,
    /// which discards BlueZ's subscription state. We do that here:
    /// abort the old chal_control/resp_control task, drop the old
    /// `ApplicationHandle`, kick any LE link so the phone reconnects
    /// against the fresh application, then `build_and_register_peer`
    /// with the cached bond key and swap the new state into the
    /// `PeerCharSet`.
    ///
    /// Lock ordering: we never hold a `notifier_slot` lock across this
    /// call — `notify_challenge` releases it before invoking us. We
    /// take the `peers` lock briefly to read `bond_key`, drop it,
    /// touch `app_handle`, then take `peers` again to swap the entry.
    async fn rebuild_peer_registration(&self, peer_id: &str) -> Result<(), PeripheralError> {
        remove_challenge_ready_marker();
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            let path = std::path::PathBuf::from(runtime).join("syauth").join("challenge-ready.last");
            let _ = std::fs::remove_file(path);
        }

        let bond_key: BondKey = {
            let peers = self.peers.lock().await;
            let entry = peers.get(peer_id).ok_or_else(|| PeripheralError::UnknownPeer {
                peer_id: peer_id.to_owned(),
            })?;
            entry.bond_key
        };
        // Abort the old per-peer control task. After this the previous
        // chal_control / resp_control characteristic_control streams
        // are owned by a dead task; dropping the `ApplicationHandle`
        // below will close them on the BlueZ side.
        {
            let peers = self.peers.lock().await;
            if let Some(entry) = peers.get(peer_id)
                && let Some(handle) = entry.task_handle.lock().await.take()
            {
                handle.abort();
            }
        }
        // Drop the old `ApplicationHandle`. bluer issues
        // `UnregisterApplication` on Drop, which prompts BlueZ to
        // forget every CCCD subscription bound to that application.
        {
            let mut slot = self.app_handle.lock().await;
            *slot = None;
        }
        // Kick any LE link so the phone reconnects against the fresh
        // application. Without this the phone's GATT cache still
        // points at the old application's characteristic handles —
        // the phone will write CCCD on a handle that no longer
        // belongs to anything, and BlueZ silently drops the write.
        if let Err(err) = self.kick_connected_peers().await {
            tracing::warn!(
                target: "syauth_transport",
                error = %err,
                "rebuild_peer_registration: kick_connected_peers failed"
            );
        }
        // Build + register a fresh application for this peer.
        let (notifier_slot, response_tx, response_rx, task) = self.build_and_register_peer(&bond_key).await?;
        // Swap the new state into the peer entry. `bond_key` is
        // copied (it is a `[u8; N]`) so the new entry stays
        // self-contained.
        {
            let mut peers = self.peers.lock().await;
            if let Some(entry) = peers.get_mut(peer_id) {
                *entry.notifier_slot.lock().await = None;
                entry.notifier_slot = notifier_slot;
                entry.response_rx = Mutex::new(response_rx);
                entry._response_tx = response_tx;
                *entry.task_handle.lock().await = Some(task);
            }
        }
        tracing::info!(
            target: "syauth_transport",
            peer_id = %peer_id,
            "rebuild_peer_registration: fresh GATT application registered, awaiting re-subscribe"
        );
        Ok(())
    }
}

#[async_trait]
impl Peripheral for PersistentPeripheral {
    async fn add_peer(&self, peer_id: &str, bond_key: &BondKey) -> Result<(), PeripheralError> {
        let peers = self.peers.lock().await;
        if peers.contains_key(peer_id) {
            return Err(PeripheralError::PeerAlreadyAdded {
                peer_id: peer_id.to_owned(),
            });
        }
        // Drop the existing peer-handle (if any) BEFORE serve_gatt_application
        // — bluer rejects a second registration while the first is live.
        drop(peers);
        {
            let mut slot = self.app_handle.lock().await;
            *slot = None;
        }
        let (notifier_slot, response_tx, response_rx, task) = self.build_and_register_peer(bond_key).await?;
        let mut peers = self.peers.lock().await;
        peers.insert(
            peer_id.to_owned(),
            PeerCharSet {
                response_rx: Mutex::new(response_rx),
                _response_tx: response_tx,
                notifier_slot,
                task_handle: Mutex::new(Some(task)),
                bond_key: *bond_key,
            },
        );
        Ok(())
    }

    async fn remove_peer(&self, peer_id: &str) -> Result<(), PeripheralError> {
        let mut peers = self.peers.lock().await;
        let Some(entry) = peers.remove(peer_id) else {
            return Err(PeripheralError::UnknownPeer {
                peer_id: peer_id.to_owned(),
            });
        };
        if let Some(handle) = entry.task_handle.lock().await.take() {
            handle.abort();
        }
        // Drop application registration; daemon will rebuild on next add.
        let mut slot = self.app_handle.lock().await;
        *slot = None;
        Ok(())
    }

    async fn set_session_uuids(&self, uuids: std::collections::HashSet<Uuid>) -> Result<(), PeripheralError> {
        let mut slot = self.adv_slot.lock().await;
        // Tear down the previous advertisement first so a passive
        // observer never sees both UUID sets simultaneously.
        slot.take();
        let advertisement = Self::build_advertisement(uuids);
        let handle = self
            .adapter
            .advertise(advertisement)
            .await
            .map_err(|err| PeripheralError::Backend {
                reason: format!("advertise: {err}"),
            })?;
        *slot = Some(handle);
        Ok(())
    }

    async fn notify_challenge(&self, peer_id: &str, frame: &[u8]) -> Result<(), PeripheralError> {
        use tokio::io::AsyncWriteExt;

        // Drain stale responses left by a previous timed-out challenge.
        {
            let peers = self.peers.lock().await;
            let peer = peers.get(peer_id).ok_or_else(|| PeripheralError::UnknownPeer {
                peer_id: peer_id.to_owned(),
            })?;
            let mut rx = peer.response_rx.lock().await;
            while rx.try_recv().is_ok() {}
        }

        // First attempt uses the currently cached writer.
        // If that writer is dead/missing, rebuild the GATT application once,
        // wait for the phone to subscribe to the fresh characteristic,
        // then retry THIS SAME challenge instead of losing it.
        for attempt in 0..2 {
            let notifier_slot = {
                let peers = self.peers.lock().await;
                let peer = peers.get(peer_id).ok_or_else(|| PeripheralError::UnknownPeer {
                    peer_id: peer_id.to_owned(),
                })?;
                peer.notifier_slot.clone()
            };

            let mut slot = notifier_slot.lock().await;

            if let Some(writer) = slot.as_mut() {
                tracing::info!(
                    target: "syauth_transport",
                    peer_id = %peer_id,
                    bytes = frame.len(),
                    attempt,
                    "notify_challenge: writing frame"
                );

                match writer.write_all(frame).await {
                    Ok(()) => {
                        if attempt == 1 {
                            tracing::info!(
                                target: "syauth_transport",
                                peer_id = %peer_id,
                                "notify_challenge: retry succeeded after GATT rebuild"
                            );
                        }
                        return Ok(());
                    }
                    Err(err) if attempt == 0 => {
                        remove_challenge_ready_marker();
                        tracing::warn!(
                            target: "syauth_transport",
                            peer_id = %peer_id,
                            error = %err,
                            "notify_challenge: cached writer dead — rebuilding GATT application and retrying"
                        );
                        *slot = None;
                    }
                    Err(err) => {
                        return Err(PeripheralError::Backend {
                            reason: format!("notify_challenge retry write: {err}"),
                        });
                    }
                }
            } else if attempt == 0 {
                remove_challenge_ready_marker();
                tracing::warn!(
                    target: "syauth_transport",
                    peer_id = %peer_id,
                    "notify_challenge: notifier missing — rebuilding GATT application and retrying"
                );
            } else {
                return Err(PeripheralError::Backend {
                    reason: format!("no active GATT subscription after rebuild for peer_id={peer_id}"),
                });
            }

            drop(slot);

            self.rebuild_peer_registration(peer_id).await?;

            // Fresh ApplicationHandle means BlueZ has forgotten the stale CCCD.
            // Wait for Android to reconnect and produce a genuinely new Notify writer.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);

            loop {
                let ready = {
                    let peers = self.peers.lock().await;
                    let peer = peers.get(peer_id).ok_or_else(|| PeripheralError::UnknownPeer {
                        peer_id: peer_id.to_owned(),
                    })?;
                    peer.notifier_slot.lock().await.is_some()
                };

                if ready {
                    tracing::info!(
                        target: "syauth_transport",
                        peer_id = %peer_id,
                        "notify_challenge: fresh subscription ready — retrying challenge"
                    );
                    break;
                }

                if tokio::time::Instant::now() >= deadline {
                    return Err(PeripheralError::Backend {
                        reason: format!("fresh GATT subscription timeout for peer_id={peer_id}"),
                    });
                }

                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }

        Err(PeripheralError::Backend {
            reason: format!("notify_challenge retry exhausted for peer_id={peer_id}"),
        })
    }

    async fn wait_for_response(&self, peer_id: &str, deadline: Duration) -> Result<Vec<u8>, PeripheralError> {
        // We hold the outer peers-map lock across the await on
        // `rx.recv()`. S-003/S-006 do not exercise concurrent
        // challenges on the SAME peer — SPEC §3 scope item #7 caps
        // in-flight challenges at one per peer — so the lock-across-
        // await pattern is correct for the persistent peripheral.
        // The fake exposes a more parallel-friendly shape because
        // its tests are the only place that exercise multi-peer
        // concurrency in CI.
        let peers = self.peers.lock().await;
        let peer = peers.get(peer_id).ok_or_else(|| PeripheralError::UnknownPeer {
            peer_id: peer_id.to_owned(),
        })?;
        let mut rx = peer.response_rx.lock().await;
        match tokio::time::timeout(deadline, rx.recv()).await {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(PeripheralError::Backend {
                reason: format!("response channel closed for peer_id={peer_id}"),
            }),
            Err(_) => Err(PeripheralError::ResponseTimeout {
                peer_id: peer_id.to_owned(),
                deadline_ms: u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// FakePeripheral — radio-free test double.
// ---------------------------------------------------------------------------

/// Test-only state recorded by [`FakePeripheral`]. Held behind a
/// `Mutex` so the fake stays `Send + Sync` like the production impl.
#[cfg(any(test, feature = "test-fake"))]
#[derive(Default)]
struct FakeState {
    /// Peers in insertion order so tests can assert on the order
    /// after a `remove_peer` in the middle of the sequence.
    peers_in_order: Vec<String>,
    /// Bond keys keyed by peer_id, mirroring the production state.
    peer_keys: HashMap<String, BondKey>,
    /// Every `set_session_uuids` argument, recorded in call order.
    session_uuid_calls: Vec<std::collections::HashSet<Uuid>>,
    /// Every `notify_challenge` argument, recorded in call order, so
    /// later daemon tests (S-006) can assert on the sequence.
    notify_calls: Vec<(String, Vec<u8>)>,
    /// Per-peer FIFO of queued response bytes. `inject_response`
    /// pushes onto the back; `wait_for_response` pops from the
    /// front. An empty FIFO when `wait_for_response` is called
    /// means "the response never arrived" — the fake returns
    /// `PeripheralError::ResponseTimeout` after the deadline.
    response_queue: HashMap<String, std::collections::VecDeque<Vec<u8>>>,
}

/// Radio-free `Peripheral` for tests and CI.
///
/// Records every call in order so test assertions read like
/// requirements. The internal state is held behind a `std::sync::Mutex`
/// (not a `tokio::sync::Mutex`) so the synchronous getters
/// (`peers`, `session_uuid_calls`, `notify_calls`) can be called from
/// inside a `#[tokio::test]` body without panicking on
/// `block_on_runtime`. The lock is held briefly and never across an
/// `await`, so a `std::sync::Mutex` is the right primitive.
#[cfg(any(test, feature = "test-fake"))]
pub struct FakePeripheral {
    state: std::sync::Mutex<FakeState>,
}

#[cfg(any(test, feature = "test-fake"))]
impl FakePeripheral {
    /// Construct a fresh `FakePeripheral` with no peers, no advertised
    /// UUIDs, and no recorded calls.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Snapshot of the currently-registered peers in insertion order
    /// (with the natural gap when a middle peer was removed).
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned by a prior thread
    /// panic. Tests never share `FakePeripheral` across panicking
    /// tasks, so this is unreachable in normal operation.
    #[must_use]
    pub fn peers(&self) -> Vec<String> {
        match self.state.lock() {
            Ok(g) => g.peers_in_order.clone(),
            Err(poisoned) => poisoned.into_inner().peers_in_order.clone(),
        }
    }

    /// Snapshot of every `set_session_uuids` argument in call order.
    /// Tests assert on the full sequence so a regression that drops or
    /// merges intermediate calls is mechanically visible.
    #[must_use]
    pub fn session_uuid_calls(&self) -> Vec<std::collections::HashSet<Uuid>> {
        match self.state.lock() {
            Ok(g) => g.session_uuid_calls.clone(),
            Err(poisoned) => poisoned.into_inner().session_uuid_calls.clone(),
        }
    }

    /// Snapshot of every `notify_challenge` argument in call order.
    #[must_use]
    pub fn notify_calls(&self) -> Vec<(String, Vec<u8>)> {
        match self.state.lock() {
            Ok(g) => g.notify_calls.clone(),
            Err(poisoned) => poisoned.into_inner().notify_calls.clone(),
        }
    }

    /// Queue a synthetic response for the next
    /// [`Peripheral::wait_for_response`] call on `peer_id`. Tests
    /// inject a valid signed response for the success path and
    /// garbage bytes for the bad-signature path. A peer that was
    /// never `add_peer`'d still accepts queued responses — the
    /// trait call surfaces the `UnknownPeer` error from
    /// `wait_for_response`, not from `inject_response`.
    pub fn inject_response(&self, peer_id: &str, bytes: Vec<u8>) {
        let mut state = self.lock_state();
        state.response_queue.entry(peer_id.to_owned()).or_default().push_back(bytes);
    }

    /// Acquire the inner lock, transparently recovering from any
    /// poisoning. Used by the trait methods so they never propagate a
    /// `PoisonError` outside the fake.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        match self.state.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(any(test, feature = "test-fake"))]
impl Default for FakePeripheral {
    fn default() -> Self {
        Self {
            state: std::sync::Mutex::new(FakeState::default()),
        }
    }
}

#[cfg(any(test, feature = "test-fake"))]
#[async_trait]
impl Peripheral for FakePeripheral {
    async fn add_peer(&self, peer_id: &str, bond_key: &BondKey) -> Result<(), PeripheralError> {
        let mut state = self.lock_state();
        if state.peer_keys.contains_key(peer_id) {
            return Err(PeripheralError::PeerAlreadyAdded {
                peer_id: peer_id.to_owned(),
            });
        }
        state.peers_in_order.push(peer_id.to_owned());
        state.peer_keys.insert(peer_id.to_owned(), *bond_key);
        Ok(())
    }

    async fn remove_peer(&self, peer_id: &str) -> Result<(), PeripheralError> {
        let mut state = self.lock_state();
        if state.peer_keys.remove(peer_id).is_none() {
            return Err(PeripheralError::UnknownPeer {
                peer_id: peer_id.to_owned(),
            });
        }
        state.peers_in_order.retain(|p| p != peer_id);
        Ok(())
    }

    async fn set_session_uuids(&self, uuids: std::collections::HashSet<Uuid>) -> Result<(), PeripheralError> {
        let mut state = self.lock_state();
        state.session_uuid_calls.push(uuids);
        Ok(())
    }

    async fn notify_challenge(&self, peer_id: &str, frame: &[u8]) -> Result<(), PeripheralError> {
        let mut state = self.lock_state();
        if !state.peer_keys.contains_key(peer_id) {
            return Err(PeripheralError::UnknownPeer {
                peer_id: peer_id.to_owned(),
            });
        }
        state.notify_calls.push((peer_id.to_owned(), frame.to_vec()));
        Ok(())
    }

    async fn wait_for_response(&self, peer_id: &str, deadline: Duration) -> Result<Vec<u8>, PeripheralError> {
        // Membership check before any wait so the typed UnknownPeer
        // branch fires deterministically when callers pass a
        // peer_id that was never `add_peer`'d.
        {
            let state = self.lock_state();
            if !state.peer_keys.contains_key(peer_id) {
                return Err(PeripheralError::UnknownPeer {
                    peer_id: peer_id.to_owned(),
                });
            }
        }
        let outcome = tokio::time::timeout(deadline, async {
            loop {
                {
                    let mut state = self.lock_state();
                    if let Some(queue) = state.response_queue.get_mut(peer_id)
                        && let Some(bytes) = queue.pop_front()
                    {
                        return bytes;
                    }
                }
                tokio::time::sleep(FAKE_RESPONSE_POLL_INTERVAL).await;
            }
        })
        .await;
        match outcome {
            Ok(bytes) => Ok(bytes),
            Err(_) => Err(PeripheralError::ResponseTimeout {
                peer_id: peer_id.to_owned(),
                deadline_ms: u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
            }),
        }
    }
}

/// Cadence at which [`FakePeripheral::wait_for_response`] polls its
/// injected-response FIFO inside the `tokio::time::timeout` wrapper.
/// A 10 ms cadence under `tokio::test(start_paused = true)` consumes
/// negligible virtual time; under wall-clock tests it makes the
/// success path return well within a millisecond of `inject_response`.
#[cfg(any(test, feature = "test-fake"))]
const FAKE_RESPONSE_POLL_INTERVAL: Duration = Duration::from_millis(10);

// ---------------------------------------------------------------------------
// Tests — unit tests for the shared builder + the trait's object safety.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // Journey: specs/journeys/JOURNEY-S-003-peripheral-library-api.md
    use super::*;

    /// Day-2 revocation must be understood on the channel a *bonded* session
    /// actually exposes. `build_and_register_peer` serves only the
    /// challenge/response pair, so the pair engine's v2-control branch never
    /// sees the frame the phone writes to the response characteristic
    /// (BUG-20260924: the frame arrived, fell into `response_tx`, and the bond
    /// survived).
    #[test]
    fn a_revoke_frame_is_recognised_on_the_writable_peer_characteristic() {
        let peer_id = [0x5au8; 16];
        let frame = Message {
            transaction: peer_id,
            operation: Operation::Revoke,
        }
        .encode();

        assert_eq!(revoke_peer_id(&frame), Some(peer_id));

        // Everything else the writable characteristic carries keeps its own
        // path: a challenge operation, the heartbeat, an RSSI sample, a
        // truncated frame.
        let capability = Message {
            transaction: peer_id,
            operation: Operation::Capability,
        }
        .encode();
        assert_eq!(revoke_peer_id(&capability), None);
        assert_eq!(revoke_peer_id(PRESENCE_HEARTBEAT), None);
        assert_eq!(revoke_peer_id(b"SYAUTH-RSSI-v1:-70"), None);
        assert_eq!(revoke_peer_id(&frame[..frame.len() - 1]), None);
    }

    #[test]
    fn rssi_telemetry_parses_signed_values_and_uses_ewma() {
        assert_eq!(parse_rssi(b"SYAUTH-RSSI-v1:-80"), Some(-80));
        assert_eq!(parse_rssi(b"SYAUTH-RSSI-v1:+1"), None);
        assert_eq!(parse_rssi(b"SYAUTH-RSSI-v1:-128"), None);
        assert_eq!(parse_rssi(b"SYAUTH-PRESENCE-v1"), None);
        assert_eq!(record_rssi(-80, None), -80.0);
        assert_eq!(record_rssi(-60, Some(-80.0)), -75.0);
    }

    /// `PersistentPeripheral::build_advertisement` carries the
    /// daemon-side defaults (local name, discoverable, the requested
    /// UUID set). Pure-function so this test runs without an adapter.
    #[test]
    fn build_advertisement_carries_local_name_and_uuids() {
        let uuid = Uuid::from_u128(0x5a4e_8e3c_1c4c_4a17_9c81_d518_a55a_3001);
        let uuids: std::collections::HashSet<Uuid> = [uuid].into_iter().collect();
        let adv = PersistentPeripheral::build_advertisement(uuids);
        assert_eq!(adv.local_name.as_deref(), Some(ADVERTISE_LOCAL_NAME));
        assert_eq!(adv.discoverable, Some(ADVERTISE_DISCOVERABLE));
        let expected: std::collections::BTreeSet<Uuid> = [uuid].into_iter().collect();
        assert_eq!(adv.service_uuids, expected);
    }

    /// Trait is object-safe and the bounds (`Send + Sync`) let an
    /// `Arc<dyn Peripheral>` compile. This is the daemon's actual
    /// usage pattern; pinning it here prevents an accidental
    /// non-object-safe extension in future steps.
    #[test]
    fn trait_is_object_safe_and_send_sync() {
        let fake = FakePeripheral::new();
        let dyn_ref: Arc<dyn Peripheral> = fake;
        // Force `Send + Sync` checks at compile time.
        fn assert_send_sync<T: Send + Sync + ?Sized>(_: &T) {}
        assert_send_sync(&*dyn_ref);
    }

    /// `From<TransportError>` mapping: `AdapterMissing` keeps its
    /// structural variant; anything else collapses to `Backend`.
    #[test]
    fn transport_error_maps_to_peripheral_error() {
        let mapped = PeripheralError::from(TransportError::AdapterMissing { name: "hci99".to_owned() });
        match mapped {
            PeripheralError::AdapterMissing { name } => assert_eq!(name, "hci99"),
            other => panic!("expected AdapterMissing, got {other:?}"),
        }
        let mapped = PeripheralError::from(TransportError::Closed);
        match mapped {
            PeripheralError::Backend { reason } => assert!(reason.contains("closed")),
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    /// FakePeripheral records both add and notify call sequences.
    #[tokio::test]
    async fn fake_peripheral_records_notify_calls() {
        let fake = FakePeripheral::new();
        fake.add_peer("a", &[0xAA; BOND_KEY_BYTES]).await.expect("add a");
        fake.notify_challenge("a", &[0x01, 0x02, 0x03]).await.expect("notify a");
        let calls = fake.notify_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "a");
        assert_eq!(calls[0].1, vec![0x01, 0x02, 0x03]);
    }

    /// FakePeripheral.inject_response → wait_for_response returns the
    /// injected bytes.
    #[tokio::test]
    async fn fake_peripheral_wait_for_response_returns_injected_bytes() {
        let fake = FakePeripheral::new();
        fake.add_peer("a", &[0xAA; BOND_KEY_BYTES]).await.expect("add a");
        fake.inject_response("a", vec![0x10, 0x20, 0x30]);
        let bytes = fake
            .wait_for_response("a", Duration::from_millis(50))
            .await
            .expect("response present");
        assert_eq!(bytes, vec![0x10, 0x20, 0x30]);
    }

    /// FakePeripheral.wait_for_response on an empty queue returns
    /// `ResponseTimeout` after the deadline. Wall-clock budget is
    /// the deadline value (50 ms) since this unit test does not
    /// pull tokio's `test-util` feature.
    #[tokio::test]
    async fn fake_peripheral_wait_for_response_times_out_when_empty() {
        let fake = FakePeripheral::new();
        fake.add_peer("a", &[0xAA; BOND_KEY_BYTES]).await.expect("add a");
        let deadline = Duration::from_millis(50);
        let result = fake.wait_for_response("a", deadline).await;
        match result {
            Err(PeripheralError::ResponseTimeout { peer_id, deadline_ms }) => {
                assert_eq!(peer_id, "a");
                assert_eq!(deadline_ms, 50);
            }
            other => panic!("expected ResponseTimeout, got {other:?}"),
        }
    }

    /// Closing the owning PAM request cancels its response wait immediately.
    #[tokio::test]
    async fn cancellable_response_wait_returns_cancelled() {
        let fake = FakePeripheral::new();
        fake.add_peer("a", &[0xAA; BOND_KEY_BYTES]).await.expect("add a");
        let (cancel_tx, mut cancel) = watch::channel(false);
        let task = tokio::spawn(async move { fake.wait_for_response_cancellable("a", Duration::from_secs(30), &mut cancel).await });
        cancel_tx.send(true).expect("cancel");
        assert!(matches!(task.await.expect("join"), Err(PeripheralError::Cancelled)));
    }

    /// FakePeripheral.wait_for_response on an unknown peer returns
    /// `UnknownPeer` deterministically (no wait).
    #[tokio::test]
    async fn fake_peripheral_wait_for_response_rejects_unknown_peer() {
        let fake = FakePeripheral::new();
        let result = fake.wait_for_response("ghost", Duration::from_millis(50)).await;
        match result {
            Err(PeripheralError::UnknownPeer { peer_id }) => assert_eq!(peer_id, "ghost"),
            other => panic!("expected UnknownPeer, got {other:?}"),
        }
    }

    /// FakePeripheral refuses to add the same peer twice.
    #[tokio::test]
    async fn fake_peripheral_rejects_duplicate_add() {
        let fake = FakePeripheral::new();
        fake.add_peer("a", &[0xAA; BOND_KEY_BYTES]).await.expect("first");
        let err = fake.add_peer("a", &[0xBB; BOND_KEY_BYTES]).await.expect_err("dup");
        match err {
            PeripheralError::PeerAlreadyAdded { peer_id } => assert_eq!(peer_id, "a"),
            other => panic!("expected PeerAlreadyAdded, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Pair-agent acquisition: a BlueZ `Busy` on a mode transition must
    // never skip the agent registration, and the effective adapter state
    // must be verified (or the acquisition must fail cleanly).
    // -----------------------------------------------------------------

    use std::{
        collections::VecDeque,
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    const MODE_TEST_RETRIES: usize = 3;
    const MODE_TEST_DELAY: std::time::Duration = std::time::Duration::from_millis(1);
    const FAKE_AGENT: u8 = 7;

    /// One scripted `set_pairable` call: the error BlueZ reports and whether
    /// the mode actually changed anyway (BlueZ can apply a transition and
    /// still answer `Busy`).
    #[derive(Clone)]
    struct ModeStep {
        error: Option<&'static str>,
        applied: bool,
    }

    /// Adapter double. It models `Discoverable` on purpose: the acquisition and
    /// release paths must never touch it, and `requested` records every flag
    /// this double was ever asked to transition.
    #[derive(Default)]
    struct FakeAdapterMode {
        discoverable: AtomicBool,
        pairable: AtomicBool,
        script: StdMutex<VecDeque<ModeStep>>,
        set_calls: AtomicUsize,
        requested: StdMutex<Vec<(&'static str, bool)>>,
    }

    impl FakeAdapterMode {
        fn new(discoverable: bool, pairable: bool) -> Self {
            Self {
                discoverable: AtomicBool::new(discoverable),
                pairable: AtomicBool::new(pairable),
                ..Default::default()
            }
        }

        fn script(&self, steps: Vec<ModeStep>) {
            *self.script.lock().expect("script") = VecDeque::from(steps);
        }

        fn requested(&self) -> Vec<(&'static str, bool)> {
            self.requested.lock().expect("requested").clone()
        }
    }

    #[async_trait]
    impl PairableControl for FakeAdapterMode {
        async fn pairable(&self) -> Result<bool, String> {
            Ok(self.pairable.load(Ordering::SeqCst))
        }

        async fn set_pairable(&self, value: bool) -> Result<(), String> {
            self.set_calls.fetch_add(1, Ordering::SeqCst);
            self.requested.lock().expect("requested").push(("pairable", value));
            let step = self.script.lock().expect("script").pop_front().unwrap_or(ModeStep {
                error: None,
                applied: true,
            });
            if step.applied {
                self.pairable.store(value, Ordering::SeqCst);
            }
            match step.error {
                Some(err) => Err(err.to_owned()),
                None => Ok(()),
            }
        }
    }

    /// The Bluetooth name is an operator-facing label, never an identity: when
    /// BlueZ cannot resolve it the caller must fall back to the transport label
    /// instead of inventing one. Deterministic without a radio (a missing
    /// adapter resolves to `None`, as does an unavailable BlueZ).
    #[tokio::test]
    async fn peer_display_name_is_none_without_a_matching_adapter() {
        assert!(peer_display_name("hci-does-not-exist-9", "00:00:00:00:00:00").await.is_none());
    }

    #[tokio::test]
    async fn acquire_never_touches_discoverable_and_verifies_pairable() {
        let adapter = FakeAdapterMode::new(false, false);
        let mut session: PairAgentSession<u8> = PairAgentSession::default();

        acquire_pair_agent_with(&adapter, &mut session, || async { Ok(FAKE_AGENT) })
            .await
            .expect("acquire");

        assert!(
            !adapter.discoverable.load(Ordering::SeqCst),
            "the global Discoverable flag is not DeskUnlock's to change"
        );
        assert!(adapter.pairable.load(Ordering::SeqCst), "pairable is verified for the session");
        assert_eq!(adapter.requested(), vec![("pairable", true)], "only Pairable is ever transitioned");
        assert!(session.is_held(), "the agent is registered once the mode is verified");

        release_pair_agent_with(&adapter, &mut session).await;

        assert!(!session.is_held(), "release drops the agent");
        assert!(
            !adapter.discoverable.load(Ordering::SeqCst),
            "release must not touch Discoverable either"
        );
        assert!(
            !adapter.pairable.load(Ordering::SeqCst),
            "Pairable is restored to the desktop's own value"
        );
        assert_eq!(adapter.requested(), vec![("pairable", true), ("pairable", false)]);
    }

    #[tokio::test]
    async fn acquire_and_release_transition_nothing_when_the_adapter_is_already_ready() {
        let adapter = FakeAdapterMode::new(true, true);
        let mut session: PairAgentSession<u8> = PairAgentSession::default();

        acquire_pair_agent_with(&adapter, &mut session, || async { Ok(FAKE_AGENT) })
            .await
            .expect("acquire");
        assert!(session.is_held(), "the agent lifecycle is independent of the flags");
        release_pair_agent_with(&adapter, &mut session).await;

        assert!(adapter.requested().is_empty(), "no transition when the adapter is already ready");
        assert!(!session.is_held());
        assert!(adapter.pairable.load(Ordering::SeqCst));
        assert!(
            adapter.discoverable.load(Ordering::SeqCst),
            "the user's own Discoverable state is preserved"
        );
    }

    #[tokio::test]
    async fn busy_with_the_desired_state_already_effective_is_a_no_op() {
        let adapter = FakeAdapterMode::new(false, true);
        let changed = ensure_pairable_with(&adapter, MODE_TEST_RETRIES, MODE_TEST_DELAY)
            .await
            .expect("verified");
        assert!(!changed);
        assert_eq!(adapter.set_calls.load(Ordering::SeqCst), 0, "no transition when already effective");
    }

    #[tokio::test]
    async fn transient_busy_is_retried_and_verified() {
        let adapter = FakeAdapterMode::new(false, false);
        adapter.script(vec![ModeStep {
            error: Some("Busy"),
            applied: true,
        }]);
        let changed = ensure_pairable_with(&adapter, MODE_TEST_RETRIES, MODE_TEST_DELAY)
            .await
            .expect("verified");
        assert!(changed);
        assert!(adapter.pairable.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn persistent_busy_without_the_desired_state_fails_cleanly() {
        let adapter = FakeAdapterMode::new(false, false);
        adapter.script(vec![
            ModeStep {
                error: Some("Busy"),
                applied: false,
            },
            ModeStep {
                error: Some("Busy"),
                applied: false,
            },
            ModeStep {
                error: Some("Busy"),
                applied: false,
            },
        ]);
        assert!(ensure_pairable_with(&adapter, MODE_TEST_RETRIES, MODE_TEST_DELAY).await.is_err());
        assert!(!adapter.pairable.load(Ordering::SeqCst), "no partially acquired adapter state");
    }

    #[tokio::test]
    async fn a_failed_acquisition_holds_no_agent_and_leaves_no_adapter_state() {
        let adapter = FakeAdapterMode::new(false, false);
        // Script the full production budget: the acquisition path must fail
        // cleanly instead of falling back to the double's default success.
        adapter.script(vec![
            ModeStep {
                error: Some("Busy"),
                applied: false,
            };
            ADAPTER_MODE_RETRIES
        ]);
        let mut session: PairAgentSession<u8> = PairAgentSession::default();

        assert!(
            acquire_pair_agent_with(&adapter, &mut session, || async { Ok(FAKE_AGENT) })
                .await
                .is_err()
        );

        assert!(!session.is_held(), "no agent is registered when the mode cannot be verified");
        assert!(!adapter.pairable.load(Ordering::SeqCst));
        assert!(!adapter.discoverable.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_failed_agent_registration_rolls_the_pairable_flag_back() {
        let adapter = FakeAdapterMode::new(false, false);
        let mut session: PairAgentSession<u8> = PairAgentSession::default();

        assert!(
            acquire_pair_agent_with(&adapter, &mut session, || async { Err("register_agent: Busy".to_owned()) })
                .await
                .is_err()
        );

        assert!(!session.is_held(), "a failed registration must not look like a held agent");
        assert!(
            !adapter.pairable.load(Ordering::SeqCst),
            "the flag this session changed is rolled back"
        );
        assert_eq!(adapter.requested(), vec![("pairable", true), ("pairable", false)]);
    }

    #[tokio::test]
    async fn a_non_busy_error_then_success_is_still_verified() {
        let adapter = FakeAdapterMode::new(false, false);
        adapter.script(vec![
            ModeStep {
                error: Some("Failed"),
                applied: false,
            },
            ModeStep {
                error: None,
                applied: true,
            },
        ]);
        let changed = ensure_pairable_with(&adapter, MODE_TEST_RETRIES, MODE_TEST_DELAY)
            .await
            .expect("verified");
        assert!(changed);
        assert_eq!(adapter.set_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn consecutive_sessions_reacquire_and_release_without_a_permanent_agent() {
        let adapter = FakeAdapterMode::new(false, false);
        let mut session: PairAgentSession<u8> = PairAgentSession::default();

        for agent in [FAKE_AGENT, FAKE_AGENT + 1] {
            acquire_pair_agent_with(&adapter, &mut session, || async { Ok(agent) })
                .await
                .expect("acquire");
            assert!(session.is_held());
            release_pair_agent_with(&adapter, &mut session).await;
            assert!(!session.is_held(), "no permanent default agent survives the session");
        }
        assert_eq!(
            adapter.requested(),
            vec![("pairable", true), ("pairable", false), ("pairable", true), ("pairable", false)]
        );
        assert!(!adapter.discoverable.load(Ordering::SeqCst));
    }

    #[test]
    fn pair_agent_session_supports_reacquire_and_never_leaks() {
        let restore = || PairAgentRestore::from_pairable(true, false);
        let mut session: PairAgentSession<u8> = PairAgentSession::default();
        assert!(!session.is_held());

        session.begin(7, restore());
        assert!(session.is_held());
        let released = session.end().expect("first release restores");
        assert_eq!(released.previous_pairable, Some(false));
        assert!(!session.is_held());
        assert!(session.end().is_none(), "a second release is a no-op, never a double restore");

        // Acquire again on the same session object (no permanent default agent).
        session.begin(9, restore());
        assert!(session.is_held());
        assert!(session.end().is_some());

        let unchanged = PairAgentRestore::from_pairable(false, true);
        assert_eq!(unchanged.previous_pairable, None, "an unchanged flag is never restored");
    }
}
