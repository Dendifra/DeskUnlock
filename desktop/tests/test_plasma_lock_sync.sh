#!/usr/bin/env bash
set -euo pipefail

root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
helper="$(dirname "$0")/../libexec/syauth-plasma-lock-sync"
run_helper() { python3 "$helper" "$@"; }

export SYAUTH_PLASMA_TEST=1
export SYAUTH_PLASMA_PAM_ETC_DIR="$root/etc/pam.d"
export SYAUTH_PLASMA_PAM_VENDOR_DIR="$root/usr/lib/pam.d"
export SYAUTH_PLASMA_QML_PATH="$root/usr/share/plasma/shells/org.kde.plasma.desktop/contents/lockscreen/MainBlock.qml"
export SYAUTH_PLASMA_LOCKSCREEN_QML_PATH="$root/usr/share/plasma/shells/org.kde.plasma.desktop/contents/lockscreen/LockScreenUi.qml"
export SYAUTH_PLASMA_STATE_DIR="$root/var/lib/syauth/plasma-lock"
mkdir -p "$SYAUTH_PLASMA_PAM_ETC_DIR" "$SYAUTH_PLASMA_PAM_VENDOR_DIR" "$(dirname "$SYAUTH_PLASMA_QML_PATH")"

cat > "$SYAUTH_PLASMA_PAM_VENDOR_DIR/kde-fingerprint" <<'EOF'
#%PAM-1.0
auth       required                    pam_shells.so
auth       requisite                   pam_nologin.so
auth       requisite                   pam_faillock.so preauth
-auth      required                    pam_fprintd.so
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
cp "$SYAUTH_PLASMA_QML_PATH" "$root/main-original"

cat > "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" <<'EOF'
import QtQuick 2.15
Item {
    id: lockScreenUi
    readonly property bool softwareRendering: GraphicsInfo.api === GraphicsInfo.Software

    function handleMessage(msg) {
        console.log(msg)
    }

    property bool uiVisible: false
    property bool seenPositionChange: false

    Connections {
        target: authenticator
        function onFailed(kind) {
            if (kind != 0) {
                return;
            }
            console.log(kind)
        }
    }

    MouseArea {
        anchors.fill: parent
        onPressed: uiVisible = true;
        onPositionChanged: {
            uiVisible = seenPositionChange;
            seenPositionChange = true;
        }
        onUiVisibleChanged: {
            if (uiVisible) {
                Window.window.requestActivate();
            }

            if (blockUI) {
                fadeoutTimer.running = false;
            } else if (uiVisible) {
                fadeoutTimer.restart();
            }
            authenticator.startAuthenticating();
        }
    }

    Keys.onPressed: event => {
        uiVisible = true;
        event.accepted = false;
    }

    Timer {
        id: graceLockTimer
        interval: 3000
        onTriggered: {
            root.clearPassword();
            authenticator.startAuthenticating();
        }
    }
}
EOF
cp "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" "$root/interaction-original"

run_helper install
grep -Fq '# DeskUnlock managed Plasma session-lock PAM override' "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
grep -Fq 'pam_syauth.so timeout=8000' "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
! grep -Fq 'pam_fprintd.so' "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
grep -Fq '// DeskUnlock managed Plasma session-lock biometric action' "$SYAUTH_PLASMA_QML_PATH"
grep -Fq 'icon.name: "fingerprint"' "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/main-original" "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor"
[[ "$(stat -c '%a' "$SYAUTH_PLASMA_STATE_DIR/LockScreenUi.qml.vendor")" == 600 ]]

python3 - "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" <<'PY'
from pathlib import Path
import sys
s = Path(sys.argv[1]).read_text()
assert s.count("// DeskUnlock managed interaction-gated authentication") == 1
assert s.count("authenticator.startAuthenticating();") == 1
assert "property bool authRequested: false" in s
assert "function requestAuthentication()" in s
assert "onPressed: {\n            uiVisible = true;\n            lockScreenUi.requestAuthentication();\n        }" in s
assert "const realMovement = seenPositionChange;" in s
assert "if (realMovement) {\n                lockScreenUi.requestAuthentication();" in s
assert "if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {\n            lockScreenUi.requestAuthentication();" in s
assert "event.accepted = false;" in s
failed = s.split("function onFailed(kind)", 1)[1].split("}", 1)[0]
assert "lockScreenUi.authRequested = false;" in failed
ui = s.split("onUiVisibleChanged", 1)[1].split("}\n    }\n\n    Keys", 1)[0]
timer = s.split("id: graceLockTimer", 1)[1]
assert "authenticator.startAuthenticating();" not in ui
assert "authenticator.startAuthenticating();" not in timer
PY

main_before="$(sha256sum "$SYAUTH_PLASMA_QML_PATH")"
interaction_before="$(sha256sum "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH")"
run_helper install
[[ "$main_before" == "$(sha256sum "$SYAUTH_PLASMA_QML_PATH")" ]]
[[ "$interaction_before" == "$(sha256sum "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH")" ]]
grep -Fq 'pam=managed' <(run_helper status)
grep -Fq 'interaction_qml=managed' <(run_helper status)

# MainBlock vendor replacement tolerates a stale legacy backup.
cp "$root/main-original" "$SYAUTH_PLASMA_QML_PATH.backup-deskunlock"
cp "$root/main-original" "$SYAUTH_PLASMA_QML_PATH"
python3 - "$SYAUTH_PLASMA_QML_PATH" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
p.write_text(p.read_text().replace("import org.kde.kirigami as Kirigami", "import org.kde.kirigami as Kirigami\n// MainBlock vendor update", 1).replace("// DeskUnlock managed Plasma session-lock biometric action\n", "", 1))
PY
cp "$SYAUTH_PLASMA_QML_PATH" "$root/main-updated"
run_helper install
cmp "$root/main-updated" "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor"
grep -Fq '// DeskUnlock managed Plasma session-lock biometric action' "$SYAUTH_PLASMA_QML_PATH"

# A new vendor version must win even while the old manual backup remains.
cp "$root/interaction-original" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH.backup-before-interaction-auth"
cp "$root/interaction-original" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"
python3 - "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
s = s.replace("import QtQuick 2.15", "import QtQuick 2.15\n// vendor update", 1)
s = s.replace("// DeskUnlock managed interaction-gated authentication\n", "", 1)
p.write_text(s)
PY
cp "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" "$root/interaction-updated"
run_helper install
cmp "$root/interaction-updated" "$SYAUTH_PLASMA_STATE_DIR/LockScreenUi.qml.vendor"
grep -Fq '// DeskUnlock managed interaction-gated authentication' "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"

run_helper remove
[[ ! -e "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint" ]]
cmp "$root/main-updated" "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/interaction-updated" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"
[[ ! -e "$SYAUTH_PLASMA_STATE_DIR/LockScreenUi.qml.vendor" ]]

# Preserve the old PAM override rejection contract.
printf '%s\n' '# custom operator PAM override' > "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
cp "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint" "$root/custom-pam"
if run_helper install >/dev/null 2>&1; then
    echo 'FAIL: external PAM override was accepted' >&2
    exit 1
fi
cmp "$root/custom-pam" "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
rm -f "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"

# MainBlock legacy manual adoption remains byte-for-byte exact.
cp "$root/main-original" "$SYAUTH_PLASMA_QML_PATH"
cp "$root/main-original" "$SYAUTH_PLASMA_QML_PATH.backup-deskunlock"
python3 - "$SYAUTH_PLASMA_QML_PATH" "$helper" <<'PY'
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
import sys
path, helper = map(Path, sys.argv[1:])
loader = SourceFileLoader("sync", str(helper.resolve()))
module = module_from_spec(spec_from_loader("sync", loader))
loader.exec_module(module)
manual = module.remove_marker(module.patch_qml(path.read_text()), module.QML_MARKER)
path.write_text(manual)
PY
run_helper install
grep -Fq '// DeskUnlock managed Plasma session-lock biometric action' "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/main-original" "$SYAUTH_PLASMA_STATE_DIR/MainBlock.qml.vendor"
run_helper remove
rm -f "$SYAUTH_PLASMA_QML_PATH.backup-deskunlock"

# Build the byte-for-byte manual patch and compare it with the expected result.
cp "$root/interaction-original" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"
cp "$root/interaction-original" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH.backup-before-interaction-auth"
python3 - "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" "$helper" "$root/manual-expected" <<'PY'
from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader
from pathlib import Path
import sys
path, helper, expected = map(Path, sys.argv[1:])
loader = SourceFileLoader("sync", str(helper.resolve()))
module = module_from_spec(spec_from_loader("sync", loader))
loader.exec_module(module)
manual = module.patch_interaction_qml(path.read_text())
manual = module.remove_marker(manual, module.INTERACTION_MARKER)
path.write_text(manual)
expected.write_text(manual)
PY
run_helper install
python3 - "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" "$root/manual-expected" <<'PY'
from pathlib import Path
import re
import sys
current, expected = map(Path, sys.argv[1:])
managed = current.read_text()
clean = re.sub(r"(?m)^[ \t]*// DeskUnlock managed interaction-gated authentication[ \t]*\n", "", managed, count=1)
assert clean == expected.read_text()
PY
cmp "$root/interaction-original" "$SYAUTH_PLASMA_STATE_DIR/LockScreenUi.qml.vendor"
run_helper remove

# Managed interaction QML without its vendor backup is a safe failure.
run_helper install
rm "$SYAUTH_PLASMA_STATE_DIR/LockScreenUi.qml.vendor"
cp "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint" "$root/missing-pam"
cp "$SYAUTH_PLASMA_QML_PATH" "$root/missing-main"
cp "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" "$root/missing-interaction"
if run_helper install >/dev/null 2>&1; then
    echo 'FAIL: missing interaction backup was accepted' >&2
    exit 1
fi
cmp "$root/missing-pam" "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"
cmp "$root/missing-main" "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/missing-interaction" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"
run_helper remove >/dev/null 2>&1 || true
rm -f "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint"

# An incompatible interaction anchor must not partially write any plan.
cp "$root/interaction-original" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"
python3 - "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
p.write_text(p.read_text().replace("onPressed: uiVisible = true;", "onPressed: uiVisible = false;", 1))
PY
cp "$SYAUTH_PLASMA_PAM_VENDOR_DIR/kde-fingerprint" "$root/vendor-pam"
cp "$SYAUTH_PLASMA_QML_PATH" "$root/incompatible-main"
cp "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH" "$root/incompatible-interaction"
if run_helper install >/dev/null 2>&1; then
    echo 'FAIL: incompatible interaction anchor was accepted' >&2
    exit 1
fi
cmp "$root/incompatible-main" "$SYAUTH_PLASMA_QML_PATH"
cmp "$root/incompatible-interaction" "$SYAUTH_PLASMA_LOCKSCREEN_QML_PATH"
[[ ! -e "$SYAUTH_PLASMA_PAM_ETC_DIR/kde-fingerprint" ]]

printf '%s\n' 'Plasma lock sync regression tests: PASS'
