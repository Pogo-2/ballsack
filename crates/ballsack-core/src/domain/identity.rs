//! Domain identifiers and key-related value types.
//!
//! These are **pure data** — no I/O, no framework dependencies.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Unique room identifier (server-assigned).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomId(pub u64);

/// Unique peer identifier within the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub u64);

/// Identifies a particular sender key generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SenderKeyId(pub u32);

// ---------------------------------------------------------------------------
// Key material value objects
// ---------------------------------------------------------------------------

/// 32-byte sender secret used for AEAD media encryption.
#[derive(Clone, Serialize, Deserialize)]
pub struct SenderSecret(pub [u8; 32]);

impl std::fmt::Debug for SenderSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SenderSecret(***)")
    }
}

/// An Ed25519 public key (32 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ed25519PublicKey(pub [u8; 32]);

/// An X25519 public key (32 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X25519PublicKey(pub [u8; 32]);

/// SHA-256 fingerprint of (Ed25519_pub || X25519_pub) — used for safety-number UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyFingerprint(pub [u8; 32]);

// ---------------------------------------------------------------------------
// Peer descriptor (sent in PeerList / PeerJoined)
// ---------------------------------------------------------------------------

/// Information about a peer visible to other participants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub display_name: String,
    pub x25519_pub: X25519PublicKey,
    pub ed25519_pub: Ed25519PublicKey,
}

// ---------------------------------------------------------------------------
// Shared sender key (used to keep media send loop & control handler in sync)
// ---------------------------------------------------------------------------

/// Thread-safe shared holder for the current sender key.
/// Updated when keys rotate; read by the media send loop on every frame.
pub type SharedSenderKey = std::sync::Arc<tokio::sync::RwLock<(SenderKeyId, SenderSecret)>>;

/// Shared target bitrate in bits/sec, readable by the encoder, writable by the
/// bitrate adaptation controller.
pub type SharedBitrateTarget = std::sync::Arc<std::sync::atomic::AtomicI32>;
