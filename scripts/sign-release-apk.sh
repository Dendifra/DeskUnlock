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
EXPECTED_CERT_SHA256=9908dc83186cb41f15acc5d4091e9ac59e0770b615c5cfa9c55b9a4fe25ef277
RELEASE_KEY_ALIAS=deskunlock-release

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
    [[ -t 0 ]] || { echo "keystore password requires an interactive terminal" >&2; exit 2; }
    read -rsp "Password keystore release: " KSPASS
fi
echo
SIGNED_TMP_DIR="$(mktemp -d)"
trap 'unset KSPASS; rm -rf "$SIGNED_TMP_DIR"' EXIT
SIGNED_TMP="$SIGNED_TMP_DIR/signed.apk"
KSPASS="$KSPASS" "$APKSIGNER" sign \
    --ks "$KEYSTORE" \
    --ks-type PKCS12 \
    --ks-key-alias "$RELEASE_KEY_ALIAS" \
    --ks-pass env:KSPASS \
    --key-pass env:KSPASS \
    --out "$SIGNED_TMP" \
    "$UNSIGNED"
unset KSPASS

CERT_INFO="$("$APKSIGNER" verify --print-certs "$SIGNED_TMP")"
mapfile -t CERT_SHA256S < <(printf '%s\n' "$CERT_INFO" | awk -F': ' '$1 ~ /^Signer #[0-9]+ certificate SHA-256 digest$/ { gsub(":", "", $2); print tolower($2) }')
if [[ ${#CERT_SHA256S[@]} -ne 1 || "${CERT_SHA256S[0]:-}" != "$EXPECTED_CERT_SHA256" ]]; then
    echo "release APK certificate SHA-256 mismatch" >&2
    exit 1
fi
mkdir -p "$(dirname "$OUT")"
mv -f -- "$SIGNED_TMP" "$OUT"
printf '%s\n' "$CERT_INFO" | grep -E "DN:|SHA-256 digest"
echo "firmato: $OUT"
