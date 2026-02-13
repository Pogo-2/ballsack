//! Bitrate adaptation use case — AIMD algorithm driven by QUIC RTT and
//! receiver reports.
//!
//! Runs a 1-second tick loop that:
//! 1. Reads the current QUIC RTT from the transport.
//! 2. Reads the latest receiver report from a channel.
//! 3. Applies Additive Increase / Multiplicative Decrease (AIMD).
//! 4. Stores the new target in a [`SharedBitrateTarget`].

use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::domain::identity::SharedBitrateTarget;

use super::ports::Transport;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Minimum Opus bitrate (bits/sec).
const MIN_BITRATE: i32 = 16_000;
/// Maximum Opus bitrate (bits/sec).
const MAX_BITRATE: i32 = 96_000;
/// Default / initial bitrate (bits/sec).
pub const DEFAULT_BITRATE: i32 = 64_000;
/// Additive increase per tick when conditions are good (bps).
const INCREASE_STEP: i32 = 4_000;
/// Multiplicative decrease factor when conditions are bad.
const DECREASE_FACTOR: f32 = 0.5;

/// RTT threshold below which conditions are considered good (ms).
const RTT_GOOD_MS: u32 = 150;
/// RTT threshold above which we trigger a decrease (ms).
const RTT_BAD_MS: u32 = 200;
/// Loss fraction below which conditions are considered good.
const LOSS_GOOD: f32 = 0.02;
/// Loss fraction above which we trigger a decrease.
const LOSS_BAD: f32 = 0.05;

// ---------------------------------------------------------------------------
// Receiver report payload (sent via mpsc from the control handler)
// ---------------------------------------------------------------------------

/// Data extracted from an incoming `ReceiverReport` control message.
#[derive(Debug, Clone)]
pub struct ReceiverReportData {
    pub loss_fraction: f32,
    pub jitter_ms: f32,
    pub buffer_depth: u16,
}

// ---------------------------------------------------------------------------
// Use case
// ---------------------------------------------------------------------------

pub struct BitrateAdaptUseCase {
    transport: Arc<dyn Transport>,
    target: SharedBitrateTarget,
    reports_rx: mpsc::Receiver<ReceiverReportData>,
}

impl BitrateAdaptUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        target: SharedBitrateTarget,
        reports_rx: mpsc::Receiver<ReceiverReportData>,
    ) -> Self {
        Self {
            transport,
            target,
            reports_rx,
        }
    }

    /// Run the adaptation loop (1-second ticks).
    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Latest report (may be stale; replaced each time a new one arrives).
        let mut latest_report: Option<ReceiverReportData> = None;

        loop {
            ticker.tick().await;

            // Drain channel — keep only the most recent report.
            while let Ok(report) = self.reports_rx.try_recv() {
                latest_report = Some(report);
            }

            // Read QUIC RTT.
            let rtt_ms = self
                .transport
                .rtt()
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0);

            let loss = latest_report
                .as_ref()
                .map(|r| r.loss_fraction)
                .unwrap_or(0.0);

            let current = self.target.load(Ordering::Relaxed);
            let new_bitrate;

            // AIMD decision
            if rtt_ms > RTT_BAD_MS || loss > LOSS_BAD {
                // Multiplicative decrease
                new_bitrate = ((current as f32) * DECREASE_FACTOR) as i32;
                debug!(
                    rtt_ms,
                    loss,
                    current,
                    new_bitrate,
                    "Bitrate DECREASE (congestion)"
                );
            } else if rtt_ms < RTT_GOOD_MS && loss < LOSS_GOOD {
                // Additive increase
                new_bitrate = current + INCREASE_STEP;
                debug!(
                    rtt_ms,
                    loss,
                    current,
                    new_bitrate,
                    "Bitrate INCREASE (good conditions)"
                );
            } else {
                // Hold steady
                new_bitrate = current;
            }

            let clamped = new_bitrate.clamp(MIN_BITRATE, MAX_BITRATE);
            if clamped != current {
                self.target.store(clamped, Ordering::Relaxed);
                info!(
                    from = current,
                    to = clamped,
                    rtt_ms,
                    loss,
                    "Bitrate target updated"
                );
            }
        }
    }
}
