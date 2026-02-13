//! Stats use case — collects and reports call quality statistics.

use std::sync::Arc;
use std::time::Duration;

use tracing::debug;

use crate::domain::control::ControlMsg;

use super::ports::{AppEvents, Transport};

/// Periodically sends stats reports and exposes them to the UI.
pub struct StatsUseCase {
    transport: Arc<dyn Transport>,
    app_events: Arc<dyn AppEvents>,
    interval: Duration,
}

impl StatsUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        app_events: Arc<dyn AppEvents>,
        interval: Duration,
    ) -> Self {
        Self {
            transport,
            app_events,
            interval,
        }
    }

    /// Run the periodic stats report loop.
    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            tokio::time::sleep(self.interval).await;

            // In a real implementation, these would come from transport-level metrics.
            let rtt_ms = 0u32; // TODO: read from transport
            let loss = 0.0f32;
            let jitter_ms = 0.0f32;
            let bitrate_in = 0u64;
            let bitrate_out = 0u64;

            debug!(rtt_ms, loss, jitter_ms, "Reporting stats");

            // Send to server
            self.transport
                .send_control(ControlMsg::StatsReport {
                    rtt_ms,
                    loss,
                    jitter_ms,
                    bitrate_in,
                    bitrate_out,
                })
                .await?;

            // Emit to UI
            self.app_events.emit_stats(rtt_ms, loss, jitter_ms).await;
        }
    }
}
