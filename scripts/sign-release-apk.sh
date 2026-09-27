#!/usr/bin/env bash
# Sign the built DeskUnlock release APK with the release keystore.
#
# The keystore path is supplied explicitly through SYAUTH_RELEASE_KEYSTORE so
# the public repository never embeds the maintainer's private filesystem layout.
# The password is read silently and passed to `apksigner` via an environment
# variable, so it never lands in argv, shell history, or a file.
#
# Usage (from any shell, e.g. fish):
#   env SYAUTH_RELEASE_KEYSTORE=/path/to/deskunlock-release.p12 \
#       bash scripts/sign-release-apk.sh [output.apk]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNSIGNED="$ROOT/syauth-android/app/build/outputs/apk/release/app-release-unsigned.apk"
KEYSTORE="${SYAUTH_RELEASE_KEYSTORE:-}"
OUT="${1:-/tmp/deskunlock-release-signed.apk}"

[[ -n "$KEYSTORE" ]] || {
    echo "SYAUTH_RELEASE_KEYSTORE is required and must point to the private release keystore" >&2
    exit 2
}

APKSIGNER="${APKSIGNER:-}"
if [[ -z "$APKSIGNER" ]]; then
    APKSIGNER="$(command -v apksigner || true)"
fi
if [[ -z "$APKSIGNER" && -x /opt/android-sdk/build-tools/34.0.0/apksigner ]]; then
    APKSIGNER=/opt/android-sdk/build-tools/34.0.0/apksigner
fi

[[ -n "$APKSIGNER" ]] || { echo "apksigner not found" >&2; exit 1; }
[[ -f "$UNSIGNED" ]] || { echo "unsigned release APK not found: $UNSIGNED" >&2; exit 1; }
[[ -f "$KEYSTORE" ]] || { echo "keystore not found" >&2; exit 1; }

# The password can come from an askpass helper instead of the terminal. Piping
# it in keeps it out of argv and out of the shell history. Falls back to the
# terminal prompt when no helper is configured.
if [[ -n "${SYAUTH_KS_ASKPASS:-}" ]]; then
    KSPASS="$("$SYAUTH_KS_ASKPASS")" || { echo "keystore password prompt cancelled" >&2; exit 1; }
    [[ -n "$KSPASS" ]] || { echo "empty keystore password" >&2; exit 1; }
else
    read -rsp "Password keystore release: " KSPASS
fi
echo
KSPASS="$KSPASS" "$APKSIGNER" sign \
    --ks "$KEYSTORE" \
    --ks-pass env:KSPASS \
    --out "$OUT" \
    "$UNSIGNED"
unset KSPASS

"$APKSIGNER" verify --print-certs "$OUT" | grep -E "DN:|SHA-256 digest"
echo "firmato: $OUT"
