#!/usr/bin/env bash
# DeskUnlock — the DMS lock-screen adaptation must never touch PAM.
#
# Why this test exists (2026-09-23): an earlier version of
# `syauth-dms-lock-patch` pointed DMS's own `assets/pam/fprint` service at
# `pam_syauth.so`, and a pacman hook re-applied it after every plasma/pam
# update. A DeskUnlock module inside a PAM stack that guards the login is how
# the operator ended up unable to get back in with the correct password — twice.
# The phone unlock is out of band now, so the adaptation may only show the
# indicator, keep DMS's own gates, and **restore** any `pam_syauth.so` it finds.
#
# | TC | Scenario                                                          |
# |----|-------------------------------------------------------------------|
# | 01 | a tree carrying `pam_syauth.so` is restored to the stock fprintd  |
# | 02 | the indicator lights up and DMS's own gate stays intact           |
# | 03 | missing anchors change nothing and report failure (exit 3)        |
# | 04 | a second run is a no-op (idempotent)                              |

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PATCH="$REPO_ROOT/desktop/libexec/syauth-dms-lock-patch"

STOCK_FPRINT=$'#%PAM-1.0\n\nauth    required    pam_fprintd.so  max-tries=5\naccount required    pam_permit.so\npassword required   pam_deny.so\nsession required    pam_permit.so\n'
PATCHED_FPRINT=$'#%PAM-1.0\n\nauth    sufficient  pam_syauth.so\naccount required    pam_permit.so\npassword required   pam_deny.so\nsession required    pam_permit.so\n'

pass=0
fail=0
ok() {
    printf 'ok   %s\n' "$1"
    pass=$((pass + 1))
}
bad() {
    printf 'FAIL %s — %s\n' "$1" "$2"
    fail=$((fail + 1))
}

# Build a fake DMS runtime tree: <root>/<hash>/{Modules/Lock,assets/pam}.
make_tree() {
    local root="$1" fprint="$2" qml_body="$3"
    local dir="$root/3f15ae07e999b609"
    mkdir -p "$dir/Modules/Lock" "$dir/assets/pam"
    printf '%s' "$fprint" >"$dir/assets/pam/fprint"
    printf '%s' "$qml_body" >"$dir/Modules/Lock/Pam.qml"
    printf '%s' "$CONTENT_QML" >"$dir/Modules/Lock/LockScreenContent.qml"
    printf '%s' "$dir"
}

STOCK_QML=$'    property bool syauthPlaceholder: false\n        property bool available: SettingsData.lockFingerprintReady\n\n    function start() {\n            if (!available || !root.lockSecured) {\n            return;\n        }\n    }\n'

# The lock screen content carries the operator's unlock gesture: Enter with an
# empty password field, or mouse movement — never when the lock merely appears,
# and never on the post-auth success signal. The fixture starts with the OLD
# post-auth trigger in place so the removal is exercised.
CONTENT_QML=$'    Connections {\n        target: root.pam\n\n        function onUnlockRequested() {\n            Quickshell.execDetached(["/usr/bin/syauth", "unlock-request"]);\n            root.unlocking = true;\n        }\n    }\n\n    Timer {\n        id: placeholderDelay\n\n        interval: 4000\n        onTriggered: root.pamState = ""\n    }\n\n    MouseArea {\n        anchors.fill: parent\n        enabled: demoMode\n        onClicked: root.unlockRequested()\n    }\n\n    TextField {\n        id: passwordField\n                        onAccepted: {\n                            if (!demoMode && !root.unlocking && !pam.passwd.active && !pam.u2fPending) {\n                                pam.passwd.start();\n                            }\n                        }\n    }\n\n    Icon {\n        name: {\n                                if (pam.u2f.active)\n                                    return "passkey";\n                                return "lock";\n        }\n    }\n'

# ---------------------------------------------------------------------------
# TC 01/02 — restore + indicator
# ---------------------------------------------------------------------------
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

# Hermetic systemctl: the patch restarts DMS after adapting a freshly
# extracted tree (DMS does not watch the QML). Capture that call instead of
# restarting the host's real session.
FAKEBIN="$T/bin"
mkdir -p "$FAKEBIN"
cat > "$FAKEBIN/systemctl" <<'EOF'
#!/usr/bin/env bash
printf 'systemctl %s\n' "$*" >> "${SYAUTH_TEST_SYSTEMCTL_LOG:?}"
exit 0
EOF
chmod +x "$FAKEBIN/systemctl"
export PATH="$FAKEBIN:$PATH"
export SYAUTH_TEST_SYSTEMCTL_LOG="$T/systemctl.log"
: > "$SYAUTH_TEST_SYSTEMCTL_LOG"

DIR="$(make_tree "$T" "$PATCHED_FPRINT" "$STOCK_QML")"

SYAUTH_DMS_DIR="$T" SYAUTH_UNLOCK_READY="$T/marker" SYAUTH_DMS_LOADED="$T/loaded" SYAUTH_PRESENCED_ACTIVE=0 bash "$PATCH" >"$T/out" 2>&1
rc=$?

if [[ $rc -eq 0 ]]; then
    ok "01 il patch esce 0 su un albero ripristinabile"
else
    bad "01 il patch esce 0 su un albero ripristinabile" "exit=$rc out=$(cat "$T/out")"
fi

if grep -q "pam_fprintd.so" "$DIR/assets/pam/fprint" && ! grep -q "pam_syauth" "$DIR/assets/pam/fprint"; then
    ok "01b il servizio PAM di DMS e' tornato stock (nessuna riga nostra)"
else
    bad "01b il servizio PAM di DMS e' tornato stock" "$(cat "$DIR/assets/pam/fprint")"
fi

if grep -q "lockPamExternallyManaged" "$DIR/Modules/Lock/Pam.qml"; then
    ok "02 l'indicatore si accende per un lock PAM gestito esternamente"
else
    bad "02 l'indicatore si accende" "qml=$(cat "$DIR/Modules/Lock/Pam.qml")"
fi

if grep -q "fprintSuppressedByPrimaryPam" "$DIR/Modules/Lock/Pam.qml"; then
    ok "02b il gate di DMS e' intatto (il telefono non viene chiesto al lock)"
else
    bad "02b il gate di DMS e' intatto" "qml=$(cat "$DIR/Modules/Lock/Pam.qml")"
fi

if grep -q "requestPhoneUnlock" "$DIR/Modules/Lock/LockScreenContent.qml"; then
    ok "02c il gesto dell'operatore chiede il telefono (Enter vuoto / movimento mouse)"
else
    bad "02c il gesto dell'operatore chiede il telefono" "content=$(cat "$DIR/Modules/Lock/LockScreenContent.qml")"
fi

if ! grep -q '^            Quickshell.execDetached' "$DIR/Modules/Lock/LockScreenContent.qml"; then
    ok "02d il vecchio trigger post-auth e' stato rimosso da onUnlockRequested"
else
    bad "02d il vecchio trigger post-auth e' stato rimosso" "content=$(cat "$DIR/Modules/Lock/LockScreenContent.qml")"
fi

if grep -q "syauthUnlockReady.loaded" "$DIR/Modules/Lock/LockScreenContent.qml" && grep -q 'return "fingerprint";' "$DIR/Modules/Lock/LockScreenContent.qml"; then
    ok "02e l'icona mostra l'impronta solo quando phone unlock e' pronto"
else
    bad "02e l'icona mostra l'impronta quando pronto" "content=$(cat "$DIR/Modules/Lock/LockScreenContent.qml")"
fi

if grep -q "syauthRequestCooldown" "$DIR/Modules/Lock/LockScreenContent.qml" && grep -q "syauthEnterX" "$DIR/Modules/Lock/LockScreenContent.qml"; then
    ok "02f una richiesta alla volta (cooldown) e movimento reale > 8px (non alla comparsa)"
else
    bad "02f cooldown + soglia movimento" "content=$(cat "$DIR/Modules/Lock/LockScreenContent.qml")"
fi

if grep -q "syauthUnlockProcess" "$DIR/Modules/Lock/LockScreenContent.qml" && grep -q "root.unlockRequested();" "$DIR/Modules/Lock/LockScreenContent.qml" && ! grep -q 'execDetached(["/usr/bin/syauth"' "$DIR/Modules/Lock/LockScreenContent.qml"; then
    ok "02g il successo del telefono emette lo sblocco di DMS (Process, non execDetached)"
else
    bad "02g il successo del telefono emette lo sblocco di DMS" "content=$(cat "$DIR/Modules/Lock/LockScreenContent.qml")"
fi

if [[ "$(grep -c 'restart dms.service' "$SYAUTH_TEST_SYSTEMCTL_LOG")" -eq 1 ]]; then
    ok "02h un albero appena adattato fa riavviare DMS una volta (per caricare il patch)"
else
    bad "02h restart DMS dopo l'adattamento" "log=$(cat "$SYAUTH_TEST_SYSTEMCTL_LOG")"
fi

if [[ "$(cat "$T/loaded" 2>/dev/null)" == "$(basename "$DIR")" ]]; then
    ok "02i il marker di caricamento registra l'albero adattato (la GUI lo legge)"
else
    bad "02i marker di caricamento" "loaded=$(cat "$T/loaded" 2>/dev/null) atteso=$(basename "$DIR")"
fi

# ---------------------------------------------------------------------------
# TC 04 — idempotent
# ---------------------------------------------------------------------------
before="$(sha256sum "$DIR/assets/pam/fprint" "$DIR/Modules/Lock/Pam.qml" "$DIR/Modules/Lock/LockScreenContent.qml")"
SYAUTH_DMS_DIR="$T" SYAUTH_UNLOCK_READY="$T/marker" SYAUTH_DMS_LOADED="$T/loaded" SYAUTH_PRESENCED_ACTIVE=0 bash "$PATCH" >/dev/null 2>&1
after="$(sha256sum "$DIR/assets/pam/fprint" "$DIR/Modules/Lock/Pam.qml" "$DIR/Modules/Lock/LockScreenContent.qml")"
if [[ "$before" == "$after" ]]; then
    ok "04 una seconda esecuzione non cambia nulla"
else
    bad "04 una seconda esecuzione non cambia nulla" "prima=$before dopo=$after"
fi

if [[ "$(grep -c 'restart dms.service' "$SYAUTH_TEST_SYSTEMCTL_LOG")" -eq 1 ]]; then
    ok "04b una seconda esecuzione non riavvia di nuovo (niente loop)"
else
    bad "04b niente restart al secondo giro" "count=$(grep -c 'restart dms.service' "$SYAUTH_TEST_SYSTEMCTL_LOG")"
fi

# ---------------------------------------------------------------------------
# TC 03 — anchors missing: the indicator is not applied, nothing is damaged,
# and the safety action still runs (a PAM service must never keep our module).
# ---------------------------------------------------------------------------
T2="$(mktemp -d)"
DIR2="$(make_tree "$T2" "$PATCHED_FPRINT" $'    property bool available: qualcosaltro\n')"
qml_before="$(sha256sum "$DIR2/Modules/Lock/Pam.qml")"
SYAUTH_DMS_DIR="$T2" SYAUTH_UNLOCK_READY="$T2/marker" SYAUTH_PRESENCED_ACTIVE=0 bash "$PATCH" >"$T2/out" 2>&1
rc2=$?
qml_after="$(sha256sum "$DIR2/Modules/Lock/Pam.qml")"
rm -rf "$T2"

if [[ $rc2 -eq 3 ]]; then
    ok "03 ancoraggi mancanti: esce 3 (segnala invece di ignorare)"
else
    bad "03 ancoraggi mancanti: esce 3" "exit=$rc2"
fi

if [[ "$qml_before" == "$qml_after" ]]; then
    ok "03b ancoraggi mancanti: il QML non viene toccato"
else
    bad "03b ancoraggi mancanti: il QML non viene toccato" "prima=$qml_before dopo=$qml_after"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
