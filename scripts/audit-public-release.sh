#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="${1:-.}"
ROOT="$(cd "$ROOT" && pwd)"
REPORT="$ROOT/PUBLICATION_AUDIT.md"
FAILS=0
WARNS=0

pass() { printf 'PASS  %s\n' "$*"; printf -- '- [x] PASS: %s\n' "$*" >>"$REPORT"; }
warn() { printf 'WARN  %s\n' "$*"; printf -- '- [ ] WARN: %s\n' "$*" >>"$REPORT"; WARNS=$((WARNS+1)); }
fail() { printf 'FAIL  %s\n' "$*"; printf -- '- [ ] FAIL: %s\n' "$*" >>"$REPORT"; FAILS=$((FAILS+1)); }

cat >"$REPORT" <<'EOF'
# DeskUnlock publication audit

Generated locally. This report intentionally avoids printing secret values.

EOF

echo "Auditing: $ROOT"

if [[ -f "$ROOT/LICENSE" ]] && grep -q 'MIT License' "$ROOT/LICENSE"; then
    pass "MIT license file present"
else
    fail "MIT license file missing or unexpected"
fi

if grep -q 'Copyright (c) 2026 syauth contributors' "$ROOT/LICENSE" 2>/dev/null; then
    pass "upstream syauth copyright notice preserved"
else
    fail "upstream syauth copyright notice not found in LICENSE"
fi

for f in README.md NOTICE.md SECURITY.md CONTRIBUTING.md THIRD_PARTY_LICENSES.md; do
    [[ -f "$ROOT/$f" ]] && pass "$f present" || fail "$f missing"
done

echo
echo "Privacy / identity scan"

# Search only source-ish files, skip Git/build caches and this report.
mapfile -t TEXTFILES < <(
    find "$ROOT" \
        -type f \
        -not -path '*/.git/*' \
        -not -path '*/target/*' \
        -not -path '*/build/*' \
        -not -name 'PUBLICATION_AUDIT.md' \
        -size -5M \
        -print 2>/dev/null
)

scan_pattern() {
    local label="$1"
    local regex="$2"
    local found=0
    local f
    for f in "${TEXTFILES[@]}"; do
        if LC_ALL=C grep -Iq . "$f" 2>/dev/null &&
           LC_ALL=C grep -Eq "$regex" "$f" 2>/dev/null; then
            printf '  MATCH %-28s %s\n' "$label" "${f#$ROOT/}"
            found=1
        fi
    done
    return "$found"
}

if scan_pattern "personal home/user" '(/home/${USER}|/Users/${USER}|(^|[^[:alnum:]_])${USER}([^[:alnum:]_]|$))'; then
    pass "no known developer username/home path detected"
else
    fail "developer username/home path detected"
fi

if scan_pattern "fixed uid 1000" '(/run/user/1000|uid[=: ]+1000|UID[=: ]+1000)'; then
    pass "no obvious fixed UID 1000 assumption detected"
else
    warn "possible fixed UID 1000 assumption detected; review matches"
fi

if scan_pattern "MAC address" '([[:xdigit:]]{2}:){5}[[:xdigit:]]{2}'; then
    pass "no MAC-address-shaped literal detected"
else
    warn "MAC-address-shaped literal detected; review test fixtures vs private device IDs"
fi

if scan_pattern "private key marker" 'BEGIN (RSA |EC |OPENSSH |)?PRIVATE KEY'; then
    pass "no private-key PEM marker detected"
else
    fail "private-key marker detected"
fi

if scan_pattern "common secret assignment" '(API[_-]?KEY|ACCESS[_-]?TOKEN|SECRET[_-]?KEY|PASSWORD)[[:space:]]*[:=][[:space:]]*[^$<{[:space:]]'; then
    pass "no obvious literal secret assignment detected"
else
    warn "possible literal credential assignment detected; inspect filenames only, never paste values publicly"
fi

echo
echo "Sensitive filenames"

SENSITIVE_NAMES="$(
    find "$ROOT" -type f \
        \( -name '.env' -o -name '.env.*' -o -name '*.pem' -o -name '*.p12' \
           -o -name '*.pfx' -o -name '*.key' -o -name '*.cred' \
           -o -name 'bonds.toml' -o -name '*private*key*' \) \
        -not -path '*/.git/*' -print 2>/dev/null || true
)"

if [[ -z "$SENSITIVE_NAMES" ]]; then
    pass "no common sensitive filenames found"
else
    warn "sensitive-looking filenames found; review before publication"
    while IFS= read -r f; do
        [[ -n "$f" ]] && printf '  FILE %s\n' "${f#$ROOT/}"
    done <<<"$SENSITIVE_NAMES"
fi

echo
echo "Large / binary artifacts"

LARGE="$(
    find "$ROOT" -type f -size +10M \
        -not -path '*/.git/*' \
        -not -path '*/target/*' \
        -printf '%P\n' 2>/dev/null || true
)"
if [[ -z "$LARGE" ]]; then
    pass "no unexpected files larger than 10 MiB"
else
    warn "large files found; review before Git commit"
    printf '%s\n' "$LARGE" | sed 's/^/  LARGE /'
fi

echo
echo "Rust dependency licensing"

if [[ -f "$ROOT/Cargo.toml" ]]; then
    if command -v cargo-deny >/dev/null 2>&1; then
        if (cd "$ROOT" && cargo deny check licenses); then
            pass "cargo-deny license check passed"
        else
            fail "cargo-deny license check failed"
        fi
    else
        warn "cargo-deny not installed; Rust dependency license audit still required"
    fi
else
    warn "Cargo.toml not found at repository root"
fi

echo
echo "Git state"
if [[ -d "$ROOT/.git" ]]; then
    if [[ -z "$(git -C "$ROOT" status --porcelain)" ]]; then
        pass "Git worktree clean"
    else
        warn "Git worktree has uncommitted/untracked changes"
    fi
else
    warn "staging tree has no .git metadata yet"
fi

echo
echo "Brand migration indicators"

OLD_REFS="$(
    grep -RIl --exclude-dir=.git --exclude-dir=target --exclude=PUBLICATION_AUDIT.md \
        -E '\bsyauth\b|pam_syauth|syauth-' "$ROOT" 2>/dev/null | wc -l
)"
printf '  files still containing syauth identifiers: %s\n' "$OLD_REFS"
if (( OLD_REFS > 0 )); then
    warn "syauth identifiers remain; classify each as upstream attribution, compatibility identifier, or rename target"
else
    pass "no syauth identifiers remain outside audit report"
fi

echo
echo "Desktop-specific dependencies"

if grep -RIl --exclude-dir=.git --exclude-dir=target --exclude=PUBLICATION_AUDIT.md \
    -E '\bdms\b|dms-shell|niri|plasmalogin' "$ROOT" >/dev/null 2>&1; then
    warn "desktop-specific integration references found; make support scope explicit"
else
    pass "no obvious DMS/Niri/Plasma-specific references detected"
fi

{
    echo
    echo "## Result"
    echo
    echo "- Failures: $FAILS"
    echo "- Warnings: $WARNS"
    echo
    if (( FAILS == 0 )); then
        echo "**Audit gate:** no hard failures. Warnings still require review before publication."
    else
        echo "**Audit gate:** NOT READY FOR PUBLICATION."
    fi
} >>"$REPORT"

echo
echo "Report: $REPORT"
echo "Failures: $FAILS  Warnings: $WARNS"

(( FAILS == 0 ))
