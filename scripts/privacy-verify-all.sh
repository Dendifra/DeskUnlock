#!/usr/bin/env bash
# The three privacy verifications, in one command, with no state to get wrong.
#
# Why this is a script: the point of the check is that anyone can run it and get
# the same answer. A block of commands to paste is not that — the temp directories
# collide on the second run, and a stray markdown fence becomes "Unknown command"
# before the check even starts.
#
#   bash scripts/privacy-verify-all.sh [tag]
#
# Default tag is the current pre-release. Exits non-zero if anything is dirty.

set -uo pipefail

TAG="${1:-v0.1.0-beta.2}"
REPO="${SYAUTH_REPO:-Dendifra/DeskUnlock}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

fail=0
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$*"; fail=1; }

echo
echo "1. The tracked tree"
if ( cd "$ROOT" && bash scripts/privacy-check.sh >/dev/null 2>&1 ); then
    ok "no real device address, no personal home path, no private key"
else
    ( cd "$ROOT" && bash scripts/privacy-check.sh ) || true
    bad "privacy-check reported findings"
fi

echo
echo "2. The remote history (a fresh clone, not the local one)"
if git clone -q --mirror "https://github.com/$REPO.git" "$WORK/history" 2>/dev/null; then
    macs="$(cd "$WORK/history" && git log --all -p 2>/dev/null |
            grep -ohEi '\b([0-9a-f]{2}:){5}[0-9a-f]{2}\b' |
            grep -vE '^(AA|BB|CC|DD|11|00|02)' | sort -u || true)"
    homes="$(cd "$WORK/history" && git log --all -p 2>/dev/null |
             grep -ohE '/home/[A-Za-z0-9_.-]+' |
             grep -vE '^/home/(user|UID|\.config)' | sort -u || true)"
    [[ -z "$macs" ]] && ok "no real device address in any commit" || { echo "$macs" | sed 's/^/      /'; bad "real device address in history"; }
    [[ -z "$homes" ]] && ok "no personal home directory in any commit" || { echo "$homes" | sed 's/^/      /'; bad "personal home directory in history"; }
else
    bad "could not clone $REPO (network? permissions?)"
fi

echo
echo "3. The published artifact for $TAG"
if gh release download "$TAG" --repo "$REPO" --dir "$WORK/rel" --clobber >/dev/null 2>&1; then
    if ( cd "$WORK/rel" && sha256sum -c SHA256SUMS >/dev/null 2>&1 ); then
        ok "every published checksum resolves"
    else
        bad "a published checksum does not resolve"
    fi
    apk="$(ls "$WORK/rel"/*.apk 2>/dev/null | head -1)"
    if [[ -n "$apk" ]]; then
        mkdir -p "$WORK/apk" && ( cd "$WORK/apk" && unzip -q -o "$apk" 'lib/*/libsyauth_mobile.so' 2>/dev/null )
        leaks="$(find "$WORK/apk" -name '*.so' -exec strings {} + 2>/dev/null | grep -c '/home/' || true)"
        if [[ "${leaks:-0}" == "0" ]]; then
            ok "no build path in the shipped libraries ($(find "$WORK/apk" -name '*.so' | wc -l) ABIs checked)"
        else
            bad "$leaks build paths in the shipped libraries"
        fi
        if /opt/android-sdk/build-tools/34.0.0/apksigner verify --print-certs "$apk" 2>/dev/null |
           grep -q "CN=DeskUnlock Release"; then
            ok "signed with the DeskUnlock release certificate"
        else
            bad "the APK does not carry the release certificate"
        fi
    else
        bad "no APK in the release"
    fi
else
    bad "could not download release $TAG"
fi

echo
if [[ "$fail" == "0" ]]; then
    printf '\033[32mprivacy: clean on all three levels\033[0m\n\n'
else
    printf '\033[31mprivacy: FINDINGS ABOVE\033[0m\n\n'
fi
exit "$fail"
