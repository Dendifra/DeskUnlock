//! Durable staging, promotion and recovery for the V2 pairing transaction.
//!
//! One on-disk format, shared by the daemon-owned pair engine and the crash
//! recovery service:
//!
//! - `<bond_dir>/pairing-v2.journal` — secret-free transaction marker,
//! - `<bond_dir>/pairing-v2.pending` — staged inactive bond store,
//! - `<bond_dir>/keys/.pending-<peer_id>.bin` — staged bond key.
//!
//! The active trust (`bonds.toml` + `keys/<peer_id>.bin`) is written only by
//! [`promote`], which runs exclusively after the bilateral V2 `COMMITTED`.
//! Before that, a peer exists only in the staging area and never as an active
//! key. An interrupted transaction leaves the staging intact so the recovery
//! service can decide the outcome later; there is no arbitrary rollback of an
//! uncertain transaction.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::bond::{Bond, BondError, BondStore};

/// Active bond store filename.
pub const BONDS_FILE_NAME: &str = "bonds.toml";
/// Per-peer key directory name under the bond directory.
pub const KEYS_DIR_NAME: &str = "keys";
/// Per-peer key file extension.
pub const KEY_FILE_EXT: &str = ".bin";
/// Secret-free durable marker for an in-flight V2 commit.
pub const JOURNAL_FILE_NAME: &str = "pairing-v2.journal";
/// Staged inactive bond store filename.
pub const PENDING_BOND_FILE_NAME: &str = "pairing-v2.pending";
/// Prefix of the staged per-peer key file.
pub const PENDING_KEY_PREFIX: &str = ".pending-";

/// Recovery error surface.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    /// Filesystem failure while staging, promoting or discarding.
    #[error("pair recovery i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// Bond store failure while staging or promoting.
    #[error("pair recovery bond error: {0}")]
    Bond(#[from] BondError),
    /// The journal or staged bond is missing or malformed.
    #[error("pair recovery state is malformed")]
    Malformed,
}

/// Coarse journal state. It distinguishes a staged attempt that has not yet
/// recorded a local commit decision from one that has, so recovery can tell a
/// definitely aborted transaction from a possibly committed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalState {
    /// Staged; no local commit decision recorded yet.
    PreCommit,
    /// The local commit decision is durable.
    CommitPending,
}

impl JournalState {
    fn as_str(self) -> &'static str {
        match self {
            Self::PreCommit => "pre_commit",
            Self::CommitPending => "commit_pending",
        }
    }
}

/// Parsed `pairing-v2.journal`. Contains no secret material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRecord {
    /// Transaction identifier bound to the staged attempt.
    pub transaction: [u8; 16],
    /// Stable peer identifier.
    pub peer_id: String,
    /// BlueZ address captured at staging time.
    pub peer_address: String,
    /// Durable local decision state.
    pub state: JournalState,
}

/// Path of the active bond store.
#[must_use]
pub fn bonds_path(bond_dir: &Path) -> PathBuf {
    bond_dir.join(BONDS_FILE_NAME)
}

/// Path of the recovery journal.
#[must_use]
pub fn journal_path(bond_dir: &Path) -> PathBuf {
    bond_dir.join(JOURNAL_FILE_NAME)
}

/// Path of the staged inactive bond store.
#[must_use]
pub fn pending_bond_path(bond_dir: &Path) -> PathBuf {
    bond_dir.join(PENDING_BOND_FILE_NAME)
}

/// Path of the per-peer key directory.
#[must_use]
pub fn keys_dir(bond_dir: &Path) -> PathBuf {
    bond_dir.join(KEYS_DIR_NAME)
}

/// Path of the active per-peer key.
#[must_use]
pub fn active_key_path(bond_dir: &Path, peer_id: &str) -> PathBuf {
    keys_dir(bond_dir).join(format!("{peer_id}{KEY_FILE_EXT}"))
}

/// Path of the staged per-peer key.
#[must_use]
pub fn pending_key_path(bond_dir: &Path, peer_id: &str) -> PathBuf {
    keys_dir(bond_dir).join(format!("{PENDING_KEY_PREFIX}{peer_id}{KEY_FILE_EXT}"))
}

/// Write the secret-free journal, creating the bond directory (mode 0700).
pub fn write_journal(
    bond_dir: &Path,
    transaction: [u8; 16],
    peer_id: &str,
    peer_address: &str,
    state: JournalState,
) -> Result<(), RecoveryError> {
    fs::create_dir_all(bond_dir)?;
    #[cfg(unix)]
    fs::set_permissions(bond_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let id = transaction.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    fs::write(
        journal_path(bond_dir),
        format!(
            "version=2\ntransaction_id={id}\npeer_id={peer_id}\npeer_address={peer_address}\nstate={}\n",
            state.as_str()
        ),
    )?;
    Ok(())
}

/// Record the durable local commit decision in the journal. Runs after the
/// local COMMIT decision and before the bilateral COMMITTED exchange.
pub fn mark_commit_pending(bond_dir: &Path) -> Result<(), RecoveryError> {
    let Some(record) = read_journal(bond_dir)? else {
        return Err(RecoveryError::Malformed);
    };
    write_journal(
        bond_dir,
        record.transaction,
        &record.peer_id,
        &record.peer_address,
        JournalState::CommitPending,
    )
}

/// Read and parse the recovery journal. `Ok(None)` means no journal exists,
/// i.e. there is nothing to recover.
pub fn read_journal(bond_dir: &Path) -> Result<Option<JournalRecord>, RecoveryError> {
    let text = match fs::read_to_string(journal_path(bond_dir)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name).and_then(|value| value.strip_prefix('=')))
            .map(str::to_owned)
    };
    let id_text = field("transaction_id").ok_or(RecoveryError::Malformed)?;
    if id_text.len() != 32 {
        return Err(RecoveryError::Malformed);
    }
    let mut transaction = [0u8; 16];
    for (index, byte) in transaction.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&id_text[index * 2..index * 2 + 2], 16).map_err(|_| RecoveryError::Malformed)?;
    }
    let state = match field("state").as_deref().unwrap_or("pre_commit") {
        "pre_commit" => JournalState::PreCommit,
        "commit_pending" => JournalState::CommitPending,
        _ => return Err(RecoveryError::Malformed),
    };
    Ok(Some(JournalRecord {
        transaction,
        peer_id: field("peer_id").ok_or(RecoveryError::Malformed)?,
        peer_address: field("peer_address").ok_or(RecoveryError::Malformed)?,
        state,
    }))
}

/// Durably stage one inactive bond and its key, then write the journal.
///
/// After a successful call the peer exists only in the staging area. No
/// active trust is written here.
pub fn stage(
    bond_dir: &Path,
    transaction: [u8; 16],
    peer_id: &str,
    peer_address: &str,
    bond: &Bond,
    bond_key: &[u8; crate::bond::BOND_KEY_DERIVED_BYTES],
) -> Result<(), RecoveryError> {
    fs::create_dir_all(bond_dir)?;
    #[cfg(unix)]
    fs::set_permissions(bond_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let mut pending = BondStore::empty();
    pending.add(bond.clone())?;
    pending.save(&pending_bond_path(bond_dir))?;

    let staged_key = pending_key_path(bond_dir, peer_id);
    let parent = staged_key.parent().ok_or(RecoveryError::Malformed)?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    let staged_tmp = staged_key.with_extension("bin.tmp");
    fs::write(&staged_tmp, bond_key)?;
    #[cfg(unix)]
    fs::set_permissions(&staged_tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    fs::rename(&staged_tmp, &staged_key)?;

    write_journal(bond_dir, transaction, peer_id, peer_address, JournalState::PreCommit)
}

/// Promote the staged bond/key for `peer_id` into the active trust, then
/// clear the recovery state.
///
/// This is the only path that makes a peer an active key, and it must run
/// only after the bilateral V2 `COMMITTED`.
pub fn promote(bond_dir: &Path, peer_id: &str) -> Result<(), RecoveryError> {
    let staged = BondStore::load(&pending_bond_path(bond_dir))?;
    let bond = staged
        .list()
        .iter()
        .find(|bond| bond.peer_id == peer_id)
        .cloned()
        .ok_or(RecoveryError::Malformed)?;

    let staged_key = pending_key_path(bond_dir, peer_id);
    let final_key = active_key_path(bond_dir, peer_id);
    if staged_key.exists() {
        let key = fs::read(&staged_key)?;
        if key.len() != crate::bond::BOND_KEY_DERIVED_BYTES {
            return Err(RecoveryError::Malformed);
        }
        let parent = final_key.parent().ok_or(RecoveryError::Malformed)?;
        fs::create_dir_all(parent)?;
        let final_tmp = final_key.with_extension("bin.tmp");
        fs::write(&final_tmp, key)?;
        #[cfg(unix)]
        fs::set_permissions(&final_tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        fs::rename(&final_tmp, &final_key)?;
    } else if !final_key.exists() {
        return Err(RecoveryError::Malformed);
    }

    let active_path = bonds_path(bond_dir);
    let mut active = BondStore::load(&active_path)?;
    if active.list().iter().any(|existing| existing.peer_id == bond.peer_id) {
        active.remove(&bond.peer_id)?;
    }
    active.add(bond)?;
    active.save(&active_path)?;

    discard(bond_dir, peer_id);
    Ok(())
}

/// Drop the staged state for `peer_id` after a definite pre-commit abort.
/// Never touches the active trust and never fails.
pub fn discard(bond_dir: &Path, peer_id: &str) {
    let _ = fs::remove_file(journal_path(bond_dir));
    let _ = fs::remove_file(pending_bond_path(bond_dir));
    let _ = fs::remove_file(pending_key_path(bond_dir, peer_id));
}

/// Drop every staged artifact under `bond_dir`, including staged keys whose
/// peer id is only known from the pending store.
pub fn discard_all(bond_dir: &Path) {
    if let Ok(store) = BondStore::load(&pending_bond_path(bond_dir)) {
        for bond in store.list() {
            let _ = fs::remove_file(pending_key_path(bond_dir, &bond.peer_id));
        }
    }
    let _ = fs::remove_file(journal_path(bond_dir));
    let _ = fs::remove_file(pending_bond_path(bond_dir));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bond::{BondStatus, bond_key_from_pubkeys, peer_id_from_pubkey};
    use ::time::OffsetDateTime;

    const TRANSACTION: [u8; 16] = [7; 16];
    const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
    const HOST: [u8; 32] = [1u8; 32];
    const PHONE: [u8; 32] = [2u8; 32];

    fn peer_id() -> String {
        peer_id_from_pubkey(&PHONE)
    }

    fn bond() -> Bond {
        Bond {
            peer_id: peer_id(),
            pubkey: PHONE,
            name: "phone".to_owned(),
            created_at: OffsetDateTime::now_utc(),
            status: BondStatus::Bonded,
        }
    }

    fn bond_key() -> [u8; crate::bond::BOND_KEY_DERIVED_BYTES] {
        bond_key_from_pubkeys(&HOST, &PHONE)
    }

    #[test]
    fn stage_writes_no_active_trust_and_journal_roundtrips() {
        let dir = tempfile::tempdir().expect("tempdir");
        stage(dir.path(), TRANSACTION, &peer_id(), ADDRESS, &bond(), &bond_key()).expect("stage");

        assert!(!bonds_path(dir.path()).exists(), "staging must not write active bonds");
        assert!(
            !active_key_path(dir.path(), &peer_id()).exists(),
            "staging must not write an active key"
        );
        assert!(pending_bond_path(dir.path()).exists());
        assert!(pending_key_path(dir.path(), &peer_id()).exists());
        let journal = read_journal(dir.path()).expect("journal").expect("present");
        assert_eq!(journal.transaction, TRANSACTION);
        assert_eq!(journal.peer_id, peer_id());
        assert_eq!(journal.peer_address, ADDRESS);
        assert_eq!(
            journal.state,
            JournalState::PreCommit,
            "staging must not claim a commit decision the engine has not made"
        );

        mark_commit_pending(dir.path()).expect("mark commit");
        let journal = read_journal(dir.path()).expect("journal").expect("present");
        assert_eq!(journal.state, JournalState::CommitPending);
    }

    #[test]
    fn promote_is_the_only_active_trust_writer_and_clears_recovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        stage(dir.path(), TRANSACTION, &peer_id(), ADDRESS, &bond(), &bond_key()).expect("stage");

        promote(dir.path(), &peer_id()).expect("promote");

        assert!(bonds_path(dir.path()).exists());
        assert!(active_key_path(dir.path(), &peer_id()).exists());
        assert!(!pending_bond_path(dir.path()).exists(), "promotion clears the pending store");
        assert!(
            !pending_key_path(dir.path(), &peer_id()).exists(),
            "promotion clears the staged key"
        );
        assert!(!journal_path(dir.path()).exists(), "promotion clears the journal");
        let active = BondStore::load(&bonds_path(dir.path())).expect("active store");
        assert_eq!(active.list().len(), 1);
        assert_eq!(active.list()[0].peer_id, peer_id());
    }

    #[test]
    fn promote_is_idempotent_and_replaces_the_same_peer() {
        let dir = tempfile::tempdir().expect("tempdir");
        for _ in 0..2 {
            stage(dir.path(), TRANSACTION, &peer_id(), ADDRESS, &bond(), &bond_key()).expect("stage");
            promote(dir.path(), &peer_id()).expect("promote");
        }
        let active = BondStore::load(&bonds_path(dir.path())).expect("active store");
        assert_eq!(active.list().len(), 1, "re-promotion must not duplicate the peer");
    }

    #[test]
    fn discard_clears_staging_without_touching_active_trust() {
        let dir = tempfile::tempdir().expect("tempdir");
        stage(dir.path(), TRANSACTION, &peer_id(), ADDRESS, &bond(), &bond_key()).expect("stage");

        discard(dir.path(), &peer_id());

        assert!(!journal_path(dir.path()).exists());
        assert!(!pending_bond_path(dir.path()).exists());
        assert!(!pending_key_path(dir.path(), &peer_id()).exists());
        assert!(!bonds_path(dir.path()).exists());
        assert!(!active_key_path(dir.path(), &peer_id()).exists());
    }

    #[test]
    fn missing_journal_reads_as_none_and_malformed_reads_as_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_journal(dir.path()).expect("no journal"), None);
        fs::write(journal_path(dir.path()), "version=2\ntransaction_id=zz\n").expect("write");
        assert!(matches!(read_journal(dir.path()), Err(RecoveryError::Malformed)));
    }
}
