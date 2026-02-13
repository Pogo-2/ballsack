//! JoinRoom use case.
//!
//! Authenticates with the server, sends JoinRoom, and waits for the initial PeerList.

use std::sync::Arc;

use tracing::info;

use crate::domain::control::ControlMsg;
use crate::domain::identity::RoomId;

use super::ports::{AppEvents, E2eeKeystore, RoomState, Transport};

/// Orchestrates joining a room.
pub struct JoinRoomUseCase {
    transport: Arc<dyn Transport>,
    keystore: Arc<dyn E2eeKeystore>,
    room_state: Arc<dyn RoomState>,
    app_events: Arc<dyn AppEvents>,
}

impl JoinRoomUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        keystore: Arc<dyn E2eeKeystore>,
        room_state: Arc<dyn RoomState>,
        app_events: Arc<dyn AppEvents>,
    ) -> Self {
        Self {
            transport,
            keystore,
            room_state,
            app_events,
        }
    }

    /// Execute the join-room flow.
    ///
    /// 1. Send `Auth` with the given token.
    /// 2. Send `JoinRoom` with our public keys.
    /// 3. Wait for `PeerList` and populate `RoomState`.
    pub async fn execute(
        &self,
        token: String,
        room_id: RoomId,
        display_name: String,
    ) -> anyhow::Result<()> {
        // --- 1. Authenticate ---
        self.transport
            .send_control(ControlMsg::Auth {
                token: token.clone(),
            })
            .await?;
        info!("Sent Auth");

        // --- 2. Join room ---
        let x25519_pub = self.keystore.our_x25519_pub();
        let ed25519_pub = self.keystore.our_ed25519_pub();

        self.transport
            .send_control(ControlMsg::JoinRoom {
                room_id,
                display_name: display_name.clone(),
                x25519_pub,
                ed25519_pub,
            })
            .await?;
        info!(?room_id, "Sent JoinRoom");

        // --- 3. Wait for PeerList ---
        loop {
            let msg = self.transport.recv_control().await?;
            match msg {
                ControlMsg::PeerList { peers } => {
                    info!(count = peers.len(), "Received PeerList");
                    for peer in peers {
                        self.app_events.emit_peer_joined(&peer).await;
                        self.room_state.add_peer(peer);
                    }
                    break;
                }
                other => {
                    // Might receive other messages during handshake; log and skip.
                    tracing::debug!(?other, "Ignoring message while waiting for PeerList");
                }
            }
        }

        Ok(())
    }
}
