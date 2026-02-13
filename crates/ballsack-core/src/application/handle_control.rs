//! HandleControl use case — dispatches incoming control messages.
//!
//! Runs as a long-lived loop, updating room state, key store, and emitting UI events.

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::domain::control::ControlMsg;
use crate::domain::identity::{PeerId, RoomId, SharedSenderKey};

use super::bitrate_adapt::ReceiverReportData;
use super::key_distribute::KeyDistributeUseCase;
use super::ports::{AppEvents, E2eeKeystore, RoomState, Transport};

/// Continuously receives and dispatches control messages.
pub struct HandleControlUseCase {
    transport: Arc<dyn Transport>,
    keystore: Arc<dyn E2eeKeystore>,
    room_state: Arc<dyn RoomState>,
    app_events: Arc<dyn AppEvents>,
    our_peer_id: PeerId,
    room_id: RoomId,
    key_distribute: Arc<KeyDistributeUseCase>,
    /// The current sender key used by the media send loop.
    shared_key: SharedSenderKey,
    /// Channel for forwarding receiver reports to the bitrate adapter.
    report_tx: Option<mpsc::Sender<ReceiverReportData>>,
}

impl HandleControlUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        keystore: Arc<dyn E2eeKeystore>,
        room_state: Arc<dyn RoomState>,
        app_events: Arc<dyn AppEvents>,
        our_peer_id: PeerId,
        room_id: RoomId,
        key_distribute: Arc<KeyDistributeUseCase>,
        shared_key: SharedSenderKey,
        report_tx: Option<mpsc::Sender<ReceiverReportData>>,
    ) -> Self {
        Self {
            transport,
            keystore,
            room_state,
            app_events,
            our_peer_id,
            room_id,
            key_distribute,
            shared_key,
            report_tx,
        }
    }

    /// Run the control-message dispatch loop forever (until connection drops).
    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            let msg = self.transport.recv_control().await?;
            self.handle(msg).await?;
        }
    }

    async fn handle(&self, msg: ControlMsg) -> anyhow::Result<()> {
        match msg {
            ControlMsg::PeerJoined { peer } => {
                info!(peer_id = ?peer.peer_id, name = %peer.display_name, "Peer joined");
                self.app_events.emit_peer_joined(&peer).await;
                self.room_state.add_peer(peer);

                // Re-distribute our CURRENT sender key so the new peer can
                // decrypt media that is already being sent.
                let (key_id, ref secret) = *self.shared_key.read().await;
                if let Err(e) = self
                    .key_distribute
                    .distribute_existing(self.room_id, key_id, secret)
                    .await
                {
                    warn!("Failed to re-distribute sender key after PeerJoined: {e}");
                }
            }

            ControlMsg::PeerLeft { peer_id } => {
                info!(?peer_id, "Peer left");
                self.room_state.remove_peer(peer_id);
                self.app_events.emit_peer_left(peer_id).await;
                // TODO: trigger sender key rotation for forward secrecy.
            }

            ControlMsg::KeyDistribute {
                sender_id,
                sender_key_id,
                sealed_keys,
                ..
            } => {
                // Find the entry sealed for us.
                let our_entry = sealed_keys.iter().find(|e| e.recipient_id == self.our_peer_id);
                if let Some(entry) = our_entry {
                    match self.keystore.unseal_sender_secret(&entry.sealed_sender_secret) {
                        Ok(secret) => {
                            self.keystore
                                .store_peer_sender_key(sender_id, sender_key_id, secret);
                            info!(?sender_id, ?sender_key_id, "Stored new sender key");
                        }
                        Err(e) => {
                            warn!(?sender_id, ?sender_key_id, "Failed to unseal sender key: {e}");
                        }
                    }
                } else {
                    warn!(?sender_id, "KeyDistribute did not contain an entry for us");
                }
            }

            ControlMsg::KeyRotate {
                sender_id,
                new_sender_key_id,
                sealed_keys,
                ..
            } => {
                let our_entry = sealed_keys.iter().find(|e| e.recipient_id == self.our_peer_id);
                if let Some(entry) = our_entry {
                    match self.keystore.unseal_sender_secret(&entry.sealed_sender_secret) {
                        Ok(secret) => {
                            self.keystore
                                .store_peer_sender_key(sender_id, new_sender_key_id, secret);
                            info!(?sender_id, ?new_sender_key_id, "Rotated sender key");
                        }
                        Err(e) => {
                            warn!(?sender_id, "Failed to unseal rotated key: {e}");
                        }
                    }
                }
            }

            ControlMsg::VideoKeyframeRequest { .. } => {
                // Will be handled by the media encoder (video adapter).
                // For now just log.
                info!("Received VideoKeyframeRequest (not yet handled)");
            }

            ControlMsg::ReceiverReport {
                loss_fraction,
                jitter_ms,
                buffer_depth,
                ..
            } => {
                if let Some(ref tx) = self.report_tx {
                    let _ = tx
                        .try_send(ReceiverReportData {
                            loss_fraction,
                            jitter_ms,
                            buffer_depth,
                        });
                }
                info!(loss_fraction, jitter_ms, buffer_depth, "Received ReceiverReport");
            }

            ControlMsg::StatsReport {
                rtt_ms,
                loss,
                jitter_ms,
                ..
            } => {
                self.app_events.emit_stats(rtt_ms, loss, jitter_ms).await;
            }

            other => {
                tracing::debug!(?other, "HandleControl: ignoring unhandled message");
            }
        }
        Ok(())
    }
}
