#!/usr/bin/env bash
set -euo pipefail

script=desktop/dms/build-dms-syauth.sh
pam=desktop/dms/build-dms-syauth.sh
fail() { echo "FAIL: $*" >&2; exit 1; }
contains() { grep -Fq "$1" "$2" || fail "$3"; }

contains 'pointerReengagementSent' "$script" 'lock-surface pointer one-shot is missing'
contains 'LockSurface.qml' "$script" 'runtime lock surface is missing'
contains 'lock epoch started' "$script" 'lock epoch log is missing'
contains 'startSyauthAuth' "$script" 'central auth entry point is missing'
contains 'authGeneration' "$script" 'auth generation is missing'
contains 'AUTH_IN_FLIGHT' "$script" 'single-flight state is missing'
contains 'AUTH_CONSUMED' "$script" 'consumed state is missing'
contains 'local-reengagement' "$script" 'early local trigger is missing'
contains 'passwd.active' "$script" 'password fallback trigger is missing'
contains 'already in-flight' "$script" 'in-flight suppression log is missing'
contains 'stale generation response ignored' "$script" 'stale response guard is missing'
contains 'auth generation success' "$script" 'success lifecycle log is missing'
contains 'auth generation denied' "$script" 'denied lifecycle log is missing'
contains 'auth generation timeout' "$script" 'timeout lifecycle log is missing'
contains 'auth generation cancelled' "$script" 'cancel lifecycle log is missing'
contains 'onPointChanged' "$script" 'pointer wake edge is missing'
contains 'Keys.onPressed' "$script" 'keyboard wake edge is missing'
contains 'syauth.startSyauthAuth' "$script" 'central auth path is not wired'
contains 'Muovi il mouse o premi un tasto per sbloccare con DeskUnlock' "$script" 'lock guidance is missing'
contains "Conferma l'impronta sul telefono" "$script" 'phone guidance is missing'
contains 'Usa la password per accedere' "$script" 'password fallback guidance is missing'
contains 'syauthGeneration' "$script" 'generation is not bound to the PAM context'
contains 'requestGeneration !== root.syauthGeneration' "$script" 'completion generation check is missing'
contains 'phone-return' "$script" 'return path is not unified'
contains 'root.resetAuthFlows' "$script" 'auth cancellation cleanup is missing'
contains '++root.syauthGeneration' "$script" 'lock epoch invalidation is missing'
contains 'same locked session' "$script" 'single-flight contract is missing'
contains 'do not retry' "$script" 'failed auth retry policy is missing'
contains 'syauth-idle-lock' 'desktop/systemd/syauth-idle-lock.service' 'idle lock service is missing'

if grep -Eiq '(/dev/input|key(code|text)|mouse(x|y|position)|challenge[^[:alnum:]]*bytes|signature[^[:alnum:]]*bytes|nonce)' "$script"; then
    fail 'sensitive input or challenge material added to DMS lifecycle logging'
fi

echo 'Auth lifecycle tests: 29 passed'
