# BUG-20260610: GUI pairing confirmation ownership

## Summary
GUI pairing reached Android `createBond(TRANSPORT_LE)` but BlueZ SMP timed out because the daemon and CLI both owned the default pairing agent.

## Reproduction
- Method: confirmed field trace plus regression tests.
- Evidence: Android reported `SMP_RSP_TIMEOUT`; Android CDM reported `ScanResult is not a subclass of BluetoothDevice`.

## Expected Behavior
`syauth-presenced` owns the BlueZ agent and GATT pair service. The GUI receives the numeric-comparison request and must explicitly confirm it.

## Actual Behavior
The daemon auto-accepted while `syauth pair` registered a competing default agent and GATT application.

## Root Cause Analysis
The two BlueZ agent registrations made request dispatch non-authoritative. The CDM result decoder also requested a `BluetoothDevice` from payloads that may contain a `ScanResult`.

## Fix
The daemon agent delegates confirmation through a private runtime socket with bounded timeout and closed failure. GUI mode is a socket client and no longer creates a BlueZ agent or GATT application. CDM extraction dispatches by runtime Parcelable type. The existing companion service is declared as a `CompanionDeviceService` with the required binding permission and action.

## Traceability
- Regression tests: `crates/syauth-transport/src/pairing.rs`
- Fixed files: `crates/syauth-transport/src/{pairing.rs,peripheral.rs}`, `crates/syauth-cli/src/{main.rs,pair_backend.rs}`, `syauth-android/app/src/main/{AndroidManifest.xml,kotlin/com/sy/syauth/android/bg/SyauthCompanionService.kt,kotlin/com/sy/syauth/android/pair/impl/AndroidCdmPairCompanionScanner.kt}`
