#!/usr/bin/env bash
set -euo pipefail

EXPECTED=9908dc83186cb41f15acc5d4091e9ac59e0770b615c5cfa9c55b9a4fe25ef277
FAKE_EXPECTED_PASS='test$!#:\pass'
export FAKE_EXPECTED_PASS
TEST_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ "${FAKE_APKSIGNER:-0}" == 1 ]]; then
    case "$1" in
        sign)
            [[ "${KSPASS:-}" == "$FAKE_EXPECTED_PASS" ]] || { echo 'fixture: password characters changed' >&2; exit 93; }
            [[ " $* " == *" --ks-type PKCS12 "* ]] || { echo 'fixture: missing PKCS12 type' >&2; exit 91; }
            [[ " $* " == *" --ks-key-alias deskunlock-release "* ]] || { echo 'fixture: wrong/missing keystore alias' >&2; exit 92; }
            [[ " $* " == *" --key-pass env:KSPASS "* ]] || { echo 'fixture: key password is not explicitly bounded' >&2; exit 94; }
            input="${@: -1}"
            while [[ $# -gt 0 ]]; do
                [[ "$1" != --out ]] || { output="$2"; shift; }
                shift
            done
            cp "$input" "$output"
            ;;
        verify)
            printf 'Signer #1 certificate DN: CN=DeskUnlock Release\n'
            printf 'Signer #1 certificate SHA-256 digest: %s\n' "${FAKE_CERT_SHA256:?}"
            if [[ -n "${FAKE_CERT_SHA256_2:-}" ]]; then
                printf 'Signer #2 certificate SHA-256 digest: %s\n' "$FAKE_CERT_SHA256_2"
            fi
            ;;
    esac
    exit 0
fi

ROOT="$WORK/repo"
mkdir -p "$ROOT/scripts" "$ROOT/syauth-android/app/build/outputs/apk/release"
cp "$(dirname "$TEST_SCRIPT")/../sign-release-apk.sh" "$ROOT/scripts/"
printf 'test input' > "$ROOT/syauth-android/app/build/outputs/apk/release/app-release-unsigned.apk"
printf 'test keystore' > "$WORK/keystore"
cat > "$WORK/askpass" <<'EOF'
#!/bin/sh
printf '%s' 'test$!#:\pass'
EOF
chmod +x "$WORK/askpass"
printf '#!/usr/bin/env bash\nexec bash %q "$@"\n' "$TEST_SCRIPT" > "$WORK/apksigner"
chmod +x "$WORK/apksigner"

mkdir -p "$WORK/final"
NON_TTY_OUT="$WORK/final/non-tty.apk"
if printf '%s\n' "$FAKE_EXPECTED_PASS" | env SYAUTH_RELEASE_KEYSTORE="$WORK/keystore" \
    APKSIGNER="$WORK/apksigner" FAKE_APKSIGNER=1 FAKE_CERT_SHA256="$EXPECTED" \
    bash "$ROOT/scripts/sign-release-apk.sh" "$NON_TTY_OUT" >"$WORK/non-tty.log" 2>&1; then
    echo 'FAIL password was accepted without a terminal' >&2
    exit 1
fi
grep -q 'requires an interactive terminal' "$WORK/non-tty.log" || { cat "$WORK/non-tty.log"; exit 1; }
[[ ! -e "$NON_TTY_OUT" ]] || { echo 'FAIL non-TTY signing created an output' >&2; exit 1; }
printf 'ok password input rejected without an interactive terminal\n'

TTY_OUT="$WORK/final/interactive.apk"
TTY_CMD="env -u SYAUTH_KS_ASKPASS SYAUTH_RELEASE_KEYSTORE=$WORK/keystore APKSIGNER=$WORK/apksigner FAKE_APKSIGNER=1 FAKE_CERT_SHA256=$EXPECTED bash $ROOT/scripts/sign-release-apk.sh $TTY_OUT"
printf '%s\n' "$FAKE_EXPECTED_PASS" | script -qE never -ec "$TTY_CMD" /dev/null >"$WORK/interactive.log" 2>&1
[[ -f "$TTY_OUT" ]] || { cat "$WORK/interactive.log"; echo 'FAIL TTY signing rejected' >&2; exit 1; }
! grep -Fq "$FAKE_EXPECTED_PASS" "$WORK/interactive.log" || { echo 'FAIL interactive password was echoed' >&2; exit 1; }
printf 'ok hidden TTY prompt preserves shell-special password characters\n'

BAD_OUT="$WORK/final/mismatch.apk"
if env SYAUTH_RELEASE_KEYSTORE="$WORK/keystore" SYAUTH_KS_ASKPASS="$WORK/askpass" \
    APKSIGNER="$WORK/apksigner" FAKE_APKSIGNER=1 FAKE_CERT_SHA256="0000000000000000000000000000000000000000000000000000000000000000" \
    bash "$ROOT/scripts/sign-release-apk.sh" "$BAD_OUT" >"$WORK/mismatch.log" 2>&1; then
    echo 'FAIL mismatched signer was accepted' >&2
    exit 1
else
    grep -q 'certificate SHA-256 mismatch' "$WORK/mismatch.log" || { cat "$WORK/mismatch.log"; exit 1; }
fi
[[ ! -e "$BAD_OUT" ]] || { echo 'FAIL mismatched APK reached output path' >&2; exit 1; }
printf 'ok mismatched signer rejected before output\n'

GOOD_OUT="$WORK/final/valid.apk"
env SYAUTH_RELEASE_KEYSTORE="$WORK/keystore" SYAUTH_KS_ASKPASS="$WORK/askpass" \
    APKSIGNER="$WORK/apksigner" FAKE_APKSIGNER=1 FAKE_CERT_SHA256="$EXPECTED" \
    bash "$ROOT/scripts/sign-release-apk.sh" "$GOOD_OUT" >"$WORK/valid.log" 2>&1
[[ -f "$GOOD_OUT" ]] || { echo 'FAIL matching signer was rejected' >&2; exit 1; }
grep -q "$EXPECTED" "$WORK/valid.log" || { echo 'FAIL verified signer fingerprint was not reported' >&2; exit 1; }
printf 'ok matching signer accepted after verification\n'

MULTI_OUT="$WORK/final/multiple.apk"
if env SYAUTH_RELEASE_KEYSTORE="$WORK/keystore" SYAUTH_KS_ASKPASS="$WORK/askpass" \
    APKSIGNER="$WORK/apksigner" FAKE_APKSIGNER=1 FAKE_CERT_SHA256="$EXPECTED" \
    FAKE_CERT_SHA256_2="0000000000000000000000000000000000000000000000000000000000000000" \
    bash "$ROOT/scripts/sign-release-apk.sh" "$MULTI_OUT" >"$WORK/multiple.log" 2>&1; then
    echo 'FAIL multiple signer certificates were accepted' >&2
    exit 1
fi
[[ ! -e "$MULTI_OUT" ]] || { echo 'FAIL multi-signer APK reached output path' >&2; exit 1; }
printf 'ok multi-signer APK rejected before output\n'

RELEASE_LOCAL="$(dirname "$TEST_SCRIPT")/../release-local.sh"
SIGN_LINE="$(grep -nF 'bash scripts/sign-release-apk.sh "$OUT/deskunlock-$TAG-signed.apk"' "$RELEASE_LOCAL" | cut -d: -f1)"
UPLOAD_LINE="$(grep -nF 'gh release upload "$TAG"' "$RELEASE_LOCAL" | cut -d: -f1)"
(( SIGN_LINE < UPLOAD_LINE )) || { echo 'FAIL release upload precedes verified signing' >&2; exit 1; }
grep -qF 'bash scripts/sign-release-apk.sh "$(APK_FILE)"' "$(dirname "$TEST_SCRIPT")/../../Makefile" || { echo 'FAIL make release-apk bypasses certificate verifier' >&2; exit 1; }
printf 'ok release entry points use verified signing before upload\n'
