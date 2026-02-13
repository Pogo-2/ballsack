//! Port traits (interfaces) that use cases depend on.
//!
//! Adapters implement these traits; use cases never reference Quinn, cpal, etc.

use async_trait::async_trait;
use bytes::Bytes;

use crate::domain::control::ControlMsg;
use crate::domain::identity::{
    Ed25519PublicKey, KeyFingerprint, PeerId, PeerInfo, SenderKeyId, SenderSecret,
    X25519PublicKey,
};
use crate::domain::media::MediaHeader;

// ---------------------------------------------------------------------------
// Transport (QUIC abstraction)
// ---------------------------------------------------------------------------

/// Abstracts one QUIC connection: a reliable control stream + unreliable datagrams.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Send a control message over the reliable bidirectional stream.
    async fn send_control(&self, msg: ControlMsg) -> anyhow::Result<()>;

    /// Receive the next control message from the reliable stream.
    async fn recv_control(&self) -> anyhow::Result<ControlMsg>;

    /// Send a raw media datagram (header + ciphertext, already serialized).
    async fn send_datagram(&self, data: Bytes) -> anyhow::Result<()>;

    /// Receive the next raw media datagram.
    async fn recv_datagram(&self) -> anyhow::Result<Bytes>;

    /// Actively close the connection. Background tasks that are blocked on
    /// `recv_control` / `recv_datagram` will get an error and exit.
    fn close(&self);
}

// ---------------------------------------------------------------------------
// ControlCodec (serialization)
// ---------------------------------------------------------------------------

/// Encodes / decodes control messages to/from bytes (e.g. CBOR).
pub trait ControlCodec: Send + Sync {
    fn encode(&self, msg: &ControlMsg) -> anyhow::Result<Vec<u8>>;
    fn decode(&self, data: &[u8]) -> anyhow::Result<ControlMsg>;
}

// ---------------------------------------------------------------------------
// E2EE key store + media crypto
// ---------------------------------------------------------------------------

/// Manages identity keys, sender keys, sealing/unsealing, and media AEAD.
#[async_trait]
pub trait E2eeKeystore: Send + Sync {
    // -- Identity --

    /// Our own Ed25519 public key.
    fn our_ed25519_pub(&self) -> Ed25519PublicKey;
    /// Our own X25519 public key.
    fn our_x25519_pub(&self) -> X25519PublicKey;
    /// Compute fingerprint for a peer's public keys.
    fn fingerprint(
        &self,
        ed25519_pub: &Ed25519PublicKey,
        x25519_pub: &X25519PublicKey,
    ) -> KeyFingerprint;

    // -- Sender key management --

    /// Generate a fresh sender key, returning (id, secret).
    fn generate_sender_key(&self) -> (SenderKeyId, SenderSecret);

    /// Seal (encrypt) a sender secret for a specific recipient's X25519 public key.
    fn seal_sender_secret(
        &self,
        recipient_x25519_pub: &X25519PublicKey,
        secret: &SenderSecret,
    ) -> anyhow::Result<Vec<u8>>;

    /// Unseal (decrypt) a sender secret that was sealed for us.
    fn unseal_sender_secret(&self, sealed: &[u8]) -> anyhow::Result<SenderSecret>;

    /// Store a received sender key for a remote peer.
    fn store_peer_sender_key(
        &self,
        peer_id: PeerId,
        key_id: SenderKeyId,
        secret: SenderSecret,
    );

    /// Retrieve the sender key for a remote peer (supports keeping last K keys).
    fn get_peer_sender_key(
        &self,
        peer_id: PeerId,
        key_id: SenderKeyId,
    ) -> Option<SenderSecret>;

    // -- Media AEAD --

    /// Encrypt a media payload, returning ciphertext (includes AEAD tag).
    fn encrypt_media(
        &self,
        secret: &SenderSecret,
        header: &MediaHeader,
        plaintext: &[u8],
    ) -> anyhow::Result<Vec<u8>>;

    /// Decrypt a media payload ciphertext.
    fn decrypt_media(
        &self,
        secret: &SenderSecret,
        header: &MediaHeader,
        ciphertext: &[u8],
    ) -> anyhow::Result<Vec<u8>>;
}

// ---------------------------------------------------------------------------
// Media capture & playback
// ---------------------------------------------------------------------------

/// Produces encoded audio/video frames from capture devices.
#[async_trait]
pub trait MediaCapture: Send {
    /// Grab the next encoded frame. Returns (payload_bytes, is_audio).
    async fn next_frame(&mut self) -> anyhow::Result<CapturedFrame>;
}

/// A captured and encoded media frame ready for encryption + send.
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    /// Raw encoded bytes (Opus frame or video chunk payload).
    pub data: Vec<u8>,
    /// True if audio, false if video.
    pub is_audio: bool,
    /// Timestamp in codec ticks (48 kHz audio / 90 kHz video).
    pub timestamp: u32,
    /// For video: optional chunking metadata.
    pub video_chunk: Option<VideoChunkMeta>,
}

#[derive(Debug, Clone)]
pub struct VideoChunkMeta {
    pub frame_id: u32,
    pub chunk_index: u16,
    pub chunk_count: u16,
    pub is_keyframe: bool,
}

/// Accepts decrypted media payloads for jitter-buffering, decoding, and playout.
#[async_trait]
pub trait MediaPlayback: Send + Sync {
    /// Push a decrypted audio payload for playout.
    async fn push_audio(
        &self,
        peer_id: PeerId,
        ssrc: u32,
        seq: u16,
        timestamp: u32,
        opus_frame: &[u8],
    ) -> anyhow::Result<()>;

    /// Push a decrypted video chunk for reassembly and playout.
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
    ) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// Room state (in-memory peer tracking)
// ---------------------------------------------------------------------------

/// Tracks current peers in a room (client-side view).
pub trait RoomState: Send + Sync {
    fn peers(&self) -> Vec<PeerInfo>;
    fn add_peer(&self, peer: PeerInfo);
    fn remove_peer(&self, peer_id: PeerId);
    fn get_peer(&self, peer_id: PeerId) -> Option<PeerInfo>;
}

// ---------------------------------------------------------------------------
// App events (UI bridge)
// ---------------------------------------------------------------------------

/// Emits events toward the UI layer (Tauri or otherwise).
#[async_trait]
pub trait AppEvents: Send + Sync {
    async fn emit_peer_joined(&self, peer: &PeerInfo);
    async fn emit_peer_left(&self, peer_id: PeerId);
    async fn emit_stats(&self, rtt_ms: u32, loss: f32, jitter_ms: f32);
}
