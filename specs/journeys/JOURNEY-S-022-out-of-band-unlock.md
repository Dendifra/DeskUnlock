# JOURNEY-S-022 — Out-of-band unlock (`syauth unlock-request`)

## Why this exists

`pam_syauth.so` was wired into the **lock** PAM service (`/etc/pam.d/dankshell`)
so the lock screen could be opened with the phone. On 2026-09-23 that failed
the way PAM integration always fails when the second factor is asynchronous:
the module holds the authentication phase while it waits for the phone, the
lock screen's own prompt is starved, and the operator could not get back in
**even with the correct password**. Restarting the daemon "fixed" it only
because a dead daemon fails the module instantly.

A lock screen must never depend on our process being healthy. This journey
moves the unlock **out of band**: the lock PAM service goes back to stock,
the password path is never touched, and the phone can only ever cause one
action — `loginctl unlock-session`.

## Scope

**In**

- `syauth unlock-request [--socket PATH] [--bond-dir DIR] [--peer-id ID]
  [--timeout-secs N] [--dry-run]`.
- Peer selection identical to `pam_syauth`: newest `Bonded` record in
  `<bond-dir>/bonds.toml`.
- One `Request::Challenge` / `Response::Challenge` round trip on the daemon's
  Unix socket — the same wire shape the PAM module already uses. No new
  protocol, no new daemon endpoint.
- On `ok == true` only: `<loginctl> unlock-session`, one fixed argument, no
  shell.
- `--dry-run` stops after verification, for testing the lock-screen hook
  without unlocking anything.

**Out**

- Any PAM module in a lock service. This journey *removes* that integration.
- Any lock-screen patch. The trigger side (point 2 of the plan) is a separate
  change and lands only after this verb is proven.
- Session locking, screensaver control, or anything that could make the
  operator's access *worse* than stock.

## Safety contract

| Situation | Behaviour |
|---|---|
| Phone approves, session locked | `loginctl unlock-session`, exit 0 |
| Phone refuses (`denied`, `replay`, `offline`, …) | nothing happens, exit ≠ 0 |
| Daemon down / socket missing | nothing happens, exit ≠ 0, stderr `daemon unreachable` |
| `--dry-run` | verification only, `loginctl` never spawned |
| Session already unlocked | `loginctl unlock-session` is a no-op |
| No bonded peer | refusal before any socket or process work |

The worst outcome of a failure is that the operator types their password on a
stock lock screen. `SYAUTH_LOGINCTL_BIN` exists so tests can prove the "was
`loginctl` called?" column without touching a live session.

## Test matrix

| TC | Scenario | Test |
|----|----------|------|
| 01 | verified phone → session unlocked | `unlocks_when_the_daemon_verifies_the_phone` |
| 02 | refused challenge → session untouched | `a_refused_challenge_leaves_the_session_untouched` |
| 03 | `--dry-run` never spawns the unlocker | `a_dry_run_never_runs_the_unlocker` |
| 04 | dead daemon → refusal, session untouched | `a_dead_daemon_is_a_refusal_not_an_unlock` |
| 05 | newest bonded peer is challenged | `the_challenge_targets_the_newest_bonded_peer` |
| 06 | peer selection: revoked newer bond skipped | `a_revoked_newer_bond_is_not_chosen` |
| 07 | empty store refused before any I/O | `an_empty_store_is_refused_before_touching_anything` |
| 08 | socket path shape shared with daemon + PAM | `the_default_socket_lives_under_the_runtime_dir` |

## Completion definition

- `cargo test -p syauth-cli` green (unit + integration).
- `cargo clippy -p syauth-cli --all-targets -- -D warnings` clean.
- `syauth unlock-request --dry-run` verified by hand against the live daemon
  with the phone present, **before** anything calls it automatically.
- `/etc/pam.d/dankshell` contains zero DeskUnlock lines, and the lock screen
  is proven to work with the password alone.

## Follow-ups (not this journey)

- Lock-screen trigger that calls this verb on an unlock attempt (point 2).
- Lock-screen indicator that appears **only** when a verified unlock is
  actually possible (point 3) — never an icon that lies.
- Revocation propagation between phone and desktop (point 4).
