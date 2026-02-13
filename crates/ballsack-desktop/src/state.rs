//! Managed Tauri state — holds Arc-wrapped adapters and use cases from ballsack-core.

use std::sync::atomic::{AtomicBool, AtomicI32};
use std::sync::Arc;

use tokio::sync::Mutex;

use ballsack_core::adapters::crypto::e2ee::InMemoryE2eeKeystore;
use ballsack_core::adapters::crypto::identity::IdentityKeyPair;
use ballsack_core::adapters::media::audio::{CpalAudioCapture, CpalAudioPlayback};
use ballsack_core::adapters::media::video::VideoPlaybackAdapter;
use ballsack_core::adapters::playback::CombinedPlayback;
use ballsack_core::adapters::quic::client::QuicClientTransport;
use ballsack_core::adapters::room_state::InMemoryRoomState;
use ballsack_core::application::bitrate_adapt::{BitrateAdaptUseCase, DEFAULT_BITRATE};
use ballsack_core::application::handle_control::HandleControlUseCase;
use ballsack_core::application::join_room::JoinRoomUseCase;
use ballsack_core::application::key_distribute::KeyDistributeUseCase;
use ballsack_core::application::ports::{AppEvents, E2eeKeystore, RoomState, Transport};
use ballsack_core::application::receive_media::ReceiveMediaUseCase;
use ballsack_core::application::send_media::SendMediaUseCase;
use ballsack_core::application::stats::StatsUseCase;
use ballsack_core::domain::identity::{PeerId, RoomId, SenderSecret, SharedSenderKey};

// ---------------------------------------------------------------------------
// Call session (created on start_call, dropped on end_call)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct CallSession {
    pub room_id: RoomId,
    pub our_peer_id: PeerId,
    pub shared_key: SharedSenderKey,
    pub transport: Arc<dyn Transport>,
    pub keystore: Arc<dyn E2eeKeystore>,
    pub room_state: Arc<dyn RoomState>,
    /// Mute flag for the microphone (shared with the capture task).
    pub mute_flag: Arc<AtomicBool>,
    /// Audio playback handle — stopped on drop.
    pub audio_playback: Arc<CpalAudioPlayback>,
}

impl Drop for CallSession {
    fn drop(&mut self) {
        // Stop the playout loop so it doesn't leak across sessions.
        self.audio_playback.stop();
    }
}

// ---------------------------------------------------------------------------
// App-level state managed by Tauri
// ---------------------------------------------------------------------------

pub struct AppState {
    /// Current active call session (None when not in a call).
    pub session: Mutex<Option<CallSession>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Build all adapters, connect, join a room, distribute keys, and spawn
/// background tasks. Returns a [`CallSession`].
pub async fn build_call_session(
    server_addr: std::net::SocketAddr,
    room_id: RoomId,
    display_name: String,
    token: String,
    app_events: Arc<dyn AppEvents>,
    input_device: Option<String>,
) -> anyhow::Result<CallSession> {
    // Adapters
    let identity = IdentityKeyPair::generate();
    let keystore: Arc<dyn E2eeKeystore> = Arc::new(InMemoryE2eeKeystore::new(identity));
    let room_state: Arc<dyn RoomState> = Arc::new(InMemoryRoomState::new());

    // Connect
    let transport: Arc<dyn Transport> =
        QuicClientTransport::connect(server_addr, "localhost").await?;

    // Join room — the server assigns our PeerId
    let join_room = JoinRoomUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_state.clone(),
        app_events.clone(),
    );
    let our_peer_id = join_room
        .execute(token, room_id, display_name)
        .await?;

    // Distribute initial sender key and store in shared holder
    let key_distribute = Arc::new(KeyDistributeUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_state.clone(),
        our_peer_id,
    ));
    let (sender_key_id, sender_secret) = key_distribute.execute(room_id).await?;
    let shared_key: SharedSenderKey = Arc::new(tokio::sync::RwLock::new((
        sender_key_id,
        SenderSecret(sender_secret.0),
    )));

    // Spawn background tasks

    // Shared bitrate target for adaptation (encoder reads, adapter writes).
    let bitrate_target = Arc::new(AtomicI32::new(DEFAULT_BITRATE));

    // Channel for forwarding receiver reports from control handler → bitrate adapter.
    let (report_tx, report_rx) = tokio::sync::mpsc::channel(16);

    // Control handler (shares key_distribute + shared_key so it can
    // re-distribute the CURRENT key when a new peer joins)
    let handle_control = Arc::new(HandleControlUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_state.clone(),
        app_events.clone(),
        our_peer_id,
        room_id,
        key_distribute.clone(),
        shared_key.clone(),
        Some(report_tx),
    ));
    let hc = handle_control.clone();
    tokio::spawn(async move {
        if let Err(e) = hc.run().await {
            tracing::error!("Control handler exited: {e}");
        }
    });

    // Bitrate adaptation controller (AIMD)
    let bitrate_adapt = BitrateAdaptUseCase::new(
        transport.clone(),
        bitrate_target.clone(),
        report_rx,
    );
    tokio::spawn(async move {
        if let Err(e) = bitrate_adapt.run().await {
            tracing::error!("Bitrate adapter exited: {e}");
        }
    });

    // Media receiver — real audio playback via cpal + Opus decoding
    let audio_playback = Arc::new(CpalAudioPlayback::new(10)?);
    // Attach transport so playout loop can send ReceiverReport messages.
    audio_playback
        .set_transport(transport.clone(), our_peer_id)
        .await;
    audio_playback.start(); // spawns the 20ms playout mixer loop
    let audio_pb_handle = audio_playback.clone(); // kept in CallSession for stop-on-drop
    let video_playback = Arc::new(VideoPlaybackAdapter::new(10));
    let combined_playback = Arc::new(CombinedPlayback {
        audio: audio_playback,
        video: video_playback,
    });
    let receive_media = Arc::new(ReceiveMediaUseCase::new(
        transport.clone(),
        keystore.clone(),
        combined_playback,
    ));
    let rm = receive_media.clone();
    tokio::spawn(async move {
        if let Err(e) = rm.run().await {
            tracing::error!("Media receiver exited: {e}");
        }
    });

    // Stats reporter
    let stats = Arc::new(StatsUseCase::new(
        transport.clone(),
        app_events.clone(),
        std::time::Duration::from_secs(5),
    ));
    let st = stats.clone();
    tokio::spawn(async move {
        if let Err(e) = st.run().await {
            tracing::error!("Stats reporter exited: {e}");
        }
    });

    // Real audio capture from microphone via cpal + Opus encoding
    let mut audio_capture = CpalAudioCapture::with_device(input_device.as_deref())?;
    let mute_flag = audio_capture.mute_handle();
    audio_capture.set_bitrate_target(bitrate_target);

    // Audio send loop (reads key from shared_key on each frame)
    let send_media = Arc::new(SendMediaUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_id,
        our_peer_id,
        rand::random(),
        rand::random(),
    ));
    let sm = send_media.clone();
    let sk = shared_key.clone();
    tokio::spawn(async move {
        if let Err(e) = sm.run(&mut audio_capture, sk).await {
            tracing::error!("Audio send loop exited: {e}");
        }
    });

    Ok(CallSession {
        room_id,
        our_peer_id,
        shared_key,
        transport,
        keystore,
        room_state,
        mute_flag,
        audio_playback: audio_pb_handle,
    })
}
