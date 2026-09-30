#!/usr/bin/env bash
set -euo pipefail

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
helper="$(dirname "$0")/../libexec/syauth-plasma-lock-sync"

export SYAUTH_PLASMA_TEST=1
export SYAUTH_PLASMA_PAM_ETC_DIR="$root/etc/pam.d"
export SYAUTH_PLASMA_PAM_VENDOR_DIR="$root/usr/lib/pam.d"
export SYAUTH_PLASMA_QML_PATH="$root/usr/share/plasma/shells/org.kde.plasma.desktop/contents/lockscreen/MainBlock.qml"
export SYAUTH_PLASMA_STATE_DIR="$root/var/lib/syauth/plasma-lock"
mkdir -p "$SYAUTH_PLASMA_PAM_ETC_DIR" "$SYAUTH_PLASMA_PAM_VENDOR_DIR" "$(dirname "$SYAUTH_PLASMA_QML_PATH")"

cat > "$SYAUTH_PLASMA_PAM_VENDOR_DIR/kde-fingerprint" <<'EOF'
#%PAM-1.0
auth       required                    pam_shells.so
auth       requisite                   pam_nologin.so
auth       requisite                   pam_faillock.so preauth
auth       required                    pam_fprintd.so
auth       optional                    pam_permit.so
auth       required                    pam_env.so

account    include                     system-local-login
password   required                    pam_deny.so
session    include                     system-local-login
EOF

cat > "$SYAUTH_PLASMA_QML_PATH" <<'EOF'
import org.kde.kirigami as Kirigami

Item {
    PlasmaExtras.PasswordField {
        id: password
        placeholderText: "Password"
    }
}
EOF
cp "$SYAUTH_PLASMA_QML_PATH" "$root/qml-original"

"$helper" install
grep -Fq '# DeskUnlock managed Plasma session-lock PAM override' "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
grep -Fq 'pam_syauth.so timeout=8000' "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
! grep -Fq 'pam_fprintd.so' "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
grep -Fq '// DeskUnlock managed Plasma session-lock biometric action' "$SYAUTH_PLASMA_QML_PATH"
grep -Fq 'icon.name: "fingerprint"' "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/qml-original" "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor"

sha_before="$(sha256sum "$SYAUTH_PLASMA_QML_PATH" | cut -d' ' -f1)"
"$helper" install
sha_after="$(sha256sum "$SYAUTH_PLASMA_QML_PATH" | cut -d' ' -f1)"
[[ "$sha_before" == "$sha_after" ]]

cat > "$SYAUTH_PLASMA_QML_PATH" <<'EOF'
import org.kde.kirigami as Kirigami

// vendor update marker
Item {
    PlasmaExtras.PasswordField {
        id: password
        placeholderText: "Password"
    }
}
EOF
cp "$SYAUTH_PLASMA_QML_PATH" "$root/qml-updated"
"$helper" install
cmp "$root/qml-updated" "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor"
grep -Fq '// DeskUnlock managed Plasma session-lock biometric action' "$SYAUTH_PLASMA_QML_PATH"

"$helper" remove
[[ ! -e "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint" ]]
cmp "$root/qml-updated" "$SYAUTH_PLASMA_QML_PATH"
[[ ! -e "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor" ]]

printf '%s\n' '# custom operator PAM override' > "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
cp "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint" "$root/custom-pam"
if "$helper" install >/dev/null 2>&1; then
    echo 'FAIL: external PAM override was accepted' >&2
    exit 1
fi
cmp "$root/custom-pam" "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"

rm -f "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
cp "$root/qml-original" "${SYAUTH_PLASMA_QML_PATH}.backup-deskunlock"
python3 - "$SYAUTH_PLASMA_QML_PATH" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace(
    "    PlasmaExtras.PasswordField {\n",
    "    PlasmaExtras.PasswordField {\n"
    "        leftActions: Kirigami.Action {\n"
    "            icon.name: \"fingerprint\"\n"
    "            visible: authenticator.authenticatorTypes & ScreenLocker.Authenticator.Fingerprint\n"
    "            text: i18ndc(\"plasma_shell_org.kde.plasma.desktop\", \"@info:tooltip\", \"Biometric authentication\")\n"
    "        }\n",
    1,
)
p.write_text(s)
PY
"$helper" install
grep -Fq '// DeskUnlock managed Plasma session-lock biometric action' "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/qml-original" "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor"

printf '%s\n' 'Plasma lock sync regression tests: PASS'
