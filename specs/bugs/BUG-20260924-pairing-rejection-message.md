# BUG-20260924-pairing-rejection-message: a refused pairing told the operator nothing

## Summary
Starting the pairing from the phone while the computer's pairing is not armed
makes the desktop publish `Reject` and close the session (SPEC §6 T-004, by
design). The phone then rolled the CDM association back and showed nothing the
operator could act on — "non succede nulla e poi non associa" (2026-09-24).

## Evidence
- Desktop journal, 2026-09-24 18:57:49:
  `pair engine published operation operation=Capability` →
  `pair engine received peer operation operation=Capability` →
  `pair engine published operation operation=Reject` →
  `pair session rejected by operator peer=AA:BB:CC:DD:EE:11`.
- `pair-confirm.sock` existed but had **no listener** (stale inode from a closed
  pairing window), so `PairingBroker::exchange` failed closed. Correct refusal,
  useless message.
- The phone could not name the cause: `RealPairBackend.waitFor` collapsed every
  terminal operation into `null`, so the OOB round reported the generic
  `remote confirmation failed`.

## Root cause
The reason string is produced where the information is lost. `waitFor` reads the
peer's `Reject` and returns `null`; the caller then reports a timeout-flavoured
sentence. Nothing downstream can recover "the computer refused".

## Fix
- `RealPairBackend.confirmationFailureReason(lastPeerOperation)` keeps `Reject`
  distinct from a timeout, and produces `PEER_REJECTED_REASON`
  (`pair/api/PairBackend.kt`).
- `PairingScreen.failure_message(reason)` turns that identifier into the next
  action: *open DeskUnlock on the computer and press "Associa telefono", then try
  again*.

## Not a deviation
T-004 is untouched: the computer still refuses an un-armed inbound request. The
change only stops the refusal from being a silent no-op. The phone-initiated
flow stays: one arming click on the computer, then everything from the app.

## Verification
- `PairRejectionMessageTest` (4 tests): a refusal keeps its meaning, a timeout
  is not reported as a refusal, and each maps to the right screen text.
- Android suite: 173/173.
