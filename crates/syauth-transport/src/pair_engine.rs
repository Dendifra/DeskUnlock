//! Daemon-owned DeskUnlock pairing engine.
//!
//! One owner: `syauth-presenced` drives the versioned V2 transaction over the
//! authenticated GATT pair service. The GUI is a presentation and
//! confirmation client only.
//!
//! Invariants enforced here:
//!
//! - a Bluetooth bond is transport only; it never completes DeskUnlock;
//! - the phone-pubkey write stages a session but persists no trust;
//! - the DeskUnlock OOB confirmation is a real application decision,
//!   separate from the BlueZ LESC confirmation;
//! - trust is persisted exactly once, at the V2 commit boundary, and only
//!   after the local commit decision is durable;
//! - a link that disappears after the commit decision is `Uncertain`, never
//!   a silent success or a silent rollback.

use std::{sync::Arc, time::Duration};

use bluer::{
    Uuid,
    gatt::local::{
        Characteristic, CharacteristicControlEvent, CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, ReqError, Service,
        characteristic_control,
    },
};
use futures::{FutureExt, Stream, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use syauth_core::pair_transaction::{
    LocalEvent, MESSAGE_LEN, Message, Operation, Role, STATUS_MESSAGE_LEN, StatusMessage, StatusState, Transaction,
};
use tokio::{io::AsyncReadExt, sync::mpsc, time::Instant};

use crate::{
    bluez::{
        PAIR_PUBKEY_LEN, SYAUTH_PAIR_HOST_NAME_V1_CHAR_UUID, SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID, SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID,
        SYAUTH_PAIR_SERVICE_UUID, SYAUTH_PAIR_V2_CONTROL_CHAR_UUID, SYAUTH_PAIR_V2_STATUS_CHAR_UUID,
    },
    pairing::PairingBroker,
};

/// Bounded wait for one V2 control message from the authenticated peer.
pub const PAIR_MESSAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Budget for the **peer's CAPABILITY** on the first round of an attempt.
///
/// The phone publishes its CAPABILITY only after the operator has read and
/// accepted the DeskUnlock OOB words, so this one wait is paced by a person,
/// not by the radio. Every later step is machine-paced and keeps
/// [`PAIR_MESSAGE_TIMEOUT`]; the commit sequence stays tight.
pub const PAIR_CAPABILITY_TIMEOUT: Duration = Duration::from_secs(180);
/// Effective wait used by the engine. Unit tests use a short bound so the
/// negative paths terminate quickly.
#[cfg(not(test))]
const PAIR_MESSAGE_TIMEOUT_INTERNAL: Duration = PAIR_MESSAGE_TIMEOUT;
/// Human-paced first round; shortened under `cfg(test)` so suites stay fast.
#[cfg(not(test))]
const PAIR_CAPABILITY_TIMEOUT_INTERNAL: Duration = PAIR_CAPABILITY_TIMEOUT;
#[cfg(test)]
const PAIR_MESSAGE_TIMEOUT_INTERNAL: Duration = Duration::from_millis(400);
#[cfg(test)]
const PAIR_CAPABILITY_TIMEOUT_INTERNAL: Duration = Duration::from_millis(400);
/// Poll interval while draining the control inbox.
const PAIR_MESSAGE_POLL: Duration = Duration::from_millis(50);

/// How long the engine waits for the peer to read the message currently in the
/// status register before publishing the next one. The peer polls that register
/// every [`PAIR_MESSAGE_POLL`], so a healthy session clears it almost at once.
pub const PAIR_STATUS_READ_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(test))]
const PAIR_STATUS_READ_TIMEOUT_INTERNAL: Duration = PAIR_STATUS_READ_TIMEOUT;
#[cfg(test)]
const PAIR_STATUS_READ_TIMEOUT_INTERNAL: Duration = Duration::from_millis(100);

/// Window to collect one ATT write payload from a bluer `Io` reader.
///
/// `read_to_end` never returns on these readers: BlueZ acknowledges the write
/// as soon as the payload is buffered and keeps the stream open for the next
/// one, so waiting for end-of-stream deadlocks the handler (the peer sees an
/// ack while the payload is silently lost).
const CONTROL_READ_WINDOW: Duration = Duration::from_millis(250);

/// Split one incoming v2-control payload into complete V2 frames.
///
/// The bluer `Io` reader is a byte **stream**: writes that arrive before the
/// app reads them concatenate, so a single read can carry several frames (the
/// phone publishes its `CONFIRM` and `PREPARED` back to back and BlueZ
/// delivers them as one 36-byte payload). Handing that payload to
/// [`PairServiceState::accept_control`] loses every message in it, because the
/// strict decoders require one frame per slice.
///
/// Foreign or truncated bytes are resynchronised one byte at a time, so a
/// desynced stream can never stall the channel.
///
/// Returns how many frames were accepted.
fn frame_control_payload(payload: &[u8], mut accept: impl FnMut(Vec<u8>)) -> usize {
    let mut offset = 0;
    let mut framed = 0;
    while offset < payload.len() {
        let rest = &payload[offset..];
        if rest.len() >= STATUS_MESSAGE_LEN && StatusMessage::decode(&rest[..STATUS_MESSAGE_LEN]).is_ok() {
            accept(rest[..STATUS_MESSAGE_LEN].to_vec());
            framed += 1;
            offset += STATUS_MESSAGE_LEN;
            continue;
        }
        if rest.len() >= MESSAGE_LEN && Message::decode(&rest[..MESSAGE_LEN]).is_ok() {
            accept(rest[..MESSAGE_LEN].to_vec());
            framed += 1;
            offset += MESSAGE_LEN;
            continue;
        }
        offset += 1;
    }
    framed
}
async fn read_bounded<R>(reader: &mut R, max: usize, window: Duration) -> Option<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut out: Vec<u8> = Vec::new();
    loop {
        let mut chunk = [0u8; 64];
        let room = (max - out.len()).min(chunk.len());
        match tokio::time::timeout(window, reader.read(&mut chunk[..room])).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(n)) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() >= max {
                    break;
                }
            }
        }
    }
    (!out.is_empty()).then_some(out)
}
/// Maximum accepted V2 control frame size (one message or status frame).
/// Maximum accepted V2 control payload in one read. Sized so a burst of
/// coalesced messages is collected whole rather than truncated mid-frame.
const PAIR_CONTROL_MAX_BYTES: usize = 256;

/// Shared state of the always-present DeskUnlock pair service. The GATT
/// characteristics read and write this state; the pair engine drives the V2
/// transaction over it.
pub struct PairServiceState {
    host_pubkey: [u8; PAIR_PUBKEY_LEN],
    host_name: Vec<u8>,
    status: std::sync::Mutex<Vec<u8>>,
    /// True from a publish until the peer's next read of the status register.
    ///
    /// The peer polls a **last-value** register, so publishing twice before it
    /// reads once silently destroys the first message. The engine paces itself
    /// on this flag: a message is published only after the peer consumed the
    /// previous one (`publish_paced`).
    unread: AtomicBool,
    inbox: std::sync::Mutex<Vec<Vec<u8>>>,
    session: std::sync::Mutex<Option<([u8; 16], StatusState)>>,
}

impl PairServiceState {
    /// Build the pair-service state for `host_pubkey` and the encoded
    /// host-name payload.
    #[must_use]
    pub fn new(host_pubkey: [u8; PAIR_PUBKEY_LEN], host_name: Vec<u8>) -> Self {
        Self {
            host_pubkey,
            host_name,
            status: std::sync::Mutex::new(Vec::new()),
            unread: AtomicBool::new(false),
            inbox: std::sync::Mutex::new(Vec::new()),
            session: std::sync::Mutex::new(None),
        }
    }

    fn publish(&self, message: Message) {
        tracing::info!(target: "syauth_transport", operation = ?message.operation, "pair engine published operation");
        if let Ok(mut status) = self.status.lock() {
            *status = message.encode().to_vec();
        }
        self.unread.store(true, Ordering::SeqCst);
    }

    /// Publish one protocol message, first waiting for the peer to read the
    /// message already in the register (bounded: a peer that stopped reading
    /// must not wedge the session forever).
    async fn publish_paced(&self, message: Message) {
        let deadline = Instant::now() + PAIR_STATUS_READ_TIMEOUT_INTERNAL;
        while self.unread.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                tracing::warn!(
                    target: "syauth_transport",
                    "peer has not read the previous message; publishing anyway",
                );
                break;
            }
            tokio::time::sleep(PAIR_MESSAGE_POLL).await;
        }
        self.publish(message);
    }

    fn publish_status(&self, response: StatusMessage) {
        if let Ok(mut status) = self.status.lock() {
            *status = response.encode().to_vec();
        }
        self.unread.store(true, Ordering::SeqCst);
    }

    fn status_snapshot(&self, offset: usize) -> Vec<u8> {
        // A read consumes the unread mark: the peer now holds this message, so
        // the engine may publish the next one.
        self.unread.store(false, Ordering::SeqCst);
        self.status
            .lock()
            .map(|status| status.get(offset..).unwrap_or_default().to_vec())
            .unwrap_or_default()
    }

    /// Reset the per-attempt state so a retry can never inherit a previous
    /// session's published message, control inbox or transaction snapshot.
    fn begin_session(&self) {
        if let Ok(mut status) = self.status.lock() {
            status.clear();
        }
        self.unread.store(false, Ordering::SeqCst);
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.clear();
        }
        if let Ok(mut session) = self.session.lock() {
            *session = None;
        }
    }

    fn remember_session(&self, id: [u8; 16], status: StatusState) {
        if let Ok(mut session) = self.session.lock() {
            *session = Some((id, status));
        }
    }

    fn current_session(&self) -> Option<([u8; 16], StatusState)> {
        self.session.lock().ok().and_then(|session| *session)
    }

    /// Accept one authenticated V2 control write. Status queries are answered
    /// from the current session snapshot and never enter the commit inbox.
    /// Malformed frames are dropped and can never advance the transaction.
    fn accept_control(&self, bytes: Vec<u8>) {
        if let Ok(query) = StatusMessage::decode(&bytes) {
            if query.state.is_none() {
                if let Some((id, status)) = self.current_session() {
                    if id == query.transaction {
                        self.publish_status(StatusMessage::response(id, query.nonce, status));
                    }
                }
            }
            return;
        }
        if let Ok(mut inbox) = self.inbox.lock() {
            inbox.push(bytes);
        }
    }

    fn take_message(&self, id: [u8; 16]) -> Option<Operation> {
        let mut inbox = self.inbox.lock().ok()?;
        let index = inbox
            .iter()
            .position(|bytes| Message::decode(bytes).map(|message| message.transaction == id).unwrap_or(false))?;
        let bytes = inbox.remove(index);
        Message::decode(&bytes).ok().map(|message| message.operation)
    }
}

/// Durable action the pair engine asks the daemon to perform at the V2
/// commit boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairCommitPhase {
    /// Durably stage the inactive bond + key + journal (`state=pre_commit`),
    /// before COMMIT. Writes no active trust.
    Stage,
    /// Record the durable local commit decision in the journal
    /// (`state=commit_pending`). Runs before the COMMITTED exchange, so
    /// recovery can tell a definitely aborted attempt from a possibly
    /// committed one.
    MarkCommit,
    /// Promote the staged bond/key into the active trust. Runs only after
    /// the bilateral V2 `COMMITTED`.
    Promote,
    /// Drop the staging after a definite pre-commit abort. Never touches
    /// active trust.
    Discard,
    /// Day-2 revocation: the phone says the association is over.
    ///
    /// Carries the revoking peer's id in the request's `transaction` field (16
    /// raw bytes = a 32-character `peer_id`), because a revocation is not part
    /// of any pairing transaction. The daemon marks that bond `Revoked` so it
    /// stops being served — the desktop-side half of "dissociate on one side,
    /// dissociated on both".
    Revoke,
}

/// Daemon-side request for one commit-boundary action.
///
/// The engine never writes trust itself: the daemon owns the on-disk staging
/// format, and [`PairCommitPhase::Promote`] runs only after the bilateral V2
/// `COMMITTED`.
pub struct PairCommitRequest {
    /// Which durable action to perform.
    pub phase: PairCommitPhase,
    /// Transaction identifier bound to this attempt.
    pub transaction: [u8; 16],
    /// Phone Ed25519 public key captured from the authenticated GATT write.
    pub phone_pubkey: [u8; PAIR_PUBKEY_LEN],
    /// Desktop host public key used to derive the same bond_key.
    pub host_pubkey: [u8; PAIR_PUBKEY_LEN],
    /// Transport display label; never the trust identity.
    pub peer: String,
    /// Durable action outcome.
    pub reply: tokio::sync::oneshot::Sender<bool>,
}

/// Failure surface of the daemon-owned pair engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairEngineError {
    /// The operator rejected the DeskUnlock OOB confirmation.
    Rejected,
    /// The peer aborted, or the exchange timed out before the commit.
    Aborted,
    /// The link disappeared after the commit decision; outcome is unknown.
    Uncertain,
    /// The commit boundary could not durably persist the bond.
    Persistence,
}

/// Build the always-present pair-mode service with the full DeskUnlock V2
/// contract: authenticated `host-name`, `host-pubkey`, `v2-status`,
/// `v2-control` and `phone-pubkey`, using the identifiers already defined by
/// the protocol.
pub fn build_pair_service(
    state: Arc<PairServiceState>,
) -> (
    Service,
    impl Stream<Item = CharacteristicControlEvent> + Unpin + Send + 'static,
    impl Stream<Item = CharacteristicControlEvent> + Unpin + Send + 'static,
) {
    let (phone_control, phone_handle) = characteristic_control();
    let (v2_control, v2_handle) = characteristic_control();

    let host_name = state.host_name.clone();
    let host_pubkey = state.host_pubkey;
    let status_state = Arc::clone(&state);

    let service = Service {
        uuid: SYAUTH_PAIR_SERVICE_UUID,
        primary: true,
        characteristics: vec![
            Characteristic {
                uuid: SYAUTH_PAIR_HOST_NAME_V1_CHAR_UUID,
                read: Some(CharacteristicRead {
                    read: true,
                    encrypt_authenticated_read: true,
                    fun: Box::new(move |request| {
                        let result = host_name
                            .get(usize::from(request.offset)..)
                            .map(<[u8]>::to_vec)
                            .ok_or(ReqError::InvalidOffset);
                        async move { result }.boxed()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Characteristic {
                uuid: SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID,
                read: Some(CharacteristicRead {
                    read: true,
                    encrypt_authenticated_read: true,
                    fun: Box::new(move |_| {
                        let bytes = host_pubkey.to_vec();
                        async move { Ok(bytes) }.boxed()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Characteristic {
                uuid: SYAUTH_PAIR_V2_STATUS_CHAR_UUID,
                read: Some(CharacteristicRead {
                    read: true,
                    encrypt_authenticated_read: true,
                    fun: Box::new(move |request| {
                        let bytes = status_state.status_snapshot(usize::from(request.offset));
                        async move { Ok(bytes) }.boxed()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Characteristic {
                uuid: SYAUTH_PAIR_V2_CONTROL_CHAR_UUID,
                write: Some(CharacteristicWrite {
                    write: true,
                    write_without_response: true,
                    encrypt_authenticated_write: true,
                    method: CharacteristicWriteMethod::Io,
                    ..Default::default()
                }),
                control_handle: v2_handle,
                ..Default::default()
            },
            Characteristic {
                uuid: SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID,
                write: Some(CharacteristicWrite {
                    write: true,
                    write_without_response: true,
                    encrypt_authenticated_write: true,
                    method: CharacteristicWriteMethod::Io,
                    ..Default::default()
                }),
                control_handle: phone_handle,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    (service, phone_control, v2_control)
}

/// Resolve the peer display label for a BlueZ device address. The address is
/// a transport label only and never becomes a trust identity.
#[must_use]
pub fn peer_label(address: &str) -> String {
    address.to_ascii_uppercase()
}

/// Run the daemon-owned pair engine for the **lifetime of the pair GATT
/// service**.
///
/// One DeskUnlock session runs at a time. Each `phone-pubkey` write opens a
/// fresh, isolated attempt with its own transaction; after success, reject,
/// abort, cancel, timeout or any pre-trust error the engine is immediately
/// ready for the next write — no `rebuild_application`, no daemon restart,
/// and no state from the previous transaction.
///
/// A bare pubkey write stages the session but persists **no** trust: the bond
/// is only persisted after the V2 `COMMIT_ACK` (commit boundary) and the
/// GUI's application-level OOB decision.
pub async fn run_pair_session(
    state: Arc<PairServiceState>,
    broker: PairingBroker,
    commit_tx: mpsc::Sender<PairCommitRequest>,
    mut phone_control: impl Stream<Item = CharacteristicControlEvent> + Unpin + Send + 'static,
    v2_control: impl Stream<Item = CharacteristicControlEvent> + Unpin + Send + 'static,
) {
    // The v2-control reader lives for the whole GATT service, not one
    // attempt: a retry must always find a live consumer on the control
    // stream, otherwise BlueZ's AcquireWrite fails and the peer's exchange
    // dies before it reaches the engine.
    let control_state = Arc::clone(&state);
    // The reader task needs its own handle to the commit-boundary channel: a
    // day-2 `Revoke` write from the phone travels the same path as the commit
    // phases, with the peer id in the transaction field.
    let revoke_tx = commit_tx.clone();
    let reader = tokio::spawn(async move {
        tracing::info!(target: "syauth_transport", "v2-control reader started");
        let mut v2_control = Box::pin(v2_control);
        while let Some(event) = v2_control.next().await {
            let CharacteristicControlEvent::Write(request) = event else {
                continue;
            };
            let Ok(mut reader) = request.accept() else {
                tracing::warn!(target: "syauth_transport", "v2-control write refused before read");
                continue;
            };
            let Some(bytes) = read_bounded(&mut reader, PAIR_CONTROL_MAX_BYTES, CONTROL_READ_WINDOW).await else {
                tracing::warn!(target: "syauth_transport", "v2-control write dropped: no payload");
                continue;
            };
            let framed = frame_control_payload(&bytes, |frame| match Message::decode(&frame) {
                Ok(message) if message.operation == Operation::Revoke => {
                    // Not part of any pairing transaction: the phone is telling us
                    // it dropped this bond. Hand it to the daemon; the peer id
                    // travels in the transaction field (16 raw bytes = 32 hex).
                    let (reply, _answer) = tokio::sync::oneshot::channel();
                    let request = PairCommitRequest {
                        phase: PairCommitPhase::Revoke,
                        transaction: message.transaction,
                        phone_pubkey: [0; PAIR_PUBKEY_LEN],
                        host_pubkey: [0; PAIR_PUBKEY_LEN],
                        peer: String::new(),
                        reply,
                    };
                    if let Err(err) = revoke_tx.try_send(request) {
                        tracing::warn!(target: "syauth_transport", error = %err, "day-2 revoke request dropped");
                    }
                }
                _ => control_state.accept_control(frame),
            });
            if framed == 0 {
                tracing::warn!(target: "syauth_transport", bytes = bytes.len(), "v2-control payload carried no complete frame");
            } else {
                tracing::info!(target: "syauth_transport", bytes = bytes.len(), frames = framed, "v2-control write received");
            }
        }
        tracing::info!(target: "syauth_transport", "v2-control reader ended");
    });

    while let Some((phone_pubkey, address)) = wait_for_phone_pubkey(&mut phone_control).await {
        run_pair_attempt(&state, &broker, &commit_tx, phone_pubkey, &address).await;
    }
    reader.abort();
}

/// Run exactly one attempt on the live engine and log its outcome.
///
/// Starts by resetting the per-attempt state, so the same
/// [`PairServiceState`] can serve attempt after attempt.
async fn run_pair_attempt(
    state: &Arc<PairServiceState>,
    broker: &PairingBroker,
    commit_tx: &mpsc::Sender<PairCommitRequest>,
    phone_pubkey: [u8; PAIR_PUBKEY_LEN],
    address: &str,
) {
    state.begin_session();
    let peer = peer_label(address);
    match drive_pair_transaction(state, broker, commit_tx, phone_pubkey, &peer).await {
        Ok(()) => {
            tracing::info!(target: "syauth_transport", peer = %peer, "pair session reached TrustEstablished");
        }
        Err(PairEngineError::Rejected) => {
            tracing::info!(target: "syauth_transport", peer = %peer, "pair session rejected by operator");
        }
        Err(PairEngineError::Aborted) => {
            tracing::info!(target: "syauth_transport", peer = %peer, "pair session aborted before commit");
        }
        Err(PairEngineError::Uncertain) => {
            tracing::warn!(target: "syauth_transport", peer = %peer, "pair session commit outcome uncertain; recovery required");
        }
        Err(PairEngineError::Persistence) => {
            tracing::warn!(target: "syauth_transport", peer = %peer, "pair session commit persistence failed");
        }
    }
    tracing::info!(target: "syauth_transport", peer = %peer, "pair engine ready for the next session");
}

async fn wait_for_phone_pubkey(
    control: &mut (impl Stream<Item = CharacteristicControlEvent> + Unpin),
) -> Option<([u8; PAIR_PUBKEY_LEN], String)> {
    loop {
        match control.next().await? {
            CharacteristicControlEvent::Write(request) => {
                let address = request.device_address().to_string();
                let Ok(mut reader) = request.accept() else {
                    continue;
                };
                let Some(bytes) = read_bounded(&mut reader, PAIR_PUBKEY_LEN, CONTROL_READ_WINDOW).await else {
                    tracing::warn!(target: "syauth_transport", %address, "phone-pubkey write ignored: no payload");
                    continue;
                };
                if bytes.len() != PAIR_PUBKEY_LEN {
                    tracing::warn!(
                        target: "syauth_transport",
                        %address,
                        bytes = bytes.len(),
                        "phone-pubkey write ignored: unexpected length",
                    );
                    continue;
                }
                let mut buf = [0u8; PAIR_PUBKEY_LEN];
                buf.copy_from_slice(&bytes);
                tracing::info!(target: "syauth_transport", %address, "phone-pubkey write accepted");
                return Some((buf, address));
            }
            CharacteristicControlEvent::Notify(_) => continue,
        }
    }
}

/// Per-attempt context shared by the commit-boundary helpers.
struct PairCommitContext<'a> {
    commit_tx: &'a mpsc::Sender<PairCommitRequest>,
    transaction_id: [u8; 16],
    host_pubkey: [u8; PAIR_PUBKEY_LEN],
    phone_pubkey: [u8; PAIR_PUBKEY_LEN],
    peer: &'a str,
}

impl PairCommitContext<'_> {
    /// Send one commit-boundary action to the daemon and wait for its durable
    /// outcome.
    async fn request(&self, phase: PairCommitPhase) -> bool {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        if self
            .commit_tx
            .send(PairCommitRequest {
                phase,
                transaction: self.transaction_id,
                phone_pubkey: self.phone_pubkey,
                host_pubkey: self.host_pubkey,
                peer: self.peer.to_owned(),
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return false;
        }
        reply_rx.await.unwrap_or(false)
    }
}

async fn drive_pair_transaction(
    state: &Arc<PairServiceState>,
    broker: &PairingBroker,
    commit_tx: &mpsc::Sender<PairCommitRequest>,
    phone_pubkey: [u8; PAIR_PUBKEY_LEN],
    peer: &str,
) -> Result<(), PairEngineError> {
    let host_pubkey = state.host_pubkey;
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).map_err(|_| PairEngineError::Aborted)?;
    let mut transaction = Transaction::new(id, Role::Coordinator);
    let context = PairCommitContext {
        commit_tx,
        transaction_id: id,
        host_pubkey,
        phone_pubkey,
        peer,
    };

    // CAPABILITY. The phone reads the desktop offer and answers on v2-control
    // only after the operator accepted the OOB words on the phone, so this
    // round is human-paced.
    state
        .publish_paced(Message {
            transaction: id,
            operation: Operation::Capability,
        })
        .await;
    let operation = wait_for_operation_within(state, id, PAIR_CAPABILITY_TIMEOUT_INTERNAL).await?;
    if operation != Operation::Capability {
        return Err(PairEngineError::Aborted);
    }
    apply_remote(&mut transaction, id, operation)?;
    // Our own capability offer is a local fact, exactly as on the phone.
    transaction.local(LocalEvent::Capability).map_err(|_| PairEngineError::Aborted)?;
    // The keys really were exchanged over authenticated GATT before this.
    transaction
        .local(LocalEvent::VerifiedExchange)
        .map_err(|_| PairEngineError::Aborted)?;
    state.remember_session(id, transaction.status());

    // DESKUNLOCK CONFIRMATION. Real application decision, independent of the
    // transport (BlueZ LESC) confirmation. No GUI means a closed failure.
    if !broker.request_oob_confirmation(peer.to_owned(), host_pubkey, phone_pubkey).await {
        state.publish(Message {
            transaction: id,
            operation: Operation::Reject,
        });
        let _ = transaction.local(LocalEvent::Abort(Operation::Reject));
        state.remember_session(id, transaction.status());
        return Err(PairEngineError::Rejected);
    }
    transaction.local(LocalEvent::Confirm).map_err(|_| PairEngineError::Aborted)?;
    state
        .publish_paced(Message {
            transaction: id,
            operation: Operation::Confirm,
        })
        .await;
    let operation = wait_for_operation(state, id).await?;
    if operation != Operation::Confirm {
        return Err(PairEngineError::Aborted);
    }
    apply_remote(&mut transaction, id, operation)?;

    // DURABLE STAGING (inactive bond + key + journal), before PREPARED.
    // This makes the peer recoverable but NOT active: no bonds.toml entry and
    // no keys/<peer_id>.bin are written here.
    if !context.request(PairCommitPhase::Stage).await {
        state.publish(Message {
            transaction: id,
            operation: Operation::Error,
        });
        let _ = transaction.local(LocalEvent::Abort(Operation::Error));
        state.remember_session(id, transaction.status());
        return Err(PairEngineError::Persistence);
    }

    // PREPARED.
    transaction.local(LocalEvent::Prepared).map_err(|_| PairEngineError::Aborted)?;
    // Wait for the peer's PREPARED *before* publishing ours. The phone polls a
    // last-value status characteristic, so a publish here would overwrite the
    // CONFIRM we just sent whenever the peer had not read it yet — the peer
    // then spins on a stale register and fails on its own timeout. One message
    // in flight per step is the invariant that keeps the channel lossless.
    let operation = match wait_for_operation(state, id).await {
        Ok(operation) => operation,
        Err(err) => {
            return Err(abort_with_discard(state, &context, &mut transaction, err).await);
        }
    };
    if operation != Operation::Prepared {
        return Err(abort_with_discard(state, &context, &mut transaction, PairEngineError::Aborted).await);
    }
    apply_remote(&mut transaction, id, operation)?;
    state
        .publish_paced(Message {
            transaction: id,
            operation: Operation::Prepared,
        })
        .await;

    // DURABLE LOCAL COMMIT DECISION. The journal must say `commit_pending`
    // before the state machine leaves PREPARED, otherwise recovery cannot
    // distinguish "definitely aborted" from "possibly committed".
    if !context.request(PairCommitPhase::MarkCommit).await {
        state.publish(Message {
            transaction: id,
            operation: Operation::Error,
        });
        let _ = transaction.local(LocalEvent::Abort(Operation::Error));
        state.remember_session(id, transaction.status());
        return Err(PairEngineError::Persistence);
    }

    // COMMIT.
    transaction.local(LocalEvent::Commit).map_err(|_| PairEngineError::Aborted)?;
    state
        .publish_paced(Message {
            transaction: id,
            operation: Operation::Commit,
        })
        .await;
    let operation = match wait_for_operation(state, id).await {
        Ok(operation) => operation,
        Err(err) => {
            return Err(abort_with_discard(state, &context, &mut transaction, err).await);
        }
    };
    if operation != Operation::CommitAck {
        return Err(abort_with_discard(state, &context, &mut transaction, PairEngineError::Aborted).await);
    }
    apply_remote(&mut transaction, id, operation)?;
    state.remember_session(id, transaction.status());

    // BILATERAL COMMITTED. Only after both sides verify their committed
    // record may the staged bond become active trust.
    state
        .publish_paced(Message {
            transaction: id,
            operation: Operation::Committed,
        })
        .await;
    match wait_for_operation(state, id).await {
        Ok(Operation::Committed) => {
            apply_remote(&mut transaction, id, Operation::Committed)?;
            if !context.request(PairCommitPhase::Promote).await {
                // The commit decision is durable but the local promotion
                // failed. The journal and pending state are retained so
                // recovery can finish; no active trust is claimed.
                let _ = transaction.local(LocalEvent::Disconnected);
                state.remember_session(id, transaction.status());
                return Err(PairEngineError::Persistence);
            }
            transaction.local(LocalEvent::Committed).map_err(|_| PairEngineError::Uncertain)?;
            state.remember_session(id, transaction.status());
            // TrustEstablished: the only point at which the GUI may render
            // the terminal success state.
            broker.notify_bonded(peer.to_owned()).await;
            Ok(())
        }
        _ => {
            // UNCERTAIN. The journal, pending bond and pending key are left
            // intact and no active trust is written; reconcile decides later.
            let _ = transaction.local(LocalEvent::Disconnected);
            state.remember_session(id, transaction.status());
            Err(PairEngineError::Uncertain)
        }
    }
}

/// Definite pre-commit abort: record the local abort and drop the staging.
/// Never touches active trust.
async fn abort_with_discard(
    state: &Arc<PairServiceState>,
    context: &PairCommitContext<'_>,
    transaction: &mut Transaction,
    error: PairEngineError,
) -> PairEngineError {
    state.publish(Message {
        transaction: context.transaction_id,
        operation: Operation::Error,
    });
    let _ = transaction.local(LocalEvent::Abort(Operation::Error));
    state.remember_session(context.transaction_id, transaction.status());
    let _ = context.request(PairCommitPhase::Discard).await;
    error
}

fn apply_remote(transaction: &mut Transaction, id: [u8; 16], operation: Operation) -> Result<(), PairEngineError> {
    transaction
        .remote(Message {
            transaction: id,
            operation,
        })
        .map_err(|_| PairEngineError::Aborted)
}

async fn wait_for_operation(state: &Arc<PairServiceState>, id: [u8; 16]) -> Result<Operation, PairEngineError> {
    wait_for_operation_within(state, id, PAIR_MESSAGE_TIMEOUT_INTERNAL).await
}

/// Wait for one operation of `id` with an explicit budget.
async fn wait_for_operation_within(state: &Arc<PairServiceState>, id: [u8; 16], budget: Duration) -> Result<Operation, PairEngineError> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(operation) = state.take_message(id) {
            tracing::info!(target: "syauth_transport", operation = ?operation, "pair engine received peer operation");
            if matches!(
                operation,
                Operation::Reject | Operation::Cancel | Operation::Timeout | Operation::Error
            ) {
                return Err(PairEngineError::Aborted);
            }
            return Ok(operation);
        }
        if Instant::now() >= deadline {
            tracing::warn!(target: "syauth_transport", "pair engine timed out waiting for the peer's operation");
            return Err(PairEngineError::Aborted);
        }
        tokio::time::sleep(PAIR_MESSAGE_POLL).await;
    }
}

/// Encoded `host-name` metadata: one version byte plus the display name,
/// truncated without splitting a UTF-8 code point. This is the only
/// authenticated application identity; the OS/CDM label is provisional.
#[must_use]
pub fn host_name_payload(name: &str) -> Vec<u8> {
    let mut display: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    while display.len() > 128 {
        display.pop();
    }
    if display.is_empty() {
        display = "Computer DeskUnlock".to_owned();
    }
    let mut bytes = vec![1];
    bytes.extend_from_slice(display.as_bytes());
    bytes
}

/// The pair service UUID, re-exported so callers can assert the contract.
#[must_use]
pub fn pair_service_uuid() -> Uuid {
    SYAUTH_PAIR_SERVICE_UUID
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: bluer's `Io` write reader acknowledges the write and then
    /// keeps the stream open for the next one, so `read_to_end` never returns
    /// and the payload is silently lost while the peer believes it wrote. The
    /// bounded read must return the payload of a stream that never closes.
    /// The register is a **last-value** store: without the unread mark, two
    /// publishes in quick succession silently destroy the first message
    /// (observed on hardware as `published Prepared` 103 µs after
    /// `published Commit`).
    #[test]
    fn a_publish_marks_the_register_unread_until_the_peer_reads_it() {
        let state = PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation"));
        assert!(!state.unread.load(Ordering::SeqCst), "a fresh session has nothing pending");
        state.publish(Message {
            transaction: [1u8; 16],
            operation: Operation::Confirm,
        });
        assert!(state.unread.load(Ordering::SeqCst), "the peer has not read CONFIRM yet");
        let _ = state.status_snapshot(0);
        assert!(!state.unread.load(Ordering::SeqCst), "the peer's read clears the mark");
    }

    #[tokio::test]
    async fn publish_paced_waits_for_the_peer_read_then_fails_open() {
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        state.publish(Message {
            transaction: [1u8; 16],
            operation: Operation::Confirm,
        });
        let started = Instant::now();
        state
            .publish_paced(Message {
                transaction: [1u8; 16],
                operation: Operation::Prepared,
            })
            .await;

        assert!(
            started.elapsed() >= PAIR_STATUS_READ_TIMEOUT_INTERNAL,
            "a peer that stopped reading must not block the publish forever"
        );
        assert_eq!(
            Some(Operation::Prepared),
            Message::decode(&state.status_snapshot(0)).ok().map(|message| message.operation),
            "the bounded wait fails open: the message is published anyway"
        );
    }

    #[tokio::test]
    async fn a_bounded_read_returns_the_payload_of_a_stream_that_never_closes() {
        use tokio::io::AsyncWriteExt;

        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_all(&[2u8; 18]).await.expect("write payload");
        // `writer` is deliberately kept alive: no end-of-stream is ever seen.
        let payload = read_bounded(&mut reader, PAIR_CONTROL_MAX_BYTES, Duration::from_millis(50))
            .await
            .expect("payload collected without waiting for EOF");
        assert_eq!(18, payload.len());

        // A second write on the same open stream is collected too: the reader
        // must keep serving attempt after attempt.
        writer.write_all(&[3u8; 27]).await.expect("write payload");
        let next = read_bounded(&mut reader, PAIR_CONTROL_MAX_BYTES, Duration::from_millis(50))
            .await
            .expect("second payload");
        assert_eq!(27, next.len());
    }

    /// The mechanism itself: on the same open stream `read_to_end` never
    /// completes. This is why the bounded read exists, and it is asserted so a
    /// future "simplification" back to `read_to_end` fails loudly here.
    #[tokio::test]
    async fn read_to_end_never_completes_on_a_stream_the_peer_keeps_open() {
        use tokio::io::AsyncWriteExt;

        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_all(&[2u8; 18]).await.expect("write payload");
        let mut sink = Vec::new();
        let completed = tokio::time::timeout(Duration::from_millis(50), reader.read_to_end(&mut sink)).await;
        assert!(
            completed.is_err(),
            "read_to_end must not complete while the peer keeps the write stream open"
        );
    }

    /// Regression: the bluer `Io` reader is a byte stream, so two messages the
    /// phone published back to back arrive as ONE payload (observed on
    /// hardware as `bytes=36`). Reframing must recover both, otherwise the
    /// coordinator waits for a message that already arrived and then aborts.
    #[test]
    fn a_coalesced_control_payload_is_reframed_into_every_message() {
        let id = [3u8; 16];
        let mut payload = Vec::new();
        payload.extend_from_slice(
            &Message {
                transaction: id,
                operation: Operation::Confirm,
            }
            .encode(),
        );
        payload.extend_from_slice(
            &Message {
                transaction: id,
                operation: Operation::Prepared,
            }
            .encode(),
        );

        let mut seen: Vec<Operation> = Vec::new();
        let framed = frame_control_payload(&payload, |frame| {
            seen.push(Message::decode(&frame).expect("frame decodes").operation);
        });

        assert_eq!(2, framed, "both coalesced messages must be recovered");
        assert_eq!(vec![Operation::Confirm, Operation::Prepared], seen);
    }

    #[test]
    fn a_desynced_control_payload_resynchronises_on_the_next_frame() {
        let id = [4u8; 16];
        let message = Message {
            transaction: id,
            operation: Operation::Committed,
        }
        .encode();
        // Junk in front of a valid frame, plus a truncated tail that must be
        // dropped instead of blocking the channel.
        let mut payload = vec![0x00, 0x2a, 0xff];
        payload.extend_from_slice(&message);
        payload.extend_from_slice(&message[..5]);

        let mut seen: Vec<Operation> = Vec::new();
        let framed = frame_control_payload(&payload, |frame| {
            seen.push(Message::decode(&frame).expect("frame decodes").operation);
        });

        assert_eq!(1, framed);
        assert_eq!(vec![Operation::Committed], seen);
    }

    #[test]
    fn pair_service_state_publishes_and_takes_only_the_matching_transaction() {
        let state = PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation"));
        state.accept_control(
            Message {
                transaction: [1u8; 16],
                operation: Operation::Capability,
            }
            .encode()
            .to_vec(),
        );
        assert_eq!(state.take_message([2u8; 16]), None);
        assert_eq!(state.take_message([1u8; 16]), Some(Operation::Capability));
        assert_eq!(state.take_message([1u8; 16]), None);
    }

    #[test]
    fn status_query_is_answered_without_entering_the_commit_inbox() {
        let state = PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation"));
        state.remember_session([3u8; 16], StatusState::Committed);
        let query = StatusMessage::query([3u8; 16], 42);
        state.accept_control(query.encode().to_vec());
        assert!(state.take_message([3u8; 16]).is_none());
        let response = StatusMessage::decode(&state.status_snapshot(0)).expect("status response");
        assert_eq!(response.nonce, 42);
        assert_eq!(response.state, Some(StatusState::Committed));
    }

    #[test]
    fn malformed_control_never_advances_and_is_ignored() {
        let state = PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation"));
        state.accept_control(vec![0u8; 3]);
        state.accept_control(vec![9u8; 64]);
        assert!(state.take_message([0u8; 16]).is_none());
    }

    #[test]
    fn host_name_payload_is_bounded_and_versioned() {
        assert_eq!(host_name_payload(" workstation\n"), b"\x01workstation");
        assert_eq!(host_name_payload(""), b"\x01Computer DeskUnlock");
        let long = host_name_payload(&"è".repeat(200));
        assert!(long.len() <= 129);
        assert!(std::str::from_utf8(&long[1..]).is_ok());
    }

    #[test]
    fn pair_service_exposes_the_full_v2_contract() {
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let (service, _, _) = build_pair_service(state);
        assert_eq!(service.uuid, pair_service_uuid());
        let uuids: Vec<Uuid> = service.characteristics.iter().map(|c| c.uuid).collect();
        for expected in [
            SYAUTH_PAIR_HOST_NAME_V1_CHAR_UUID,
            SYAUTH_PAIR_HOST_PUBKEY_CHAR_UUID,
            SYAUTH_PAIR_V2_STATUS_CHAR_UUID,
            SYAUTH_PAIR_V2_CONTROL_CHAR_UUID,
            SYAUTH_PAIR_PHONE_PUBKEY_CHAR_UUID,
        ] {
            assert!(uuids.contains(&expected), "pair service missing {expected}");
        }
    }

    // -----------------------------------------------------------------
    // Trust-boundary tests. These drive the real on-disk staging format
    // (pairing-v2.journal / pairing-v2.pending / keys/.pending-<peer_id>.bin)
    // through a temporary bond directory and prove that active trust is
    // written exactly once, only after the bilateral V2 COMMITTED.
    // -----------------------------------------------------------------

    const PHONE_PUBKEY: [u8; PAIR_PUBKEY_LEN] = [9u8; PAIR_PUBKEY_LEN];

    /// Temporary bond directory with the private mode the trust store
    /// requires.
    fn temp_bond_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
        }
        dir
    }

    fn peer_id_fixture() -> String {
        syauth_core::peer_id_from_pubkey(&PHONE_PUBKEY)
    }

    /// Answers the coordinator's published messages like a well-behaved peer,
    /// replying only for the operations in `reply_through`.
    fn start_peer(state: Arc<PairServiceState>, reply_through: Vec<Operation>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut answered: Vec<Operation> = Vec::new();
            loop {
                if let Ok(message) = Message::decode(&state.status_snapshot(0)) {
                    let operation = message.operation;
                    if !answered.contains(&operation) && reply_through.contains(&operation) {
                        let response = match operation {
                            Operation::Capability => Operation::Capability,
                            Operation::Confirm => Operation::Confirm,
                            Operation::Prepared => Operation::Prepared,
                            Operation::Commit => Operation::CommitAck,
                            Operation::Committed => Operation::Committed,
                            _ => Operation::Error,
                        };
                        state.accept_control(
                            Message {
                                transaction: message.transaction,
                                operation: response,
                            }
                            .encode()
                            .to_vec(),
                        );
                        answered.push(operation);
                        // The phone never waits for the desktop's PREPARED: once
                        // both confirmations are present it publishes its own.
                        // Model that, or the coordinator would wait forever for
                        // a message the peer only sends after seeing ours.
                        if operation == Operation::Confirm && reply_through.contains(&Operation::Prepared) {
                            state.accept_control(
                                Message {
                                    transaction: message.transaction,
                                    operation: Operation::Prepared,
                                }
                                .encode()
                                .to_vec(),
                            );
                            answered.push(Operation::Prepared);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
    }

    /// Answers the DeskUnlock OOB request with `accept`. The caller registers
    /// the in-process GUI session first so the engine never races the socket
    /// fallback.
    fn start_gui(broker: PairingBroker, mut events: mpsc::Receiver<crate::pairing::GuiEvent>, accept: bool) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                if let crate::pairing::GuiEvent::Oob(request) = event {
                    let _ = broker.respond(request.id, accept).await;
                }
            }
        })
    }

    /// Performs the real commit-boundary actions in `bond_dir` using the shared
    /// recovery format, and returns how many promotions happened.
    fn start_commit_consumer(mut rx: mpsc::Receiver<PairCommitRequest>, bond_dir: std::path::PathBuf) -> tokio::task::JoinHandle<usize> {
        tokio::spawn(async move {
            let mut promotions = 0usize;
            while let Some(request) = rx.recv().await {
                let peer_id = syauth_core::peer_id_from_pubkey(&request.phone_pubkey);
                let outcome = match request.phase {
                    PairCommitPhase::Stage => {
                        let bond_key = syauth_core::bond_key_from_pubkeys(&request.host_pubkey, &request.phone_pubkey);
                        let bond = syauth_core::Bond {
                            peer_id: peer_id.clone(),
                            pubkey: request.phone_pubkey,
                            name: "phone".to_owned(),
                            created_at: ::time::OffsetDateTime::now_utc(),
                            status: syauth_core::BondStatus::Bonded,
                        };
                        syauth_core::pair_recovery::stage(&bond_dir, request.transaction, &peer_id, &request.peer, &bond, &bond_key).is_ok()
                    }
                    PairCommitPhase::MarkCommit => syauth_core::pair_recovery::mark_commit_pending(&bond_dir).is_ok(),
                    PairCommitPhase::Promote => {
                        promotions += 1;
                        syauth_core::pair_recovery::promote(&bond_dir, &peer_id).is_ok()
                    }
                    PairCommitPhase::Discard => {
                        syauth_core::pair_recovery::discard(&bond_dir, &peer_id);
                        true
                    }
                    // Day-2 revocation: this consumer only models the commit
                    // boundary, so a revocation is accepted and ignored here.
                    PairCommitPhase::Revoke => true,
                };
                let _ = request.reply.send(outcome);
            }
            promotions
        })
    }

    struct ActiveTrust {
        bonds: bool,
        key: bool,
    }

    fn active_trust(bond_dir: &std::path::Path) -> ActiveTrust {
        ActiveTrust {
            bonds: syauth_core::pair_recovery::bonds_path(bond_dir).exists(),
            key: syauth_core::pair_recovery::active_key_path(bond_dir, &peer_id_fixture()).exists(),
        }
    }

    #[tokio::test]
    async fn pre_commit_abort_writes_no_active_trust_and_clears_staging() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, true);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());
        // The peer stops before COMMIT_ACK.
        let peer = start_peer(
            Arc::clone(&state),
            vec![Operation::Capability, Operation::Confirm, Operation::Prepared],
        );

        let result = drive_pair_transaction(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;

        assert_eq!(result, Err(PairEngineError::Aborted));
        let trust = active_trust(dir.path());
        assert!(!trust.bonds, "no active bonds.toml before a bilateral commit");
        assert!(!trust.key, "no active key before a bilateral commit");
        // A definite pre-commit abort drops the staging.
        assert!(!syauth_core::pair_recovery::journal_path(dir.path()).exists());
        assert!(!syauth_core::pair_recovery::pending_bond_path(dir.path()).exists());
        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 0, "no promotion before commit");
        peer.abort();
        gui.abort();
    }

    #[tokio::test]
    async fn commit_ack_without_remote_committed_is_uncertain_and_keeps_recovery() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, true);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());
        // The peer answers COMMIT but vanishes before COMMITTED.
        let peer = start_peer(
            Arc::clone(&state),
            vec![Operation::Capability, Operation::Confirm, Operation::Prepared, Operation::Commit],
        );

        let result = drive_pair_transaction(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;

        assert_eq!(result, Err(PairEngineError::Uncertain));
        let trust = active_trust(dir.path());
        assert!(!trust.bonds, "an uncertain transaction must not promote bonds.toml");
        assert!(!trust.key, "an uncertain transaction must not promote an active key");
        assert!(
            syauth_core::pair_recovery::journal_path(dir.path()).exists(),
            "the journal must survive for reconcile"
        );
        assert!(syauth_core::pair_recovery::pending_bond_path(dir.path()).exists());
        assert!(syauth_core::pair_recovery::pending_key_path(dir.path(), &peer_id_fixture()).exists());
        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 0, "no promotion without bilateral completion");
        peer.abort();
        gui.abort();
    }

    /// Regression: the phone polls a **last-value** status characteristic, so
    /// the coordinator must not publish the next message until the peer has
    /// answered the previous one. Publishing PREPARED before the peer's own
    /// PREPARED overwrote the CONFIRM the peer had not read yet: the pairing
    /// then died on the peer's timeout while this side waited.
    #[tokio::test]
    async fn the_coordinator_holds_the_confirm_until_the_peer_answers() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, true);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());
        // The peer answers CAPABILITY and CONFIRM only: it never sends PREPARED.
        let peer = start_peer(Arc::clone(&state), vec![Operation::Capability, Operation::Confirm]);

        // What the peer would read while this side waits for its PREPARED.
        let observed = Arc::clone(&state);
        let watcher = tokio::spawn(async move {
            let mut seen: Vec<Operation> = Vec::new();
            for _ in 0..40 {
                if let Ok(message) = Message::decode(&observed.status_snapshot(0))
                    && seen.last() != Some(&message.operation)
                {
                    seen.push(message.operation);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            seen
        });

        let result = drive_pair_transaction(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;
        let seen = watcher.await.expect("watcher");

        assert_eq!(result, Err(PairEngineError::Aborted));
        assert!(seen.contains(&Operation::Confirm), "the peer must have seen CONFIRM: {seen:?}");
        assert!(
            !seen.contains(&Operation::Prepared),
            "PREPARED must not be published before the peer's PREPARED arrives: {seen:?}"
        );
        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 0, "no promotion on an aborted attempt");
        peer.abort();
        gui.abort();
    }

    #[tokio::test]
    async fn bilateral_committed_promotes_active_trust_exactly_once() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, true);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());
        let peer = start_peer(
            Arc::clone(&state),
            vec![
                Operation::Capability,
                Operation::Confirm,
                Operation::Prepared,
                Operation::Commit,
                Operation::Committed,
            ],
        );

        let result = drive_pair_transaction(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;

        assert_eq!(result, Ok(()));
        let trust = active_trust(dir.path());
        assert!(trust.bonds, "bilateral COMMITTED promotes bonds.toml");
        assert!(trust.key, "bilateral COMMITTED promotes the active key");
        assert!(
            !syauth_core::pair_recovery::journal_path(dir.path()).exists(),
            "promotion clears the journal"
        );
        assert!(!syauth_core::pair_recovery::pending_bond_path(dir.path()).exists());
        assert!(!syauth_core::pair_recovery::pending_key_path(dir.path(), &peer_id_fixture()).exists());
        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 1, "promotion happens exactly once");
        peer.abort();
        gui.abort();
    }

    #[tokio::test]
    async fn operator_reject_persists_no_trust_and_leaves_no_staging() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, false);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());
        let peer = start_peer(
            Arc::clone(&state),
            vec![
                Operation::Capability,
                Operation::Confirm,
                Operation::Prepared,
                Operation::Commit,
                Operation::Committed,
            ],
        );

        let result = drive_pair_transaction(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;

        assert_eq!(result, Err(PairEngineError::Rejected));
        let trust = active_trust(dir.path());
        assert!(!trust.bonds);
        assert!(!trust.key);
        assert!(!syauth_core::pair_recovery::journal_path(dir.path()).exists());
        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 0);
        peer.abort();
        gui.abort();
    }

    // -----------------------------------------------------------------
    // Engine lifecycle: one long-lived engine must serve attempt after
    // attempt on the SAME PairServiceState, with no rebuild_application
    // and no daemon restart.
    // -----------------------------------------------------------------

    const FULL_EXCHANGE: [Operation; 5] = [
        Operation::Capability,
        Operation::Confirm,
        Operation::Prepared,
        Operation::Commit,
        Operation::Committed,
    ];

    #[tokio::test]
    async fn a_second_attempt_runs_on_the_same_live_engine_without_rebuild() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, true);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());

        // Attempt 1: the peer stops before COMMIT_ACK.
        let peer_one = start_peer(
            Arc::clone(&state),
            vec![Operation::Capability, Operation::Confirm, Operation::Prepared],
        );
        run_pair_attempt(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;
        peer_one.abort();
        let trust = active_trust(dir.path());
        assert!(!trust.bonds, "a pre-commit abort must not write active trust");
        assert!(!trust.key);

        // A late message from the aborted attempt must not contaminate the retry.
        state.accept_control(
            Message {
                transaction: [0xAB; 16],
                operation: Operation::Committed,
            }
            .encode()
            .to_vec(),
        );

        // Attempt 2: SAME engine, new transaction, full exchange.
        let peer_two = start_peer(Arc::clone(&state), FULL_EXCHANGE.to_vec());
        run_pair_attempt(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;
        peer_two.abort();

        let trust = active_trust(dir.path());
        assert!(trust.bonds, "the retry must reach TrustEstablished on the same engine");
        assert!(trust.key);
        assert!(!syauth_core::pair_recovery::journal_path(dir.path()).exists());
        assert!(!syauth_core::pair_recovery::pending_bond_path(dir.path()).exists());
        assert!(!syauth_core::pair_recovery::pending_key_path(dir.path(), &peer_id_fixture()).exists());

        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 1, "exactly one promotion across both attempts");
        gui.abort();
    }

    #[tokio::test]
    async fn two_consecutive_aborts_then_a_valid_attempt_succeeds() {
        let dir = temp_bond_dir();
        let state = Arc::new(PairServiceState::new([7u8; PAIR_PUBKEY_LEN], host_name_payload("workstation")));
        let broker = PairingBroker::default();
        let gui_events = broker.register_gui().await.expect("gui");
        let gui = start_gui(broker.clone(), gui_events, true);
        let (commit_tx, commit_rx) = mpsc::channel::<PairCommitRequest>(4);
        let consumer = start_commit_consumer(commit_rx, dir.path().to_path_buf());

        for _ in 0..2 {
            let peer = start_peer(
                Arc::clone(&state),
                vec![Operation::Capability, Operation::Confirm, Operation::Prepared],
            );
            run_pair_attempt(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;
            peer.abort();
        }
        let trust = active_trust(dir.path());
        assert!(!trust.bonds, "consecutive aborts must never write active trust");
        assert!(!trust.key);

        // The third attempt on the same engine must still work.
        let peer_three = start_peer(Arc::clone(&state), FULL_EXCHANGE.to_vec());
        run_pair_attempt(&state, &broker, &commit_tx, PHONE_PUBKEY, "peer").await;
        peer_three.abort();

        let trust = active_trust(dir.path());
        assert!(trust.bonds, "the engine must still work after two aborts");
        assert!(trust.key);

        drop(commit_tx);
        assert_eq!(consumer.await.expect("consumer"), 1, "exactly one promotion across three attempts");
        gui.abort();
    }
}
