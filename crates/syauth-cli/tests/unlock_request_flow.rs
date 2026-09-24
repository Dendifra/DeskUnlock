// Journey: specs/journeys/JOURNEY-S-022-out-of-band-unlock.md
//
// Integration tests for `syauth unlock-request` — the out-of-band unlock
// path that replaced `pam_syauth.so` in the *lock* PAM service
// (`dankshell`). The entire point of this verb is that it can only ever do
// one thing, so every test asserts not just the exit code but whether the
// session was touched: `loginctl` is replaced by a recording script, and
// the assertions check that script's call log.
//
// | TC | Scenario                                              |
// |----|-------------------------------------------------------|
// | 01 | `unlocks_when_the_daemon_verifies_the_phone`           |
// | 02 | `a_refused_challenge_leaves_the_session_untouched`     |
// | 03 | `a_dry_run_never_runs_the_unlocker`                    |
// | 04 | `a_dead_daemon_is_a_refusal_not_an_unlock`             |
// | 05 | `the_challenge_targets_the_newest_bonded_peer`         |

#![allow(clippy::expect_used)]

use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use assert_cmd::Command;
use syauth_core::{Bond, BondStatus, BondStore, SigningKey, peer_id_from_pubkey};
use syauth_presenced::{Request, Response, read_frame_blocking, write_frame_blocking};
use tempfile::TempDir;
use time::OffsetDateTime;

/// The one argument the real `loginctl` gets. Pinned here so a rename in
/// the verb cannot silently change what the tests observe.
const UNLOCK_SESSION_ARG: &str = "unlock-session";

/// Marker file name the fake `loginctl` appends to.
const LOGINCTL_MARKER: &str = "loginctl-calls";

fn syauth() -> Command {
    Command::cargo_bin("syauth").expect("locate built syauth binary")
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A `loginctl` stand-in that records its arguments instead of unlocking
/// anything. This is what lets a test prove "the session was not touched".
struct FakeLoginctl {
    script: PathBuf,
    marker: PathBuf,
}

impl FakeLoginctl {
    fn install(dir: &Path) -> Self {
        let script = dir.join("fake-loginctl");
        let marker = dir.join(LOGINCTL_MARKER);
        fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\n", marker.display()),
        )
        .expect("write fake loginctl");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod fake loginctl");
        Self { script, marker }
    }

    /// Every invocation, one line of arguments per call.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.marker)
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    fn was_called(&self) -> bool {
        !self.calls().is_empty()
    }
}

/// Fake daemon: accepts one connection, records the `peer_id` the client
/// asked about, answers with the given verdict, exits.
struct FakeChallengeDaemon {
    _handle: thread::JoinHandle<()>,
    seen_peers: Arc<Mutex<Vec<String>>>,
}

impl FakeChallengeDaemon {
    fn new(socket: &Path, ok: bool, reason: &str) -> Self {
        let listener = UnixListener::bind(socket).expect("bind fake daemon listener");
        let seen_peers = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen_peers);
        let reason = reason.to_string();
        let handle = thread::spawn(move || {
            let Ok((mut stream, _addr)) = listener.accept() else {
                return;
            };
            let Ok(Request::Challenge { peer_id, .. }) = read_frame_blocking::<_, Request>(&mut stream) else {
                return;
            };
            sink.lock().expect("lock peer sink").push(peer_id);
            let response = Response::Challenge {
                ok,
                signature: None,
                reason,
            };
            let _ = write_frame_blocking(&mut stream, &response);
        });
        Self {
            _handle: handle,
            seen_peers,
        }
    }

    fn challenged_peers(&self) -> Vec<String> {
        self.seen_peers.lock().expect("lock peer sink").clone()
    }
}

/// Build a bond whose `peer_id` is derived from its pubkey, exactly like a
/// real pairing does.
fn bond(seed: u8, status: BondStatus, seconds: i64) -> Bond {
    let pubkey = SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes();
    Bond {
        peer_id: peer_id_from_pubkey(&pubkey),
        pubkey,
        name: format!("phone-{seed}"),
        created_at: OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(seconds),
        status,
    }
}

/// Write `bonds.toml` where the verb expects it (`<dir>/bonds.toml`).
fn write_bonds(dir: &Path, bonds: &[Bond]) {
    let mut store = BondStore::empty();
    for entry in bonds {
        store.add(entry.clone()).expect("add bond");
    }
    store.save(&dir.join("bonds.toml")).expect("save bonds");
}

/// Tempdir-rooted bond dir with 0o700 perms, matching a real install.
fn bond_dir(td: &TempDir) -> PathBuf {
    let dir = td.path().join("syauth");
    fs::create_dir_all(&dir).expect("mkdir bond dir");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("chmod bond dir");
    dir
}

/// The verb under test, wired to the hermetic fixtures.
fn unlock_request(dir: &Path, socket: &Path, loginctl: &FakeLoginctl, extra: &[&str]) -> Command {
    let mut cmd = syauth();
    cmd.arg("unlock-request")
        .arg("--bond-dir")
        .arg(dir)
        .arg("--socket")
        .arg(socket)
        .arg("--timeout-secs")
        .arg("5")
        .env("SYAUTH_LOGINCTL_BIN", &loginctl.script)
        .args(extra);
    cmd
}

// ---------------------------------------------------------------------------
// TC 01 — the happy path: verified phone, session unlocked
// ---------------------------------------------------------------------------

#[test]
fn unlocks_when_the_daemon_verifies_the_phone() {
    let td = TempDir::new().expect("tempdir");
    let dir = bond_dir(&td);
    let peer = bond(1, BondStatus::Bonded, 100);
    write_bonds(&dir, std::slice::from_ref(&peer));
    let socket = td.path().join("auth.sock");
    let daemon = FakeChallengeDaemon::new(&socket, true, "ok");
    let loginctl = FakeLoginctl::install(td.path());

    let assert = unlock_request(&dir, &socket, &loginctl, &[]).assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    assert!(stdout.contains("unlocked"), "stdout was {stdout:?}");
    assert!(stdout.contains(&peer.peer_id), "stdout was {stdout:?}");
    assert_eq!(daemon.challenged_peers(), vec![peer.peer_id.clone()]);
    assert_eq!(loginctl.calls(), vec![UNLOCK_SESSION_ARG.to_string()]);
}

// ---------------------------------------------------------------------------
// TC 02 — the safety property: a refusal never unlocks
// ---------------------------------------------------------------------------

#[test]
fn a_refused_challenge_leaves_the_session_untouched() {
    let td = TempDir::new().expect("tempdir");
    let dir = bond_dir(&td);
    write_bonds(&dir, &[bond(1, BondStatus::Bonded, 100)]);
    let socket = td.path().join("auth.sock");
    let _daemon = FakeChallengeDaemon::new(&socket, false, "denied");
    let loginctl = FakeLoginctl::install(td.path());

    let assert = unlock_request(&dir, &socket, &loginctl, &[]).assert().failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();

    assert!(stderr.contains("denied"), "stderr was {stderr:?}");
    assert!(
        !loginctl.was_called(),
        "loginctl must never run after a refusal, saw {:?}",
        loginctl.calls()
    );
}

// ---------------------------------------------------------------------------
// TC 03 — dry run: verify only
// ---------------------------------------------------------------------------

#[test]
fn a_dry_run_never_runs_the_unlocker() {
    let td = TempDir::new().expect("tempdir");
    let dir = bond_dir(&td);
    write_bonds(&dir, &[bond(1, BondStatus::Bonded, 100)]);
    let socket = td.path().join("auth.sock");
    let _daemon = FakeChallengeDaemon::new(&socket, true, "ok");
    let loginctl = FakeLoginctl::install(td.path());

    let assert = unlock_request(&dir, &socket, &loginctl, &["--dry-run"]).assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    assert!(stdout.contains("dry-run=1"), "stdout was {stdout:?}");
    assert!(
        !loginctl.was_called(),
        "--dry-run must not unlock, saw {:?}",
        loginctl.calls()
    );
}

// ---------------------------------------------------------------------------
// TC 04 — daemon down: fail closed, session untouched
// ---------------------------------------------------------------------------

#[test]
fn a_dead_daemon_is_a_refusal_not_an_unlock() {
    let td = TempDir::new().expect("tempdir");
    let dir = bond_dir(&td);
    write_bonds(&dir, &[bond(1, BondStatus::Bonded, 100)]);
    // No listener bound at this path: the connect must fail fast.
    let socket = td.path().join("missing.sock");
    let loginctl = FakeLoginctl::install(td.path());

    let assert = unlock_request(&dir, &socket, &loginctl, &[]).assert().failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();

    assert!(stderr.contains("daemon unreachable"), "stderr was {stderr:?}");
    assert!(
        !loginctl.was_called(),
        "a dead daemon must not unlock anything, saw {:?}",
        loginctl.calls()
    );
}

// ---------------------------------------------------------------------------
// TC 05 — peer selection follows the newest completed pairing
// ---------------------------------------------------------------------------

#[test]
fn the_challenge_targets_the_newest_bonded_peer() {
    let td = TempDir::new().expect("tempdir");
    let dir = bond_dir(&td);
    let older = bond(1, BondStatus::Bonded, 100);
    let newer = bond(2, BondStatus::Bonded, 200);
    let revoked = bond(3, BondStatus::Revoked { reason: "test".to_string() }, 300);
    write_bonds(&dir, &[older.clone(), newer.clone(), revoked]);
    let socket = td.path().join("auth.sock");
    let daemon = FakeChallengeDaemon::new(&socket, true, "ok");
    let loginctl = FakeLoginctl::install(td.path());

    unlock_request(&dir, &socket, &loginctl, &[]).assert().success();

    assert_eq!(daemon.challenged_peers(), vec![newer.peer_id.clone()]);
    assert_ne!(daemon.challenged_peers(), vec![older.peer_id]);
}
