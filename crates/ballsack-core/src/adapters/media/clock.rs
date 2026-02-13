//! Playout clock for A/V sync.
//!
//! Provides a monotonic reference clock and per-sender offset estimation.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::domain::identity::PeerId;

/// Audio clock rate: 48 kHz.
pub const AUDIO_CLOCK_RATE: u32 = 48_000;

/// Video clock rate: 90 kHz.
pub const VIDEO_CLOCK_RATE: u32 = 90_000;

/// Audio frame duration at 48 kHz for 20 ms frames.
pub const AUDIO_FRAME_TICKS: u32 = AUDIO_CLOCK_RATE / 50; // 960

/// Manages playout timing.
pub struct PlayoutClock {
    epoch: Instant,
    /// Per-sender offset: maps peer_id to estimated offset in microseconds
    /// between the sender's timestamp domain and our local clock.
    offsets: Mutex<HashMap<u64, i64>>,
}

impl PlayoutClock {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            offsets: Mutex::new(HashMap::new()),
        }
    }

    /// Local elapsed time in microseconds since the clock was created.
    pub fn now_us(&self) -> i64 {
        self.epoch.elapsed().as_micros() as i64
    }

    /// Convert a sender's RTP-style timestamp to local microseconds.
    ///
    /// `clock_rate` is 48000 for audio or 90000 for video.
    pub fn sender_ts_to_local_us(
        &self,
        peer_id: PeerId,
        sender_ts: u32,
        clock_rate: u32,
    ) -> i64 {
        let sender_us = (sender_ts as i64) * 1_000_000 / (clock_rate as i64);
        let offsets = self.offsets.lock().unwrap();
        let offset = offsets.get(&peer_id.0).copied().unwrap_or(0);
        sender_us + offset
    }

    /// Update the offset estimate for a sender.
    ///
    /// Call this when you first receive from a new sender, passing the sender
    /// timestamp and the local arrival time.
    pub fn update_offset(
        &self,
        peer_id: PeerId,
        sender_ts: u32,
        clock_rate: u32,
        arrival_local_us: i64,
    ) {
        let sender_us = (sender_ts as i64) * 1_000_000 / (clock_rate as i64);
        let offset = arrival_local_us - sender_us;
        let mut offsets = self.offsets.lock().unwrap();
        offsets.insert(peer_id.0, offset);
    }
}

impl Default for PlayoutClock {
    fn default() -> Self {
        Self::new()
    }
}
