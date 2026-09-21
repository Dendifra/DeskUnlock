#!/usr/bin/env bash
set -euo pipefail

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT

export SYAUTH_PROXIMITY_TEST=1
export XDG_RUNTIME_DIR="$root/runtime"
export XDG_CONFIG_HOME="$root/config"
export SYAUTH_TEST_ACTION_LOG="$root/actions.log"
export SYAUTH_TEST_NOW_MS=1000000
export SYAUTH_TEST_HEARTBEAT_AGE_MS=0
export SYAUTH_TEST_READY=1
mkdir -p "$XDG_RUNTIME_DIR/syauth"
: > "$SYAUTH_TEST_ACTION_LOG"

source "$(dirname "$0")/../desktop/bin/syauth-proximity"

SYAUTH_TEST_LOCKED=0
SYAUTH_TEST_AUTH_RC=0

session_id() { echo test-session; }
session_is_locked() { [[ "${SYAUTH_TEST_LOCKED:-0}" == 1 ]]; }
lock_session() {
    printf 'LOCK\n' >> "$SYAUTH_TEST_ACTION_LOG"
    SYAUTH_TEST_LOCKED=1
    return 0
}
request_auto_auth() {
    printf 'AUTH\n' >> "$SYAUTH_TEST_ACTION_LOG"
    return "$SYAUTH_TEST_AUTH_RC"
}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

assert_eq() {
    local expected="$1" actual="$2" message="$3"
    [[ "$expected" == "$actual" ]] || fail "$message: expected '$expected', got '$actual'"
}

assert_file_contains() {
    local needle="$1" path="$2" message="$3"
    grep -Fqx "$needle" "$path" || fail "$message: missing '$needle'"
}

assert_no_actions() {
    [[ ! -s "$SYAUTH_TEST_ACTION_LOG" ]] || fail "$1: unexpected actions: $(tr '\n' ' ' < "$SYAUTH_TEST_ACTION_LOG")"
}

actions() {
    wc -l < "$SYAUTH_TEST_ACTION_LOG" | tr -d ' '
}

reset_case() {
    rm -f "$CONFIG_PATH" "$RUNTIME_STATE" "$RSSI_STATE" "$SYAUTH_TEST_ACTION_LOG"
    mkdir -p "$RUNTIME_SUBDIR"
    : > "$SYAUTH_TEST_ACTION_LOG"
    SYAUTH_TEST_LOCKED=0
    SYAUTH_TEST_AUTH_RC=0
    SYAUTH_TEST_HEARTBEAT_AGE_MS=0
    SYAUTH_TEST_READY=1
    SYAUTH_TEST_NOW_MS=1000000
    load_config
    load_runtime
}

write_sample() {
    local filtered="$1" timestamp="$2"
    mkdir -p "$RUNTIME_SUBDIR"
    printf 'raw=%s\nfiltered=%s\nsample_epoch_ms=%s\n' "$filtered" "$filtered" "$timestamp" > "$RSSI_STATE"
}

tick_sample() {
    local filtered="$1" advance_ms="${2:-1000}"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + advance_ms))
    write_sample "$filtered" "$SYAUTH_TEST_NOW_MS"
    engine_tick
}

bootstrap_near() {
    reset_case
    tick_sample -53 0
    tick_sample -54 1000
    tick_sample -52 1000
    [[ -n "$BASELINE" ]] || fail "baseline did not bootstrap"
}

far_lock() {
    bootstrap_near
    tick_sample -61 1000
    tick_sample -61 8000
    assert_eq 1 "$(grep -c '^LOCK$' "$SYAUTH_TEST_ACTION_LOG")" "stable FAR lock"
}

test_near_baseline_bootstrap() {
    bootstrap_near
    grep -Eq '^baseline=-5[23]\.[0-9]{2}$' "$CONFIG_PATH" || fail "baseline is not the three-sample near average"
}

test_baseline_does_not_follow_departure() {
    bootstrap_near
    local before after
    before=$(grep '^baseline=' "$CONFIG_PATH")
    tick_sample -60 1000
    tick_sample -61 1000
    after=$(grep '^baseline=' "$CONFIG_PATH")
    assert_eq "$before" "$after" "baseline followed an away movement"
}

test_near_state_is_stable() {
    bootstrap_near
    tick_sample -53 1000
    assert_eq NEAR "$PROXIMITY_STATE" "near state"
}

test_single_weak_spike_does_not_become_far() {
    bootstrap_near
    tick_sample -61 1000
    assert_eq 0 "$(actions)" "single weak RSSI spike"
    tick_sample -53 1000
    assert_eq NEAR "$PROXIMITY_STATE" "single spike hysteresis"
}

test_mid_hysteresis_avoids_flap() {
    bootstrap_near
    tick_sample -58 1000
    tick_sample -58 2000
    assert_eq MID "$PROXIMITY_STATE" "mid transition"
    tick_sample -57 1000
    assert_eq MID "$PROXIMITY_STATE" "mid to near persistence"
    tick_sample -53 4000
    assert_eq NEAR "$PROXIMITY_STATE" "near return persistence"
}

test_far_persistence_locks_once() {
    far_lock
    tick_sample -61 1000
    assert_eq 1 "$(actions)" "far lock repeated"
    grep -Fx 'lock_reason=PROXIMITY' "$RUNTIME_STATE" >/dev/null || fail "proximity lock reason missing"
}

test_stale_rssi_with_heartbeat_does_not_become_far() {
    bootstrap_near
    rm -f "$RSSI_STATE"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + RSSI_STALE_AFTER_MS + 1000))
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "stale RSSI with heartbeat"
    assert_no_actions "stale RSSI with heartbeat"
}

test_lost_heartbeat_becomes_absent_and_locks() {
    bootstrap_near
    SYAUTH_TEST_HEARTBEAT_AGE_MS=$((HEARTBEAT_STALE_AFTER_MS + 1))
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 1000))
    engine_tick
    assert_eq ABSENT "$PROXIMITY_STATE" "lost heartbeat state"
    assert_eq 1 "$(actions)" "heartbeat fallback lock"
}

test_proximity_lock_reason_is_runtime_only() {
    far_lock
    [[ -e "$RUNTIME_STATE" ]] || fail "runtime state missing"
    grep -Fx 'lock_reason=PROXIMITY' "$RUNTIME_STATE" >/dev/null || fail "runtime lock reason"
    [[ ! -e "$CONFIG_PATH.raw" ]] || fail "raw RSSI sidecar persisted"
}

test_proximity_return_sends_one_auth() {
    far_lock
    SYAUTH_TEST_READY=1
    tick_sample -53 1000
    tick_sample -53 4000
    assert_eq 2 "$(actions)" "proximity return action count"
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "proximity return auth"
}

test_phone_remaining_near_does_not_spam() {
    far_lock
    tick_sample -53 1000
    tick_sample -53 4000
    tick_sample -53 5000
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "near phone auth spam"
}

test_denied_auth_has_no_retry() {
    far_lock
    SYAUTH_TEST_AUTH_RC=1
    tick_sample -53 1000
    tick_sample -53 4000
    tick_sample -53 5000
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "denied auth retry"
}

test_timeout_auth_has_no_retry() {
    far_lock
    SYAUTH_TEST_AUTH_RC=124
    tick_sample -53 1000
    tick_sample -53 4000
    tick_sample -53 5000
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "timeout auth retry"
}

test_manual_lock_near_does_not_auto_auth() {
    bootstrap_near
    SYAUTH_TEST_LOCKED=1
    engine_tick
    assert_eq MANUAL_OR_OTHER "$LOCK_REASON" "manual lock reason"
    assert_no_actions "manual lock near"
}

test_manual_lock_explicit_trigger_remains_dms_owned() {
    grep -Fq 'function onActiveChanged' desktop/dms/build-dms-syauth.sh || fail "DMS manual trigger missing"
    grep -Fq 'passwd.active' desktop/dms/build-dms-syauth.sh || fail "DMS passwd trigger missing"
}

test_auth_cancellation_guards_remain_present() {
    grep -Fq 'syauth.abort()' desktop/dms/build-dms-syauth.sh || fail "DMS abort guard missing"
    grep -Fq 'syauthGeneration' desktop/dms/build-dms-syauth.sh || fail "DMS generation guard missing"
    grep -Fq 'cancel' syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/ChallengeApprovalActivity.kt || fail "Android cancellation path missing"
}

test_no_late_notification_contract_remains_present() {
    grep -Fq 'cancelSink' syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/ChallengeApprovalActivity.kt || fail "Android cancel sink missing"
    grep -Fq 'writeResponse' syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/PersistentGattClient.kt || fail "GATT response path missing"
}

test_disconnect_reconnect_reaches_absent_then_near() {
    bootstrap_near
    SYAUTH_TEST_HEARTBEAT_AGE_MS=$((HEARTBEAT_STALE_AFTER_MS + 1))
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 1000))
    engine_tick
    assert_eq ABSENT "$PROXIMITY_STATE" "disconnect state"
    SYAUTH_TEST_HEARTBEAT_AGE_MS=0
    tick_sample -53 1000
    tick_sample -53 4000
    assert_eq NEAR "$PROXIMITY_STATE" "reconnect state"
}

test_restart_does_not_invent_proximity_lock() {
    reset_case
    PROXIMITY_STATE=FAR
    LOCK_ISSUED=0
    save_runtime
    SYAUTH_TEST_LOCKED=0
    engine_initialize
    assert_eq MID "$PROXIMITY_STATE" "restart FAR reset"
    assert_no_actions "restart proximity lock"
}

test_restart_status_recovers_without_markers() {
    reset_case
    rm -f "$HEARTBEAT_MARKER" "$READY_MARKER" "$RSSI_STATE"
    status=$(status_cmd)
    grep -Fx 'state=ABSENT' <<<"$status" || fail "restart status state"
}

test_restart_does_not_use_stale_presence_fallback() {
    reset_case
    PROXIMITY_STATE=NEAR
    SEEN_PRESENCE=1
    save_runtime
    SYAUTH_TEST_HEARTBEAT_AGE_MS=$((HEARTBEAT_STALE_AFTER_MS + 1))
    SYAUTH_TEST_LOCKED=0
    engine_initialize
    engine_tick
    assert_no_actions "stale presence after restart"
}

test_balanced_is_default_profile() {
    reset_case
    assert_eq balanced "$PROFILE" "default profile"
}

test_profile_changes_share_engine() {
    reset_case
    set_profile_cmd wide
    load_config
    assert_eq wide "$PROFILE" "wide profile write"
    set_profile_cmd near
    load_config
    assert_eq near "$PROFILE" "near profile write"
}

test_pair_lifecycle_reset_invalidates_baseline() {
    bootstrap_near
    [[ -n "$BASELINE" ]] || fail "baseline missing before lifecycle reset"
    reset_cmd
    load_config
    assert_eq "" "$BASELINE" "lifecycle baseline reset"
    grep -Fq 'reset_proximity_baseline' desktop/bin/syauth-device || fail "pair lifecycle reset hook missing"
    grep -Fq 'reset_proximity_baseline || true' desktop/bin/syauth-device || fail "pair lifecycle reset call missing"
}

test_raw_rssi_is_not_persisted_in_config() {
    bootstrap_near
    ! grep -Eq '^(raw|filtered|sample_epoch_ms)=' "$CONFIG_PATH" || fail "raw RSSI persisted in config"
}

test_legacy_peer_config_is_migrated_without_identity() {
    reset_case
    mkdir -p "$CONFIG_DIR"
    printf 'algorithm_version=1\nenabled=1\nprofile=balanced\nbaseline=-53\nbaseline_peer=AA:BB:CC:DD:EE:FF\n' > "$CONFIG_PATH"
    load_config
    ! grep -q 'baseline_peer\|AA:BB:CC' "$CONFIG_PATH" || fail "peer identity persisted during migration"
    assert_eq "" "$BASELINE" "legacy peer baseline invalidation"
}

test_privacy_state_contains_only_allowed_values() {
    bootstrap_near
    while IFS='=' read -r key _; do
        case "$key" in
            algorithm_version|enabled|profile|baseline) ;;
            *) fail "unexpected config key: $key" ;;
        esac
    done < "$CONFIG_PATH"
    while IFS='=' read -r key _; do
        case "$key" in
            state|pending_state|pending_since_ms|bootstrap_count|bootstrap_sum|bootstrap_min|bootstrap_max|last_sample_ms|seen_presence|lock_reason|lock_issued|auto_auth_sent) ;;
            *) fail "unexpected runtime key: $key" ;;
        esac
    done < "$RUNTIME_STATE"
}

test_dms_fingerprint_indicator_is_preserved() {
    grep -Fq 'if (pam.syauthAvailable)' desktop/dms/build-dms-syauth.sh || fail "DMS syauth state condition missing"
    grep -Fq 'return "fingerprint";' desktop/dms/build-dms-syauth.sh || fail "DMS fingerprint indicator missing"
    ! grep -Fq '/usr/share/icons/hicolor/256x256/apps/deskunlock.png' desktop/dms/build-dms-syauth.sh || fail "DMS logo path leaked into lock screen"
}

test_android_rssi_transport_contract_is_present() {
    grep -Fq 'RSSI_TELEMETRY_PREFIX' syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/PersistentGattClient.kt || fail "Android RSSI telemetry missing"
    grep -Fq 'GattOperationGate' syauth-android/app/src/main/kotlin/com/sy/syauth/android/bg/PersistentGattClient.kt || fail "Android GATT serialization missing"
}

tests=(
    test_near_baseline_bootstrap
    test_baseline_does_not_follow_departure
    test_near_state_is_stable
    test_single_weak_spike_does_not_become_far
    test_mid_hysteresis_avoids_flap
    test_far_persistence_locks_once
    test_stale_rssi_with_heartbeat_does_not_become_far
    test_lost_heartbeat_becomes_absent_and_locks
    test_proximity_lock_reason_is_runtime_only
    test_proximity_return_sends_one_auth
    test_phone_remaining_near_does_not_spam
    test_denied_auth_has_no_retry
    test_timeout_auth_has_no_retry
    test_manual_lock_near_does_not_auto_auth
    test_manual_lock_explicit_trigger_remains_dms_owned
    test_auth_cancellation_guards_remain_present
    test_no_late_notification_contract_remains_present
    test_disconnect_reconnect_reaches_absent_then_near
    test_restart_does_not_invent_proximity_lock
    test_restart_status_recovers_without_markers
    test_restart_does_not_use_stale_presence_fallback
    test_balanced_is_default_profile
    test_profile_changes_share_engine
    test_pair_lifecycle_reset_invalidates_baseline
    test_raw_rssi_is_not_persisted_in_config
    test_legacy_peer_config_is_migrated_without_identity
    test_privacy_state_contains_only_allowed_values
    test_dms_fingerprint_indicator_is_preserved
    test_android_rssi_transport_contract_is_present
)

for test in "${tests[@]}"; do
    "$test"
    printf 'ok - %s\n' "$test"
done
printf 'Proximity engine regression tests: %d passed\n' "${#tests[@]}"
