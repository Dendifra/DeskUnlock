# BUG-20260926-2318: BlueZ transient startup failure leaves the daemon socket-only

## Summary
If BlueZ has not exposed the adapter when `syauth-presenced` starts, the daemon keeps the PAM socket alive but never retries GATT initialization.

## Reproduction
- Method: manual runtime reproduction on the current desktop boot.
- Command: `journalctl --user -u syauth-presenced.service --since '12 hours ago' --no-pager`
- Evidence:
  - `23:12:24` — `orchestrator not started; daemon will serve socket only`.
  - Reason: `BlueZ adapter open failed: peripheral backend error: adapter set_powered: the target object was either not present or removed`.
  - `23:12:25` — `syauth-check-orchestrator: orchestrator FALLITO`.
  - The service remains active, but no later GATT registration occurs.
  - Android logcat shows repeated `liveness: no GATT callback ... forcing reconnect`; the phone process remains alive while the desktop has no GATT peripheral.

## Expected Behavior
A transient BlueZ adapter absence is retried and the GATT peripheral becomes available without restarting the user service or reopening the PAM socket. The DMS readiness marker is also recreated after a daemon restart so the GUI returns to green.

## Actual Behavior
The one-shot initialization returns `(None, None, None)` and the daemon never attempts backend initialization again.

## Root Cause Analysis
`run()` starts backend initialization once. `maybe_spawn_orchestrator()` returned immediately when `PersistentPeripheral::new()` saw the transient BlueZ error, and the published `BackendState` stayed empty for the lifetime of the daemon. The PAM socket therefore remained healthy while the desktop GATT peripheral never returned.

## Fix
BlueZ peripheral initialization now retries every five seconds and exits the retry wait immediately on daemon shutdown. The socket remains available while Bluetooth comes up, and no manual service restart is required. The DMS patcher now rewrites its loaded-tree marker on every successful active-tree pass, without restarting DMS when the tree is already adapted; this prevents a presenced/runtime restart from leaving the GUI stuck on `Riavvia DMS`.

## Traceability
- Failing test: `runtime::day2_revoke_tests::bluez_retry_wait_stops_when_shutdown_arrives` (the shutdown path must interrupt the retry wait).
- Fixed in: `crates/syauth-presenced/src/runtime.rs`.
