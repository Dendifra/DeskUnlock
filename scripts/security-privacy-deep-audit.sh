#!/usr/bin/env bash
# Deep, read-only security/privacy audit for a public DeskUnlock repository.
#
# It scans the tracked tree, every reachable Git ref/history, Git metadata,
# workflow supply-chain pins and (when gh is available) the published release.
# It never changes the repository, refs, releases, files, or system state.
#
# Usage:
#   bash scripts/security-privacy-deep-audit.sh [release-tag]
# Optional:
#   SYAUTH_AUDIT_USER=<local-login-to-detect> bash scripts/security-privacy-deep-audit.sh

set -uo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || {
    echo "ERROR: run inside the DeskUnlock Git repository" >&2
    exit 2
}
cd "$ROOT" || exit 2

TAG="${1:-$(git tag --sort=-version:refname | head -1)}"
AUDIT_USER="${SYAUTH_AUDIT_USER:-$(id -un 2>/dev/null || true)}"
HOST_SHORT="$(hostname -s 2>/dev/null || true)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAILS=0
WARNS=0
PASS=0

ok()   { printf 'PASS  %s\n' "$*"; PASS=$((PASS+1)); }
warn() { printf 'WARN  %s\n' "$*"; WARNS=$((WARNS+1)); }
bad()  { printf 'FAIL  %s\n' "$*"; FAILS=$((FAILS+1)); }
section() { printf '\n===== %s =====\n' "$*"; }

print_hits() {
    local file="$1"
    [[ -s "$file" ]] && sed -n '1,80p' "$file" | sed 's/^/  /'
}

# Build one textual view of every reachable diff. This catches strings that
# disappeared from HEAD but remain reachable from a branch or tag.
HISTORY="$TMP/history.txt"
git log --all --decorate=full --format='commit %H%nAuthor: %an <%ae>%nCommitter: %cn <%ce>%n%B' -p --no-ext-diff --text >"$HISTORY" 2>/dev/null || true

section "Repository / refs"
printf 'root: %s\n' "$ROOT"
printf 'HEAD: %s\n' "$(git rev-parse --short=12 HEAD)"
printf 'branch: %s\n' "$(git branch --show-current)"
printf 'release tag: %s\n' "${TAG:-<none>}"

if [[ -n "$(git status --porcelain)" ]]; then
    warn "working tree is dirty (audit remains read-only)"
else
    ok "working tree clean"
fi

if [[ -n "$TAG" ]] && git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    ok "release tag $TAG exists locally"
    if git show-ref --verify --quiet refs/remotes/origin/master; then
        if git merge-base --is-ancestor "$TAG^{commit}" origin/master 2>/dev/null; then
            ok "$TAG is contained in origin/master"
        else
            bad "$TAG is NOT contained in origin/master (published release is ahead/divergent from default branch)"
        fi
    fi
else
    warn "no release tag available for release checks"
fi

printf '\nRemote refs not merged into origin/master:\n'
if git show-ref --verify --quiet refs/remotes/origin/master; then
    UNMERGED="$TMP/unmerged.txt"
    git for-each-ref --format='%(refname:short)' refs/remotes/origin/ | while read -r ref; do
        [[ "$ref" == "origin/master" || "$ref" == "origin/HEAD" ]] && continue
        if ! git merge-base --is-ancestor "$ref" origin/master 2>/dev/null; then
            printf '%s\n' "$ref"
        fi
    done >"$UNMERGED"
    if [[ -s "$UNMERGED" ]]; then
        print_hits "$UNMERGED"
        warn "public remote refs exist outside master history; review/remove stale backup/WIP refs"
    else
        ok "all public remote refs are contained in origin/master"
    fi
fi

section "Tracked-tree privacy"

scan_git_grep() {
    local label="$1" regex="$2" out="$3"
    git grep -nEI "$regex" -- . 2>/dev/null >"$out" || true
    if [[ -s "$out" ]]; then
        print_hits "$out"
        bad "$label found in tracked tree"
    else
        ok "no $label in tracked tree"
    fi
}

# Known local login/host are contextual rather than hard-coded into the repo.
if [[ -n "$AUDIT_USER" && "$AUDIT_USER" != "root" && "$AUDIT_USER" != "user" ]]; then
    OUT="$TMP/tree-user.txt"
    git grep -nEI "(^|[^[:alnum:]_])${AUDIT_USER}([^[:alnum:]_]|$)|/home/${AUDIT_USER}(/|$)|/Users/${AUDIT_USER}(/|$)" -- . 2>/dev/null >"$OUT" || true
    if [[ -s "$OUT" ]]; then
        print_hits "$OUT"
        bad "local developer username/home path found in tracked tree"
    else
        ok "no local developer username/home path in tracked tree"
    fi
fi

if [[ -n "$HOST_SHORT" && ${#HOST_SHORT} -ge 4 ]]; then
    OUT="$TMP/tree-host.txt"
    git grep -nF "$HOST_SHORT" -- . 2>/dev/null >"$OUT" || true
    if [[ -s "$OUT" ]]; then
        print_hits "$OUT"
        warn "local hostname appears in tracked tree"
    else
        ok "local hostname absent from tracked tree"
    fi
fi

OUT="$TMP/tree-home.txt"
git grep -nE '/home/[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+' -- . 2>/dev/null \
    | grep -Ev '/home/(user|UID|\.config)(/|$)|/Users/(user|example)(/|$)' >"$OUT" || true
if [[ -s "$OUT" ]]; then
    print_hits "$OUT"
    bad "personal-looking home paths found in tracked tree"
else
    ok "no personal-looking home paths in tracked tree"
fi

OUT="$TMP/tree-mounts.txt"
git grep -nE '/mnt/(Dati|Backups|GoogleDrive)(/|$)|/run/media/[A-Za-z0-9_.-]+/' -- . 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then
    print_hits "$OUT"
    bad "machine-specific mount/backup paths found in tracked tree"
else
    ok "no machine-specific mount/backup paths in tracked tree"
fi

OUT="$TMP/tree-mac.txt"
git grep -nEio '([[:xdigit:]]{2}:){5}[[:xdigit:]]{2}' -- . 2>/dev/null \
    | grep -Ev ':(AA|BB|CC|DD|00|11|02):|^(.*:)?(AA|BB|CC|DD|00|11|02):' >"$OUT" || true
if [[ -s "$OUT" ]]; then
    print_hits "$OUT"
    warn "MAC-address-shaped literals found; confirm every hit is a fixture"
else
    ok "no non-fixture MAC-address-shaped literals detected"
fi

# Common Android/Samsung-style serials. This is deliberately only a warning:
# hashes, fixture IDs and protocol constants can have similar shapes.
OUT="$TMP/tree-device-id.txt"
git grep -nE '\bR[A-Z0-9]{9,13}\b|\b[A-Z0-9]{12,16}\b' -- docs specs README.md SECURITY.md CHANGELOG.md 2>/dev/null \
    | grep -Ev '\b(SHA256|STRONGBOX|BLUETOOTH|DESKUNLOCK|SYAUTH|SPEC|ROADMAP|JOURNEY|ANDROID|PAM_AUTH|PAM_SUCCESS)\b' >"$OUT" || true
if [[ -s "$OUT" ]]; then
    print_hits "$OUT"
    warn "device/serial-like identifiers found in public documentation"
else
    ok "no obvious device/serial-like identifiers in public documentation"
fi

OUT="$TMP/tree-keys.txt"
git grep -nE 'BEGIN (RSA |OPENSSH |EC |PGP |)?PRIVATE KEY|BEGIN PRIVATE KEY' -- . 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "private-key material marker found"; else ok "no private-key material marker"; fi

OUT="$TMP/tree-secrets.txt"
git grep -nEI '(api[_-]?key|access[_-]?token|refresh[_-]?token|secret[_-]?key|client[_-]?secret|password)[[:space:]]*[:=][[:space:]]*["'\'' ]?[A-Za-z0-9_+/.=-]{12,}' -- . 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "credential-like literal assignments found; inspect each hit"; else ok "no obvious literal credential assignments"; fi

OUT="$TMP/tree-sensitive-names.txt"
git ls-files | grep -Ei '(^|/)(\.env($|\.)|.*\.(pem|p12|pfx|key|cred)$|bonds\.toml$|.*private.*key.*)' >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "sensitive-looking filenames are tracked"; else ok "no sensitive-looking tracked filenames"; fi

section "Reachable Git history privacy"

history_check() {
    local label="$1" regex="$2" mode="${3:-E}" out="$TMP/history-$RANDOM.txt"
    if [[ "$mode" == "F" ]]; then
        grep -nF "$regex" "$HISTORY" >"$out" 2>/dev/null || true
    else
        grep -nE "$regex" "$HISTORY" >"$out" 2>/dev/null || true
    fi
    if [[ -s "$out" ]]; then
        print_hits "$out"
        bad "$label remains reachable in Git history"
    else
        ok "$label absent from reachable Git history"
    fi
}

if [[ -n "$AUDIT_USER" && "$AUDIT_USER" != "root" && "$AUDIT_USER" != "user" ]]; then
    history_check "local developer home/login" "/home/${AUDIT_USER}(/|$)|(^|[^[:alnum:]_])${AUDIT_USER}([^[:alnum:]_]|$)"
fi
history_check "machine-specific mount/backup path" '/mnt/(Dati|Backups|GoogleDrive)(/|$)|/run/media/[A-Za-z0-9_.-]+/'
history_check "private-key marker" 'BEGIN (RSA |OPENSSH |EC |PGP |)?PRIVATE KEY|BEGIN PRIVATE KEY'

OUT="$TMP/history-macs.txt"
grep -Eio '([[:xdigit:]]{2}:){5}[[:xdigit:]]{2}' "$HISTORY" 2>/dev/null \
    | grep -Ev '^(AA|BB|CC|DD|00|11|02):' | sort -u >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "real-looking MAC addresses remain reachable in history"; else ok "no real-looking MAC addresses in reachable history"; fi

OUT="$TMP/history-device.txt"
grep -nE '\bR[A-Z0-9]{9,13}\b' "$HISTORY" 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "Android/device-serial-like identifiers remain reachable in history"; else ok "no obvious Android/device serial in reachable history"; fi

# Commit author/committer email is public by design, but a non-noreply personal
# address is worth surfacing before publication.
OUT="$TMP/history-email.txt"
git log --all --format='%ae%n%ce' | sort -u \
    | grep -Ev '(^$|@users\.noreply\.github\.com$|@example\.(com|org)$)' >"$OUT" || true
if [[ -s "$OUT" ]]; then print_hits "$OUT"; warn "non-noreply author/committer email present in Git metadata"; else ok "Git metadata uses noreply/example addresses only"; fi

section "Secret-scanner integration"
if command -v gitleaks >/dev/null 2>&1; then
    if gitleaks git --redact --no-banner --exit-code 1 . >"$TMP/gitleaks.txt" 2>&1; then
        ok "gitleaks reports no reachable secrets"
    else
        print_hits "$TMP/gitleaks.txt"
        bad "gitleaks reported findings"
    fi
else
    warn "gitleaks not installed; generic secret scan skipped (project-specific checks still ran)"
fi

section "GitHub Actions supply chain"
OUT="$TMP/unpinned-actions.txt"
git grep -nE 'uses:[[:space:]]*[^[:space:]]+@' -- '.github/workflows/*.yml' '.github/workflows/*.yaml' 2>/dev/null \
    | grep -Ev '@[0-9a-f]{40}([[:space:]#]|$)' >"$OUT" || true
if [[ -s "$OUT" ]]; then
    print_hits "$OUT"
    warn "GitHub Actions references are not pinned to immutable 40-hex SHAs"
else
    ok "GitHub Actions are pinned to immutable SHAs"
fi

OUT="$TMP/workflow-write.txt"
git grep -nE 'contents:[[:space:]]*write|actions:[[:space:]]*write|id-token:[[:space:]]*write' -- '.github/workflows/*.yml' '.github/workflows/*.yaml' 2>/dev/null >"$OUT" || true
if [[ -s "$OUT" ]]; then
    print_hits "$OUT"
    warn "workflow write permissions present; verify every one is necessary"
else
    ok "no elevated workflow write permission found"
fi

section "Published release"
if [[ -z "$TAG" ]]; then
    warn "release checks skipped: no tag"
elif ! command -v gh >/dev/null 2>&1; then
    warn "gh not installed; release asset audit skipped"
else
    REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || true)"
    if [[ -z "$REPO" ]]; then
        warn "could not resolve GitHub repository for release audit"
    else
        mkdir -p "$TMP/release"
        if gh release download "$TAG" --repo "$REPO" --dir "$TMP/release" --clobber >/dev/null 2>&1; then
            ok "downloaded published release $TAG for independent inspection"
            if [[ -f "$TMP/release/SHA256SUMS" ]] && (cd "$TMP/release" && sha256sum -c SHA256SUMS >/dev/null 2>&1); then
                ok "published SHA256SUMS verifies every listed asset"
            else
                bad "published checksums are missing or do not verify"
            fi

            APK="$(find "$TMP/release" -maxdepth 1 -type f -name '*.apk' | head -1)"
            if [[ -n "$APK" ]]; then
                mkdir -p "$TMP/apk"
                unzip -q -o "$APK" -d "$TMP/apk" >/dev/null 2>&1 || true
                OUT="$TMP/apk-paths.txt"
                find "$TMP/apk" -type f -size -50M -exec strings {} + 2>/dev/null \
                    | grep -E '/home/[A-Za-z0-9_.-]+|/Users/[A-Za-z0-9_.-]+|/mnt/(Dati|Backups|GoogleDrive)(/|$)' \
                    | sort -u >"$OUT" || true
                if [[ -s "$OUT" ]]; then print_hits "$OUT"; bad "published APK embeds machine/user filesystem paths"; else ok "published APK has no obvious machine/user filesystem paths"; fi

                APKSIGNER="$(command -v apksigner || true)"
                if [[ -z "$APKSIGNER" && -x /opt/android-sdk/build-tools/34.0.0/apksigner ]]; then
                    APKSIGNER=/opt/android-sdk/build-tools/34.0.0/apksigner
                fi
                if [[ -n "$APKSIGNER" ]] && "$APKSIGNER" verify --print-certs "$APK" >"$TMP/apkcert.txt" 2>&1; then
                    ok "published APK signature verifies"
                else
                    warn "could not independently verify APK signing certificate"
                fi
            else
                warn "release contains no APK"
            fi
        else
            bad "could not download published release $TAG"
        fi
    fi
fi

section "Tag authenticity"
if [[ -n "$TAG" ]] && git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    TAG_TYPE="$(git cat-file -t "$TAG" 2>/dev/null || true)"
    if [[ "$TAG_TYPE" == "tag" ]]; then
        if git cat-file -p "$TAG" | grep -qE '^-----BEGIN (PGP|SSH) SIGNATURE-----$'; then
            ok "$TAG is cryptographically signed"
        else
            warn "$TAG is annotated but unsigned"
        fi
    else
        warn "$TAG is a lightweight/unsigned tag"
    fi
fi

section "Summary"
printf 'PASS: %d  WARN: %d  FAIL: %d\n' "$PASS" "$WARNS" "$FAILS"
if (( FAILS > 0 )); then
    echo "RESULT: FINDINGS — do not call the public repository privacy-clean yet."
    exit 1
fi
if (( WARNS > 0 )); then
    echo "RESULT: no hard privacy failure, but warnings require review."
    exit 0
fi
echo "RESULT: clean on tracked tree, reachable history, workflows and release checks."
