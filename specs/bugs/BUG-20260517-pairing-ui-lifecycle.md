# BUG-20260517: Pairing UI lifecycle

## Summary
The desktop pairing dialog sent cancellation and closed before backend cleanup, while successful pairing lacked a distinct final action; Android also exposed persistence at the UI state-machine boundary.

## Reproduction
- Method: focused desktop unit tests and Android JVM tests.
- Tests: `desktop/tests/test_pairing_dialog.py`; `PairingViewModelTest` and `PairingScreenTest`.
- Evidence: the focused tests assert single structured cancellation, bounded fallback cleanup, bonded final action, and pre-commit Android cancellation.

## Expected Behavior
Cancellation sends one protocol cancel and waits for backend completion. A real BONDED event shows a final action without writing a bond from that action. Home refreshes from the persisted bond.

## Actual Behavior
The desktop dialog synchronously waited and then accepted, had no separate success action, and Android's ViewModel owned the bond persistence callback.

## Root Cause Analysis
The desktop dialog conflated cancellation, process termination, and dialog closure. Its bonded branch renamed the cancel button instead of presenting a terminal success state. Android injected the bond writer into the ViewModel rather than keeping persistence behind the pairing backend's coordinated commit boundary.

## Fix
Desktop cancellation is one-shot, asynchronous, bounded, and cleaned up on process exit; forced kill is timeout-only. BONDED hides the transactional actions and exposes `Fine`. Android routes committed persistence through `PairBackend.persistBond`, adds a pre-commit cancel action for OOB confirmation, and renders `Computer associato` / `Fine`.

## Traceability
- Failing behavior coverage: `desktop/tests/test_pairing_dialog.py` and Android pairing tests.
- Fixed files: `desktop/bin/syauth-settings`, `desktop/tests/test_pairing_dialog.py`, `syauth-android/app/src/main/kotlin/com/sy/syauth/android/MainActivity.kt`, `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/{PairingScreen.kt,PairingViewModel.kt}`, `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/api/PairBackend.kt`, `syauth-android/app/src/main/kotlin/com/sy/syauth/android/pair/impl/RealPairBackend.kt`, and focused Android tests.
