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
lock_surface="$work/DankMaterialShell/quickshell/Modules/Lock/LockSurface.qml"

python3 - "$pam" "$lock_screen" "$lock_surface" <<'PY'
from pathlib import Path
import sys

pam = Path(sys.argv[1])
lock_screen = Path(sys.argv[2])
lock_surface = Path(sys.argv[3])
s = pam.read_text(encoding="utf-8")

needle_state = "    property bool unlockInProgress: false\n"
if needle_state not in s:
    raise SystemExit("Pam.qml layout mismatch: root state anchor not found")
s = s.replace(needle_state, needle_state + "    property bool syauthAvailable: false\n    property string syauthAuthState: \"AUTH_READY\"\n    property int syauthGeneration: 0\n", 1)

needle_fprint = """    PamContext {
        id: fprint
"""
if needle_fprint not in s:
    raise SystemExit("Pam.qml layout mismatch: fprint anchor not found")

syauth_block = """    function requestSyauthAuth(source: string, explicit: bool): bool {
        return syauth.startSyauthAuth(source, explicit);
    }

    PamContext {
        id: syauth

        property int requestGeneration: 0

        // One single-flight generation per same locked session; do not retry automatically.
        function startSyauthAuth(source: string, explicit: bool): bool {
            if (!root.lockSecured || root.unlockInProgress)
                return false;
            if (root.syauthAuthState === \"AUTH_IN_FLIGHT\" || active) {
                console.log(\"DeskUnlock auth generation ignored because already in-flight source=\" + source);
                return false;
            }
            if (root.syauthAuthState === \"AUTH_CONSUMED\" && !explicit)
                return false;

            ++root.syauthGeneration;
            requestGeneration = root.syauthGeneration;
            root.syauthAuthState = \"AUTH_IN_FLIGHT\";
            syauthTimeout.restart();
            console.log(\"DeskUnlock auth generation started source=\" + source);
            root.syauthAvailable = start();
            if (!root.syauthAvailable) {
                root.syauthAuthState = \"AUTH_CONSUMED\";
                console.log(\"DeskUnlock auth generation timeout generation=\" + requestGeneration);
            }
            return root.syauthAvailable;
        }

        config: \"syauth-dms\"
        configDirectory: \"/etc/pam.d\"

        onCompleted: res => {
            if (!root.lockSecured || requestGeneration !== root.syauthGeneration) {
                console.log(\"DeskUnlock stale generation response ignored\");
                return;
            }
            if (root.syauthAuthState !== \"AUTH_IN_FLIGHT\") {
                console.log(\"DeskUnlock stale generation response ignored\");
                return;
            }
            root.syauthAuthState = \"AUTH_CONSUMED\";
            syauthTimeout.stop();
            if (res === PamResult.Success) {
                console.log(\"DeskUnlock auth generation success\");
                if (!root.unlockInProgress) {
                    passwd.abort();
                    fprint.abort();
                    u2f.abort();
                    root.proceedAfterPrimaryAuth();
                }
                return;
            }
            root.syauthAvailable = false;
            console.log(\"DeskUnlock auth generation denied\");
            // A later deliberate local edge may start a new generation.
        }
    }

    Timer {
        id: syauthTimeout
        interval: 20000
        repeat: false
        onTriggered: {
            if (root.syauthAuthState !== \"AUTH_IN_FLIGHT\")
                return;
            root.syauthAuthState = \"AUTH_CONSUMED\";
            root.syauthAvailable = false;
            if (syauth.active)
                syauth.abort();
            console.log(\"DeskUnlock auth generation timeout\");
        }
    }

    IpcHandler {
        target: \"syauth\"

        function phoneReturned(): void {
            syauth.startSyauthAuth(\"phone-return\", false);
        }
    }

    Connections {
        target: passwd

        function onActiveChanged(): void {
            if (passwd.active)
                syauth.startSyauthAuth(\"passwd.active\", false);
        }
    }

"""

s = s.replace(needle_fprint, syauth_block + needle_fprint, 1)

needle_lock = """        fprint.checkAvail();
        u2f.checkAvail();
"""
if needle_lock not in s:
    raise SystemExit("Pam.qml layout mismatch: lock anchor not found")

replacement_lock = """        root.syauthAvailable = true;
        fprint.checkAvail();
        u2f.checkAvail();
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
            ++root.syauthGeneration;
            root.syauthAvailable = false;
            root.syauthAuthState = "AUTH_READY";
            syauthTimeout.stop();
            if (syauth.active) {
                syauth.abort();
                console.log("DeskUnlock auth generation cancelled");
            }
            root.resetAuthFlows();
            return;
        }
""", 1)

pam.write_text(s, encoding="utf-8")

surface = lock_surface.read_text(encoding="utf-8")
needle_surface_state = "    required property bool isLocked\n"
if needle_surface_state not in surface:
    raise SystemExit("LockSurface.qml layout mismatch: lock state anchor not found")
surface = surface.replace(needle_surface_state, needle_surface_state + "    property bool pointerReengagementSent: false\n", 1)

needle_surface_keys = """    Keys.onPressed: event => {
        if (videoScreensaver.active && videoScreensaver.inputEnabled) {
            videoScreensaver.dismiss();
            event.accepted = true;
        }
    }
"""
if needle_surface_keys not in surface:
    raise SystemExit("LockSurface.qml layout mismatch: key event anchor not found")
surface = surface.replace(needle_surface_keys, """    Keys.onPressed: event => {
        if (videoScreensaver.active && videoScreensaver.inputEnabled) {
            videoScreensaver.dismiss();
            event.accepted = true;
        }
        if (root.isLocked)
            root.pam.requestSyauthAuth(\"local-reengagement\", true);
    }
""", 1)

needle_surface_locked = """    onIsLockedChanged: {
        if (isLocked) {
            forceActiveFocus();
"""
if needle_surface_locked not in surface:
    raise SystemExit("LockSurface.qml layout mismatch: lock transition anchor not found")
surface = surface.replace(needle_surface_locked, """    onIsLockedChanged: {
        if (isLocked) {
            pointerReengagementSent = false;
            console.log("DeskUnlock lock epoch started");
            forceActiveFocus();
""", 1)

needle_surface_rect = """    Rectangle {
        anchors.fill: parent
        color: \"transparent\"
    }
"""
if needle_surface_rect not in surface:
    raise SystemExit("LockSurface.qml layout mismatch: surface anchor not found")
surface = surface.replace(needle_surface_rect, """    HoverHandler {
        enabled: root.isLocked && !videoScreensaver.active
        onPointChanged: {
            if (root.pointerReengagementSent)
                return;
            root.pointerReengagementSent = true;
            root.pam.requestSyauthAuth(\"local-reengagement\", false);
        }
    }

    Rectangle {
        anchors.fill: parent
        color: \"transparent\"
    }
""", 1)
lock_surface.write_text(surface, encoding="utf-8")

ui = lock_screen.read_text(encoding="utf-8")
needle_icon = '''                                if (pam.u2fPending)
                                    return "passkey";
                                if (pam.fprint.tries >= SettingsData.maxFprintTries)
'''
replacement_icon = '''                                if (pam.u2fPending)
                                    return "passkey";
                                if (pam.u2f.active)
                                    return "passkey";
                                if (pam.syauthAvailable)
                                    return "fingerprint";
                                if (pam.fprint.tries >= SettingsData.maxFprintTries)
'''
if needle_icon not in ui:
    raise SystemExit("LockScreenContent.qml layout mismatch: lock icon anchor not found")
ui = ui.replace(needle_icon, replacement_icon, 1)

needle_feedback = """        if (pam.lockMessage && pam.lockMessage.length > 0)
            return pam.lockMessage;
"""
if needle_feedback not in ui:
    raise SystemExit("LockScreenContent.qml layout mismatch: feedback anchor not found")
ui = ui.replace(needle_feedback, """        if (!pam.passwd.active && pam.syauthAuthState === \"AUTH_IN_FLIGHT\")
            return \"Conferma l'impronta sul telefono\";
        if (!pam.passwd.active && pam.syauthAuthState === \"AUTH_READY\" && pam.syauthAvailable)
            return \"Muovi il mouse o premi un tasto per sbloccare con DeskUnlock\";
        if (!pam.passwd.active && pam.syauthAuthState === \"AUTH_CONSUMED\" && !pam.syauthAvailable)
            return \"Usa la password per accedere\";
        if (pam.lockMessage && pam.lockMessage.length > 0)
            return pam.lockMessage;
""", 1)

needle_hint = """        return pam && (pam.u2fState === \"waiting\" || pam.u2fState === \"insert\") && !pam.u2fPending;
"""
if needle_hint not in ui:
    raise SystemExit("LockScreenContent.qml layout mismatch: hint anchor not found")
ui = ui.replace(needle_hint, """        return pam && (((pam.u2fState === \"waiting\" || pam.u2fState === \"insert\") && !pam.u2fPending) || pam.syauthAuthState === \"AUTH_IN_FLIGHT\" || (pam.syauthAuthState === \"AUTH_READY\" && pam.syauthAvailable));
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
