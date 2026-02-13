//! Tauri command handlers — invoked from the React frontend via `invoke()`.

use std::sync::Arc;

use serde::Serialize;
use tauri::AppHandle;
use tracing::info;

use ballsack_core::adapters::media::audio::{enumerate_input_devices, AudioDeviceInfo};
use ballsack_core::domain::identity::RoomId;

use crate::events::TauriAppEvents;
use crate::state::{build_call_session, AppState};

// ---------------------------------------------------------------------------
// Response types (must be Serialize for Tauri IPC)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCallResult {
    pub room_id: u64,
    pub peer_id: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerEntry {
    pub peer_id: u64,
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Join a room, distribute keys, and start sending/receiving media.
#[tauri::command]
pub async fn start_call(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    room_id: u64,
    display_name: String,
    input_device: Option<String>,
) -> Result<StartCallResult, String> {
    info!(room_id, %display_name, ?input_device, "start_call invoked");

    let app_events: Arc<dyn ballsack_core::application::ports::AppEvents> =
        Arc::new(TauriAppEvents::new(app.clone()));

    let server_addr: std::net::SocketAddr = "127.0.0.1:4433"
        .parse()
        .map_err(|e| format!("bad server addr: {e}"))?;

    let session = build_call_session(
        server_addr,
        RoomId(room_id),
        display_name,
        "dev-token".into(),
        app_events,
        input_device,
    )
    .await
    .map_err(|e| format!("Failed to start call: {e}"))?;

    let result = StartCallResult {
        room_id: session.room_id.0,
        peer_id: session.our_peer_id.0,
    };

    let mut guard = state.session.lock().await;
    *guard = Some(session);

    info!("Call started successfully");
    Ok(result)
}

/// Leave the current call.
#[tauri::command]
pub async fn end_call(
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    info!("end_call invoked");
    let mut guard = state.session.lock().await;
    // Actively close the QUIC connection so the server is notified immediately.
    // This unblocks all background tasks waiting on recv_control / recv_datagram.
    if let Some(session) = guard.as_ref() {
        session.transport.close();
    }
    *guard = None;
    info!("Call ended");
    Ok(())
}

/// Get the list of peers in the current room.
#[tauri::command]
pub async fn get_peers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PeerEntry>, String> {
    let guard = state.session.lock().await;
    match guard.as_ref() {
        Some(session) => {
            let peers = session
                .room_state
                .peers()
                .into_iter()
                .map(|p| PeerEntry {
                    peer_id: p.peer_id.0,
                    display_name: p.display_name,
                })
                .collect();
            Ok(peers)
        }
        None => Ok(vec![]),
    }
}

/// Set the microphone mute state.
#[tauri::command]
pub async fn set_muted(
    state: tauri::State<'_, AppState>,
    muted: bool,
) -> Result<bool, String> {
    let guard = state.session.lock().await;
    match guard.as_ref() {
        Some(session) => {
            session
                .mute_flag
                .store(muted, std::sync::atomic::Ordering::Relaxed);
            info!(muted, "Microphone mute toggled");
            Ok(muted)
        }
        None => Err("Not in a call".into()),
    }
}

/// List available audio input devices.
#[tauri::command]
pub async fn list_audio_devices() -> Result<Vec<AudioDeviceInfo>, String> {
    Ok(enumerate_input_devices())
}

/// Get the current call stats (placeholder — real stats will come from transport metrics).
#[tauri::command]
pub async fn get_stats() -> Result<serde_json::Value, String> {
    // TODO: pull real stats from transport adapter.
    Ok(serde_json::json!({
        "rttMs": 0,
        "loss": 0.0,
        "jitterMs": 0.0,
        "bitrateIn": 0,
        "bitrateOut": 0
    }))
}
