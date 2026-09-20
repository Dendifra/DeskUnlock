#!/usr/bin/env bash
set -euo pipefail

DMS_REPO="https://github.com/AvengeMedia/DankMaterialShell.git"
DMS_COMMIT="aa4b99def48637d86a69620c0a8f3cc6aa0c4092"
OUTPUT="${1:-$PWD/dms-syauth}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

git clone --quiet "$DMS_REPO" "$work/DankMaterialShell"
git -C "$work/DankMaterialShell" checkout --quiet "$DMS_COMMIT"
git -C "$work/DankMaterialShell" submodule update --init --recursive --quiet

pam="$work/DankMaterialShell/quickshell/Modules/Lock/Pam.qml"
lock_screen="$work/DankMaterialShell/quickshell/Modules/Lock/LockScreenContent.qml"

python3 - "$pam" "$lock_screen" <<'PY'
from pathlib import Path
import sys

pam = Path(sys.argv[1])
lock_screen = Path(sys.argv[2])
s = pam.read_text(encoding="utf-8")

needle_state = "    property bool unlockInProgress: false\n"
if needle_state not in s:
    raise SystemExit("Pam.qml layout mismatch: root state anchor not found")
s = s.replace(needle_state, needle_state + "    property bool syauthAvailable: false\n", 1)

needle_fprint = """    PamContext {
        id: fprint
"""
if needle_fprint not in s:
    raise SystemExit("Pam.qml layout mismatch: fprint anchor not found")

syauth_block = """    PamContext {
        id: syauth

        function startIfAvailable(): void {
            if (!root.lockSecured || root.unlockInProgress || active)
                return;
            if (start())
                root.syauthAvailable = true;
        }

        config: "syauth-dms"
        configDirectory: "/etc/pam.d"

        onCompleted: res => {
            if (!root.lockSecured)
                return;

            if (res === PamResult.Success) {
                if (!root.unlockInProgress) {
                    passwd.abort();
                    fprint.abort();
                    u2f.abort();
                    root.proceedAfterPrimaryAuth();
                }
            }
        }
    }

    IpcHandler {
        target: "syauth"

        function phoneReturned(): void {
            if (root.lockSecured && !root.unlockInProgress && !syauth.active)
                syauth.startIfAvailable();
        }
    }

    Connections {
        target: passwd

        function onActiveChanged(): void {
            if (passwd.active && root.lockSecured && !root.unlockInProgress && !syauth.active)
                syauth.startIfAvailable();
        }
    }

"""

s = s.replace(needle_fprint, syauth_block + needle_fprint, 1)

needle_timer = """    Timer {
        id: errorRetry
"""
if needle_timer not in s:
    raise SystemExit("Pam.qml layout mismatch: timer anchor not found")

syauth_timer = """    Timer {
        id: syauthStartTimer

        interval: 1500
        repeat: false
        onTriggered: {
            if (root.lockSecured && !syauth.active)
                syauth.startIfAvailable();
        }
    }

"""
s = s.replace(needle_timer, syauth_timer + needle_timer, 1)

needle_lock = """        fprint.checkAvail();
        u2f.checkAvail();
"""
if needle_lock not in s:
    raise SystemExit("Pam.qml layout mismatch: lock anchor not found")

replacement_lock = """        fprint.checkAvail();
        u2f.checkAvail();
        syauthStartTimer.restart();
"""
s = s.replace(needle_lock, replacement_lock, 1)

needle_unlock = """        if (!lockSecured) {
            root.resetAuthFlows();
            return;
        }
"""
if needle_unlock not in s:
    raise SystemExit("Pam.qml layout mismatch: unlock reset anchor not found")
s = s.replace(needle_unlock, """        if (!lockSecured) {
            root.syauthAvailable = false;
            root.resetAuthFlows();
            return;
        }
""", 1)

pam.write_text(s, encoding="utf-8")

ui = lock_screen.read_text(encoding="utf-8")
needle_icon = '''                                if (pam.u2fPending)
                                    return "passkey";
                                if (pam.fprint.tries >= SettingsData.maxFprintTries)
'''
replacement_icon = '''                                if (pam.u2fPending)
                                    return "passkey";
                                if (pam.u2f.active)
                                    return "passkey";
                                if (pam.fprint.tries >= SettingsData.maxFprintTries)
'''
if needle_icon not in ui:
    raise SystemExit("LockScreenContent.qml layout mismatch: lock icon anchor not found")
ui = ui.replace(needle_icon, replacement_icon, 1)

needle_syauth_logo = """                            Behavior on opacity {
                                NumberAnimation {
                                    duration: Theme.mediumDuration
                                    easing.type: Theme.standardEasing
                                }
                            }
                        }
                    }

                    FocusScope {
                        id: passwordField
"""
if needle_syauth_logo not in ui:
    raise SystemExit("LockScreenContent.qml layout mismatch: logo anchor not found")
ui = ui.replace(needle_syauth_logo, """                            Behavior on opacity {
                                NumberAnimation {
                                    duration: Theme.mediumDuration
                                    easing.type: Theme.standardEasing
                                }
                            }
                        }

                        Image {
                            anchors.centerIn: parent
                            width: 20
                            height: 20
                            source: root.encodeFileUrl("/usr/share/icons/hicolor/256x256/apps/deskunlock.png")
                            sourceSize: Qt.size(20, 20)
                            fillMode: Image.PreserveAspectFit
                            visible: pam.syauthAvailable && !pam.u2fPending && !pam.u2f.active
                            opacity: pam.passwd.active ? 0 : 1

                            Behavior on opacity {
                                NumberAnimation {
                                    duration: Theme.mediumDuration
                                    easing.type: Theme.standardEasing
                                }
                            }
                        }
                    }

                    FocusScope {
                        id: passwordField
""", 1)
lock_screen.write_text(ui, encoding="utf-8")
PY

(
  cd "$work/DankMaterialShell"
  make build GOFLAGS=-trimpath
)

built="$work/DankMaterialShell/core/bin/dms"
if [[ ! -x "$built" ]]; then
  echo "ERROR: expected DMS binary not produced: $built" >&2
  exit 1
fi

install -Dm755 "$built" "$OUTPUT"

echo "Built DeskUnlock DMS bridge:"
echo "  source commit: $DMS_COMMIT"
echo "  output: $OUTPUT"
go version -m "$OUTPUT" 2>/dev/null | sed -n '1,8p' || true
