# BUG-20260926-172757: Plasma greeter waits for phone backend readiness

## Summary
On a clean boot, Plasma Login becomes visible before the DeskUnlock PAM backend is ready, so phone authentication is unavailable during the first part of the login screen.

## Reproduction
- Method: live boot journal inspection; no session was started by the probe.
- Commands:
  - `journalctl -b -u user@1000.service --no-pager -o short-monotonic`
  - `journalctl -b --no-pager -o short-monotonic | grep -E 'plasmalogin|syauth-presenced|bluetooth'`
- Evidence:
  - Plasma Login starts at monotonic `20.200s`.
  - `syauth-presenced` user unit starts at `9.916s` but blocks in `syauth-wait-bluez`.
  - Bluetooth becomes active at `36.141s`.
  - The authentication socket listens at `37.065s`.
  - The measured gap between the greeter and the authentication socket is about `16.9s`.

## Expected Behavior
After installation, the phone authentication path is usable when the Plasma greeter is available, while the normal password fallback remains usable if Bluetooth or the phone is unavailable.

## Actual Behavior
The greeter is visible while `pam_syauth.so` has no ready per-user socket, so an early phone-authentication attempt falls through as unavailable.

## Root Cause Analysis
The packaged user daemon is enabled by the user manager but its `ExecStartPre` waits for two consecutive `Powered: yes` responses from `bluetoothctl`. On this boot, the system Bluetooth service is delayed by the local-filesystem dependency chain, while `plasmalogin.service` starts earlier. The PAM socket is therefore created after the greeter is already interactive.

## Fix
Make the authentication endpoint available before Bluetooth is ready and retain the BLE/orchestrator readiness as an explicit state. PAM requests received during the radio startup window must wait for the bounded backend readiness deadline instead of being rejected immediately. The installation path must enable the user manager lifecycle and configure the Plasma PAM integration idempotently for a fresh install.

## Traceability
- Failing evidence: boot journal timestamps above.
- Regression tests: to be added under `desktop/tests/` and the daemon/PAM integration suite.
- Fixed in: pending.
