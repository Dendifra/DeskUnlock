# BUG-20260924-app-reconnect-after-outage: phone never reconnects after a long desktop outage

## Summary
After the desktop was away for more than a few seconds (master OFF, a daemon
restart, a long boot), the Android `PersistentGattClient` spent its one-shot
discovery recovery and then went silent forever. When the desktop came back,
the phone did not reconnect on its own: no `presence.last` / `rssi.last`, no
subscription, proximity and unlock dead until the app was force-restarted.

## Reproduction
- Method: live Pixel 8 over USB (`adb logcat`) + desktop journal, with a
  controlled `syauth-control off` for 60 s then `syauth-control on`.
- Evidence (before fix):
  ```
  17:14:10 escalating incomplete discovery to one fresh GATT
  17:14:18 GATT discovery retry exhausted
  17:14:18 fresh GATT discovery recovery exhausted      ← gave up
  ... (no further client activity; presence/rssi MISSING)
  ```
- A plain daemon restart (`systemctl --user restart syauth-presenced`) always
  reconnected (`Notify event — phone subscribed`), so the bug needed an outage
  longer than the one-shot recovery could absorb.

## Expected Behavior
When the desktop returns, the phone reconnects within a bounded time with no
user action and no app restart.

## Actual Behavior
The client used its single fresh-GATT recovery, then `scheduleDiscoveryRetry`
logged "fresh GATT discovery recovery exhausted" and stopped retrying.

## Root Cause Analysis
`discoveryRecoveryReconnectUsed` gates the recovery to one fresh GATT handshake
per client lifetime. Once spent, the exhausted branch did nothing, and the
disconnect watchdog was cancelled (the link was up), so nothing ever retried.

## Fix
`PersistentGattClient`: on exhaustion, keep retrying a fresh handshake on a slow
cadence instead of stopping — `scheduleDiscoveryRecoveryRetry()` re-arms after
`DISCOVERY_RECOVERY_RETRY_MS` (15 s), clearing the one-shot flag so the next
cycle can escalate again. Cancelled on `stop`, `reconnectFresh` and a
successful discovery.

## Traceability
- Regression test:
  `PersistentGattClientTest::failed_fresh_discovery_keeps_retrying_on_a_slow_cadence`
  (replaces `failed_fresh_discovery_does_not_reconnect_again`, which pinned the
  buggy behaviour).
- Fixed file:
  `syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/PersistentGattClient.kt`
- Hardware verification (2026-09-24, master OFF 60 s → ON):
  ```
  17:31:02 fresh GATT discovery recovery exhausted; retrying in 15000ms
  17:31:17 start: opening autoConnect=true → conn state new=2
  17:31:19 services discovered → descriptor write → presence heartbeat armed
  17:31:18 desktop: Notify event — phone subscribed
  ```
