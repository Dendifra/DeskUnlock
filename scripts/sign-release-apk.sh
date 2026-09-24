#!/usr/bin/env bash
# Sign the built DeskUnlock release APK with the release keystore.
#
# The password is read silently and passed to `apksigner` via an environment
# variable, so it never lands in argv, shell history, or a file.
#
# Usage (from any shell, e.g. fish):
#   bash scripts/sign-release-apk.sh [output.apk]
#
# Override the keystore with SYAUTH_RELEASE_KEYSTORE if it lives elsewhere.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNSIGNED="$ROOT/syauth-android/app/build/outputs/apk/release/app-release-unsigned.apk"
KEYSTORE="${SYAUTH_RELEASE_KEYSTORE:-/mnt/Dati/Backups/DeskUnlock/deskunlock-release.p12}"
OUT="${1:-/tmp/deskunlock-reconnectfix.apk}"

APKSIGNER="${APKSIGNER:-}"
if [[ -z "$APKSIGNER" ]]; then
    APKSIGNER="$(command -v apksigner || true)"
fi
if [[ -z "$APKSIGNER" && -x /opt/android-sdk/build-tools/34.0.0/apksigner ]]; then
    APKSIGNER=/opt/android-sdk/build-tools/34.0.0/apksigner
fi

[[ -n "$APKSIGNER" ]] || { echo "apksigner not found" >&2; exit 1; }
[[ -f "$UNSIGNED" ]] || { echo "unsigned release APK not found: $UNSIGNED" >&2; exit 1; }
[[ -f "$KEYSTORE" ]] || { echo "keystore not found: $KEYSTORE" >&2; exit 1; }

read -rsp "Password keystore release: " KSPASS
echo
KSPASS="$KSPASS" "$APKSIGNER" sign \
    --ks "$KEYSTORE" \
    --ks-pass env:KSPASS \
    --out "$OUT" \
    "$UNSIGNED"
unset KSPASS

"$APKSIGNER" verify --print-certs "$OUT" | grep -E "DN:|SHA-256 digest"
echo "firmato: $OUT"
