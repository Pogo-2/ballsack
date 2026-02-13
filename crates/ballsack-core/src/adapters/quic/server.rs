//! Quinn-based QUIC SFU server.
//!
//! Accepts client connections, maintains room state, routes control messages
//! and forwards media datagrams to all other peers in a room.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use quinn::{Connection, Endpoint, RecvStream, SendStream, ServerConfig};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::application::ports::ControlCodec;
use crate::domain::control::ControlMsg;
use crate::domain::identity::{PeerId, PeerInfo, RoomId};
use crate::domain::media::MEDIA_HEADER_SIZE;

use super::codec::CborControlCodec;

// ---------------------------------------------------------------------------
// Per-peer connection state on the server
// ---------------------------------------------------------------------------

struct PeerConn {
    peer_id: PeerId,
    connection: Connection,
    control_send: Mutex<SendStream>,
    // recv is handled by the per-peer task
}

// ---------------------------------------------------------------------------
// Room state on the server
// ---------------------------------------------------------------------------

struct Room {
    peers: HashMap<u64, Arc<PeerConn>>,
}

impl Room {
    fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// SFU Server
// ---------------------------------------------------------------------------

/// Simple SFU relay server.
pub struct SfuServer {
    endpoint: Endpoint,
    rooms: Arc<RwLock<HashMap<u64, RwLock<Room>>>>,
    codec: CborControlCodec,
    next_peer_id: Arc<std::sync::atomic::AtomicU64>,
}

impl SfuServer {
    /// Create and bind the server.
    pub fn new(bind_addr: SocketAddr) -> anyhow::Result<Self> {
        let (server_config, _cert_der) = Self::generate_self_signed_config()?;
        let endpoint = Endpoint::server(server_config, bind_addr)?;
        info!(%bind_addr, "SFU server listening");

        Ok(Self {
            endpoint,
            rooms: Arc::new(RwLock::new(HashMap::new())),
            codec: CborControlCodec,
            next_peer_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }

    /// Run the accept loop.
    pub async fn run(self: Arc<Self>) -> anyhow::Result<()> {
        while let Some(incoming) = self.endpoint.accept().await {
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                match incoming.await {
                    Ok(conn) => {
                        if let Err(e) = server.handle_connection(conn).await {
                            warn!("Connection handler error: {e}");
                        }
                    }
                    Err(e) => warn!("Failed to accept connection: {e}"),
                }
            });
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Connection handling
    // -----------------------------------------------------------------------

    async fn handle_connection(self: &Arc<Self>, connection: Connection) -> anyhow::Result<()> {
        let peer_id = PeerId(
            self.next_peer_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        info!(?peer_id, "New connection");

        // Accept the control bidirectional stream from the client.
        let (control_send, mut control_recv) = connection.accept_bi().await?;

        let peer_conn = Arc::new(PeerConn {
            peer_id,
            connection: connection.clone(),
            control_send: Mutex::new(control_send),
        });

        // Wait for Auth + JoinRoom
        let room_id = self
            .handle_handshake(&peer_conn, &mut control_recv)
            .await?;

        // Spawn datagram forwarder for this peer
        let server = Arc::clone(self);
        let conn_clone = connection.clone();
        let rid = room_id;
        let pid = peer_id;
        tokio::spawn(async move {
            if let Err(e) = server.forward_datagrams(conn_clone, rid, pid).await {
                warn!(?pid, "Datagram forwarder ended: {e}");
            }
        });

        // Control message loop
        let result = self
            .control_loop(&peer_conn, &mut control_recv, room_id)
            .await;

        // Cleanup: remove peer from room
        self.remove_peer_from_room(room_id, peer_id).await;

        result
    }

    async fn handle_handshake(
        &self,
        peer_conn: &Arc<PeerConn>,
        control_recv: &mut RecvStream,
    ) -> anyhow::Result<RoomId> {
        // Read Auth
        let msg = self.read_control_msg(control_recv).await?;
        match msg {
            ControlMsg::Auth { token } => {
                info!(peer_id = ?peer_conn.peer_id, "Auth received (token: {}...)", &token[..token.len().min(8)]);
                // TODO: validate token
            }
            other => anyhow::bail!("Expected Auth, got {:?}", other),
        }

        // Read JoinRoom
        let msg = self.read_control_msg(control_recv).await?;
        match msg {
            ControlMsg::JoinRoom {
                room_id,
                display_name,
                x25519_pub,
                ed25519_pub,
            } => {
                info!(peer_id = ?peer_conn.peer_id, ?room_id, %display_name, "JoinRoom");

                let peer_info = PeerInfo {
                    peer_id: peer_conn.peer_id,
                    display_name,
                    x25519_pub,
                    ed25519_pub,
                };

                // Get or create the room
                let existing_peers = self
                    .add_peer_to_room(room_id, Arc::clone(peer_conn))
                    .await;

                // Send PeerList to the new peer
                self.send_control_msg(peer_conn, &ControlMsg::PeerList {
                    peers: existing_peers,
                })
                .await?;

                // Notify existing peers
                self.broadcast_to_room(
                    room_id,
                    peer_conn.peer_id,
                    &ControlMsg::PeerJoined {
                        peer: peer_info,
                    },
                )
                .await;

                Ok(room_id)
            }
            other => anyhow::bail!("Expected JoinRoom, got {:?}", other),
        }
    }

    async fn control_loop(
        &self,
        peer_conn: &Arc<PeerConn>,
        control_recv: &mut RecvStream,
        room_id: RoomId,
    ) -> anyhow::Result<()> {
        loop {
            let msg = match self.read_control_msg(control_recv).await {
                Ok(m) => m,
                Err(_) => break, // stream closed
            };

            match &msg {
                ControlMsg::KeyDistribute { .. } | ControlMsg::KeyRotate { .. } => {
                    // Broadcast key messages to all other peers in the room.
                    self.broadcast_to_room(room_id, peer_conn.peer_id, &msg)
                        .await;
                }
                ControlMsg::VideoKeyframeRequest {
                    target_sender_id, ..
                } => {
                    // Forward to the target peer.
                    self.send_to_peer_in_room(room_id, *target_sender_id, &msg)
                        .await;
                }
                ControlMsg::StatsReport { .. } => {
                    // Server logs; does not forward.
                    tracing::debug!(peer_id = ?peer_conn.peer_id, "StatsReport received");
                }
                _ => {
                    tracing::debug!(peer_id = ?peer_conn.peer_id, ?msg, "Ignoring control message");
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Datagram forwarding (SFU media relay)
    // -----------------------------------------------------------------------

    async fn forward_datagrams(
        &self,
        connection: Connection,
        room_id: RoomId,
        sender_peer_id: PeerId,
    ) -> anyhow::Result<()> {
        loop {
            let datagram = connection.read_datagram().await?;

            // Validate minimum header size
            if datagram.len() < MEDIA_HEADER_SIZE + 2 {
                warn!("Datagram too short, dropping");
                continue;
            }

            // Forward to all other peers in the room
            let rooms = self.rooms.read().await;
            if let Some(room_lock) = rooms.get(&room_id.0) {
                let room = room_lock.read().await;
                for (pid, peer) in &room.peers {
                    if *pid != sender_peer_id.0 {
                        if let Err(e) = peer.connection.send_datagram(datagram.clone()) {
                            warn!(peer_id = pid, "Failed to forward datagram: {e}");
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Room management
    // -----------------------------------------------------------------------

    /// Add a peer to a room; returns existing peer list (for PeerList response).
    async fn add_peer_to_room(
        &self,
        room_id: RoomId,
        peer_conn: Arc<PeerConn>,
    ) -> Vec<PeerInfo> {
        let mut rooms = self.rooms.write().await;
        let room_lock = rooms
            .entry(room_id.0)
            .or_insert_with(|| RwLock::new(Room::new()));
        let mut room = room_lock.write().await;

        // Collect existing peer infos (we don't store PeerInfo on server in this
        // prototype; we'd need a separate store — for now return empty and rely
        // on the JoinRoom message having been broadcast to set up state).
        // TODO: Store PeerInfo on server for proper PeerList.
        let existing: Vec<PeerInfo> = Vec::new();

        room.peers.insert(peer_conn.peer_id.0, peer_conn);
        existing
    }

    async fn remove_peer_from_room(&self, room_id: RoomId, peer_id: PeerId) {
        let rooms = self.rooms.read().await;
        if let Some(room_lock) = rooms.get(&room_id.0) {
            let mut room = room_lock.write().await;
            room.peers.remove(&peer_id.0);
        }

        // Notify others
        self.broadcast_to_room(room_id, peer_id, &ControlMsg::PeerLeft { peer_id })
            .await;

        info!(?peer_id, ?room_id, "Peer removed from room");
    }

    async fn broadcast_to_room(
        &self,
        room_id: RoomId,
        exclude_peer: PeerId,
        msg: &ControlMsg,
    ) {
        let rooms = self.rooms.read().await;
        if let Some(room_lock) = rooms.get(&room_id.0) {
            let room = room_lock.read().await;
            for (pid, peer) in &room.peers {
                if *pid != exclude_peer.0 {
                    if let Err(e) = self.send_control_msg(peer, msg).await {
                        warn!(peer_id = pid, "Failed to send control message: {e}");
                    }
                }
            }
        }
    }

    async fn send_to_peer_in_room(
        &self,
        room_id: RoomId,
        target: PeerId,
        msg: &ControlMsg,
    ) {
        let rooms = self.rooms.read().await;
        if let Some(room_lock) = rooms.get(&room_id.0) {
            let room = room_lock.read().await;
            if let Some(peer) = room.peers.get(&target.0) {
                if let Err(e) = self.send_control_msg(peer, msg).await {
                    warn!(?target, "Failed to send control message to target: {e}");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Wire helpers
    // -----------------------------------------------------------------------

    async fn read_control_msg(&self, recv: &mut RecvStream) -> anyhow::Result<ControlMsg> {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut payload = vec![0u8; len];
        recv.read_exact(&mut payload).await?;

        self.codec.decode(&payload)
    }

    async fn send_control_msg(
        &self,
        peer: &PeerConn,
        msg: &ControlMsg,
    ) -> anyhow::Result<()> {
        let payload = self.codec.encode(msg)?;
        let len = (payload.len() as u32).to_be_bytes();

        let mut send = peer.control_send.lock().await;
        send.write_all(&len).await?;
        send.write_all(&payload).await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Self-signed TLS config (dev only)
    // -----------------------------------------------------------------------

    fn generate_self_signed_config() -> anyhow::Result<(ServerConfig, Vec<u8>)> {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let cert_der = cert.cert.der().to_vec();
        let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(
            cert.key_pair.serialize_der(),
        );

        let server_crypto = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert_der.clone())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key_der),
            )?;

        let mut transport_config = quinn::TransportConfig::default();
        transport_config.max_idle_timeout(Some(
            quinn::IdleTimeout::try_from(std::time::Duration::from_secs(30))?,
        ));
        // Enable datagrams
        transport_config.datagram_receive_buffer_size(Some(65535));

        let mut server_config = ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)?,
        ));
        server_config.transport_config(Arc::new(transport_config));

        Ok((server_config, cert_der))
    }
}
