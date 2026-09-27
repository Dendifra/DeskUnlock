#!/usr/bin/env bash
# Fast tracked-tree privacy gate. This runs under `make lint` and intentionally
# stays cheaper than `security-privacy-deep-audit.sh`, which scans all refs,
# history and published artifacts.
#
# Usage:
#   bash scripts/privacy-check.sh
# Optional local identity check:
#   SYAUTH_AUDIT_USER=<login> bash scripts/privacy-check.sh

set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

FICTIONAL_MAC='^(AA|BB|CC|DD|00|11|02):'
ALLOWED_HOME='^/home/(user|UID|\.config)(/|$)'
AUDIT_USER="${SYAUTH_AUDIT_USER:-$(id -un 2>/dev/null || true)}"
fail=0

report() {
    local title="$1"
    local body="$2"
    [[ -z "$body" ]] && return 0
    echo "PRIVACY: $title" >&2
    printf '%s\n' "$body" | sed 's/^/  /' >&2
    fail=1
}

macs="$(git grep -hoEi '\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b' -- . 2>/dev/null |
        grep -Ev "$FICTIONAL_MAC" | sort -u || true)"
report "real-looking device address in the tracked tree:" "$macs"

homes="$(git grep -hoE '/home/[A-Za-z0-9_.-]+(/[^[:space:]"'\''`)]*)?' -- . 2>/dev/null |
         grep -Ev "$ALLOWED_HOME" | sort -u || true)"
report "personal-looking home directory in the tracked tree:" "$homes"

mounts="$(git grep -hoE '/mnt/(Dati|Backups|GoogleDrive)(/[^[:space:]"'\''`)]*)?|/run/media/[A-Za-z0-9_.-]+(/[^[:space:]"'\''`)]*)?' -- . 2>/dev/null |
          sort -u || true)"
report "machine-specific mount/backup path in the tracked tree:" "$mounts"

if [[ -n "$AUDIT_USER" && "$AUDIT_USER" != "root" && "$AUDIT_USER" != "user" ]]; then
    user_hits="$(git grep -nEI "(^|[^[:alnum:]_])${AUDIT_USER}([^[:alnum:]_]|$)|/home/${AUDIT_USER}(/|$)" -- . 2>/dev/null || true)"
    report "local developer login appears in the tracked tree:" "$user_hits"
fi

# Samsung/Android serials commonly start with R and are long uppercase/digit
# identifiers. The pattern is intentionally limited to public docs/specs to
# avoid confusing hashes/constants in source code with device IDs.
serials="$(git grep -nE '\bR[A-Z0-9]{9,13}\b' -- README.md SECURITY.md CHANGELOG.md docs specs 2>/dev/null || true)"
report "device-serial-shaped identifier in public documentation:" "$serials"

keys="$(git grep -lE 'BEGIN (RSA |OPENSSH |EC |PGP |)?PRIVATE KEY|BEGIN PRIVATE KEY' -- . 2>/dev/null || true)"
report "private-key material marker is tracked:" "$keys"

sensitive_names="$(git ls-files | grep -Ei '(^|/)(\.env($|\.)|.*\.(pem|p12|pfx|key|cred)$|bonds\.toml$|.*private.*key.*)' || true)"
report "sensitive-looking filename is tracked:" "$sensitive_names"

if [[ "$fail" == "0" ]]; then
    echo "privacy-check: clean"
fi
exit "$fail"
