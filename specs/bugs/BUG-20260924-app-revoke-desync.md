# BUG-20260924-app-revoke-desync: app can't revoke, desktop keeps a stale bond

## Summary
Dissociating from the phone does nothing: the desktop GUI keeps showing the
phone as associated, the service stays active for phone login and proximity,
and the phone's own GUI keeps showing the bond. A later login therefore starts
a phone-unlock challenge against a phone that is gone (the notification never
arrives). The desktop and the app disagree about whether a bond exists.

## Reproduction (2026-09-24, clean-state reset first)
- Clear both sides (desktop `bonds.toml` = `schema_version = 1`, app
  `pm clear`), pair fresh from the app: pairing succeeds
  (`TrustEstablished`, `bond added`, `phone subscribed`).
- Press **Revoke** in the app.
- Result: desktop `bond removed` never happens; app log says
  `revoke requested without a persisted bond record`; desktop + app GUIs keep
  showing "associated".

## Evidence
- App: `revoke requested without a persisted bond record`.
- App pairing capture: `post-bond exchange complete` is present, but the
  persister's own log line `syauth.persister: persisting LESC bond` is **absent**
  — the local bond file was never written even though the desktop committed it.
- Desktop: no revoke received; bond `5a5481f3…` stays `bonded`.
- Earlier (dirtier state) the app also rejected the desktop's revoke:
  `revoke frame rejected: the peer id does not match this phone's bond`.

## Root causes

### 1. The revoke read a file the caller had already deleted (app side)
`onBondRevokeTapped` hands the service a fire-and-forget `startService` intent
and then *synchronously* deletes `filesDir/syauth-bond.toml` in the same tap.
When the service finally handles `ACTION_REVOKE_BOND` it re-read that file to
derive the frame's `ownId`, got `null`, logged `revoke requested without a
persisted bond record` and returned without sending anything.

This is not a race that sometimes wins: `startService` only enqueues, while the
deletion runs before the main thread picks the intent up, so the in-app revoke
failed **every** time. Fixed: `sendRevoke` reads `clientBonds[clientKey]`, the
record the service already holds for the live client (populated alongside
`clients` in `injectClientsForBonds`).

### 2. The computer had no code path for the frame in a bonded session (desktop side)
Sending the frame was not enough. `Operation::Revoke` was handled **only** in
the pair engine's `v2-control` reader (`pair_engine.rs:477-496`), and that
reader exists only in the *pair-mode* GATT app. Once a bond exists the desktop
serves `build_and_register_peer` (`peripheral.rs:848-877`), whose GATT app
exposes **only** the challenge/response characteristics. The phone writes the
revoke frame to the response characteristic (`PersistentGattClient.send` →
`writeResponse`), so the frame arrived and fell into `response_tx` — the
challenge-response channel — and the bond survived. No log appeared because the
already-active reader drains the bytes instead of emitting a fresh `Write
event`.

Fixed: `peripheral.rs` gained `revoke_peer_id()` and a branch in the bonded
reader that routes the frame to `pair_commit_tx` as
`PairCommitRequest { phase: Revoke, .. }` — the same commit channel and the same
`revoke_bond_by_peer_id` the pair engine already uses, so no revocation logic is
duplicated.

### Correction to an earlier hypothesis
The first pass blamed a missing persist at pair time, based on the
`persisting LESC bond` log being absent from the pairing capture. That capture
filtered tags with `-s "syauth:*"`, which does not match `syauth.persister`;
the absence was an artefact. Counter-evidence: `onBondRevokeTapped` returns early
when `bondRecord.value` is null, and it did **not** return early — so the record
(and therefore the file written at pairing) was present. The pairing persists
correctly.

## Fix plan (one at a time)
1. ~~Fix the app's pairing persistence~~ — not a defect; the pairing persists.
2. **Make the app's revoke use the in-memory record** (done: `sendRevoke` now
   reads `clientBonds[clientKey]`).
3. Guard login + proximity on a real bond / live phone presence.
   - **GUI done**: the `syauth-settings` system-status card no longer colours
     "Associazione" or "Ultimo sblocco" green unless the phone is actually
     here. `association_display()` uses the same 60 s presence heartbeat the
     fingerprint row already trusts (`phone_present()`), and
     `last_unlock_display()` refuses to go green on a stale timestamp. With a
     bond file but no phone the row now reads "Telefono non connesso".
   - **Still open**: the greeter/PAM and proximity runtime paths (the desktop
     keeps the daemon off a phone it cannot reach).

## Verification
- Desktop, 2026-09-24 19:36:43, after pressing Revoke in the app:
  `day-2 revocation applied from the phone peer_id="886727a2…"` →
  `last bond revoked: stopping the daemon (nothing left to serve)` →
  `phase=Revoke`; `bonds.toml` rewritten at 19:36:43, the bond flipped to
  `revoked:phone: device dissociated`, `syauth list` shows no bonded peer and the
  daemon stopped cleanly. Reproduced twice (19:28:27 and 19:36:43).
  **Closed.**
- `peripheral::tests::a_revoke_frame_is_recognised_on_the_writable_peer_characteristic`
  (a `Revoke` frame is recognised; capability / heartbeat / RSSI / truncated
  frames keep their own paths).
- Workspace: 526 passed, 0 failed. `cargo clippy -p syauth-transport -- -D warnings` clean.
- `SyauthCompanionServiceTest.a_revoke_still_sends_when_the_bond_file_was_already_deleted`
  (red before the app-side fix, green after).
- `PairRejectionMessageTest` (4 tests) and the Android suite: 173/173.
- `python3 tests/settings_system_status.py -v` — 12/12.
- `make lint` — `cargo fmt --check` still reports pre-existing drift in 6
  unrelated `crates/` files (none touched by this bug); raised with the
  operator rather than silently reformatting the tree.
- Consistency requirement still open: no bond / no live phone must not leave
  phone login or proximity active (fix 3).

## Deferred finding (optimisation, not a defect)
Pairing works, but the phone can take ~20 s to come alive afterwards. Captured
2026-09-24 19:33:

```
19:33:24.039  services discovered status=0 n=6
19:33:24.039  W  required GATT attributes missing; retrying discovery   (x4)
19:33:27.573  services discovered status=0 n=6
              <- 13 s gap: DISCOVERY_RECOVERY_RETRY_MS (15 s)
19:33:40.751  start: opening autoConnect=true
19:33:43.569  services discovered status=0 n=6
```

The phone connects while the computer is still serving the *pair-mode* GATT
app: it finds services, but not the challenge/response pair, burns its retries
and then rebuilds the whole GATT client. Candidate directions: let the
"required attributes missing" case retry without the full recovery escalation,
and/or shorten that escalation wait. Not started — the operator asked to
optimise after the release path works.

## Traceability
- Captures: `/tmp/repair11_logcat.txt`, `/tmp/repair12_logcat.txt`,
  `/tmp/repair12_journal.txt`, `/tmp/revoke2_*`, `/tmp/revoke3_*`.
- Consistency requirement still open: no bond / no live phone must not leave
  phone login or proximity active (fix 3).
