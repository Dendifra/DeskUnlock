//! Versioned, secret-free pairing transaction state machine.
//!
//! This is the application commit protocol, not a replacement for LESC or
//! the existing key exchange. Callers must bind a transaction to one
//! authenticated connection and durably stage both bonds before COMMIT.
//! A lost connection after COMMIT is *uncertain*, never an implicit rollback.

use thiserror::Error;

/// Pairing transaction protocol version (independent of unlock frames).
pub const VERSION: u8 = 2;
/// One version byte, a 128-bit transaction identifier, and one operation byte.
pub const MESSAGE_LEN: usize = 18;
/// Fixed-size authenticated status exchange: version, transaction, nonce,
/// operation and state.
pub const STATUS_MESSAGE_LEN: usize = 27;
/// Canonical neutral state carried by a status query.
pub const STATUS_QUERY_STATE: u8 = 0;

/// Messages carried inside the authenticated pairing connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Operation {
    /// Offer/accept coordinated pairing version 2.
    Capability = 1,
    /// Explicit OOB confirmation; never generated automatically.
    Confirm = 2,
    /// Explicit mismatch decision.
    Reject = 3,
    /// Explicit cancellation before the commit decision.
    Cancel = 4,
    /// Deadline expired before the commit decision.
    Timeout = 5,
    /// The sender has durably staged its inactive bond.
    Prepared = 6,
    /// Coordinator has durably recorded the commit decision.
    Commit = 7,
    /// Participant has durably recorded that same decision.
    CommitAck = 8,
    /// Sender has verified its committed local bond.
    Committed = 9,
    /// Query the durable outcome after reconnecting.
    Status = 10,
    /// Failure; after COMMIT it cannot imply rollback.
    Error = 11,
    /// Query the durable outcome after reconnecting.
    StatusQuery = 12,
    /// Reply to a status query; the payload is a [`StatusState`].
    StatusResponse = 13,
    /// Day-2 revocation. The sender is telling the peer that the association
    /// is over: the phone sends it when the operator dissociates in the app,
    /// so the desktop does not keep serving a bond the phone has dropped.
    ///
    /// The `transaction` field carries the sender's `peer_id` as 16 raw bytes
    /// (a `peer_id` is 32 hex characters), which is how the receiver knows
    /// *which* bond to revoke. A peer can only ever name a bond, and the write
    /// arrives over the authenticated BLE link of an already-bonded device.
    Revoke = 14,
}

/// Bounded wire message. Contains no key material or identifying peer data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    /// Fresh random identifier, bound to this authenticated pairing attempt.
    pub transaction: [u8; 16],
    /// Transaction operation.
    pub operation: Operation,
}

/// Invalid input never advances the transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransactionError {
    /// Missing/unsupported version or malformed operation/length.
    #[error("incompatible pairing protocol")]
    Incompatible,
    /// A message belongs to another attempt.
    #[error("different pairing transaction")]
    WrongTransaction,
    /// A prerequisite, role, or ordering requirement was not met.
    #[error("invalid pairing transition")]
    InvalidTransition,
}

impl Message {
    /// Fixed size encoding, fitting the default ATT payload.
    pub fn encode(self) -> [u8; MESSAGE_LEN] {
        let mut bytes = [0; MESSAGE_LEN];
        bytes[0] = VERSION;
        bytes[1..17].copy_from_slice(&self.transaction);
        bytes[17] = self.operation as u8;
        bytes
    }

    /// Strict decoding: reject legacy, truncated, extra and unknown input.
    pub fn decode(bytes: &[u8]) -> Result<Self, TransactionError> {
        if bytes.len() != MESSAGE_LEN || bytes[0] != VERSION {
            return Err(TransactionError::Incompatible);
        }
        let operation = match bytes[17] {
            1 => Operation::Capability,
            2 => Operation::Confirm,
            3 => Operation::Reject,
            4 => Operation::Cancel,
            5 => Operation::Timeout,
            6 => Operation::Prepared,
            7 => Operation::Commit,
            8 => Operation::CommitAck,
            9 => Operation::Committed,
            10 => Operation::Status,
            11 => Operation::Error,
            12 => Operation::StatusQuery,
            13 => Operation::StatusResponse,
            14 => Operation::Revoke,
            _ => return Err(TransactionError::Incompatible),
        };
        let mut transaction = [0; 16];
        transaction.copy_from_slice(&bytes[1..17]);
        Ok(Self { transaction, operation })
    }
}

/// Coarse, secret-free outcome exposed during authenticated recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StatusState {
    /// No durable commit decision exists.
    PreCommit = 1,
    /// A commit decision exists but bilateral completion is not proven.
    CommitPending = 2,
    /// The local commit decision is durable.
    Committed = 3,
    /// Both sides verified the committed bond.
    Bonded = 4,
    /// The transaction was certainly aborted before commit.
    Aborted = 5,
    /// The peer cannot determine its durable outcome.
    Unknown = 6,
}

impl StatusState {
    fn decode(value: u8) -> Result<Self, TransactionError> {
        match value {
            1 => Ok(Self::PreCommit),
            2 => Ok(Self::CommitPending),
            3 => Ok(Self::Committed),
            4 => Ok(Self::Bonded),
            5 => Ok(Self::Aborted),
            6 => Ok(Self::Unknown),
            _ => Err(TransactionError::Incompatible),
        }
    }
}

/// Authenticated, nonce-bound status query/response. The nonce prevents a
/// response captured in an earlier reconnect from being accepted as current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusMessage {
    /// Transaction identifier bound to the authenticated session.
    pub transaction: [u8; 16],
    /// Fresh query nonce echoed by the response.
    pub nonce: u64,
    /// None for a query, Some for a response.
    pub state: Option<StatusState>,
}

impl StatusMessage {
    /// Build a status query.
    pub fn query(transaction: [u8; 16], nonce: u64) -> Self {
        Self {
            transaction,
            nonce,
            state: None,
        }
    }
    /// Build a status response.
    pub fn response(transaction: [u8; 16], nonce: u64, state: StatusState) -> Self {
        Self {
            transaction,
            nonce,
            state: Some(state),
        }
    }

    /// Encode the strict status wire format.
    pub fn encode(self) -> [u8; STATUS_MESSAGE_LEN] {
        let mut bytes = [0; STATUS_MESSAGE_LEN];
        bytes[0] = VERSION;
        bytes[1..17].copy_from_slice(&self.transaction);
        bytes[17..25].copy_from_slice(&self.nonce.to_be_bytes());
        bytes[25] = if self.state.is_some() {
            Operation::StatusResponse as u8
        } else {
            Operation::StatusQuery as u8
        };
        bytes[26] = self.state.map_or(STATUS_QUERY_STATE, |state| state as u8);
        bytes
    }

    /// Decode and validate the strict status wire format.
    pub fn decode(bytes: &[u8]) -> Result<Self, TransactionError> {
        if bytes.len() != STATUS_MESSAGE_LEN || bytes[0] != VERSION {
            return Err(TransactionError::Incompatible);
        }
        let mut transaction = [0; 16];
        transaction.copy_from_slice(&bytes[1..17]);
        let nonce = u64::from_be_bytes(bytes[17..25].try_into().map_err(|_| TransactionError::Incompatible)?);
        match (bytes[25], bytes[26]) {
            (12, STATUS_QUERY_STATE) => Ok(Self::query(transaction, nonce)),
            (12, _) | (13, STATUS_QUERY_STATE) => Err(TransactionError::Incompatible),
            (13, value) => Ok(Self::response(transaction, nonce, StatusState::decode(value)?)),
            _ => Err(TransactionError::Incompatible),
        }
    }
}

/// Persistent transaction phase. Only `Bonded` authorizes a success UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Awaiting capability negotiation on a verified LESC connection.
    Negotiating,
    /// Keys exchanged; two independent explicit confirmations are required.
    OobPending,
    /// Both confirmations received; both inactive records must be durable.
    Preparing,
    /// Both inactive records are durable; coordinator may decide COMMIT.
    Prepared,
    /// Commit decision exists; cancellation can no longer claim rollback.
    CommitPending,
    /// Decision durably acknowledged; waiting for both verified records.
    Committed,
    /// Both peers explicitly reported verified committed records.
    Bonded,
    /// Aborted before any commit decision; inactive staging may be removed.
    Aborted(Operation),
    /// Outcome must be reconciled from durable state, never guessed.
    Uncertain,
}

/// Local role. Only the desktop coordinator may initiate COMMIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Desktop drives the commit decision.
    Coordinator,
    /// Phone acknowledges the durable decision.
    Participant,
}

/// Security prerequisites are asserted by the transport, not wire messages.
/// Keeping them out of `Operation` prevents a peer from asserting our LESC,
/// key-exchange or local persistence state for us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalEvent {
    /// Capability accepted after verified LESC and completed key exchange.
    VerifiedExchange,
    /// Local protocol capability offer.
    Capability,
    /// Explicit local OOB match decision.
    Confirm,
    /// Inactive bond was durably persisted and read back locally.
    Prepared,
    /// Local commit decision was durably persisted.
    Commit,
    /// Participant durably recorded the coordinator's decision.
    CommitAck,
    /// Committed local bond was durably persisted and read back.
    Committed,
    /// Link disappeared. After the commit decision the result is uncertain.
    Disconnected,
    /// Explicit local abort or local failure.
    Abort(Operation),
}

/// Deterministic transition validator shared by desktop/mobile integrations.
/// Persistence and transport are deliberately outside this type: neither a
/// successful write callback nor a UI click is proof of durable remote commit.
#[derive(Debug, Clone)]
pub struct Transaction {
    id: [u8; 16],
    role: Role,
    phase: Phase,
    capabilities: [bool; 2],
    confirmed: [bool; 2],
    prepared: [bool; 2],
    committed: [bool; 2],
}

fn phase_code(phase: Phase) -> u8 {
    match phase {
        Phase::Negotiating => 0,
        Phase::OobPending => 1,
        Phase::Preparing => 2,
        Phase::Prepared => 3,
        Phase::CommitPending => 4,
        Phase::Committed => 5,
        Phase::Bonded => 6,
        Phase::Aborted(_) => 7,
        Phase::Uncertain => 8,
    }
}

fn decode_phase(code: u8) -> Result<Phase, TransactionError> {
    Ok(match code {
        0 => Phase::Negotiating,
        1 => Phase::OobPending,
        2 => Phase::Preparing,
        3 => Phase::Prepared,
        4 => Phase::CommitPending,
        5 => Phase::Committed,
        6 => Phase::Bonded,
        8 => Phase::Uncertain,
        _ => return Err(TransactionError::Incompatible),
    })
}

impl Transaction {
    /// Serialize only non-secret recovery metadata for restart.
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(27);
        bytes.extend_from_slice(&[
            VERSION,
            match self.role {
                Role::Coordinator => 0,
                Role::Participant => 1,
            },
            phase_code(self.phase),
        ]);
        bytes.extend_from_slice(&self.id);
        bytes.extend(self.capabilities.into_iter().map(u8::from));
        bytes.extend(self.confirmed.into_iter().map(u8::from));
        bytes.extend(self.prepared.into_iter().map(u8::from));
        bytes.extend(self.committed.into_iter().map(u8::from));
        bytes
    }

    /// Restore non-secret transaction metadata after a process restart.
    pub fn restore(bytes: &[u8]) -> Result<Self, TransactionError> {
        if bytes.len() != 27 || bytes[0] != VERSION {
            return Err(TransactionError::Incompatible);
        }
        let role = match bytes[1] {
            0 => Role::Coordinator,
            1 => Role::Participant,
            _ => return Err(TransactionError::Incompatible),
        };
        let phase = decode_phase(bytes[2])?;
        let mut id = [0; 16];
        id.copy_from_slice(&bytes[3..19]);
        let flags = |offset: usize| -> Result<[bool; 2], TransactionError> {
            match (bytes[offset], bytes[offset + 1]) {
                (0..=1, 0..=1) => Ok([bytes[offset] != 0, bytes[offset + 1] != 0]),
                _ => Err(TransactionError::Incompatible),
            }
        };
        let transaction = Self {
            id,
            role,
            phase,
            capabilities: flags(19)?,
            confirmed: flags(21)?,
            prepared: flags(23)?,
            committed: flags(25)?,
        };
        transaction.validate()?;
        Ok(transaction)
    }

    /// Restore a coarse durable state when the journal survived a restart.
    pub fn from_recovery_status(id: [u8; 16], role: Role, status: StatusState) -> Result<Self, TransactionError> {
        let phase = match status {
            StatusState::PreCommit => Phase::Prepared,
            StatusState::CommitPending => Phase::CommitPending,
            StatusState::Committed => Phase::Committed,
            StatusState::Bonded => Phase::Bonded,
            StatusState::Aborted => Phase::Aborted(Operation::Error),
            StatusState::Unknown => Phase::Uncertain,
        };
        let committed = match phase {
            Phase::Bonded => [true; 2],
            Phase::Committed => [true, false],
            _ => [false; 2],
        };
        let tx = Self {
            id,
            role,
            phase,
            capabilities: [true; 2],
            confirmed: [true; 2],
            prepared: [true; 2],
            committed,
        };
        tx.validate()?;
        Ok(tx)
    }

    /// Begin one fresh, authenticated pairing attempt.
    pub fn new(id: [u8; 16], role: Role) -> Self {
        Self {
            id,
            role,
            phase: Phase::Negotiating,
            capabilities: [false; 2],
            confirmed: [false; 2],
            prepared: [false; 2],
            committed: [false; 2],
        }
    }

    fn validate(&self) -> Result<(), TransactionError> {
        let all = [true; 2];
        let valid = match self.phase {
            Phase::Negotiating => !self.capabilities[0] || self.capabilities[1],
            Phase::OobPending => self.capabilities == all && self.confirmed != all,
            Phase::Preparing => self.capabilities == all && self.confirmed == all && self.prepared != all,
            Phase::Prepared => self.capabilities == all && self.confirmed == all && self.prepared == all && self.committed == [false; 2],
            Phase::CommitPending => {
                self.capabilities == all && self.confirmed == all && self.prepared == all && self.committed == [false; 2]
            }
            Phase::Committed => self.capabilities == all && self.confirmed == all && self.prepared == all && self.committed != all,
            Phase::Bonded => self.capabilities == all && self.confirmed == all && self.prepared == all && self.committed == all,
            Phase::Aborted(_) | Phase::Uncertain => true,
        };
        valid.then_some(()).ok_or(TransactionError::InvalidTransition)
    }

    /// Current phase; callers must persist a commit decision before emitting it.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Coarse state safe to expose over the authenticated recovery channel.
    pub fn status(&self) -> StatusState {
        match self.phase {
            Phase::Negotiating | Phase::OobPending | Phase::Preparing | Phase::Prepared => StatusState::PreCommit,
            Phase::CommitPending => StatusState::CommitPending,
            Phase::Uncertain => StatusState::Unknown,
            Phase::Committed => StatusState::Committed,
            Phase::Bonded => StatusState::Bonded,
            Phase::Aborted(_) => StatusState::Aborted,
        }
    }

    /// Reconcile a validated peer status without downgrading a commit.
    pub fn reconcile_status(&mut self, remote: StatusState) -> Result<Phase, TransactionError> {
        let local = self.status();
        match (local, remote) {
            (StatusState::Unknown, _) | (_, StatusState::Unknown) => self.phase = Phase::Uncertain,
            (StatusState::PreCommit, StatusState::Aborted) => self.phase = Phase::Aborted(Operation::Error),
            (StatusState::PreCommit, StatusState::PreCommit) => {}
            (StatusState::CommitPending, StatusState::Committed) => self.phase = Phase::Committed,
            (StatusState::CommitPending, StatusState::Bonded) => {
                self.committed = [true; 2];
                self.phase = Phase::Bonded;
            }
            (StatusState::Committed, StatusState::CommitPending) => {}
            (StatusState::Committed, StatusState::Committed)
            | (StatusState::Committed, StatusState::Bonded)
            | (StatusState::Bonded, StatusState::Committed)
            | (StatusState::Bonded, StatusState::Bonded) => {
                self.committed = [true; 2];
                self.phase = Phase::Bonded;
            }
            (StatusState::Aborted, StatusState::Aborted) => {}
            (StatusState::Aborted, StatusState::PreCommit) => {}
            _ => self.phase = Phase::Uncertain,
        }
        Ok(self.phase)
    }

    /// Validate and reconcile a nonce-bound response against this transaction.
    pub fn apply_status_response(&mut self, response: StatusMessage, nonce: u64) -> Result<Phase, TransactionError> {
        if response.transaction != self.id || response.nonce != nonce || response.state.is_none() {
            return Err(TransactionError::WrongTransaction);
        }
        self.reconcile_status(response.state.ok_or(TransactionError::Incompatible)?)
    }

    /// Apply a local fact. Failure leaves the previous state intact.
    pub fn local(&mut self, event: LocalEvent) -> Result<(), TransactionError> {
        use LocalEvent as E;
        match event {
            E::Capability if self.phase == Phase::Negotiating && !self.capabilities[0] => {
                self.capabilities[0] = true;
            }
            E::VerifiedExchange if self.phase == Phase::Negotiating && self.capabilities == [true; 2] => {
                self.phase = Phase::OobPending;
            }
            E::Confirm => self.confirm(0)?,
            E::Prepared => self.prepare(0)?,
            E::Commit if self.role == Role::Coordinator && self.phase == Phase::Prepared => {
                self.phase = Phase::CommitPending;
            }
            E::CommitAck if self.role == Role::Participant && self.phase == Phase::CommitPending => {
                self.phase = Phase::Committed;
            }
            E::Committed => self.record_committed(0)?,
            E::Disconnected => self.disconnect(),
            E::Abort(operation) => self.abort(operation)?,
            _ => return Err(TransactionError::InvalidTransition),
        }
        Ok(())
    }

    /// Apply a remote message bound to the current attempt.
    pub fn remote(&mut self, message: Message) -> Result<(), TransactionError> {
        if message.transaction != self.id {
            return Err(TransactionError::WrongTransaction);
        }
        match message.operation {
            Operation::Capability if self.phase == Phase::Negotiating && !self.capabilities[1] => {
                self.capabilities[1] = true;
            }
            Operation::Confirm => self.confirm(1)?,
            Operation::Prepared => self.prepare(1)?,
            Operation::Commit if self.role == Role::Participant && self.phase == Phase::Prepared => {
                self.phase = Phase::CommitPending;
            }
            Operation::CommitAck if self.role == Role::Coordinator && self.phase == Phase::CommitPending => {
                self.phase = Phase::Committed;
            }
            Operation::Committed => self.record_committed(1)?,
            Operation::Status | Operation::StatusQuery | Operation::StatusResponse => (), // status never advances a commit
            op @ (Operation::Reject | Operation::Cancel | Operation::Timeout | Operation::Error) => self.abort(op)?,
            _ => return Err(TransactionError::InvalidTransition),
        }
        Ok(())
    }

    fn confirm(&mut self, side: usize) -> Result<(), TransactionError> {
        if self.phase != Phase::OobPending || self.confirmed[side] {
            return Err(TransactionError::InvalidTransition);
        }
        self.confirmed[side] = true;
        if self.confirmed == [true; 2] {
            self.phase = Phase::Preparing;
        }
        Ok(())
    }

    fn prepare(&mut self, side: usize) -> Result<(), TransactionError> {
        if self.phase != Phase::Preparing || self.prepared[side] {
            return Err(TransactionError::InvalidTransition);
        }
        self.prepared[side] = true;
        if self.prepared == [true; 2] {
            self.phase = Phase::Prepared;
        }
        Ok(())
    }

    fn record_committed(&mut self, side: usize) -> Result<(), TransactionError> {
        if self.phase != Phase::Committed || self.committed[side] {
            return Err(TransactionError::InvalidTransition);
        }
        self.committed[side] = true;
        if self.committed == [true; 2] {
            self.phase = Phase::Bonded;
        }
        Ok(())
    }

    fn disconnect(&mut self) {
        self.phase = match self.phase {
            Phase::CommitPending | Phase::Committed | Phase::Uncertain => Phase::Uncertain,
            Phase::Bonded | Phase::Aborted(_) => self.phase,
            _ => Phase::Aborted(Operation::Cancel),
        };
    }

    fn abort(&mut self, operation: Operation) -> Result<(), TransactionError> {
        if !matches!(
            operation,
            Operation::Reject | Operation::Cancel | Operation::Timeout | Operation::Error
        ) || matches!(self.phase, Phase::Bonded | Phase::Aborted(_))
        {
            return Err(TransactionError::InvalidTransition);
        }
        self.phase = match self.phase {
            Phase::CommitPending | Phase::Committed | Phase::Uncertain => Phase::Uncertain,
            _ => Phase::Aborted(operation),
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: [u8; 16] = [7; 16];
    fn message(operation: Operation) -> Message {
        Message {
            transaction: ID,
            operation,
        }
    }
    fn exchanged(role: Role) -> Transaction {
        let mut t = Transaction::new(ID, role);
        t.remote(message(Operation::Capability)).unwrap();
        t.local(LocalEvent::Capability).unwrap();
        t.local(LocalEvent::VerifiedExchange).unwrap();
        t
    }
    fn prepared(role: Role) -> Transaction {
        let mut t = exchanged(role);
        t.local(LocalEvent::Confirm).unwrap();
        assert_eq!(t.phase(), Phase::OobPending);
        t.remote(message(Operation::Confirm)).unwrap();
        t.local(LocalEvent::Prepared).unwrap();
        t.remote(message(Operation::Prepared)).unwrap();
        assert_eq!(t.phase(), Phase::Prepared);
        t
    }
    #[test]
    fn recovery_metadata_roundtrips_without_secrets() {
        let t = exchanged(Role::Coordinator);
        let restored = Transaction::restore(&t.serialize()).unwrap();
        assert_eq!(restored.phase(), Phase::OobPending);
        assert_eq!(restored.serialize(), t.serialize());
        assert!(Transaction::restore(&t.serialize()[..26]).is_err());
    }

    #[test]
    fn complete_two_party_exchange_requires_both_verified_records() {
        let mut desktop = prepared(Role::Coordinator);
        let mut phone = prepared(Role::Participant);
        desktop.local(LocalEvent::Commit).unwrap();
        phone.remote(message(Operation::Commit)).unwrap();
        phone.local(LocalEvent::CommitAck).unwrap();
        desktop.remote(message(Operation::CommitAck)).unwrap();
        for t in [&mut desktop, &mut phone] {
            t.local(LocalEvent::Committed).unwrap();
            assert_eq!(t.phase(), Phase::Committed);
            t.remote(message(Operation::Committed)).unwrap();
            assert_eq!(t.phase(), Phase::Bonded);
        }
    }
    #[test]
    fn reject_cancel_timeout_error_from_either_side_never_bond() {
        for op in [Operation::Reject, Operation::Cancel, Operation::Timeout, Operation::Error] {
            for local in [true, false] {
                let mut t = exchanged(Role::Coordinator);
                if local {
                    t.local(LocalEvent::Abort(op)).unwrap();
                } else {
                    t.remote(message(op)).unwrap();
                }
                assert_eq!(t.phase(), Phase::Aborted(op));
                assert!(t.local(LocalEvent::Committed).is_err());
            }
        }
    }
    #[test]
    fn missing_ack_or_disconnect_after_commit_is_uncertain() {
        for event in [
            LocalEvent::Disconnected,
            LocalEvent::Abort(Operation::Timeout),
            LocalEvent::Abort(Operation::Error),
        ] {
            let mut t = prepared(Role::Coordinator);
            t.local(LocalEvent::Commit).unwrap();
            assert!(t.local(LocalEvent::Committed).is_err());
            t.local(event).unwrap();
            assert_eq!(t.phase(), Phase::Uncertain);
            t.remote(message(Operation::Status)).unwrap();
            assert_eq!(t.phase(), Phase::Uncertain);
            assert!(t.remote(message(Operation::CommitAck)).is_err());
        }
    }
    #[test]
    fn disconnect_before_commit_aborts_only_inactive_staging() {
        for mut t in [exchanged(Role::Coordinator), prepared(Role::Coordinator)] {
            t.local(LocalEvent::Disconnected).unwrap();
            assert_eq!(t.phase(), Phase::Aborted(Operation::Cancel));
        }
    }
    #[test]
    fn legacy_and_malformed_packets_fail_closed() {
        let valid = message(Operation::Capability).encode();
        assert_eq!(Message::decode(&valid).unwrap(), message(Operation::Capability));
        for n in 0..MESSAGE_LEN {
            assert!(Message::decode(&valid[..n]).is_err());
        }
        let mut legacy = valid;
        legacy[0] = 1;
        assert!(Message::decode(&legacy).is_err());
        legacy[0] = VERSION;
        legacy[17] = 255;
        assert!(Message::decode(&legacy).is_err());
        assert!(Message::decode(&[0; MESSAGE_LEN + 1]).is_err());
    }
    #[test]
    fn status_query_response_is_nonce_bound_and_strict() {
        let tx = exchanged(Role::Coordinator);
        let query = StatusMessage::query(ID, 41);
        assert_eq!(StatusMessage::decode(&query.encode()).unwrap(), query);
        let response = StatusMessage::response(ID, 41, tx.status());
        let mut restored = Transaction::restore(&tx.serialize()).unwrap();
        restored.apply_status_response(response, 41).unwrap();
        assert!(restored.apply_status_response(response, 42).is_err());
        assert!(StatusMessage::decode(&response.encode()[..25]).is_err());
        let wrong = StatusMessage::response([8; 16], 41, tx.status());
        assert!(restored.apply_status_response(wrong, 41).is_err());
    }

    #[test]
    fn reconciliation_matrix_never_downgrades_commit() {
        let mut pending = prepared(Role::Coordinator);
        pending.local(LocalEvent::Commit).unwrap();
        let mut pending = Transaction::restore(&pending.serialize()).unwrap();
        assert_eq!(pending.reconcile_status(StatusState::Committed).unwrap(), Phase::Committed);

        let mut committed = pending.clone();
        assert_eq!(committed.reconcile_status(StatusState::CommitPending).unwrap(), Phase::Committed);
        assert_eq!(committed.reconcile_status(StatusState::Committed).unwrap(), Phase::Bonded);
        let reloaded = Transaction::restore(&committed.serialize()).unwrap();
        assert_eq!(reloaded.phase(), Phase::Bonded);

        let mut pre = exchanged(Role::Coordinator);
        assert_eq!(
            pre.reconcile_status(StatusState::Aborted).unwrap(),
            Phase::Aborted(Operation::Error)
        );
        let mut unknown = exchanged(Role::Coordinator);
        assert_eq!(unknown.reconcile_status(StatusState::Unknown).unwrap(), Phase::Uncertain);
    }

    #[test]
    fn malformed_status_operation_and_state_fail_closed() {
        let query = StatusMessage::query(ID, 1).encode();
        let mut bad = query;
        bad[25] = Operation::StatusResponse as u8;
        assert!(StatusMessage::decode(&bad).is_err());
        bad[25] = Operation::StatusQuery as u8;
        bad[26] = 99;
        assert!(StatusMessage::decode(&bad).is_err());
        bad[25] = 99;
        bad[26] = STATUS_QUERY_STATE;
        assert!(StatusMessage::decode(&bad).is_err());
        bad = query;
        bad[0] = VERSION - 1;
        assert!(StatusMessage::decode(&bad).is_err());
    }

    #[test]
    fn unknown_status_preserves_fail_closed_uncertainty() {
        let tx = exchanged(Role::Coordinator);
        let mut restored = Transaction::restore(&tx.serialize()).unwrap();
        restored
            .apply_status_response(StatusMessage::response(ID, 9, StatusState::Unknown), 9)
            .unwrap();
        assert_eq!(restored.phase(), Phase::Uncertain);
    }

    #[test]
    fn replay_wrong_role_and_skipping_gates_are_rejected() {
        let mut t = Transaction::new(ID, Role::Coordinator);
        assert!(t.local(LocalEvent::VerifiedExchange).is_err());
        assert!(t.local(LocalEvent::Confirm).is_err());
        assert!(t.local(LocalEvent::Committed).is_err());
        assert!(
            t.remote(Message {
                transaction: [8; 16],
                operation: Operation::Capability
            })
            .is_err()
        );
        let mut t = prepared(Role::Participant);
        assert!(t.local(LocalEvent::Commit).is_err());
        assert!(t.remote(message(Operation::CommitAck)).is_err());
        let mut t = prepared(Role::Coordinator);
        t.local(LocalEvent::Commit).unwrap();
        t.remote(message(Operation::CommitAck)).unwrap();
        assert!(t.remote(message(Operation::CommitAck)).is_err());
        let mut t = Transaction::new(ID, Role::Coordinator);
        t.remote(message(Operation::Capability)).unwrap();
        assert!(t.remote(message(Operation::Capability)).is_err());
        assert_eq!(t.phase(), Phase::Negotiating);
    }

    /// The day-2 revocation op is on the wire now: a phone that dissociates must
    /// be able to tell the desktop, and the peer id travels in the transaction
    /// field (16 raw bytes = 32 hex characters).
    #[test]
    fn the_revoke_operation_round_trips_with_the_peer_id_in_the_transaction() {
        let peer_id_bytes = [0xAB; 16];
        let encoded = Message {
            transaction: peer_id_bytes,
            operation: Operation::Revoke,
        }
        .encode();
        assert_eq!(encoded[17], 14, "the wire tag is part of the contract");
        let decoded = Message::decode(&encoded).expect("round trip");
        assert_eq!(decoded.operation, Operation::Revoke);
        assert_eq!(decoded.transaction, peer_id_bytes);
    }

    /// A revocation is not a pairing step: feeding one to a live transaction
    /// must not move it, otherwise a dissociating phone could unblock a pairing
    /// it is not part of.
    #[test]
    fn a_revoke_operation_never_advances_a_pairing_transaction() {
        let mut t = Transaction::new(ID, Role::Coordinator);
        let before = t.phase();
        assert!(t.remote(message(Operation::Revoke)).is_err());
        assert_eq!(t.phase(), before);
    }
}
