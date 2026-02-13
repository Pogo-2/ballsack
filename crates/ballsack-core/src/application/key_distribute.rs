//! KeyDistribute use case — generates a sender key and seals it for all peers.

use std::sync::Arc;

use tracing::info;

use crate::domain::control::{ControlMsg, SealedKeyEntry};
use crate::domain::identity::{PeerId, RoomId, SenderKeyId, SenderSecret};

use super::ports::{E2eeKeystore, RoomState, Transport};

/// Generates and distributes a new sender key to all current room peers.
pub struct KeyDistributeUseCase {
    transport: Arc<dyn Transport>,
    keystore: Arc<dyn E2eeKeystore>,
    room_state: Arc<dyn RoomState>,
    our_peer_id: PeerId,
}

impl KeyDistributeUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        keystore: Arc<dyn E2eeKeystore>,
        room_state: Arc<dyn RoomState>,
        our_peer_id: PeerId,
    ) -> Self {
        Self {
            transport,
            keystore,
            room_state,
            our_peer_id,
        }
    }

    /// Generate a new sender key and distribute it. Returns (key_id, secret) for
    /// the caller to use for encryption.
    pub async fn execute(
        &self,
        room_id: RoomId,
    ) -> anyhow::Result<(SenderKeyId, SenderSecret)> {
        let (key_id, secret) = self.keystore.generate_sender_key();

        let peers = self.room_state.peers();
        let mut sealed_keys = Vec::with_capacity(peers.len());

        for peer in &peers {
            let sealed = self
                .keystore
                .seal_sender_secret(&peer.x25519_pub, &secret)?;
            sealed_keys.push(SealedKeyEntry {
                recipient_id: peer.peer_id,
                recipient_x25519_fingerprint: peer.x25519_pub.0[..8].to_vec(),
                sealed_sender_secret: sealed,
            });
        }

        self.transport
            .send_control(ControlMsg::KeyDistribute {
                room_id,
                sender_id: self.our_peer_id,
                sender_key_id: key_id,
                algo: "ChaCha20Poly1305".to_string(),
                sealed_keys,
                signature: None, // TODO: sign with Ed25519
            })
            .await?;

        info!(?key_id, recipients = peers.len(), "Distributed sender key");

        Ok((key_id, secret))
    }
}
