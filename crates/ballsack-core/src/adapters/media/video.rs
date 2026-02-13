//! Video adapter: capture stub, chunking, reassembly, and jitter buffer.

use std::collections::BTreeMap;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::trace;

use crate::application::ports::{CapturedFrame, MediaCapture, MediaPlayback, VideoChunkMeta};
use crate::domain::identity::PeerId;

// ---------------------------------------------------------------------------
// Video capture (stub)
// ---------------------------------------------------------------------------

/// Produces video frames (stub: generates dummy keyframes on a timer).
///
/// Replace internals with actual camera capture + encoder (e.g. libvpx / OpenH264).
pub struct VideoCaptureSource {
    timestamp: u32,
    frame_id: u32,
    /// Frames per second.
    fps: u32,
    /// Clock rate: 90 kHz for video.
    clock_rate: u32,
}

impl VideoCaptureSource {
    pub fn new(fps: u32) -> Self {
        Self {
            timestamp: 0,
            frame_id: 0,
            fps,
            clock_rate: 90_000,
        }
    }

    /// Chunk a single encoded frame into MTU-safe pieces.
    pub fn chunk_frame(
        frame_id: u32,
        is_keyframe: bool,
        encoded: &[u8],
        max_chunk_payload: usize,
    ) -> Vec<CapturedFrame> {
        let chunk_count = ((encoded.len() + max_chunk_payload - 1) / max_chunk_payload).max(1);
        let mut chunks = Vec::with_capacity(chunk_count);

        for (i, chunk_data) in encoded.chunks(max_chunk_payload).enumerate() {
            // Build the inner plaintext: frame_id(4) + chunk_index(2) + chunk_count(2) + is_keyframe(1) + data
            let mut payload =
                Vec::with_capacity(4 + 2 + 2 + 1 + chunk_data.len());
            payload.extend_from_slice(&frame_id.to_be_bytes());
            payload.extend_from_slice(&(i as u16).to_be_bytes());
            payload.extend_from_slice(&(chunk_count as u16).to_be_bytes());
            payload.push(if is_keyframe { 1 } else { 0 });
            payload.extend_from_slice(chunk_data);

            chunks.push(CapturedFrame {
                data: payload,
                is_audio: false,
                timestamp: 0, // filled by caller
                video_chunk: Some(VideoChunkMeta {
                    frame_id,
                    chunk_index: i as u16,
                    chunk_count: chunk_count as u16,
                    is_keyframe,
                }),
            });
        }

        chunks
    }
}

#[async_trait]
impl MediaCapture for VideoCaptureSource {
    async fn next_frame(&mut self) -> anyhow::Result<CapturedFrame> {
        // Wait for the frame interval
        let interval = std::time::Duration::from_millis(1000 / self.fps as u64);
        tokio::time::sleep(interval).await;

        // Generate a dummy frame (in real impl: capture + encode).
        let is_keyframe = self.frame_id % 30 == 0; // keyframe every ~2 seconds at 15fps
        let dummy_encoded = vec![0u8; 800]; // ~800 bytes dummy

        let ts = self.timestamp;
        self.timestamp = self
            .timestamp
            .wrapping_add(self.clock_rate / self.fps);
        let fid = self.frame_id;
        self.frame_id += 1;

        // For simplicity, return a single chunk (real impl: call chunk_frame for large frames)
        let mut payload = Vec::with_capacity(4 + 2 + 2 + 1 + dummy_encoded.len());
        payload.extend_from_slice(&fid.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes()); // chunk_index=0
        payload.extend_from_slice(&1u16.to_be_bytes()); // chunk_count=1
        payload.push(if is_keyframe { 1 } else { 0 });
        payload.extend_from_slice(&dummy_encoded);

        Ok(CapturedFrame {
            data: payload,
            is_audio: false,
            timestamp: ts,
            video_chunk: Some(VideoChunkMeta {
                frame_id: fid,
                chunk_index: 0,
                chunk_count: 1,
                is_keyframe,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Video reassembly + jitter buffer
// ---------------------------------------------------------------------------

/// Reassembles chunked video frames and provides a jitter buffer.
#[allow(dead_code)]
struct VideoFrame {
    frame_id: u32,
    chunk_count: u16,
    chunks_received: u16,
    is_keyframe: bool,
    /// chunk_index -> encoded bytes
    chunks: BTreeMap<u16, Vec<u8>>,
}

impl VideoFrame {
    fn is_complete(&self) -> bool {
        self.chunks_received == self.chunk_count
    }

    fn reassemble(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 0..self.chunk_count {
            if let Some(data) = self.chunks.get(&i) {
                out.extend_from_slice(data);
            }
        }
        out
    }
}

/// Per-sender video jitter buffer.
struct VideoJitterBuffer {
    /// frame_id -> VideoFrame
    frames: BTreeMap<u32, VideoFrame>,
    /// Max number of incomplete frames to buffer.
    max_pending: usize,
}

impl VideoJitterBuffer {
    fn new(max_pending: usize) -> Self {
        Self {
            frames: BTreeMap::new(),
            max_pending,
        }
    }

    fn push_chunk(
        &mut self,
        frame_id: u32,
        chunk_index: u16,
        chunk_count: u16,
        is_keyframe: bool,
        encoded_bytes: &[u8],
    ) -> Option<Vec<u8>> {
        let frame = self.frames.entry(frame_id).or_insert_with(|| VideoFrame {
            frame_id,
            chunk_count,
            chunks_received: 0,
            is_keyframe,
            chunks: BTreeMap::new(),
        });

        if !frame.chunks.contains_key(&chunk_index) {
            frame.chunks.insert(chunk_index, encoded_bytes.to_vec());
            frame.chunks_received += 1;
        }

        if frame.is_complete() {
            let data = frame.reassemble();
            self.frames.remove(&frame_id);
            Some(data)
        } else {
            // Evict old incomplete frames
            while self.frames.len() > self.max_pending {
                if let Some((&oldest, _)) = self.frames.iter().next() {
                    self.frames.remove(&oldest);
                }
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Video playback adapter
// ---------------------------------------------------------------------------

/// Video playback managing per-peer reassembly and jitter buffers.
pub struct VideoPlaybackAdapter {
    /// (peer_id, ssrc) -> jitter buffer
    buffers: Mutex<BTreeMap<(u64, u32), VideoJitterBuffer>>,
    max_pending_frames: usize,
}

impl VideoPlaybackAdapter {
    pub fn new(max_pending_frames: usize) -> Self {
        Self {
            buffers: Mutex::new(BTreeMap::new()),
            max_pending_frames,
        }
    }
}

#[async_trait]
impl MediaPlayback for VideoPlaybackAdapter {
    async fn push_audio(
        &self,
        _peer_id: PeerId,
        _ssrc: u32,
        _seq: u16,
        _timestamp: u32,
        _opus_frame: &[u8],
    ) -> anyhow::Result<()> {
        // Video adapter ignores audio.
        Ok(())
    }

    async fn push_video(
        &self,
        peer_id: PeerId,
        ssrc: u32,
        _seq: u16,
        _timestamp: u32,
        frame_id: u32,
        chunk_index: u16,
        chunk_count: u16,
        is_keyframe: bool,
        encoded_bytes: &[u8],
    ) -> anyhow::Result<()> {
        let mut buffers = self.buffers.lock().await;
        let buf = buffers
            .entry((peer_id.0, ssrc))
            .or_insert_with(|| VideoJitterBuffer::new(self.max_pending_frames));

        if let Some(complete_frame) = buf.push_chunk(
            frame_id,
            chunk_index,
            chunk_count,
            is_keyframe,
            encoded_bytes,
        ) {
            trace!(
                ?peer_id,
                ssrc,
                frame_id,
                bytes = complete_frame.len(),
                "Complete video frame ready for decode"
            );
            // TODO: decode and render
        }

        Ok(())
    }
}
