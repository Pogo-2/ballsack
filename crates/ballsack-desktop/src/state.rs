//! Managed Tauri state — holds Arc-wrapped adapters and use cases from ballsack-core.

use std::sync::Arc;

use tokio::sync::Mutex;

use ballsack_core::adapters::crypto::e2ee::InMemoryE2eeKeystore;
use ballsack_core::adapters::crypto::identity::IdentityKeyPair;
use ballsack_core::adapters::media::audio::{AudioCaptureSource, AudioPlaybackAdapter};
use ballsack_core::adapters::media::video::VideoPlaybackAdapter;
use ballsack_core::adapters::playback::CombinedPlayback;
use ballsack_core::adapters::quic::client::QuicClientTransport;
use ballsack_core::adapters::room_state::InMemoryRoomState;
use ballsack_core::application::handle_control::HandleControlUseCase;
use ballsack_core::application::join_room::JoinRoomUseCase;
use ballsack_core::application::key_distribute::KeyDistributeUseCase;
use ballsack_core::application::ports::{AppEvents, E2eeKeystore, RoomState, Transport};
use ballsack_core::application::receive_media::ReceiveMediaUseCase;
use ballsack_core::application::send_media::SendMediaUseCase;
use ballsack_core::application::stats::StatsUseCase;
use ballsack_core::domain::identity::{PeerId, RoomId, SenderKeyId, SenderSecret};

// ---------------------------------------------------------------------------
// Call session (created on start_call, dropped on end_call)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct CallSession {
    pub room_id: RoomId,
    pub our_peer_id: PeerId,
    pub sender_key_id: SenderKeyId,
    pub sender_secret: SenderSecret,
    pub transport: Arc<dyn Transport>,
    pub keystore: Arc<dyn E2eeKeystore>,
    pub room_state: Arc<dyn RoomState>,
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
) -> anyhow::Result<CallSession> {
    // Adapters
    let identity = IdentityKeyPair::generate();
    let keystore: Arc<dyn E2eeKeystore> = Arc::new(InMemoryE2eeKeystore::new(identity));
    let room_state: Arc<dyn RoomState> = Arc::new(InMemoryRoomState::new());

    // Connect
    let transport: Arc<dyn Transport> =
        QuicClientTransport::connect(server_addr, "localhost").await?;

    let our_peer_id = PeerId(rand::random::<u64>());

    // Join room
    let join_room = JoinRoomUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_state.clone(),
        app_events.clone(),
    );
    join_room
        .execute(token, room_id, display_name)
        .await?;

    // Distribute initial sender key
    let key_distribute = Arc::new(KeyDistributeUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_state.clone(),
        our_peer_id,
    ));
    let (sender_key_id, sender_secret) = key_distribute.execute(room_id).await?;

    // Spawn background tasks

    // Control handler
    let handle_control = Arc::new(HandleControlUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_state.clone(),
        app_events.clone(),
        our_peer_id,
    ));
    let hc = handle_control.clone();
    tokio::spawn(async move {
        if let Err(e) = hc.run().await {
            tracing::error!("Control handler exited: {e}");
        }
    });

    // Media receiver
    let audio_playback = Arc::new(AudioPlaybackAdapter::new(3));
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

    // Audio send loop
    let send_media = Arc::new(SendMediaUseCase::new(
        transport.clone(),
        keystore.clone(),
        room_id,
        our_peer_id,
        rand::random(),
        rand::random(),
    ));
    let sm = send_media.clone();
    let kid = sender_key_id;
    let secret_clone = SenderSecret(sender_secret.0);
    tokio::spawn(async move {
        let mut audio_capture = AudioCaptureSource::new();
        if let Err(e) = sm
            .run(&mut audio_capture, kid, secret_clone)
            .await
        {
            tracing::error!("Audio send loop exited: {e}");
        }
    });

    Ok(CallSession {
        room_id,
        our_peer_id,
        sender_key_id,
        sender_secret,
        transport,
        keystore,
        room_state,
    })
}
