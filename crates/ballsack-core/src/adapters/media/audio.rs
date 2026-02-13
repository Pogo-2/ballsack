//! Audio adapter: capture, Opus encode/decode, and jitter buffer.
//!
//! Implements the [`MediaCapture`] port (audio capture) and contributes to
//! [`MediaPlayback`] (audio jitter buffer + decode + playout).

use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::trace;

use crate::application::ports::{CapturedFrame, MediaCapture, MediaPlayback};
use crate::domain::identity::PeerId;

use super::clock::AUDIO_FRAME_TICKS;

// ---------------------------------------------------------------------------
// Audio capture (stub for now — real impl would use cpal + audiopus)
// ---------------------------------------------------------------------------

/// Audio capture source that produces Opus-encoded frames.
///
/// For the prototype, this generates silence frames on a 20 ms timer.
/// Replace internals with cpal capture + audiopus::coder::Encoder for real audio.
pub struct AudioCaptureSource {
    /// Timestamp counter (48 kHz ticks).
    timestamp: u32,
    /// Frame duration in ticks.
    frame_ticks: u32,
}

impl AudioCaptureSource {
    pub fn new() -> Self {
        Self {
            timestamp: 0,
            frame_ticks: AUDIO_FRAME_TICKS,
        }
    }
}

#[async_trait]
impl MediaCapture for AudioCaptureSource {
    async fn next_frame(&mut self) -> anyhow::Result<CapturedFrame> {
        // Wait 20 ms (one frame period)
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Generate a silence Opus frame (real impl would capture from mic + encode)
        // A minimal Opus silence frame is ~3 bytes.
        let opus_frame = vec![0xF8, 0xFF, 0xFE]; // Opus silence (DTX-ish)

        let ts = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(self.frame_ticks);

        Ok(CapturedFrame {
            data: opus_frame,
            is_audio: true,
            timestamp: ts,
            video_chunk: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Audio jitter buffer
// ---------------------------------------------------------------------------

/// Per-sender, per-ssrc audio jitter buffer.
///
/// Buffers incoming Opus frames and releases them at a steady playout rate.
/// Target buffer depth: 20–60 ms (1–3 frames).
#[allow(dead_code)]
struct AudioJitterBuffer {
    /// Ordered map of seq -> opus frame bytes.
    buffer: BTreeMap<u16, Vec<u8>>,
    /// Next sequence number we expect to play out.
    next_playout_seq: u16,
    /// Maximum buffered frames before we start dropping.
    max_frames: usize,
}

impl AudioJitterBuffer {
    fn new(max_frames: usize) -> Self {
        Self {
            buffer: BTreeMap::new(),
            next_playout_seq: 0,
            max_frames,
        }
    }

    /// Insert a frame into the buffer.
    fn insert(&mut self, seq: u16, opus_frame: Vec<u8>) {
        if self.buffer.len() >= self.max_frames {
            // Drop oldest to make room.
            if let Some((&oldest_seq, _)) = self.buffer.iter().next() {
                self.buffer.remove(&oldest_seq);
            }
        }
        self.buffer.insert(seq, opus_frame);
    }

    /// Pull the next frame for playout, if available.
    #[allow(dead_code)]
    fn pull(&mut self) -> Option<Vec<u8>> {
        if let Some(frame) = self.buffer.remove(&self.next_playout_seq) {
            self.next_playout_seq = self.next_playout_seq.wrapping_add(1);
            Some(frame)
        } else {
            // Packet loss — advance anyway so we don't get stuck.
            self.next_playout_seq = self.next_playout_seq.wrapping_add(1);
            None // Caller should use Opus PLC or insert silence.
        }
    }
}

// ---------------------------------------------------------------------------
// Audio playback adapter
// ---------------------------------------------------------------------------

/// Audio playback manager handling per-peer jitter buffers.
///
/// Implements the audio portion of [`MediaPlayback`].
pub struct AudioPlaybackAdapter {
    /// (peer_id, ssrc) -> jitter buffer
    buffers: Mutex<BTreeMap<(u64, u32), AudioJitterBuffer>>,
    /// Max buffered frames per stream (20ms each).
    max_buffer_frames: usize,
}

impl AudioPlaybackAdapter {
    pub fn new(max_buffer_frames: usize) -> Self {
        Self {
            buffers: Mutex::new(BTreeMap::new()),
            max_buffer_frames,
        }
    }

    fn get_or_create_buffer<'a>(
        buffers: &'a mut BTreeMap<(u64, u32), AudioJitterBuffer>,
        peer_id: PeerId,
        ssrc: u32,
        max_frames: usize,
    ) -> &'a mut AudioJitterBuffer {
        buffers
            .entry((peer_id.0, ssrc))
            .or_insert_with(|| AudioJitterBuffer::new(max_frames))
    }
}

#[async_trait]
impl MediaPlayback for AudioPlaybackAdapter {
    async fn push_audio(
        &self,
        peer_id: PeerId,
        ssrc: u32,
        seq: u16,
        _timestamp: u32,
        opus_frame: &[u8],
    ) -> anyhow::Result<()> {
        let mut buffers = self.buffers.lock().await;
        let buf = Self::get_or_create_buffer(
            &mut buffers,
            peer_id,
            ssrc,
            self.max_buffer_frames,
        );
        buf.insert(seq, opus_frame.to_vec());
        trace!(?peer_id, ssrc, seq, "Buffered audio frame");
        Ok(())
    }

    async fn push_video(
        &self,
        _peer_id: PeerId,
        _ssrc: u32,
        _seq: u16,
        _timestamp: u32,
        _frame_id: u32,
        _chunk_index: u16,
        _chunk_count: u16,
        _is_keyframe: bool,
        _encoded_bytes: &[u8],
    ) -> anyhow::Result<()> {
        // Audio adapter ignores video — the combined MediaPlayback should
        // dispatch to the correct sub-adapter. For now, no-op.
        Ok(())
    }
}
