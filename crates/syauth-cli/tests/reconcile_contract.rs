//! Contract tests for the crash-recovery convergence.
//!
//! Reconcile is crash recovery for a staged V2 transaction only. It must be
//! journal-triggered, must never run as a normal pairing step, and must exit
//! successfully when there is nothing to reconcile.

use std::path::Path;

fn repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../").join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

#[test]
fn reconcile_path_unit_watches_the_v2_journal_without_busy_looping() {
    let text = repo_file("desktop/systemd/syauth-reconcile.path");
    assert!(
        text.contains("PathChanged=/var/lib/syauth/pairing-v2.journal"),
        "reconcile.path must watch the V2 journal on change: {text}"
    );
    // Measured on hardware: PathExists with a oneshot target re-triggers the
    // moment the unit exits while the file still exists — 30 starts in one
    // second, which trips the start limit and leaves the whole user session
    // `degraded` after every pairing.
    assert!(
        !text.contains("PathExists=/var/lib/syauth/pairing-v2.journal"),
        "PathExists busy-loops a oneshot target; a change-trigger is required: {text}"
    );
    assert!(
        !text.contains("bonds.toml"),
        "reconcile.path must not trigger on the active bond store: {text}"
    );
}

#[test]
fn reconcile_retry_timer_covers_a_journal_left_by_a_crash() {
    // A change trigger cannot see a journal that already existed when the
    // session started, so an independent retry must exist.
    let text = repo_file("desktop/systemd/syauth-reconcile.timer");
    assert!(
        text.contains("Unit=syauth-reconcile.service"),
        "the retry timer must drive the reconciler: {text}"
    );
    assert!(
        text.contains("OnUnitActiveSec="),
        "the retry timer must repeat periodically: {text}"
    );
}

#[test]
fn device_control_never_runs_reconcile_as_a_pair_step() {
    let text = repo_file("desktop/bin/syauth-device");
    assert!(
        !text.contains("syauth-reconcile"),
        "ordinary pair/change must not invoke crash recovery: {text}"
    );
}

#[test]
fn gui_confirmation_client_emits_no_bare_waiting_state() {
    let text = repo_file("crates/syauth-cli/src/pair_backend.rs");
    assert!(
        !text.contains("waiting_confirmation"),
        "the GUI client must not emit a waiting state before a real request exists"
    );
    assert!(
        text.contains("waiting_lesc_confirmation"),
        "the transport confirmation must be labelled as such"
    );
}
