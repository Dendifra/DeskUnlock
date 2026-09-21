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

write_sample_values() {
    local raw="$1" filtered="$2" timestamp="$3"
    mkdir -p "$RUNTIME_SUBDIR"
    printf 'raw=%s\nfiltered=%s\nsample_epoch_ms=%s\n' "$raw" "$filtered" "$timestamp" > "$RSSI_STATE"
}

write_sample() {
    local filtered="$1" timestamp="$2"
    write_sample_values "$filtered" "$filtered" "$timestamp"
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

test_strong_near_samples_do_not_raise_baseline() {
    bootstrap_near
    BASELINE=-57.00
    save_config
    for _ in {1..20}; do
        tick_sample -51 1000
    done
    assert_eq -57.00 "$BASELINE" "strong samples raised baseline"
    assert_eq -57.00 "$(grep '^baseline=' "$CONFIG_PATH" | cut -d= -f2)" "persisted baseline raised"
}

test_weaker_near_samples_adapt_baseline_downward() {
    bootstrap_near
    BASELINE=-57.00
    save_config
    tick_sample -58 1000
    assert_eq -57.05 "$BASELINE" "weaker NEAR sample did not adapt baseline"
    tick_sample -58 1000
    [[ "$BASELINE" != "-57.05" ]] || fail "baseline did not continue conservative downward adaptation"
}

test_far_samples_never_train_baseline() {
    bootstrap_near
    BASELINE=-57.00
    save_config
    tick_sample -65 1000
    tick_sample -65 8000
    assert_eq -57.00 "$BASELINE" "FAR sample trained baseline"
    assert_eq FAR "$PROXIMITY_STATE" "FAR training fixture state"
}

test_mid_and_far_states_never_train_baseline() {
    bootstrap_near
    BASELINE=-57.00
    save_config
    tick_sample -62 1000
    tick_sample -62 2000
    assert_eq MID "$PROXIMITY_STATE" "MID training fixture state"
    tick_sample -65 2000
    assert_eq -57.00 "$BASELINE" "MID/FAR sample trained baseline"
}

test_normal_near_signal_stays_near_after_strong_samples() {
    bootstrap_near
    BASELINE=-57.00
    save_config
    for _ in {1..100}; do
        tick_sample -51 1000
    done
    for _ in {1..8}; do
        tick_sample -60 1000
    done
    assert_eq NEAR "$PROXIMITY_STATE" "normal NEAR signal became FAR after strong samples"
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

test_departure_behavior_remains_conservative() {
    bootstrap_near
    tick_sample -61 1000
    assert_eq NEAR "$PROXIMITY_STATE" "departure single weak sample"
    tick_sample -61 1000
    assert_eq NEAR "$PROXIMITY_STATE" "departure persistence start"
    tick_sample -61 7000
    assert_eq FAR "$PROXIMITY_STATE" "departure sustained weak samples"
}

test_proximity_lock_reason_starts_proximity() {
    far_lock
    assert_eq PROXIMITY "$LOCK_REASON" "proximity lock provenance"
}

test_proximity_lock_reason_survives_pending_locked_tick() {
    far_lock
    LOCK_ISSUED=$LOCK_REQUEST_PENDING
    LOCK_REASON=PROXIMITY
    SYAUTH_TEST_LOCKED=0
    save_runtime
    engine_tick
    assert_eq PROXIMITY "$LOCK_REASON" "pending lock provenance"
    SYAUTH_TEST_LOCKED=1
    engine_tick
    assert_eq PROXIMITY "$LOCK_REASON" "observed lock provenance"
}

test_proximity_lock_reason_survives_multiple_locked_ticks() {
    far_lock
    engine_tick
    engine_tick
    assert_eq PROXIMITY "$LOCK_REASON" "multiple locked tick provenance"
}

test_settling_does_not_train_baseline() {
    far_lock
    local before="$BASELINE"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq "$RETURN_MODE_SETTLING" "$RETURN_MODE" "settling fixture mode"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -53 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq "$before" "$BASELINE" "settling sample trained baseline"
}

test_single_strong_raw_spike_does_not_return_near() {
    far_lock
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq FAR "$PROXIMITY_STATE" "single strong raw spike"
}

test_same_rssi_sample_is_counted_once() {
    far_lock
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq 1 "$FAST_RETURN_SAMPLE_COUNT" "first fast return streak"
    engine_tick
    assert_eq 1 "$FAST_RETURN_SAMPLE_COUNT" "duplicate fast return streak"
    assert_eq FAR "$PROXIMITY_STATE" "duplicate sample state"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "distinct second sample state"
}

test_sustained_strong_raw_samples_return_quickly() {
    far_lock
    local started="$SYAUTH_TEST_NOW_MS"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq FAR "$PROXIMITY_STATE" "first strong raw sample"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "sustained strong raw return"
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "fast return auth"
    assert_eq 4000 "$((SYAUTH_TEST_NOW_MS - started))" "fast return latency"
}

test_sustained_strong_raw_samples_return_from_mid() {
    far_lock
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 1000))
    write_sample_values -58 -58 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -58 -58 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq MID "$PROXIMITY_STATE" "MID return starting state"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq MID "$PROXIMITY_STATE" "MID first strong raw sample"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "MID sustained strong raw return"
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "MID fast return auth"
}

test_alternating_raw_samples_do_not_flap() {
    far_lock
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -61 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq FAR "$PROXIMITY_STATE" "alternating raw samples"
    assert_eq 0 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG" || true)" "alternating raw auth"
}

test_fast_return_waits_for_late_challenge_ready() {
    far_lock
    SYAUTH_TEST_READY=0
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "fast return without challenge ready"
    assert_eq 0 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG" || true)" "early fast return auth"
    SYAUTH_TEST_READY=1
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "late challenge-ready auth"
}

test_fast_return_waits_for_late_heartbeat() {
    far_lock
    SYAUTH_TEST_HEARTBEAT_AGE_MS=$((HEARTBEAT_STALE_AFTER_MS + 1))
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "fast return without heartbeat"
    assert_eq 0 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG" || true)" "early heartbeat auth"
    SYAUTH_TEST_HEARTBEAT_AGE_MS=0
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "late heartbeat auth"
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
    assert_eq PROXIMITY "$LOCK_REASON" "return proximity provenance"
}

test_mid_to_near_return_sends_one_auth() {
    far_lock
    tick_sample -58 1000
    tick_sample -58 2000
    assert_eq MID "$PROXIMITY_STATE" "return MID state"
    tick_sample -53 1000
    tick_sample -53 4000
    assert_eq NEAR "$PROXIMITY_STATE" "return NEAR state"
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "MID to NEAR auth"
}

test_return_waits_for_late_challenge_ready() {
    far_lock
    PROXIMITY_STATE=NEAR
    STATE_CHANGED=0
    SYAUTH_TEST_READY=0
    maybe_auto_auth || true
    assert_eq 0 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG" || true)" "early challenge-ready auth"
    SYAUTH_TEST_READY=1
    maybe_auto_auth || true
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "late challenge-ready auth"
    maybe_auto_auth || true
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "late challenge-ready retry"
}

test_return_waits_for_late_heartbeat() {
    far_lock
    PROXIMITY_STATE=NEAR
    STATE_CHANGED=0
    SYAUTH_TEST_HEARTBEAT_AGE_MS=$((HEARTBEAT_STALE_AFTER_MS + 1))
    maybe_auto_auth || true
    assert_eq 0 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG" || true)" "early heartbeat auth"
    SYAUTH_TEST_HEARTBEAT_AGE_MS=0
    maybe_auto_auth || true
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "late heartbeat auth"
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
    assert_eq "$RETURN_MODE_MANUAL_WAIT" "$RETURN_MODE" "manual wait mode"
    assert_no_actions "manual lock near"
}

test_manual_lock_near_stays_quiet() {
    bootstrap_near
    SYAUTH_TEST_LOCKED=1
    engine_tick
    tick_sample -53 1000
    tick_sample -54 1000
    assert_eq "$RETURN_MODE_MANUAL_WAIT" "$RETURN_MODE" "manual wait remains near"
    assert_no_actions "manual near samples"
}

test_manual_lock_arms_after_far_departure() {
    bootstrap_near
    SYAUTH_TEST_LOCKED=1
    engine_tick
    tick_sample -61 1000
    tick_sample -61 8000
    assert_eq FAR "$PROXIMITY_STATE" "manual departure FAR"
    assert_eq "$RETURN_MODE_MANUAL_ARMED" "$RETURN_MODE" "manual FAR armed"
    assert_no_actions "manual departure lock"
}

test_manual_lock_arms_after_absent_departure() {
    bootstrap_near
    SYAUTH_TEST_LOCKED=1
    engine_tick
    SYAUTH_TEST_HEARTBEAT_AGE_MS=$((HEARTBEAT_STALE_AFTER_MS + 1))
    engine_tick
    assert_eq ABSENT "$PROXIMITY_STATE" "manual departure ABSENT"
    assert_eq "$RETURN_MODE_MANUAL_ARMED" "$RETURN_MODE" "manual ABSENT armed"
    assert_no_actions "manual absent lock"
}

test_manual_armed_return_sends_one_auth() {
    test_manual_lock_arms_after_far_departure
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "manual armed return state"
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "manual armed return auth"
}

test_manual_absent_armed_return_sends_one_auth() {
    test_manual_lock_arms_after_absent_departure
    SYAUTH_TEST_HEARTBEAT_AGE_MS=0
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq NEAR "$PROXIMITY_STATE" "manual ABSENT armed return state"
    assert_eq 1 "$(grep -c '^AUTH$' "$SYAUTH_TEST_ACTION_LOG")" "manual ABSENT armed return auth"
}

test_manual_wait_near_samples_never_auth() {
    bootstrap_near
    SYAUTH_TEST_LOCKED=1
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -55 -53 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -55 -53 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq "$RETURN_MODE_MANUAL_WAIT" "$RETURN_MODE" "manual wait near raw samples"
    assert_no_actions "manual wait near raw samples"
}

test_unlock_resets_proximity_provenance() {
    far_lock
    engine_tick
    SYAUTH_TEST_LOCKED=0
    engine_tick
    assert_eq MANUAL_OR_OTHER "$LOCK_REASON" "unlock lock reason reset"
    assert_eq "$LOCK_NOT_ISSUED" "$LOCK_ISSUED" "unlock issued reset"
    assert_eq 0 "$AUTO_AUTH_SENT" "unlock auth reset"
}

test_manual_lock_explicit_trigger_remains_dms_owned() {
    grep -Fq 'function onActiveChanged' desktop/dms/build-dms-syauth.sh || fail "DMS manual trigger missing"
    grep -Fq 'passwd.active' desktop/dms/build-dms-syauth.sh || fail "DMS passwd trigger missing"
    grep -Fq 'localInteractionConsumed' desktop/dms/build-dms-syauth.sh || fail "DMS local interaction one-shot missing"
    grep -Fq 'syauth.startIfAvailable()' desktop/dms/build-dms-syauth.sh || fail "DMS explicit syauth path missing"
    grep -Fq 'root.localInteractionConsumed = false' desktop/dms/build-dms-syauth.sh || fail "DMS local interaction rearm missing"
    ! grep -Eiq 'key(text|code)|mouse(position|x|y)|evdev|/dev/input' desktop/dms/build-dms-syauth.sh || fail "DMS input content capture detected"
}

test_balanced_return_delta_is_minus_three() {
    assert_eq -3 "$(profile_value balanced return_delta)" "balanced return threshold"
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

test_return_settling_blocks_stale_filtered_relock() {
    far_lock
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq "$RETURN_MODE_SETTLING" "$RETURN_MODE" "settling entered"
    SYAUTH_TEST_LOCKED=0
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -61 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq "$RETURN_MODE_SETTLING" "$RETURN_MODE" "stale filtered settling"
    assert_eq 1 "$(grep -c '^LOCK$' "$SYAUTH_TEST_ACTION_LOG")" "stale filtered relock"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -53 -53 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -53 -53 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq "$RETURN_MODE_NONE" "$RETURN_MODE" "settling completion"
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 1000))
    write_sample_values -61 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 8000))
    write_sample_values -61 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq 2 "$(grep -c '^LOCK$' "$SYAUTH_TEST_ACTION_LOG")" "new departure relock"
}

test_restart_does_not_duplicate_fast_return_sample() {
    far_lock
    SYAUTH_TEST_NOW_MS=$((SYAUTH_TEST_NOW_MS + 2000))
    write_sample_values -54 -61 "$SYAUTH_TEST_NOW_MS"
    engine_tick
    assert_eq 1 "$FAST_RETURN_SAMPLE_COUNT" "restart streak before reload"
    load_runtime
    engine_tick
    assert_eq 1 "$FAST_RETURN_SAMPLE_COUNT" "restart duplicate streak"
    assert_eq FAR "$PROXIMITY_STATE" "restart duplicate state"
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
            state|pending_state|pending_since_ms|bootstrap_count|bootstrap_sum|bootstrap_min|bootstrap_max|last_sample_ms|seen_presence|lock_reason|lock_issued|auto_auth_sent|fast_return_sample_count|fast_return_last_sample_ms|return_mode|return_settling_near_count|return_settling_last_sample_ms) ;;
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
    test_strong_near_samples_do_not_raise_baseline
    test_weaker_near_samples_adapt_baseline_downward
    test_far_samples_never_train_baseline
    test_mid_and_far_states_never_train_baseline
    test_normal_near_signal_stays_near_after_strong_samples
    test_single_weak_spike_does_not_become_far
    test_mid_hysteresis_avoids_flap
    test_departure_behavior_remains_conservative
    test_far_persistence_locks_once
    test_proximity_lock_reason_starts_proximity
    test_proximity_lock_reason_survives_pending_locked_tick
    test_proximity_lock_reason_survives_multiple_locked_ticks
    test_single_strong_raw_spike_does_not_return_near
    test_same_rssi_sample_is_counted_once
    test_sustained_strong_raw_samples_return_quickly
    test_sustained_strong_raw_samples_return_from_mid
    test_alternating_raw_samples_do_not_flap
    test_settling_does_not_train_baseline
    test_fast_return_waits_for_late_challenge_ready
    test_fast_return_waits_for_late_heartbeat
    test_stale_rssi_with_heartbeat_does_not_become_far
    test_lost_heartbeat_becomes_absent_and_locks
    test_proximity_lock_reason_is_runtime_only
    test_proximity_return_sends_one_auth
    test_mid_to_near_return_sends_one_auth
    test_return_waits_for_late_challenge_ready
    test_return_waits_for_late_heartbeat
    test_phone_remaining_near_does_not_spam
    test_denied_auth_has_no_retry
    test_timeout_auth_has_no_retry
    test_manual_lock_near_does_not_auto_auth
    test_manual_lock_near_stays_quiet
    test_manual_lock_arms_after_far_departure
    test_manual_lock_arms_after_absent_departure
    test_manual_armed_return_sends_one_auth
    test_manual_absent_armed_return_sends_one_auth
    test_manual_wait_near_samples_never_auth
    test_unlock_resets_proximity_provenance
    test_manual_lock_explicit_trigger_remains_dms_owned
    test_auth_cancellation_guards_remain_present
    test_no_late_notification_contract_remains_present
    test_return_settling_blocks_stale_filtered_relock
    test_restart_does_not_duplicate_fast_return_sample
    test_disconnect_reconnect_reaches_absent_then_near
    test_restart_does_not_invent_proximity_lock
    test_restart_status_recovers_without_markers
    test_restart_does_not_use_stale_presence_fallback
    test_balanced_is_default_profile
    test_profile_changes_share_engine
    test_balanced_return_delta_is_minus_three
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
