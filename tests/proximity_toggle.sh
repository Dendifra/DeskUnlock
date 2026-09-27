#!/usr/bin/env bash
set -euo pipefail

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT

export SYAUTH_PROXIMITY_TEST=1
export XDG_CONFIG_HOME="$root/config"
export XDG_RUNTIME_DIR="$root/runtime"
export PATH="$root/bin:$PATH"
mkdir -p "$root/bin"

cat > "$root/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${SYAUTH_TEST_SYSTEMCTL_LOG:?}"
EOF
chmod +x "$root/bin/systemctl"
export SYAUTH_TEST_SYSTEMCTL_LOG="$root/systemctl.log"
: > "$SYAUTH_TEST_SYSTEMCTL_LOG"

source "$(dirname "$0")/../desktop/bin/syauth-proximity"
unset SYAUTH_PROXIMITY_TEST

set_enabled_cmd 0
grep -Fxq -- '--user stop syauth-proximity.service' "$SYAUTH_TEST_SYSTEMCTL_LOG"
grep -Fxq 'enabled=0' "$CONFIG_PATH"

set_enabled_cmd 1
grep -Fxq -- '--user start syauth-proximity.service' "$SYAUTH_TEST_SYSTEMCTL_LOG"
grep -Fxq 'enabled=1' "$CONFIG_PATH"

echo "Proximity toggle tests: ok"
