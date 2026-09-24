#!/usr/bin/env bash
# SPEC-DEVIATION: DEV-006 — pam_syauth in the login greeter (plasmalogin) with
# `sufficient` weakens SPEC §3.2 D7 — see docs/known-gaps.md
#
# WHY: the operator wants phone unlock at the Plasma login screen (plasmalogin),
# exactly like the session lock screen, with manual password entry untouched.
# SPEC §3.2 D7 rejects `auth sufficient` in a login-guarding stack ("would
# weaken the stack"), and the 2026-09-23 lockout came from DeskUnlock sitting
# in such a stack. The operator approved the deviation on 2026-09-24.
#
# SAFETY: `sufficient` short-circuits only on success. Any failure — phone
# absent, denied, daemon down, timeout — falls through to the stock password
# stack (`auth include system-login`) immediately below, so manual login is
# never taken away. The module's `timeout=` bounds the wait so a missing phone
# cannot hang the greeter.
#
# RE-PAIR: the module resolves the daemon socket for the PAM user
# (`/run/user/<uid>/syauth/auth.sock`) and asks the daemon for the current
# bonded peer, so dissociating and re-associating (same or a different phone)
# needs no change here — the daemon reloads the new bond and the greeter asks
# the new device.
#
# Idempotent: a service that already carries the line is left alone.
set -euo pipefail

PAM_DIR="${SYAUTH_PAM_DIR:-/etc/pam.d}"
SERVICE="${SYAUTH_GREETER_SERVICE:-plasmalogin}"

if [[ "$(id -u)" -ne 0 ]]; then
    echo "enable-greeter-unlock: run as root (sudo)" >&2
    exit 1
fi

service_file="$PAM_DIR/$SERVICE"
if [[ ! -f "$service_file" ]]; then
    echo "enable-greeter-unlock: $service_file not found" >&2
    exit 1
fi

if grep -q "pam_syauth" "$service_file"; then
    echo "enable-greeter-unlock: $SERVICE already carries pam_syauth; nothing to do"
    exit 0
fi

syauth install-pam --service "$SERVICE" --pam-dir "$PAM_DIR" --with-presenced=false --yes
echo "enable-greeter-unlock: phone unlock armed at the $SERVICE greeter (password fallback preserved)"
