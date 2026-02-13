//! TauriAppEvents — implements the [`AppEvents`] port by emitting Tauri events.

use async_trait::async_trait;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use ballsack_core::application::ports::AppEvents;
use ballsack_core::domain::identity::{PeerId, PeerInfo};

/// Bridges application-layer events to the Tauri frontend via `app.emit()`.
#[derive(Clone)]
pub struct TauriAppEvents {
    app_handle: AppHandle,
}

impl TauriAppEvents {
    pub fn new(app_handle: AppHandle) -> Self {
        Self { app_handle }
    }
}

// Payload structs for the frontend (must be Serialize + Clone).

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerJoinedPayload {
    peer_id: u64,
    display_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerLeftPayload {
    peer_id: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsPayload {
    rtt_ms: u32,
    loss: f32,
    jitter_ms: f32,
}

#[async_trait]
impl AppEvents for TauriAppEvents {
    async fn emit_peer_joined(&self, peer: &PeerInfo) {
        let _ = self.app_handle.emit(
            "peer-joined",
            PeerJoinedPayload {
                peer_id: peer.peer_id.0,
                display_name: peer.display_name.clone(),
            },
        );
    }

    async fn emit_peer_left(&self, peer_id: PeerId) {
        let _ = self.app_handle.emit(
            "peer-left",
            PeerLeftPayload {
                peer_id: peer_id.0,
            },
        );
    }

    async fn emit_stats(&self, rtt_ms: u32, loss: f32, jitter_ms: f32) {
        let _ = self.app_handle.emit(
            "stats-update",
            StatsPayload {
                rtt_ms,
                loss,
                jitter_ms,
            },
        );
    }
}
