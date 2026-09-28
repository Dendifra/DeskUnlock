#!/usr/bin/env bash
# Fast tracked-tree privacy gate. This runs under `make lint`; the deeper
# security-privacy-deep-audit.sh also scans history, refs and release assets.
# Set SYAUTH_AUDIT_USER explicitly for a local-login check; CI leaves it empty.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

AUDIT_USER="${SYAUTH_AUDIT_USER:-}"
BOND_KEY_HEX_RE="bond_key_hex[[:space:]]*[=:][[:space:]]*[\\\"']?[0-9a-fA-F]{64}[\\\"']?"
fail=0

report() {
    local title="$1" body="$2"
    [[ -z "$body" ]] && return 0
    echo "PRIVACY: $title" >&2
    printf '%s\n' "$body" | sed 's/^/  /' >&2
    fail=1
}

macs="$(git grep -hoEi '\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b' -- . 2>/dev/null |
        grep -Ev '^(AA|BB|CC|DD|00|11|02):' | sort -u || true)"
report "real-looking device address in the tracked tree:" "$macs"

homes="$(git grep -nE '/home/[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+' -- . 2>/dev/null |
         grep -v '^scripts/security-privacy-deep-audit.sh:' |
         grep -Ev '\$(root|ROOT)/home/\.config|/home/(user|UID)([^A-Za-z0-9_.-]|$)|/Users/(user|example)([^A-Za-z0-9_.-]|$)' || true)"
report "personal-looking home directory in the tracked tree:" "$homes"

mounts="$(git grep -nE '/mnt/(Dati|Backups|GoogleDrive)(/|$)|/run/media/[A-Za-z0-9_.-]+/' -- . 2>/dev/null || true)"
report "machine-specific mount/backup path in the tracked tree:" "$mounts"

if [[ -n "$AUDIT_USER" && "$AUDIT_USER" != "root" && "$AUDIT_USER" != "user" ]]; then
    user_hits="$(git grep -nEI "(^|[^[:alnum:]_])${AUDIT_USER}([^[:alnum:]_]|$)|/home/${AUDIT_USER}(/|$)" -- . 2>/dev/null || true)"
    report "local developer login appears in the tracked tree:" "$user_hits"
fi

serials="$(git grep -nE '\bR[0-9][A-Z0-9]{8,12}\b' -- . 2>/dev/null || true)"
report "device serial in tracked tree:" "$serials"

identifiers="$(git grep -nE "(peer_id|peer-id)[=: ]+[0-9a-fA-F]{24,64}|${BOND_KEY_HEX_RE}|syauth\.ed25519\.[A-Za-z0-9._-]{8,}" -- README.md SECURITY.md CHANGELOG.md docs specs 2>/dev/null |
               grep -Ev 'syauth\.ed25519\.(peer-xyz|AABBCCDDEE01)' || true)"
report "real-looking peer/key identifier in public documentation:" "$identifiers"

keys="$(git grep -lE -- '-----BEGIN (RSA |OPENSSH |EC |PGP )?PRIVATE KEY-----' -- . 2>/dev/null || true)"
report "private-key material marker is tracked:" "$keys"

sensitive_names="$(git ls-files | grep -Ei '(^|/)(\.env($|\.)|.*\.(pem|p12|pfx|key|cred)$|bonds\.toml$|.*private.*key.*)' || true)"
report "sensitive-looking filename is tracked:" "$sensitive_names"

if [[ "$fail" == "0" ]]; then
    echo "privacy-check: clean"
fi
exit "$fail"
