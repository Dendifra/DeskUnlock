# BUG-20260924-repair-proximity: dissociate + re-associate kills proximity lock

## Summary
After dissociating and re-associating the phone, the desktop never received a
GATT subscription again: no `presence.last` / `rssi.last`, `proximity.state`
stuck `ABSENT`, phone unlock dead. The desktop side re-registered the new
GATT app correctly; the Android companion service kept a stale client.

Two independent Android defects combined:

1. `SyauthCompanionService.injectClientsForBonds()` reconciled clients by
   `BondRecord.peerId`, which is the Bluetooth **MAC**. A real re-pair keeps
   the same MAC and only mints a new `bondKey` / `phonePubkey`, so the stale
   client (bound to the desktop's torn-down GATT registration) was never
   rebuilt.
2. Once rebuilt, the fresh client could connect (`STATE_CONNECTED`) and call
   `discoverServices()` but never receive `onServicesDiscovered` — a discovery
   wedge with no recovery path. The disconnect watchdog had been cancelled on
   `STATE_CONNECTED`, so nothing forced a fresh handshake.

## Reproduction
- Method: live Pixel 8 over USB (`adb logcat`) + desktop journal, during a
  GUI-driven dissociate + re-associate.
- Evidence (first run, before fix):
  - desktop: `bond added`, `registering GATT app for peer uuid=…`, `peers_after=1`;
  - app: `rebuilt client …` absent, `last_connect=never`;
  - desktop `presence.last` / `rssi.last` missing → `proximity ABSENT`.

## Expected Behavior
After a re-pair the phone rebuilds its GATT client for the new bond, connects,
subscribes, and resumes presence/RSSI; proximity returns to `NEAR` with no app
restart and no user action.

## Actual Behavior
The old client survived the re-pair under the unchanged MAC, stayed wedged
against the dead GATT registration, and the desktop saw no subscription.
A fresh client could additionally wedge in `discoverServices()`.

## Root Cause Analysis
`clients` is keyed by MAC, so a same-MAC re-pair is invisible to the
reconciliation. Separately, `discoverServices()` had no watchdog: retries only
ran from an `onServicesDiscovered` callback that, in the wedge, never fired.

## Fix
- `SyauthCompanionService`: track the bond each client was built from
  (`clientBonds`) and rebuild the client when the bond changes under the same
  MAC (drop + stop the stale client, create + start a fresh one).
- `PersistentGattClient`: add a discovery watchdog
  (`DISCOVERY_WATCHDOG_MS = 10_000`) armed on every `discoverServices()` and
  cancelled on `onServicesDiscovered`; if discovery stalls it forces a fresh
  GATT handshake.

## Traceability
- Regression tests:
  - `SyauthCompanionServiceTest::reload_rebuilds_client_when_the_same_mac_gets_a_new_bond`
  - `PersistentGattClientTest::discovery_watchdog_reconnects_when_services_discovered_never_fires`
- Fixed files:
  - `syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/SyauthCompanionService.kt`
  - `syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/PersistentGattClient.kt`
- Hardware verification (2026-09-24, second re-pair after fix):
  - app `rebuilt client for a re-paired bond peer=AA:BB:CC:DD:EE:10`
  - desktop `chal_control: Notify event — phone subscribed`
  - `presence.last age=1s`, `rssi.last age=0s`, `proximity state=NEAR`.
