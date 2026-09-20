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

python3 - "$pam" <<'PY'
from pathlib import Path
import sys

p = Path(sys.argv[1])
s = p.read_text(encoding="utf-8")

needle_fprint = """    PamContext {
        id: fprint
"""
if needle_fprint not in s:
    raise SystemExit("Pam.qml layout mismatch: fprint anchor not found")

syauth_block = """    PamContext {
        id: syauth

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
                syauth.start();
        }
    }

    Connections {
        target: passwd

        function onActiveChanged(): void {
            if (passwd.active && root.lockSecured && !root.unlockInProgress && !syauth.active)
                syauth.start();
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
                syauth.start();
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

p.write_text(s, encoding="utf-8")
PY

(
  cd "$work/DankMaterialShell"
  make build
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
