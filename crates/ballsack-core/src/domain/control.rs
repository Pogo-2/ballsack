//! Control-plane message types (spec §3.1).
//!
//! These travel over the reliable QUIC bidirectional stream, encoded as CBOR.
//! Pure data — no I/O.

use serde::{Deserialize, Serialize};

use super::identity::{
    Ed25519PublicKey, PeerId, PeerInfo, RoomId, SenderKeyId, X25519PublicKey,
};

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

/// Top-level control message envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlEnvelope {
    pub msg_type: u16,
    /// Optional request id for request/response correlation.
    pub request_id: Option<u32>,
    pub payload: ControlMsg,
}

// ---------------------------------------------------------------------------
// Control message variants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMsg {
    // -- Handshake / auth --
    Hello {
        client_version: String,
        device_info: String,
    },
    Auth {
        token: String,
    },

    // -- Room membership --
    JoinRoom {
        room_id: RoomId,
        display_name: String,
        x25519_pub: X25519PublicKey,
        ed25519_pub: Ed25519PublicKey,
    },
    PeerList {
        peers: Vec<PeerInfo>,
    },
    PeerJoined {
        peer: PeerInfo,
    },
    PeerLeft {
        peer_id: PeerId,
    },

    // -- E2EE key management --
    KeyDistribute {
        room_id: RoomId,
        sender_id: PeerId,
        sender_key_id: SenderKeyId,
        /// Algorithm identifier, e.g. "ChaCha20Poly1305"
        algo: String,
        sealed_keys: Vec<SealedKeyEntry>,
        /// Optional Ed25519 signature over the message.
        signature: Option<Vec<u8>>,
    },
    KeyRotate {
        room_id: RoomId,
        sender_id: PeerId,
        new_sender_key_id: SenderKeyId,
        sealed_keys: Vec<SealedKeyEntry>,
        signature: Option<Vec<u8>>,
    },

    // -- Media control --
    VideoKeyframeRequest {
        target_sender_id: PeerId,
        ssrc: u32,
    },

    // -- Stats --
    StatsReport {
        rtt_ms: u32,
        loss: f32,
        jitter_ms: f32,
        bitrate_in: u64,
        bitrate_out: u64,
    },
}

// ---------------------------------------------------------------------------
// Sub-types
// ---------------------------------------------------------------------------

/// One entry inside `KeyDistribute.sealed_keys` / `KeyRotate.sealed_keys`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedKeyEntry {
    pub recipient_id: PeerId,
    /// First 8–32 bytes of the recipient X25519 public key (for mismatch detection).
    pub recipient_x25519_fingerprint: Vec<u8>,
    /// The sender secret, sealed (encrypted) to the recipient's X25519 public key.
    pub sealed_sender_secret: Vec<u8>,
}

// ---------------------------------------------------------------------------
// msg_type constants (mirrored for ControlEnvelope.msg_type)
// ---------------------------------------------------------------------------

impl ControlMsg {
    pub fn msg_type_id(&self) -> u16 {
        match self {
            Self::Hello { .. } => 0x01,
            Self::Auth { .. } => 0x02,
            Self::JoinRoom { .. } => 0x10,
            Self::PeerList { .. } => 0x11,
            Self::PeerJoined { .. } => 0x12,
            Self::PeerLeft { .. } => 0x13,
            Self::KeyDistribute { .. } => 0x20,
            Self::KeyRotate { .. } => 0x21,
            Self::VideoKeyframeRequest { .. } => 0x30,
            Self::StatsReport { .. } => 0x40,
        }
    }
}
