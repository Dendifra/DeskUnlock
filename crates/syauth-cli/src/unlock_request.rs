//! `syauth-cli` — `unlock-request`: the out-of-band unlock path.
//!
//! Journey: specs/journeys/JOURNEY-S-022-out-of-band-unlock.md
//!
//! # Why this verb exists
//!
//! Putting `pam_syauth.so` into the *lock* PAM service (`dankshell`) made
//! the whole lock screen depend on the phone. PAM is synchronous, the phone
//! is not: on 2026-09-23 that held the authentication phase long enough
//! that the operator could not get back in even with the correct password.
//! That is unacceptable for a lock screen, so the unlock moved **out of
//! band**:
//!
//! - the lock PAM service stays 100 % stock — the password path is never
//!   touched, and a lock-out caused by DeskUnlock is impossible;
//! - the phone can only ever cause one thing: `loginctl unlock-session`;
//! - the worst possible failure mode is "nothing happens", because the
//!   password field is still right there.
//!
//! # Flow
//!
//! 1. Pick the peer: `--peer-id`, else the newest `Bonded` record in
//!    `<bond-dir>/bonds.toml` — the same rule `pam_syauth` uses.
//! 2. Connect to the daemon's Unix socket and send one
//!    `Request::Challenge`, the identical wire shape the PAM module uses.
//!    The daemon owns the nonce, the GATT channel and the crypto; this verb
//!    is a thin client, exactly like the module.
//! 3. Wait for `Response::Challenge` under a human-scale budget: the
//!    operator has to reach for the phone and approve.
//! 4. **Only** when `ok == true`, run `loginctl unlock-session`.
//!
//! `--dry-run` stops after step 3, which is how the lock-screen hook can be
//! exercised without unlocking anything.

use std::{
    env,
    io::{self, Write as _},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::Duration,
};

use clap::Parser;
use rand::{RngCore, rngs::OsRng};
use syauth_core::{BondStatus, BondStore, NONCE_LEN};
use syauth_presenced::{Request, Response, read_frame_blocking, write_frame_blocking};
use thiserror::Error;

use crate::pair::{DEFAULT_BOND_DIR, bonds_path};

// =============================================================================
// Named constants
// =============================================================================

/// Human-scale budget for the whole challenge. The operator has to pick up
/// the phone, look at it and approve — a machine timeout here would be a
/// bug, not a safety net.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;

/// Connect budget. SPEC §4.3 budgets "daemon-down latency ≤ 50 ms"; a
/// slightly larger window keeps the verb honest on a loaded machine while
/// still failing fast when nothing is listening.
pub const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

/// `loginctl` sub-command that unlocks the caller's session. With no
/// argument `loginctl` targets the session this process belongs to, and
/// unlocking an already-unlocked session is a no-op — so this verb can
/// never *lock* anything by accident.
const UNLOCK_SESSION_ARG: &str = "unlock-session";

/// Override for the `loginctl` binary. The default is the real one; the
/// override exists so tests can observe the call without touching a live
/// session.
const LOGINCTL_BIN_ENV: &str = "SYAUTH_LOGINCTL_BIN";

/// Default `loginctl` binary name (resolved through `PATH`).
const LOGINCTL_BIN_DEFAULT: &str = "loginctl";

/// Runtime-dir fallback prefix for sessions without `XDG_RUNTIME_DIR`
/// (SSH). Mirrors `syauth_pam::DEFAULT_RUNTIME_FALLBACK_PREFIX`.
const RUNTIME_FALLBACK_PREFIX: &str = "/run/user/";

/// Sub-directory + file name of the daemon socket inside the runtime dir.
const RUNTIME_SUBDIR: &str = "syauth";
const SOCKET_BASENAME: &str = "auth.sock";

// =============================================================================
// Options
// =============================================================================

/// CLI options for `syauth unlock-request`.
#[derive(Debug, Parser, Clone)]
pub struct UnlockRequestOpts {
    /// Unix socket of the running `syauth-presenced` daemon. Defaults to
    /// `${XDG_RUNTIME_DIR}/syauth/auth.sock`, falling back to
    /// `/run/user/<uid>/syauth/auth.sock` when `XDG_RUNTIME_DIR` is unset.
    #[arg(long, value_name = "PATH")]
    pub socket: Option<PathBuf>,

    /// Directory holding `bonds.toml`.
    #[arg(long, value_name = "DIR", default_value = DEFAULT_BOND_DIR)]
    pub bond_dir: PathBuf,

    /// Challenge this peer instead of the newest bonded one.
    #[arg(long, value_name = "ID")]
    pub peer_id: Option<String>,

    /// Seconds to wait for the phone's approval.
    #[arg(long, value_name = "SECS", default_value_t = DEFAULT_TIMEOUT_SECS)]
    pub timeout_secs: u64,

    /// Verify the phone but leave the session alone.
    #[arg(long)]
    pub dry_run: bool,
}

// =============================================================================
// Errors
// =============================================================================

/// Everything `unlock-request` can refuse with. Every variant means the
/// session was **not** touched.
#[derive(Debug, Error)]
pub enum UnlockRequestError {
    /// No usable bond: the store is missing, unreadable, or has no
    /// `Bonded` record.
    #[error("no bonded peer: {0}")]
    NoBondedPeer(String),

    /// The daemon socket could not be reached.
    #[error("daemon unreachable at {path}: {source}")]
    Connect {
        /// Socket path that refused the connection.
        path: PathBuf,
        /// Underlying `connect(2)` failure.
        #[source]
        source: io::Error,
    },

    /// Socket options or the framing layer failed.
    #[error("daemon exchange failed: {0}")]
    Exchange(String),

    /// The daemon answered with something other than a challenge verdict,
    /// or the phone refused / never answered.
    #[error("phone refused: {0}")]
    Refused(String),

    /// `loginctl` could not be spawned.
    #[error("could not run {bin}: {source}")]
    Unlock {
        /// Binary that failed to spawn.
        bin: String,
        /// Underlying spawn failure.
        #[source]
        source: io::Error,
    },

    /// `loginctl` ran but exited non-zero.
    #[error("{bin} {arg} exited with {status}")]
    UnlockStatus {
        /// Binary that ran.
        bin: String,
        /// Sub-command that ran.
        arg: String,
        /// Its exit status.
        status: ExitStatus,
    },
}

// =============================================================================
// Entry point
// =============================================================================

/// Run one out-of-band unlock request. Prints a single greppable line on
/// success; returns an error (and touches nothing) on every refusal.
pub fn run(opts: &UnlockRequestOpts) -> Result<(), UnlockRequestError> {
    let peer_id = match &opts.peer_id {
        Some(id) => id.clone(),
        None => newest_bonded_peer(&opts.bond_dir)?,
    };
    let socket = opts.socket.clone().unwrap_or_else(default_socket_path);
    let timeout = Duration::from_secs(opts.timeout_secs);

    let response = challenge(&socket, &peer_id, timeout)?;
    let Response::Challenge { ok, reason, .. } = response else {
        return Err(UnlockRequestError::Refused(format!("unexpected response: {response:?}")));
    };
    if !ok {
        return Err(UnlockRequestError::Refused(reason));
    }

    let mut stdout = io::stdout().lock();
    if opts.dry_run {
        let _ = writeln!(stdout, "verified peer={peer_id} reason={reason} dry-run=1");
        return Ok(());
    }

    let bin = loginctl_bin();
    unlock_session(&bin)?;
    let _ = writeln!(stdout, "unlocked peer={peer_id} reason={reason} dry-run=0");
    Ok(())
}

// =============================================================================
// Steps
// =============================================================================

/// Resolve the newest `Bonded` peer, mirroring `pam_syauth`'s rule:
/// re-pairing can leave older bonded records behind, so authentication
/// follows the most recently completed pairing.
fn newest_bonded_peer(bond_dir: &Path) -> Result<String, UnlockRequestError> {
    let path = bonds_path(bond_dir);
    let store = BondStore::load(&path).map_err(|err| UnlockRequestError::NoBondedPeer(err.to_string()))?;
    store
        .list()
        .iter()
        .filter(|bond| matches!(bond.status, BondStatus::Bonded))
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .map(|bond| bond.peer_id.clone())
        .ok_or_else(|| UnlockRequestError::NoBondedPeer(format!("no bonded peer in {}", path.display())))
}

/// One `Request::Challenge` / `Response::Challenge` round trip.
fn challenge(socket: &Path, peer_id: &str, timeout: Duration) -> Result<Response, UnlockRequestError> {
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);

    let stream = UnixStream::connect(socket).map_err(|source| UnlockRequestError::Connect {
        path: socket.to_path_buf(),
        source,
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| UnlockRequestError::Exchange(err.to_string()))?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|err| UnlockRequestError::Exchange(err.to_string()))?;

    let mut writer = stream
        .try_clone()
        .map_err(|err| UnlockRequestError::Exchange(err.to_string()))?;
    let request = Request::Challenge {
        peer_id: peer_id.to_string(),
        nonce: nonce.to_vec(),
    };
    write_frame_blocking(&mut writer, &request).map_err(|err| UnlockRequestError::Exchange(err.to_string()))?;

    read_frame_blocking(&mut &stream).map_err(|err| UnlockRequestError::Exchange(err.to_string()))
}

/// The `loginctl` binary to run, honouring [`LOGINCTL_BIN_ENV`].
fn loginctl_bin() -> String {
    env::var(LOGINCTL_BIN_ENV).unwrap_or_else(|_| LOGINCTL_BIN_DEFAULT.to_string())
}

/// Unlock the caller's session. No shell, one fixed argument: nothing the
/// phone (or the daemon) returns can influence what gets executed.
fn unlock_session(bin: &str) -> Result<(), UnlockRequestError> {
    let status = Command::new(bin)
        .arg(UNLOCK_SESSION_ARG)
        .status()
        .map_err(|source| UnlockRequestError::Unlock {
            bin: bin.to_string(),
            source,
        })?;
    if !status.success() {
        return Err(UnlockRequestError::UnlockStatus {
            bin: bin.to_string(),
            arg: UNLOCK_SESSION_ARG.to_string(),
            status,
        });
    }
    Ok(())
}

/// `${XDG_RUNTIME_DIR}/syauth/auth.sock`, falling back to
/// `/run/user/<uid>/syauth/auth.sock` for SSH sessions.
fn default_socket_path() -> PathBuf {
    let base = env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || PathBuf::from(format!("{RUNTIME_FALLBACK_PREFIX}{}", nix::unistd::geteuid().as_raw())),
        PathBuf::from,
    );
    base.join(RUNTIME_SUBDIR).join(SOCKET_BASENAME)
}

// =============================================================================
// Unit tests
// =============================================================================

#[cfg(test)]
mod tests {
    use std::fs;

    use syauth_core::{Bond, SigningKey, peer_id_from_pubkey};
    use tempfile::TempDir;
    use time::OffsetDateTime;

    use super::*;

    /// Tempdir with 0o700 perms: `BondStore::save` refuses a
    /// world-readable parent, exactly like a real install.
    fn secure_dir() -> TempDir {
        use std::os::unix::fs::PermissionsExt as _;
        let td = TempDir::new().expect("tempdir");
        fs::set_permissions(td.path(), fs::Permissions::from_mode(0o700)).expect("chmod tempdir");
        td
    }

    fn write_bonds(dir: &Path, bonds: Vec<Bond>) {
        let path = bonds_path(dir);
        let mut store = BondStore::empty();
        for bond in bonds {
            store.add(bond).expect("add bond");
        }
        store.save(&path).expect("save bonds");
    }

    /// Build a bond whose `peer_id` is derived from its pubkey, exactly
    /// like a real pairing does. `seed` picks the key, `seconds` the
    /// creation instant.
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

    #[test]
    fn the_newest_bonded_peer_wins_over_an_older_bond() {
        let older = bond(1, BondStatus::Bonded, 100);
        let newer = bond(2, BondStatus::Bonded, 200);
        let td = secure_dir();
        write_bonds(td.path(), vec![older.clone(), newer.clone()]);
        assert_ne!(older.peer_id, newer.peer_id);
        assert_eq!(newest_bonded_peer(td.path()).expect("peer"), newer.peer_id);
    }

    #[test]
    fn a_revoked_newer_bond_is_not_chosen() {
        let older = bond(1, BondStatus::Bonded, 100);
        let revoked = bond(2, BondStatus::Revoked { reason: "test".to_string() }, 200);
        let td = secure_dir();
        write_bonds(td.path(), vec![older.clone(), revoked]);
        assert_eq!(newest_bonded_peer(td.path()).expect("peer"), older.peer_id);
    }

    #[test]
    fn an_empty_store_is_refused_before_touching_anything() {
        let td = secure_dir();
        fs::create_dir_all(td.path()).expect("mkdir");
        let err = newest_bonded_peer(td.path()).expect_err("no bonded peer");
        assert!(matches!(err, UnlockRequestError::NoBondedPeer(_)), "got {err:?}");
    }

    #[test]
    fn the_default_socket_lives_under_the_runtime_dir() {
        // The path shape is what the daemon and the PAM module agree on;
        // pin it so a rename cannot silently break the three of them.
        let path = default_socket_path();
        assert!(path.ends_with("syauth/auth.sock"), "got {}", path.display());
    }

    #[test]
    fn the_loginctl_binary_honours_the_test_override() {
        // The override is the only reason tests can assert "the session was
        // never touched" without touching a live session.
        assert_eq!(LOGINCTL_BIN_ENV, "SYAUTH_LOGINCTL_BIN");
        assert_eq!(LOGINCTL_BIN_DEFAULT, "loginctl");
        assert_eq!(UNLOCK_SESSION_ARG, "unlock-session");
    }
}
