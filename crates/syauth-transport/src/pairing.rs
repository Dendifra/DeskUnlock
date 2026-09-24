//! Pairing confirmation broker shared by the daemon and the GUI client.
//!
//! Two distinct decisions flow over this local channel:
//!
//! - the **BlueZ LESC** numeric-comparison confirmation (`SYPC`), which is a
//!   transport event, and
//! - the **DeskUnlock** out-of-band confirmation (`SYPO`), which is the
//!   application-level trust decision.
//!
//! A terminal `SYPN` notice tells the GUI that the V2 transaction reached
//! `TrustEstablished`; only then may it render "associated". The transport
//! confirmation never implies trust.

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::{Mutex, mpsc, oneshot},
};

use crate::bluez::PAIR_PUBKEY_LEN;

/// Bounded wait for an explicit desktop decision.
pub const PAIR_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Budget for the **DeskUnlock OOB** decision. The operator reads four words in
/// the desktop dialog and answers, so this one is human-paced. The transport
/// (BlueZ LESC) prompt keeps [PAIR_CONFIRMATION_TIMEOUT] because the peer's own
/// Bluetooth stack times that prompt out.
pub const PAIR_OOB_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(180);
/// Runtime socket used by the GUI confirmation client.
const PAIR_CONFIRMATION_SOCKET: &str = "pair-confirm.sock";
/// Runtime socket the GUI client connects to while a pair session is open.
/// The daemon scopes its BlueZ default-agent ownership to this connection.
const PAIR_SESSION_SOCKET: &str = "pair-session.sock";
const PAIR_CONFIRMATION_MAGIC: &[u8; 4] = b"SYPC";
const PAIR_OOB_MAGIC: &[u8; 4] = b"SYPO";
const PAIR_BONDED_MAGIC: &[u8; 4] = b"SYPN";
const MAX_PEER_BYTES: usize = 256;
const MAX_FRAME_BYTES: usize = 4096;

/// Event sent to the one connected GUI session (in-process embed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequest {
    /// Monotonic request identifier.
    pub id: u64,
    /// BlueZ device address.
    pub peer: String,
    /// Numeric-comparison passkey.
    pub passkey: u32,
}

impl PairingRequest {
    /// Encode the private local confirmation message.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let peer = self.peer.as_bytes();
        let length = u16::try_from(peer.len()).ok()?;
        if peer.len() > MAX_PEER_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(18 + peer.len());
        bytes.extend_from_slice(PAIR_CONFIRMATION_MAGIC);
        bytes.extend_from_slice(&self.id.to_be_bytes());
        bytes.extend_from_slice(&self.passkey.to_be_bytes());
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(peer);
        Some(bytes)
    }

    /// Decode a private local confirmation message.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 18 || &bytes[..4] != PAIR_CONFIRMATION_MAGIC {
            return None;
        }
        let peer_len = usize::from(u16::from_be_bytes(bytes[16..18].try_into().ok()?));
        if peer_len > MAX_PEER_BYTES || bytes.len() != 18 + peer_len {
            return None;
        }
        Some(Self {
            id: u64::from_be_bytes(bytes[4..12].try_into().ok()?),
            passkey: u32::from_be_bytes(bytes[12..16].try_into().ok()?),
            peer: String::from_utf8(bytes[18..].to_vec()).ok()?,
        })
    }
}

/// DeskUnlock application-level OOB confirmation request. Distinct from the
/// BlueZ LESC confirmation: this is the real trust decision. Carries only
/// public keys; the GUI derives the OOB words locally so no secret crosses
/// the local socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OobRequest {
    /// Monotonic request identifier.
    pub id: u64,
    /// Peer display label (transport label; never the trust identity).
    pub peer: String,
    /// Desktop host public key exchanged over authenticated GATT.
    pub host_pubkey: [u8; PAIR_PUBKEY_LEN],
    /// Phone public key exchanged over authenticated GATT.
    pub phone_pubkey: [u8; PAIR_PUBKEY_LEN],
}

impl OobRequest {
    /// Encode the private local OOB request.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let peer = self.peer.as_bytes();
        let peer_len = u16::try_from(peer.len()).ok()?;
        if peer.len() > MAX_PEER_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(14 + peer.len() + 2 * PAIR_PUBKEY_LEN);
        bytes.extend_from_slice(PAIR_OOB_MAGIC);
        bytes.extend_from_slice(&self.id.to_be_bytes());
        bytes.extend_from_slice(&peer_len.to_be_bytes());
        bytes.extend_from_slice(peer);
        bytes.extend_from_slice(&self.host_pubkey);
        bytes.extend_from_slice(&self.phone_pubkey);
        Some(bytes)
    }

    /// Decode a private local OOB request.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 14 || &bytes[..4] != PAIR_OOB_MAGIC {
            return None;
        }
        let id = u64::from_be_bytes(bytes[4..12].try_into().ok()?);
        let peer_len = usize::from(u16::from_be_bytes(bytes[12..14].try_into().ok()?));
        if peer_len > MAX_PEER_BYTES || bytes.len() != 14 + peer_len + 2 * PAIR_PUBKEY_LEN {
            return None;
        }
        let peer = String::from_utf8(bytes[14..14 + peer_len].to_vec()).ok()?;
        let mut host_pubkey = [0u8; PAIR_PUBKEY_LEN];
        host_pubkey.copy_from_slice(&bytes[14 + peer_len..14 + peer_len + PAIR_PUBKEY_LEN]);
        let mut phone_pubkey = [0u8; PAIR_PUBKEY_LEN];
        phone_pubkey.copy_from_slice(&bytes[14 + peer_len + PAIR_PUBKEY_LEN..]);
        Some(Self {
            id,
            peer,
            host_pubkey,
            phone_pubkey,
        })
    }
}

/// Terminal notice: the DeskUnlock V2 transaction reached `TrustEstablished`.
/// The GUI renders "associated" only after this, never on the transport
/// confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BondedNotice {
    /// Peer display label.
    pub peer: String,
}

impl BondedNotice {
    /// Encode the terminal trust notice.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let peer = self.peer.as_bytes();
        let peer_len = u16::try_from(peer.len()).ok()?;
        if peer.len() > MAX_PEER_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(6 + peer.len());
        bytes.extend_from_slice(PAIR_BONDED_MAGIC);
        bytes.extend_from_slice(&peer_len.to_be_bytes());
        bytes.extend_from_slice(peer);
        Some(bytes)
    }

    /// Decode the terminal trust notice.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 6 || &bytes[..4] != PAIR_BONDED_MAGIC {
            return None;
        }
        let peer_len = usize::from(u16::from_be_bytes(bytes[4..6].try_into().ok()?));
        if peer_len > MAX_PEER_BYTES || bytes.len() != 6 + peer_len {
            return None;
        }
        Some(Self {
            peer: String::from_utf8(bytes[6..].to_vec()).ok()?,
        })
    }
}

/// Event delivered to the one registered GUI session (in-process embed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiEvent {
    /// BlueZ LESC numeric-comparison confirmation (transport).
    Lesc(PairingRequest),
    /// DeskUnlock application-level OOB confirmation (trust).
    Oob(OobRequest),
    /// Terminal trust notice.
    Bonded(BondedNotice),
}

/// Frame a broker payload as `[len: u32 BE][payload]`.
pub fn frame(payload: &[u8]) -> Option<Vec<u8>> {
    let len = u32::try_from(payload.len()).ok()?;
    if payload.len() > MAX_FRAME_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

/// Read one framed payload from `stream`. Returns `None` on EOF, a
/// malformed frame or a frame larger than [`MAX_FRAME_BYTES`].
pub async fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await.ok()?;
    let len = usize::try_from(u32::from_be_bytes(header)).ok()?;
    if len > MAX_FRAME_BYTES {
        return None;
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.ok()?;
    Some(payload)
}

/// Resolve the GUI confirmation socket under the per-user runtime directory.
pub fn pairing_confirmation_socket() -> Option<PathBuf> {
    runtime_socket(PAIR_CONFIRMATION_SOCKET)
}

/// Resolve the pair-session socket under the per-user runtime directory.
pub fn pairing_session_socket() -> Option<PathBuf> {
    runtime_socket(PAIR_SESSION_SOCKET)
}

fn runtime_socket(name: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(|runtime| PathBuf::from(runtime).join("syauth").join(name))
}

#[derive(Default)]
struct State {
    gui: Option<mpsc::Sender<GuiEvent>>,
    pending: HashMap<u64, oneshot::Sender<bool>>,
    next_id: u64,
    socket_active: bool,
}

/// A GUI session already owns the confirmation channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuiAlreadyConnected;

/// Coordinates one GUI session and one active confirmation at a time.
#[derive(Clone, Default)]
pub struct PairingBroker {
    state: Arc<Mutex<State>>,
}

impl PairingBroker {
    /// Register the sole in-process GUI session. A second session is rejected.
    pub async fn register_gui(&self) -> Result<mpsc::Receiver<GuiEvent>, GuiAlreadyConnected> {
        let (tx, rx) = mpsc::channel(1);
        let mut state = self.state.lock().await;
        if state.gui.is_some() {
            return Err(GuiAlreadyConnected);
        }
        state.gui = Some(tx);
        Ok(rx)
    }

    /// Remove a disconnected GUI session and fail its pending request.
    pub async fn unregister_gui(&self) {
        let mut state = self.state.lock().await;
        state.gui = None;
        for (_, sender) in state.pending.drain() {
            let _ = sender.send(false);
        }
    }

    /// Ask the GUI to confirm one BlueZ numeric-comparison request. The peer's
    /// Bluetooth stack owns this deadline, so it stays transport-paced.
    pub async fn request_confirmation(&self, peer: String, passkey: u32) -> bool {
        if let Some(accepted) = self
            .request_via_gui(
                GuiEvent::Lesc(PairingRequest {
                    id: 0,
                    peer: peer.clone(),
                    passkey,
                }),
                PAIR_CONFIRMATION_TIMEOUT,
            )
            .await
        {
            return accepted;
        }
        let Some(payload) = (PairingRequest { id: 0, peer, passkey }).encode() else {
            return false;
        };
        self.exchange(payload, true, PAIR_CONFIRMATION_TIMEOUT).await
    }

    /// Ask the GUI for the DeskUnlock application-level OOB decision. This is
    /// the real trust confirmation and is independent of the transport
    /// confirmation. Fails closed when no GUI is listening.
    pub async fn request_oob_confirmation(
        &self,
        peer: String,
        host_pubkey: [u8; PAIR_PUBKEY_LEN],
        phone_pubkey: [u8; PAIR_PUBKEY_LEN],
    ) -> bool {
        if let Some(accepted) = self
            .request_via_gui(
                GuiEvent::Oob(OobRequest {
                    id: 0,
                    peer: peer.clone(),
                    host_pubkey,
                    phone_pubkey,
                }),
                PAIR_OOB_CONFIRMATION_TIMEOUT,
            )
            .await
        {
            return accepted;
        }
        let Some(payload) = (OobRequest {
            id: 0,
            peer,
            host_pubkey,
            phone_pubkey,
        })
        .encode() else {
            return false;
        };
        self.exchange(payload, true, PAIR_OOB_CONFIRMATION_TIMEOUT).await
    }

    /// Deliver one confirmation request to the in-process GUI session, or
    /// return `None` when no such session is registered. Fails closed on a
    /// send failure, a concurrent request or a timeout.
    async fn request_via_gui(&self, event: GuiEvent, timeout: Duration) -> Option<bool> {
        let (sender, receiver) = oneshot::channel();
        let (id, gui) = {
            let mut state = self.state.lock().await;
            let gui = state.gui.clone()?;
            if !state.pending.is_empty() {
                return Some(false);
            }
            let id = state.next_id;
            state.next_id = state.next_id.wrapping_add(1);
            state.pending.insert(id, sender);
            (id, gui)
        };
        let event = match event {
            GuiEvent::Lesc(mut request) => {
                request.id = id;
                GuiEvent::Lesc(request)
            }
            GuiEvent::Oob(mut request) => {
                request.id = id;
                GuiEvent::Oob(request)
            }
            GuiEvent::Bonded(notice) => GuiEvent::Bonded(notice),
        };
        if gui.send(event).await.is_err() {
            self.unregister_gui().await;
            return Some(false);
        }
        Some(match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(accepted)) => accepted,
            _ => {
                self.state.lock().await.pending.remove(&id);
                false
            }
        })
    }

    /// Tell the GUI that the V2 transaction reached `TrustEstablished`. The
    /// GUI may render "associated" only after this notice.
    pub async fn notify_bonded(&self, peer: String) {
        if let Some(payload) = (BondedNotice { peer }).encode() {
            let _ = self.exchange(payload, false, PAIR_CONFIRMATION_TIMEOUT).await;
        }
    }

    /// Send one framed payload to the GUI socket. When `expect_response` is
    /// true, a single `1` byte means accepted. Fails closed on any error.
    async fn exchange(&self, payload: Vec<u8>, expect_response: bool, timeout: Duration) -> bool {
        {
            let mut state = self.state.lock().await;
            if state.socket_active {
                return false;
            }
            state.socket_active = true;
        }
        let result = async {
            let Some(path) = pairing_confirmation_socket() else { return false };
            let Some(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(1), UnixStream::connect(path)).await.ok() else {
                return false;
            };
            let Some(framed) = frame(&payload) else { return false };
            if stream.write_all(&framed).await.is_err() {
                return false;
            }
            if !expect_response {
                return true;
            }
            let mut response = [0u8; 1];
            tokio::time::timeout(timeout, stream.read_exact(&mut response))
                .await
                .is_ok_and(|read| read.is_ok())
                && response[0] == 1
        }
        .await;
        self.state.lock().await.socket_active = false;
        result
    }

    /// Resolve a request from the in-process GUI. Unknown or late ids are
    /// rejected.
    pub async fn respond(&self, id: u64, accepted: bool) -> bool {
        let sender = self.state.lock().await.pending.remove(&id);
        sender.is_some_and(|sender| sender.send(accepted).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_gui_fails_closed() {
        assert!(!PairingBroker::default().request_confirmation("peer".into(), 123456).await);
        assert!(
            !PairingBroker::default()
                .request_oob_confirmation("peer".into(), [1u8; 32], [2u8; 32])
                .await
        );
    }

    #[tokio::test]
    async fn confirm_and_reject_are_forwarded() {
        let broker = PairingBroker::default();
        let mut events = broker.register_gui().await.expect("first GUI");
        let confirming = broker.clone();
        let task = tokio::spawn(async move { confirming.request_confirmation("peer".into(), 123456).await });
        let GuiEvent::Lesc(request) = events.recv().await.expect("request") else {
            panic!("expected an LESC request");
        };
        assert_eq!(request.passkey, 123456);
        assert!(broker.respond(request.id, true).await);
        assert!(task.await.expect("task"));
    }

    #[tokio::test]
    async fn reject_and_disconnect_fail_closed() {
        let broker = PairingBroker::default();
        let mut events = broker.register_gui().await.expect("GUI");
        let rejecting = broker.clone();
        let task = tokio::spawn(async move { rejecting.request_confirmation("peer".into(), 1).await });
        let GuiEvent::Lesc(request) = events.recv().await.expect("request") else {
            panic!("expected an LESC request");
        };
        assert!(broker.respond(request.id, false).await);
        assert!(!task.await.expect("task"));
        let disconnecting = broker.clone();
        let task = tokio::spawn(async move { disconnecting.request_confirmation("peer".into(), 2).await });
        assert!(events.recv().await.is_some());
        broker.unregister_gui().await;
        assert!(!task.await.expect("task"));
    }

    #[tokio::test]
    async fn second_gui_and_concurrent_confirmation_are_rejected() {
        let broker = PairingBroker::default();
        let mut events = broker.register_gui().await.expect("first GUI");
        assert!(broker.register_gui().await.is_err());
        let first = broker.clone();
        let first_task = tokio::spawn(async move { first.request_confirmation("peer".into(), 1).await });
        let GuiEvent::Lesc(request) = events.recv().await.expect("request") else {
            panic!("expected an LESC request");
        };
        let second = broker.clone();
        let second_task = tokio::spawn(async move { second.request_confirmation("peer".into(), 2).await });
        assert!(events.try_recv().is_err());
        assert!(!second_task.await.expect("task"));
        assert!(broker.respond(request.id, true).await);
        assert!(first_task.await.expect("task"));
    }

    #[test]
    fn oob_request_roundtrips_and_rejects_malformed_input() {
        let request = OobRequest {
            id: 7,
            peer: "AA:BB:CC:DD:EE:FF".into(),
            host_pubkey: [0x11; PAIR_PUBKEY_LEN],
            phone_pubkey: [0x22; PAIR_PUBKEY_LEN],
        };
        let encoded = request.encode().expect("encode");
        assert_eq!(OobRequest::decode(&encoded), Some(request));
        // Truncated and trailing-garbage frames fail closed.
        assert!(OobRequest::decode(&encoded[..encoded.len() - 1]).is_none());
        let mut extra = encoded.clone();
        extra.push(0);
        assert!(OobRequest::decode(&extra).is_none());
        assert!(OobRequest::decode(b"SYPC\x00\x00\x00\x00\x00\x00\x00\x00").is_none());
    }

    #[test]
    fn bonded_notice_roundtrips_and_is_distinct_from_lesc() {
        let notice = BondedNotice { peer: "phone".into() };
        let encoded = notice.encode().expect("encode");
        assert_eq!(BondedNotice::decode(&encoded), Some(notice));
        assert!(PairingRequest::decode(&encoded).is_none());
        assert!(BondedNotice::decode(b"SYPC\x00\x00").is_none());
    }

    #[test]
    fn frame_is_length_prefixed_and_bounded() {
        let framed = frame(b"hello").expect("frame");
        assert_eq!(&framed[..4], &5u32.to_be_bytes());
        assert_eq!(&framed[4..], b"hello");
        assert!(frame(&vec![0u8; MAX_FRAME_BYTES + 1]).is_none());
    }
}
