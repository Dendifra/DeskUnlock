#!/usr/bin/env bash
# Deep, read-only security/privacy audit for the public DeskUnlock repository.
# It changes no files, refs, releases, or system state.
#
# Usage:
#   bash scripts/security-privacy-deep-audit.sh [release-tag]
# Optional local identity probe:
#   SYAUTH_AUDIT_USER=<login> bash scripts/security-privacy-deep-audit.sh

set -uo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || {
    echo "ERROR: run inside the DeskUnlock Git repository" >&2
    exit 2
}
cd "$ROOT" || exit 2

TAG="${1:-$(git tag --sort=-version:refname | head -1)}"
AUDIT_USER="${SYAUTH_AUDIT_USER:-}"
CURRENT_BRANCH="$(git branch --show-current)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PASS=0
WARN=0
FAIL=0
ok()   { printf 'PASS  %s\n' "$*"; PASS=$((PASS+1)); }
warn() { printf 'WARN  %s\n' "$*"; WARN=$((WARN+1)); }
bad()  { printf 'FAIL  %s\n' "$*"; FAIL=$((FAIL+1)); }
section() { printf '\n===== %s =====\n' "$*"; }
print_hits() { [[ -s "$1" ]] && sed -n '1,80p' "$1" | sed 's/^/  /'; }

HISTORY="$TMP/history.txt"
git log --all --decorate=full \
    --format='commit %H%nAuthor: %an <%ae>%nCommitter: %cn <%ce>%n%B' \
    -p --no-ext-diff --text >"$HISTORY" 2>/dev/null || true

section "Repository / refs"
printf 'HEAD: %s\n' "$(git rev-parse --short=12 HEAD)"
printf 'branch: %s\n' "${CURRENT_BRANCH:-<detached>}"
printf 'release tag: %s\n' "${TAG:-<none>}"

[[ -z "$(git status --porcelain)" ]] && ok "working tree clean" || warn "working tree is dirty"

if [[ -n "$TAG" ]] && git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    ok "release tag $TAG exists"
    if git show-ref --verify --quiet refs/remotes/origin/master; then
        if git merge-base --is-ancestor "$TAG^{commit}" origin/master 2>/dev/null; then
            ok "$TAG is contained in origin/master"
        else
            bad "$TAG is not contained in origin/master"
        fi
    fi
else
    warn "release tag unavailable"
fi

if git show-ref --verify --quiet refs/remotes/origin/master; then
    OUT="$TMP/unmerged-refs.txt"
    git for-each-ref --format='%(refname:short)' refs/remotes/origin/ | while read -r ref; do
        [[ "$ref" == "origin/master" || "$ref" == "origin/HEAD" ]] && continue
        [[ -n "$CURRENT_BRANCH" && "$ref" == "origin/$CURRENT_BRANCH" ]] && continue
        git merge-base --is-ancestor "$ref" origin/master 2>/dev/null || printf '%s\n' "$ref"
    done >"$OUT"
    if [[ -s "$OUT" ]]; then
        print_hits "$OUT"
        warn "public remote refs exist outside master history"
    else
        ok "no stale public remote refs outside master history"
    fi
fi

section "Tracked-tree privacy"

OUT="$TMP/home.txt"
git grep -nE '/home/[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+' -- . 2>/dev/null \
  | grep -Ev '\$root/home/\.config|/home/(user|UID)([^A-Za-z0-9_.-]|$)|/Users/(user|example)([^A-Za-z0-9_.-]|$)' >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "personal-looking home paths found"; else ok "no personal-looking home paths"; fi

OUT="$TMP/mounts.txt"
git grep -nE '/mnt/(Dati|Backups|GoogleDrive)(/|$)|/run/media/[A-Za-z0-9_.-]+/' -- . 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "machine-specific mount/backup paths found"; else ok "no machine-specific mount/backup paths"; fi

if [[ -n "$AUDIT_USER" && "$AUDIT_USER" != "root" && "$AUDIT_USER" != "user" ]]; then
    OUT="$TMP/user.txt"
    git grep -nEI "(^|[^[:alnum:]_])${AUDIT_USER}([^[:alnum:]_]|$)|/home/${AUDIT_USER}(/|$)" -- . 2>/dev/null >"$OUT" || true
    if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "configured local developer login appears in tracked tree"; else ok "configured local developer login absent"; fi
fi

OUT="$TMP/serials.txt"
git grep -nE '\bR[A-Z0-9]{9,13}\b' -- README.md SECURITY.md CHANGELOG.md docs specs 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "device-serial-shaped identifiers found in public docs"; else ok "no device serials in public docs"; fi

OUT="$TMP/private-identifiers.txt"
git grep -nE '(peer_id|peer-id)[=: ]+[0-9a-fA-F]{24,64}|bond_key_hex[=: ]+[0-9a-fA-F]{64}|syauth\.ed25519\.[A-Za-z0-9._-]{8,}' -- README.md SECURITY.md CHANGELOG.md docs specs 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "real-looking peer/key identifiers found in public docs"; else ok "no real-looking peer/key identifiers in public docs"; fi

OUT="$TMP/macs.txt"
git grep -hoEi '\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b' -- . 2>/dev/null \
  | grep -Ev '^(AA|BB|CC|DD|00|11|02):' | sort -u >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "real-looking MAC addresses found"; else ok "no real-looking MAC addresses"; fi

OUT="$TMP/private-key.txt"
git grep -nE -- '-----BEGIN (RSA |OPENSSH |EC |PGP )?PRIVATE KEY-----' -- . 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "private-key material marker found"; else ok "no private-key material marker"; fi

OUT="$TMP/secrets.txt"
git grep -nEI '(api[_-]?key|access[_-]?token|refresh[_-]?token|secret[_-]?key|client[_-]?secret|password)[[:space:]]*[:=][[:space:]]*["'\'' ]?[A-Za-z0-9_+/.=-]{12,}' -- . 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "credential-like literal assignments found; inspect manually"; else ok "no obvious literal credential assignments"; fi

OUT="$TMP/sensitive-names.txt"
git ls-files | grep -Ei '(^|/)(\.env($|\.)|.*\.(pem|p12|pfx|key|cred)$|bonds\.toml$|.*private.*key.*)' >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "sensitive-looking filenames are tracked"; else ok "no sensitive-looking tracked filenames"; fi

section "Reachable Git history privacy"

history_fail() {
    local label="$1" regex="$2" out="$TMP/h-$RANDOM.txt"
    grep -nE "$regex" "$HISTORY" >"$out" 2>/dev/null || true
    if [[ -s "$out" ]]; then print_hits "$out"; bad "$label remains reachable in history"; else ok "$label absent from reachable history"; fi
}

history_fail "machine-specific mount/backup path" '/mnt/(Dati|Backups|GoogleDrive)(/|$)|/run/media/[A-Za-z0-9_.-]+/'
history_fail "private-key material" '-----BEGIN (RSA |OPENSSH |EC |PGP )?PRIVATE KEY-----'
history_fail "device serial" '\bR[A-Z0-9]{9,13}\b'
history_fail "peer/key identifier" '(peer_id|peer-id)[=: ]+[0-9a-fA-F]{24,64}|bond_key_hex[=: ]+[0-9a-fA-F]{64}|syauth\.ed25519\.[A-Za-z0-9._-]{8,}'

if [[ -n "$AUDIT_USER" && "$AUDIT_USER" != "root" && "$AUDIT_USER" != "user" ]]; then
    history_fail "configured local developer login" "/home/${AUDIT_USER}(/|$)|(^|[^[:alnum:]_])${AUDIT_USER}([^[:alnum:]_]|$)"
fi

OUT="$TMP/history-macs.txt"
grep -Eio '\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b' "$HISTORY" 2>/dev/null \
  | grep -Ev '^(AA|BB|CC|DD|00|11|02):' | sort -u >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "real-looking MAC addresses remain reachable in history"; else ok "no real-looking MAC addresses in reachable history"; fi

OUT="$TMP/history-email.txt"
git log --all --format='%ae%n%ce' | sort -u \
  | grep -Ev '(^$|@users\.noreply\.github\.com$|^noreply@github\.com$|@example\.(com|org)$|@local$)' >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "non-noreply author/committer email remains in Git metadata"; else ok "Git metadata contains only noreply/example/local addresses"; fi

section "Secret scanner"
if command -v gitleaks >/dev/null 2>&1; then
    if gitleaks git --redact --no-banner --exit-code 1 . >"$TMP/gitleaks.txt" 2>&1; then
        ok "gitleaks reports no reachable secrets"
    else
        print_hits "$TMP/gitleaks.txt"
        bad "gitleaks reported findings"
    fi
else
    warn "gitleaks not installed; project-specific secret checks ran"
fi

section "GitHub Actions supply chain"
OUT="$TMP/unpinned.txt"
git grep -nE 'uses:[[:space:]]*[^[:space:]]+@' -- '.github/workflows/*.yml' '.github/workflows/*.yaml' 2>/dev/null \
  | grep -Ev '@[0-9a-f]{40}([[:space:]#]|$)' >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "GitHub Actions are not all pinned to immutable SHAs"; else ok "GitHub Actions pinned to immutable SHAs"; fi

OUT="$TMP/write-perms.txt"
git grep -nE 'contents:[[:space:]]*write|actions:[[:space:]]*write|id-token:[[:space:]]*write' -- '.github/workflows/*.yml' '.github/workflows/*.yaml' 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "elevated workflow permissions present; verify necessity"; else ok "no elevated workflow write permission"; fi

section "Published release"
if [[ -z "$TAG" ]]; then
    warn "release audit skipped: no tag"
elif ! command -v gh >/dev/null 2>&1; then
    warn "release audit skipped: gh unavailable"
else
    REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
    mkdir -p "$TMP/release"
    if [[ -n "$REPO" ]] && gh release download "$TAG" --repo "$REPO" --dir "$TMP/release" --clobber >/dev/null 2>&1; then
        ok "downloaded published release $TAG"
        if [[ -f "$TMP/release/SHA256SUMS" ]] && (cd "$TMP/release" && sha256sum -c SHA256SUMS >/dev/null 2>&1); then
            ok "published SHA256SUMS verifies"
        else
            bad "published checksums missing or invalid"
        fi

        APK="$(find "$TMP/release" -maxdepth 1 -type f -name '*.apk' | head -1)"
        if [[ -n "$APK" ]]; then
            mkdir -p "$TMP/apk"
            unzip -q -o "$APK" -d "$TMP/apk" >/dev/null 2>&1 || true
            OUT="$TMP/apk-paths.txt"
            find "$TMP/apk" -type f -size -50M -exec strings {} + 2>/dev/null \
              | grep -E '/home/[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+|/mnt/(Dati|Backups|GoogleDrive)(/|$)' \
              | grep -Ev '^/home/matthias/src/jnalib/' \
              | sort -u >"$OUT" || true
            if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "published APK embeds project-maintainer machine paths"; else ok "published APK has no project-maintainer machine paths"; fi

            if find "$TMP/apk" -type f -size -50M -exec strings {} + 2>/dev/null | grep -q '^/home/matthias/src/jnalib/'; then
                warn "APK contains a known third-party JNA build path; not maintainer-identifying, but reproducibility metadata remains"
            fi

            APKSIGNER="$(command -v apksigner || true)"
            if [[ -n "$APKSIGNER" ]] && "$APKSIGNER" verify --print-certs "$APK" >/dev/null 2>&1; then
                ok "published APK signature verifies"
            else
                warn "apksigner unavailable or APK certificate could not be independently checked on this runner"
            fi
        else
            warn "release contains no APK"
        fi
    else
        bad "could not download published release $TAG"
    fi
fi

section "Tag authenticity"
if [[ -n "$TAG" ]] && git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    if [[ "$(git cat-file -t "$TAG" 2>/dev/null || true)" == "tag" ]] && \
       git cat-file -p "$TAG" | grep -qE '^-----BEGIN (PGP|SSH) SIGNATURE-----$'; then
        ok "$TAG is cryptographically signed"
    else
        warn "$TAG is not cryptographically signed"
    fi
fi

section "Summary"
printf 'PASS: %d  WARN: %d  FAIL: %d\n' "$PASS" "$WARN" "$FAIL"
if (( FAIL > 0 )); then
    echo "RESULT: FINDINGS — public repository is not privacy-clean yet."
    exit 1
fi
if (( WARN > 0 )); then
    echo "RESULT: no hard privacy failure; warnings require review."
    exit 0
fi
echo "RESULT: clean."
