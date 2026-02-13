//! Standalone SFU relay server binary.

use std::sync::Arc;

use tracing::info;
use tracing_subscriber::EnvFilter;

use ballsack_core::adapters::quic::server::SfuServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let addr: std::net::SocketAddr = "0.0.0.0:4433".parse()?;
    info!(%addr, "SFU server starting");
    let server = Arc::new(SfuServer::new(addr)?);
    server.run().await
}
