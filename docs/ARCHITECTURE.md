# DeskUnlock architecture

Status: **frozen** (approved architecture). This document is the contract for
future changes. Runtime convergence work is tracked separately.

DeskUnlock turns an Android phone into a key for a Linux desktop. The design
separates five layers that are frequently conflated in phone-as-key systems.
Each layer has exactly one owner and one responsibility, and the layers are
not interchangeable.

## 1. Bluetooth / BlueZ — transport

- radio discovery and connection;
- GATT link management;
- optional operating-system Bluetooth bond.

Owner: `syauth-presenced` for the adapter usage DeskUnlock needs (advertising
the rotating session UUIDs, hosting the GATT services). The desktop's general
Bluetooth frontend remains DMS.

Rules:

- being paired with the computer (or with DMS) means only that the radio link
  can come up;
- a BlueZ bond never creates DeskUnlock trust;
- DeskUnlock does not modify `dms.service`, does not create a second DMS bar,
  and does not become the global manager of every Bluetooth device;
- the desktop adapter keeps its normal host identity (for example
  `cachyos-x8664`). The DeskUnlock identity is carried by the authenticated
  application protocol, not by renaming the adapter.

## 2. Android CDM — OS/app association

- Companion Device Manager discovery and device picker;
- OS-level association of the app to a nearby device;
- presence observation / service binding.

Owner: the Android app (`AndroidCdmPairCompanionScanner` and
`MainActivity`).

Rules:

- a CDM association is neither a Bluetooth bond nor DeskUnlock trust;
- the association is **provisional** until DeskUnlock trust exists, and is
  disassociated on every pre-trust exit (cancel, reject, error, timeout);
- CDM may be used as a proximity signal to wake the app; the unlock decision
  still requires a valid DeskUnlock response.

## 3. DeskUnlock Protocol — discovery + GATT + handshake

- DeskUnlock discovery (rotating pair-mode UUID derived from the zero bond
  key; per-bond unlock UUIDs derived from the bond key);
- GATT service and characteristic contract;
- authenticated public-key exchange (`host-pubkey` / `phone-pubkey`);
- authenticated display metadata (`host-name`);
- versioned bilateral pairing transaction:
  `CAPABILITY -> keys exchanged -> CONFIRM -> PREPARED -> COMMIT ->
  COMMIT_ACK -> COMMITTED`;
- strict ordering and fail-closed rejection of unknown versions, malformed
  messages, wrong transaction ids, replays and out-of-order operations.

Owner: the daemon-owned pair engine (`syauth-presenced` + the shared
`syauth-core` transaction state machine). The GUI is a presentation and
confirmation client, never a protocol driver.

Rules:

- one GATT service definition, one characteristic set, shared by desktop and
  phone;
- `peer_id` is always derived from the phone public key;
- names and addresses are display labels only and never select a peer.

## 4. DeskUnlock Confirmation — the real application decision

- the application-level confirmation that authorizes a pairing or an unlock;
- rendered only when there is a real decision to make.

Owner: the layer performing the operation (desktop dialog for pairing, Android
approve screen for unlock).

Rules:

- a GUI confirmation must correspond to a real action;
- there is no "waiting for your confirmation" state when nothing is pending;
- **BlueZ LESC confirmation is not DeskUnlock confirmation** (see below).

## 5. DeskUnlock Trust — keys, identity, authorization

- bond records (`bonds.toml`) and per-peer keys (`keys/<peer_id>.bin` on the
  desktop, Android Keystore on the phone);
- peer identity derived from the public key;
- the set of phones explicitly authorized by DeskUnlock.

Owner: the trust store on each side. It is written only at the bilateral
commit boundary.

Rules:

- a Bluetooth-paired phone is not a DeskUnlock key;
- a CDM-associated phone is not a DeskUnlock key;
- DeskUnlock trust always requires DeskUnlock's own handshake and proof;
- a phone that is already Bluetooth-paired must be able to complete DeskUnlock
  pairing later.

## 6. Presence / PAM — consuming trust

- proximity tracking and lock integration;
- authentication decision;
- `pam_syauth.so`;
- use of an already-trusted phone as a key.

Owner: `syauth-presenced` (proximity/orchestrator) and the PAM module.

Rules:

- this layer consumes an established DeskUnlock trust; it never creates one;
- proximity alone is not an unlock authorization;
- when the phone or Bluetooth path is unavailable, the configured PAM fallback
  continues.

## Invariants

- Bluetooth paired ≠ DeskUnlock authorized.
- Android CDM associated ≠ DeskUnlock authorized.
- DeskUnlock trust always requires its own application handshake/proof.
- A phone already Bluetooth-paired can be associated with DeskUnlock later.
- If DeskUnlock needs a Bluetooth bond, it remains a transport detail and does
  not replace DeskUnlock trust.
- DMS remains the general Bluetooth frontend of the desktop.
- DeskUnlock does not modify `dms.service`.
- DeskUnlock does not create a second DMS bar.
- DeskUnlock does not implicitly become the global manager of all Bluetooth
  devices.
- A confirmation shown by the GUI must correspond to a real action.
- There is no "waiting for your confirmation" state if the user has nothing to
  confirm.

## Target pairing state machine

| State | Meaning and entry condition | Exits |
|---|---|---|
| `Idle` | Initial state; nothing in flight. | `DiscoveringDeskUnlock` |
| `DiscoveringDeskUnlock` | Desktop (`syauth-presenced`) advertises the pair-mode UUID; the phone opens the CDM picker / BLE scan. | `TransportReady`; `Cancelled` → `Idle` |
| `TransportReady` | The transport link is usable. The sub-states below decide whether a Bluetooth bond must be created first. | `DeskUnlockHandshake` |
| `TransportReady / LescPending` | No Bluetooth bond exists; BlueZ `RequestConfirmation` fires and the UI shows the numeric comparison code. | `TransportReady` |
| `TransportReady / LescRejected` | The transport confirmation was refused. | `Idle` (no DeskUnlock state written) |
| `TransportReady / AlreadyBonded` | A Bluetooth bond already exists; LESC is skipped. Valid entry for a pre-paired phone. | `DeskUnlockHandshake` |
| `DeskUnlockHandshake` | Authenticated GATT: `host-pubkey` ⇄ `phone-pubkey`, host-name metadata, `CAPABILITY` → keys exchanged. | `ConfirmationRequired`; `TransportError` / `Timeout` → `Idle` (provisional CDM association dropped) |
| `ConfirmationRequired` | Only when a real DeskUnlock decision exists: OOB words computed from the bond key are compared on both screens. | `CommitPending`; `Reject` → `Aborted` (no trust written) |
| `CommitPending` | `stage` → `COMMIT` → `COMMIT_ACK` → `COMMITTED`; both sides persist and re-read their committed record. | `TrustEstablished`; disconnect after commit → `Uncertain` (reconcile path, no false success) |
| `TrustEstablished` | `bonds.toml` + `keys/<peer_id>.bin` promoted; `peer_id` from the public key. | `Completed` |
| `Completed` | Terminal success. | — |

Rules encoded in the state machine:

- `TransportReady` can be entered without LESC when the phone is already
  Bluetooth-paired;
- `ConfirmationRequired` is rendered only when a real request exists;
  otherwise the UI shows progress, not a waiting state;
- `TrustEstablished` is the only state that may render "associated", and only
  after the bilateral commit;
- the CDM association is provisional until `TrustEstablished` and is dropped
  on every pre-trust exit.

## BlueZ LESC confirmation is not DeskUnlock confirmation

The two confirmations occur at different layers and answer different
questions:

| | BlueZ LESC confirmation | DeskUnlock confirmation |
|---|---|---|
| Layer | Bluetooth transport | DeskUnlock protocol |
| Question | "Is this the device I am bonding with?" | "Do I authorize this phone as a DeskUnlock key?" |
| Data shown | 6-digit numeric comparison code | application-level code/words derived from the exchanged keys |
| Occurs when | a new Bluetooth bond is created | a DeskUnlock pairing or unlock decision exists |
| Occurs for an already-bonded phone | no | yes, and it is the only confirmation required |
| Result if accepted | transport bond | DeskUnlock trust (only after the bilateral commit) |

The transport confirmation is not sufficient for trust, and the application
confirmation must not be shown when there is nothing to decide.

## Ownership map

| Concern | Owner |
|---|---|
| scanning / discovery | desktop: `syauth-presenced` (advertiser); phone: CDM picker under app control |
| `Device1.Pair()` | transport; the OS/BlueZ side. DeskUnlock must not own global Bluetooth pairing policy |
| BlueZ confirmation (`Agent1` / `RequestConfirmation`) | transport; answered only when it occurs, relayed to a real user gesture |
| GATT connection | DeskUnlock transport/protocol; desktop `syauth-presenced`, phone GATT client |
| DeskUnlock handshake | the daemon-owned pair engine using the shared transaction state machine |
| key generation / storage | DeskUnlock trust; desktop host pubkey + per-peer key file, phone Android Keystore |
| CDM lifecycle | Android app; provisional until trust, disassociated on pre-trust exit |
| Cancel | the layer performing the operation; never touches a committed trust record |
| final trust state | DeskUnlock trust store only |

## Component to layer map

| Component | Layer |
|---|---|
| `crates/syauth-transport` (`bluez.rs`, `bluez_advertise.rs`, `peripheral.rs`, `pairing.rs`) | Bluetooth transport + DeskUnlock Protocol (GATT pair service) |
| `crates/syauth-presenced` (`runtime.rs`, `orchestrator.rs`) | DeskUnlock Protocol + Trust, presence/advertising |
| `crates/syauth-core` (`bond.rs`, `pair_transaction.rs`) | DeskUnlock Trust + Protocol |
| `crates/syauth-cli` (`pair.rs`, `pair_backend.rs`, `reconcile.rs`) | DeskUnlock Protocol + Trust; `pair --gui` is a presentation/confirmation client |
| `crates/syauth-pam` (`pam_syauth.so`) | Presence / PAM |
| `desktop/bin/syauth-*` | operator tooling and desktop integration |
| `syauth-android` `AndroidCdmPairCompanionScanner` | Android CDM |
| `syauth-android` GATT exchange / `PairingViewModel` | DeskUnlock Protocol + Confirmation |
| `syauth-android` `BondStore` / Keystore | DeskUnlock Trust |
| `syauth-android` `SyauthCompanionService` / approve screen | Presence + Confirmation |
| DMS (`dms.service`) | Bluetooth transport (external; untouched) |

## State and path separation

Program files belong to the package manager. Persistent private state does
not.

| Path | Contents |
|---|---|
| `/usr/bin/...` | packaged executables |
| `/usr/lib/...` | daemon, PAM module, helpers |
| `/usr/lib/systemd/user/...` | packaged user units |
| `/var/lib/syauth/...` | persistent pairing / cryptographic state |
| `/var/log/syauth/...` | persistent audit/log state |
| `/run/user/<uid>/syauth/...` | runtime sockets and markers |
| `/run/syauth/...` | runtime markers |

The exact rename/migration of existing `syauth` state paths is a compatibility
decision. Do not blindly rename state directories until migration behavior is
specified and tested.

## Boot and session flow

1. user session starts;
2. bootstrap;
3. presence daemon (`syauth-presenced`);
4. proximity watcher;
5. reconcile path/service;
6. health timer;
7. lock-screen integration.

The presence daemon registers with BlueZ, exposes the authentication socket,
and handles the paired phone transport.

## Authentication flow (Presence / PAM)

1. PAM authentication request;
2. DeskUnlock PAM module;
3. user runtime socket;
4. presence daemon;
5. BLE challenge to the paired phone;
6. phone performs biometric confirmation and signs the challenge;
7. presence daemon verifies the response;
8. PAM success.

If DeskUnlock is unavailable, PAM returns unavailable/failure as designed and
the configured fallback continues.

## Compatibility rule

Product branding and package names may become `DeskUnlock` without immediately
changing every wire-level identifier inherited from `syauth`. Protocol/state
renames must be deliberate migrations, not search-and-replace operations.
Identifiers preserved for compatibility are listed in the README section
"Compatibility identifiers".

## Conformance

This document freezes the target architecture. The current implementation is
being converged onto it in separate work; known gaps are tracked in
`specs/` and `docs/known-gaps.md`. In particular:

- the desktop GUI confirmation must be bound to a real DeskUnlock decision
  rather than to the transport LESC event;
- the daemon-owned GATT pair service must expose the same protocol contract as
  the phone expects (including authenticated display metadata);
- the recovery service is crash recovery only and must not run as a step of
  ordinary pairing.
