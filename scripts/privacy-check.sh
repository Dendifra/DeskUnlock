#!/usr/bin/env bash
# Refuse to commit the two things this project actually leaked once:
# a real device address, and the builder's home directory.
#
# Why a gate and not a review: both leaks were invisible in a diff. The addresses
# sat in evidence documents, one of them pasted from a journal by hand. The home
# path was not in any source file at all — it was inside the compiled libraries,
# and it reached a published APK before anyone saw it. Attention did not catch
# either; a mechanical check would have.
#
# This is a heuristic alarm, not a proof. The allowlist below is deliberately
# visible so that overriding it is a decision someone makes, not a rule someone
# works around.
#
# Usage: bash scripts/privacy-check.sh   (also wired into `make lint`)

set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

# Addresses the project uses as fixtures. Anything else looks like a real
# device and fails. A random real address beginning with one of these still
# slips through: that is the cost of a check that does not ask a human.
FICTIONAL_MAC='^(AA|BB|CC|DD|00|11|02):'
# The placeholders this project uses. Everything else in /home/ is a person,
# and a person's directory name does not belong in a public repository.
# `/home/.config` is not a person: it is the tail of the relative test path
# "$root/home/.config", and the regex cannot tell it apart from an absolute one.
ALLOWED_HOME='^/home/(user|UID|\.config)'

fail=0

macs="$(git grep -hoEi '\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b' -- . 2>/dev/null |
        grep -Ev "$FICTIONAL_MAC" | sort -u || true)"
if [[ -n "$macs" ]]; then
    echo "PRIVACY: real-looking device address in the tracked tree:" >&2
    echo "$macs" | sed 's/^/  /' >&2
    echo "  use AA:BB:CC:DD:EE:xx for fixtures, or add the range above if it is synthetic" >&2
    fail=1
fi

homes="$(git grep -hoE '/home/[A-Za-z0-9_.-]+' -- . 2>/dev/null |
         grep -Ev "$ALLOWED_HOME" | sort -u || true)"
if [[ -n "$homes" ]]; then
    echo "PRIVACY: a personal home directory in the tracked tree:" >&2
    echo "$homes" | sed 's/^/  /' >&2
    echo "  use /home/user in examples" >&2
    fail=1
fi

keys="$(git grep -lE 'BEGIN (RSA|OPENSSH|EC|PGP|PRIVATE) PRIVATE KEY' -- . 2>/dev/null || true)"
if [[ -n "$keys" ]]; then
    echo "PRIVACY: a private key is tracked:" >&2
    echo "$keys" | sed 's/^/  /' >&2
    fail=1
fi

if [[ "$fail" == "0" ]]; then
    echo "privacy-check: clean"
fi
exit "$fail"
