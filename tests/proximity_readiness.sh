#!/usr/bin/env bash
set -euo pipefail

runtime=$(mktemp -d)
trap 'rm -rf "$runtime"' EXIT
mkdir -p "$runtime/syauth"
export XDG_RUNTIME_DIR="$runtime"
export SYAUTH_PROXIMITY_TEST=1
# No heartbeat marker means sourcing only loads the pure gate helpers.
source "$(dirname "$0")/../desktop/bin/syauth-proximity"

# A: stale marker is rejected.
! readiness_is_new old old
# B: a fresh Notify token seen before the return heartbeat is accepted.
readiness_is_new fresh old
# A missing marker is never ready.
! readiness_is_new "" old
# C/I: the same token is not a new event, so callers keep their one-shot guard.
! readiness_is_new fresh fresh

echo "proximity readiness regression tests: ok"
