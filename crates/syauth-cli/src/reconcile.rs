//! Crash recovery for a staged V2 pairing transaction.
use std::{
    fs,
    path::{Path, PathBuf},
};

use clap::Args;
use rand::{RngCore, rngs::OsRng};
use syauth_core::{
    bond::{Bond, BondStore},
    pair_transaction::{Role, StatusMessage, StatusState, Transaction},
};
use syauth_transport::{DEFAULT_ADAPTER_NAME, query_pair_status};
use thiserror::Error;

use crate::pair::transaction_journal_path;

#[derive(Debug, Args)]
pub struct ReconcileOpts {
    /// BlueZ adapter used for the reconnect.
    #[arg(long, default_value = DEFAULT_ADAPTER_NAME)]
    pub adapter: String,
    /// Durable bond directory.
    #[arg(long, default_value = "/var/lib/syauth")]
    pub bond_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("recovery journal unavailable")]
    NoJournal,
    #[error("malformed recovery journal")]
    MalformedJournal,
    #[error("recovery peer status unavailable: {0}")]
    Transport(String),
    #[error("recovery transaction rejected: {0}")]
    Transaction(String),
    #[error("recovery storage error: {0}")]
    Io(#[from] std::io::Error),
    #[error("recovery bond error: {0}")]
    Bond(#[from] syauth_core::bond::BondError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileResult {
    Bonded,
    Aborted,
    Uncertain,
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(name).and_then(|v| v.strip_prefix('=')))
}

fn hex_id(text: &str) -> Result<[u8; 16], ReconcileError> {
    if text.len() != 32 {
        return Err(ReconcileError::MalformedJournal);
    }
    let mut id = [0; 16];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| ReconcileError::MalformedJournal)?;
    }
    Ok(id)
}

fn pending_path(dir: &Path) -> PathBuf {
    syauth_core::pair_recovery::pending_bond_path(dir)
}
async fn authenticated_status(adapter_name: &str, address: &str, query: StatusMessage) -> Result<StatusMessage, ReconcileError> {
    let session = bluer::Session::new().await.map_err(|e| ReconcileError::Transport(e.to_string()))?;
    let adapter = session
        .adapter(adapter_name)
        .map_err(|e| ReconcileError::Transport(e.to_string()))?;
    let address = address.parse::<bluer::Address>().map_err(|_| ReconcileError::MalformedJournal)?;
    let device = adapter.device(address).map_err(|e| ReconcileError::Transport(e.to_string()))?;
    // The address comes from the journal, not a scan/name lookup. The
    // encrypted/authenticated characteristic permissions are the peer binding.
    query_pair_status(&device, query)
        .await
        .map_err(|e| ReconcileError::Transport(e.to_string()))
}

fn clear_recovery(dir: &Path, peer_id: &str) {
    syauth_core::pair_recovery::discard(dir, peer_id);
}

/// Promote a bilaterally committed staged bond into the active trust. Uses
/// the same promotion helper as the daemon-owned pair engine, so recovery and
/// the live path can never disagree on the on-disk format.
fn finalize(dir: &Path, pending: &Bond) -> Result<(), ReconcileError> {
    syauth_core::pair_recovery::promote(dir, &pending.peer_id).map_err(|err| match err {
        syauth_core::pair_recovery::RecoveryError::Io(io) => ReconcileError::Io(io),
        other => ReconcileError::Transaction(other.to_string()),
    })
}

pub async fn run_reconcile(opts: &ReconcileOpts) -> Result<ReconcileResult, ReconcileError> {
    let dir = &opts.bond_dir;
    let journal = fs::read_to_string(transaction_journal_path(dir)).map_err(|_| ReconcileError::NoJournal)?;
    let id = hex_id(field(&journal, "transaction_id").ok_or(ReconcileError::MalformedJournal)?)?;
    let peer_id = field(&journal, "peer_id").ok_or(ReconcileError::MalformedJournal)?;
    let address = field(&journal, "peer_address").ok_or(ReconcileError::MalformedJournal)?;
    let pending = BondStore::load(&pending_path(dir))?
        .list()
        .first()
        .cloned()
        .ok_or(ReconcileError::MalformedJournal)?;
    if pending.peer_id != peer_id {
        return Err(ReconcileError::MalformedJournal);
    }

    let local_status = match field(&journal, "state").unwrap_or("commit_pending") {
        "pre_commit" => StatusState::PreCommit,
        "commit_pending" => StatusState::CommitPending,
        "aborted" => StatusState::Aborted,
        _ => return Err(ReconcileError::MalformedJournal),
    };
    let mut tx =
        Transaction::from_recovery_status(id, Role::Coordinator, local_status).map_err(|e| ReconcileError::Transaction(e.to_string()))?;
    let mut nonce_bytes = [0; 8];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = u64::from_be_bytes(nonce_bytes);
    let response = authenticated_status(&opts.adapter, address, StatusMessage::query(id, nonce)).await?;
    apply_status_decision(dir, &pending, &mut tx, response, nonce)
}

/// Apply the authenticated remote status to a recovered transaction and act on
/// the durable outcome.
///
/// This is the pure post-transport decision used by [`run_reconcile`], kept
/// separate so every terminal outcome can be exercised without a radio:
///
/// - a bilateral `Committed`/`Bonded` promotes the staged bond and key;
/// - a remote `Aborted` on a transaction with **no** durable local commit
///   decision discards the staging;
/// - anything inconclusive retains the journal and pending state so recovery
///   can run again later.
fn apply_status_decision(
    dir: &Path,
    pending: &Bond,
    tx: &mut Transaction,
    response: StatusMessage,
    nonce: u64,
) -> Result<ReconcileResult, ReconcileError> {
    let remote = response
        .state
        .ok_or_else(|| ReconcileError::Transaction("query received instead of response".to_owned()))?;
    tx.apply_status_response(response, nonce)
        .map_err(|e| ReconcileError::Transaction(e.to_string()))?;
    if remote == StatusState::Unknown || tx.phase() == syauth_core::pair_transaction::Phase::Uncertain {
        return Ok(ReconcileResult::Uncertain);
    }
    if tx.phase() == syauth_core::pair_transaction::Phase::Committed {
        tx.local(syauth_core::pair_transaction::LocalEvent::Committed)
            .map_err(|e| ReconcileError::Transaction(e.to_string()))?;
        tx.reconcile_status(remote)
            .map_err(|e| ReconcileError::Transaction(e.to_string()))?;
    }
    match tx.phase() {
        syauth_core::pair_transaction::Phase::Bonded => {
            finalize(dir, pending)?;
            Ok(ReconcileResult::Bonded)
        }
        syauth_core::pair_transaction::Phase::Aborted(_) => {
            clear_recovery(dir, &pending.peer_id);
            Ok(ReconcileResult::Aborted)
        }
        _ => Ok(ReconcileResult::Uncertain),
    }
}

pub async fn run_reconcile_cli(opts: &ReconcileOpts) -> Result<(), ReconcileError> {
    match run_reconcile(opts).await {
        Ok(ReconcileResult::Bonded) => println!("syauth-reconcile: bilateral BONDED; staged bond activated"),
        Ok(ReconcileResult::Aborted) => println!("syauth-reconcile: bilateral ABORTED; staged bond removed"),
        Ok(ReconcileResult::Uncertain) => {
            eprintln!("syauth-reconcile: outcome uncertain; recovery retained");
        }
        // Crash recovery only: no staged V2 transaction means there is
        // nothing to reconcile. This is a normal, successful no-op, not a
        // service failure.
        Err(ReconcileError::NoJournal) => {
            println!("syauth-reconcile: no staged pairing transaction; nothing to reconcile");
        }
        // The peer is simply not connected: the phone is out of range, or a
        // pairing is still in flight. The staged transaction is kept for a
        // later attempt, so this is a transient condition — reporting it as a
        // unit failure makes the path unit re-arm into `start-limit-hit` and
        // leaves the user session `degraded`, which hides real failures.
        Err(ReconcileError::Transport(reason)) => {
            eprintln!("syauth-reconcile: peer unreachable ({reason}); staged transaction kept for a later attempt");
        }
        Err(err) => return Err(err),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_journal_is_a_successful_no_op() {
        let dir = tempfile::tempdir().expect("tempdir");
        let opts = ReconcileOpts {
            adapter: "hci0".to_owned(),
            bond_dir: dir.path().to_path_buf(),
        };
        // Crash recovery has nothing to do, and "nothing to do" is success.
        assert!(matches!(run_reconcile(&opts).await, Err(ReconcileError::NoJournal)));
        assert!(
            run_reconcile_cli(&opts).await.is_ok(),
            "a missing journal must not leave the service failed"
        );
    }

    #[tokio::test]
    async fn an_unreachable_peer_is_a_transient_outcome_not_a_service_failure() {
        let dir = private_tempdir();
        stage_recovery_fixture(dir.path(), false);
        let opts = ReconcileOpts {
            adapter: "hci0".to_owned(),
            bond_dir: dir.path().to_path_buf(),
        };

        // Without a radio the reconciler stops at the authenticated transport
        // step. The unit must not be counted as failed: the path unit would
        // re-arm into `start-limit-hit` and leave the session degraded, and a
        // real failure would then be invisible.
        assert!(run_reconcile_cli(&opts).await.is_ok(), "an unreachable peer must not fail the unit");
        assert!(
            syauth_core::pair_recovery::journal_path(dir.path()).exists(),
            "the staged transaction must be kept for a later attempt"
        );
    }

    #[tokio::test]
    async fn reconcile_reads_the_state_written_by_the_daemon_engine() {
        let dir = tempfile::tempdir().expect("tempdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
        }
        let host = [1u8; 32];
        let phone = [2u8; 32];
        let peer_id = syauth_core::peer_id_from_pubkey(&phone);
        let bond_key = syauth_core::bond_key_from_pubkeys(&host, &phone);
        let bond = syauth_core::Bond {
            peer_id: peer_id.clone(),
            pubkey: phone,
            name: "phone".to_owned(),
            created_at: ::time::OffsetDateTime::now_utc(),
            status: syauth_core::BondStatus::Bonded,
        };
        // Exactly what the daemon-owned pair engine stages before COMMIT.
        syauth_core::pair_recovery::stage(dir.path(), [7u8; 16], &peer_id, "AA:BB:CC:DD:EE:FF", &bond, &bond_key).expect("stage");

        let opts = ReconcileOpts {
            adapter: "hci0".to_owned(),
            bond_dir: dir.path().to_path_buf(),
        };
        // The daemon-produced journal must be consumed, not reported as
        // missing. Without a radio the reconciler stops at the authenticated
        // transport step, which proves it parsed the staged state.
        match run_reconcile(&opts).await {
            Err(ReconcileError::Transport(_)) | Err(ReconcileError::Transaction(_)) => {}
            other => panic!("expected the reconciler to read the staged journal, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Requirement E: the reconcile decision lifecycle, driven through the
    // same `apply_status_decision` the production reconciler calls, over a
    // real `pair_recovery::stage` in a temporary bond directory.
    // -----------------------------------------------------------------

    fn private_tempdir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
        }
        dir
    }

    /// Real staged V2 state, exactly as the daemon engine produces it.
    /// `commit_decision` models whether the engine recorded its durable local
    /// commit decision before the crash.
    fn stage_recovery_fixture(dir: &Path, commit_decision: bool) -> ([u8; 16], String) {
        let host = [1u8; 32];
        let phone = [2u8; 32];
        let peer_id = syauth_core::peer_id_from_pubkey(&phone);
        let bond_key = syauth_core::bond_key_from_pubkeys(&host, &phone);
        let bond = syauth_core::Bond {
            peer_id: peer_id.clone(),
            pubkey: phone,
            name: "phone".to_owned(),
            created_at: ::time::OffsetDateTime::now_utc(),
            status: syauth_core::BondStatus::Bonded,
        };
        let transaction = [7u8; 16];
        syauth_core::pair_recovery::stage(dir, transaction, &peer_id, "AA:BB:CC:DD:EE:FF", &bond, &bond_key).expect("stage");
        if commit_decision {
            syauth_core::pair_recovery::mark_commit_pending(dir).expect("mark commit");
        }
        (transaction, peer_id)
    }

    fn recovery_transaction(dir: &Path) -> (Transaction, Bond) {
        let journal = syauth_core::pair_recovery::read_journal(dir)
            .expect("read journal")
            .expect("present");
        let local = match journal.state {
            syauth_core::pair_recovery::JournalState::PreCommit => StatusState::PreCommit,
            syauth_core::pair_recovery::JournalState::CommitPending => StatusState::CommitPending,
        };
        let tx = Transaction::from_recovery_status(journal.transaction, Role::Coordinator, local).expect("transaction");
        let pending = BondStore::load(&pending_path(dir))
            .expect("pending store")
            .list()
            .first()
            .cloned()
            .expect("pending bond");
        (tx, pending)
    }

    fn staged_present(dir: &Path, peer_id: &str) -> (bool, bool, bool) {
        (
            syauth_core::pair_recovery::journal_path(dir).exists(),
            syauth_core::pair_recovery::pending_bond_path(dir).exists(),
            syauth_core::pair_recovery::pending_key_path(dir, peer_id).exists(),
        )
    }

    #[tokio::test]
    async fn reconcile_remote_committed_or_bonded_promotes_the_staged_bond() {
        for remote in [StatusState::Committed, StatusState::Bonded] {
            let dir = private_tempdir();
            let (transaction, peer_id) = stage_recovery_fixture(dir.path(), true);
            let (mut tx, pending) = recovery_transaction(dir.path());

            let result = apply_status_decision(dir.path(), &pending, &mut tx, StatusMessage::response(transaction, 41, remote), 41)
                .expect("decision");

            assert_eq!(result, ReconcileResult::Bonded, "remote {remote:?} must promote");
            assert!(syauth_core::pair_recovery::bonds_path(dir.path()).exists(), "active bonds.toml");
            assert!(
                syauth_core::pair_recovery::active_key_path(dir.path(), &peer_id).exists(),
                "active key"
            );
            assert_eq!(
                staged_present(dir.path(), &peer_id),
                (false, false, false),
                "promotion clears journal, pending bond and pending key"
            );
        }
    }

    #[tokio::test]
    async fn reconcile_remote_aborted_discards_the_staging() {
        let dir = private_tempdir();
        // No durable local commit decision: the peer's abort is conclusive.
        let (transaction, peer_id) = stage_recovery_fixture(dir.path(), false);
        let (mut tx, pending) = recovery_transaction(dir.path());

        let result = apply_status_decision(
            dir.path(),
            &pending,
            &mut tx,
            StatusMessage::response(transaction, 42, StatusState::Aborted),
            42,
        )
        .expect("decision");

        assert_eq!(result, ReconcileResult::Aborted);
        assert!(!syauth_core::pair_recovery::bonds_path(dir.path()).exists(), "no active trust");
        assert!(
            !syauth_core::pair_recovery::active_key_path(dir.path(), &peer_id).exists(),
            "no active key"
        );
        assert_eq!(
            staged_present(dir.path(), &peer_id),
            (false, false, false),
            "an aborted transaction drops the staging"
        );
    }

    #[tokio::test]
    async fn reconcile_inconclusive_status_retains_recovery() {
        for (commit_decision, remote) in [
            (false, StatusState::PreCommit),
            (false, StatusState::Unknown),
            (true, StatusState::CommitPending),
            (true, StatusState::Unknown),
        ] {
            let dir = private_tempdir();
            let (transaction, peer_id) = stage_recovery_fixture(dir.path(), commit_decision);
            let (mut tx, pending) = recovery_transaction(dir.path());

            let result = apply_status_decision(dir.path(), &pending, &mut tx, StatusMessage::response(transaction, 43, remote), 43)
                .expect("decision");

            assert_eq!(
                result,
                ReconcileResult::Uncertain,
                "commit={commit_decision} remote={remote:?} must stay recoverable"
            );
            assert!(!syauth_core::pair_recovery::bonds_path(dir.path()).exists(), "no active trust");
            assert!(
                !syauth_core::pair_recovery::active_key_path(dir.path(), &peer_id).exists(),
                "no active key"
            );
            assert_eq!(
                staged_present(dir.path(), &peer_id),
                (true, true, true),
                "inconclusive outcome retains journal, pending bond and pending key"
            );
        }
    }
}
