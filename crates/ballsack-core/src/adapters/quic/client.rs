//! Quinn-based QUIC client Transport adapter.
//!
//! Wraps a single QUIC connection with one bidirectional control stream and
//! DATAGRAM frames for media.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use quinn::{ClientConfig, Connection, Endpoint, RecvStream, SendStream};
use tokio::sync::Mutex;
use tracing::info;

use crate::application::ports::{ControlCodec, Transport};
use crate::domain::control::ControlMsg;

use super::codec::CborControlCodec;

// ---------------------------------------------------------------------------
// QuicClientTransport
// ---------------------------------------------------------------------------

/// Client-side [`Transport`] backed by Quinn.
pub struct QuicClientTransport {
    connection: Connection,
    control_send: Mutex<SendStream>,
    control_recv: Mutex<RecvStream>,
    codec: CborControlCodec,
}

impl QuicClientTransport {
    /// Connect to a server and open the control stream.
    pub async fn connect(
        server_addr: std::net::SocketAddr,
        server_name: &str,
    ) -> anyhow::Result<Arc<Self>> {
        // Build client endpoint (0.0.0.0:0 for ephemeral port)
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;

        // For the prototype, accept any server certificate (dangerous — dev only).
        let crypto = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth();

        let client_config = ClientConfig::new(Arc::new(
            quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?,
        ));
        endpoint.set_default_client_config(client_config);

        info!(%server_addr, "Connecting to QUIC server");
        let connection = endpoint.connect(server_addr, server_name)?.await?;
        info!("QUIC connection established");

        // Open a single bidirectional stream for the control plane.
        let (send, recv) = connection.open_bi().await?;
        info!("Control stream opened");

        Ok(Arc::new(Self {
            connection,
            control_send: Mutex::new(send),
            control_recv: Mutex::new(recv),
            codec: CborControlCodec,
        }))
    }
}

#[async_trait]
impl Transport for QuicClientTransport {
    async fn send_control(&self, msg: ControlMsg) -> anyhow::Result<()> {
        let payload = self.codec.encode(&msg)?;
        let len = (payload.len() as u32).to_be_bytes();

        let mut send = self.control_send.lock().await;
        send.write_all(&len).await?;
        send.write_all(&payload).await?;
        Ok(())
    }

    async fn recv_control(&self) -> anyhow::Result<ControlMsg> {
        let mut recv = self.control_recv.lock().await;

        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut payload = vec![0u8; len];
        recv.read_exact(&mut payload).await?;

        self.codec.decode(&payload)
    }

    async fn send_datagram(&self, data: Bytes) -> anyhow::Result<()> {
        self.connection.send_datagram(data)?;
        Ok(())
    }

    async fn recv_datagram(&self) -> anyhow::Result<Bytes> {
        let data = self.connection.read_datagram().await?;
        Ok(data)
    }
}

// ---------------------------------------------------------------------------
// Dev-only certificate verifier (skip verification)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}
