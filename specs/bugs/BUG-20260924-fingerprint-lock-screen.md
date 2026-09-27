# BUG-20260924-fingerprint-lock-screen: DMS lock screen never loaded the DeskUnlock adaptation

## Summary
The DMS lock screen showed the stock lock icon (no fingerprint) and never asked
the phone on mouse movement or Enter. The GUI still reported "Sblocco con
impronta: Attivo". The unlock backend itself was healthy (`syauth
unlock-request` returned `reason=ok`), so the break was entirely in the lock
screen adaptation.

## Reproduction
- Method: live DMS session inspection (`/proc` watches, file mtimes, `qs -p`
  process) plus a manual DMS restart.
- Evidence:
  - `syauth-dms-lock-patch.timer` ran and the runtime tree was patched on disk
    (`requestPhoneUnlock` present, `fprintSuppressedByPrimaryPam` present);
  - the running `qs -p /run/user/1000/danklinux-shell/<hash>` had started
    *before* the patch wrote the files (tree mtime 13:06:22, `qs` start
    13:05:53) and Quickshell holds **no** inotify watch on the QML files — it
    does not hot-reload;
  - after `systemctl --user restart dms.service`, the patched files survived
    (DMS skips re-extraction when the hash dir exists) and the lock screen
    loaded the adaptation.

## Expected Behavior
After DMS extracts its embedded UI, the lock screen runs the patched QML: the
fingerprint icon appears when the phone unlock is armed and Enter/mouse movement
runs `syauth unlock-request`.

## Actual Behavior
DMS extracted and loaded the stock QML before the patch timer ran, and never
reloaded it, so the adaptation was inert for the whole session.

## Root Cause Analysis
DMS embeds its Quickshell UI and extracts it to a hash-named runtime dir at
startup; it loads the QML once and does not watch the files. The patch timer
applies the adaptation *after* that load, so "patched on disk" never became
"loaded in the running shell".

## Fix
- `syauth-dms-lock-patch`: when it actually changes a freshly extracted tree,
  restart `dms.service` once so the patched QML is loaded. DMS skips
  re-extraction for an existing hash dir, so the next tick sees an
  already-adapted tree and does not restart again (no loop). Verified
  hermetic: `test_dms_lock_patch.sh` fakes `systemctl` and asserts exactly one
  restart on the first run and none on the second.
- `syauth-settings`: the "Sblocco con impronta" row now reports the real state
  via `fingerprint_unlock_health()` — master on, a bonded phone, a fully
  adapted tree, the runtime marker, and a DMS process that started after the
  patch — naming each failure instead of always "Attivo".

## Traceability
- Regression tests:
  - `desktop/tests/test_dms_lock_patch.sh` TC 02h / 04b (restart once, no loop)
  - `tests/settings_fingerprint_health.py` (each failure named)
- Fixed files:
  - `desktop/libexec/syauth-dms-lock-patch`
  - `desktop/bin/syauth-settings`
- Hardware verification (2026-09-24): after the restart the tree stayed patched,
  `fingerprint_unlock_health("on", 1)` returned `(True, "Attivo")`.
