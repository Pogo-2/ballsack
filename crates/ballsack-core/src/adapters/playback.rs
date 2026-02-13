//! Combined [`MediaPlayback`] adapter that dispatches audio and video to
//! their respective sub-adapters.

use std::sync::Arc;

use async_trait::async_trait;

use crate::application::ports::MediaPlayback;
use crate::domain::identity::PeerId;

use super::media::audio::AudioPlaybackAdapter;
use super::media::video::VideoPlaybackAdapter;

/// Dispatches incoming media to the correct playback sub-adapter.
pub struct CombinedPlayback {
    pub audio: Arc<AudioPlaybackAdapter>,
    pub video: Arc<VideoPlaybackAdapter>,
}

#[async_trait]
impl MediaPlayback for CombinedPlayback {
    async fn push_audio(
        &self,
        peer_id: PeerId,
        ssrc: u32,
        seq: u16,
        timestamp: u32,
        opus_frame: &[u8],
    ) -> anyhow::Result<()> {
        self.audio
            .push_audio(peer_id, ssrc, seq, timestamp, opus_frame)
            .await
    }

    async fn push_video(
        &self,
        peer_id: PeerId,
        ssrc: u32,
        seq: u16,
        timestamp: u32,
        frame_id: u32,
        chunk_index: u16,
        chunk_count: u16,
        is_keyframe: bool,
        encoded_bytes: &[u8],
    ) -> anyhow::Result<()> {
        self.video
            .push_video(
                peer_id,
                ssrc,
                seq,
                timestamp,
                frame_id,
                chunk_index,
                chunk_count,
                is_keyframe,
                encoded_bytes,
            )
            .await
    }
}
