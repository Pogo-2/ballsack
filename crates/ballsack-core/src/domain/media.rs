//! Media-plane types: datagram header and payload structures (spec §3.2).
//!
//! Pure data — no I/O.

use serde::{Deserialize, Serialize};

use super::identity::{PeerId, RoomId, SenderKeyId};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Magic bytes at the start of every media datagram.
pub const MEDIA_MAGIC: u16 = 0xBEEF;

/// Current wire-format version.
pub const MEDIA_VERSION: u8 = 1;

// ---------------------------------------------------------------------------
// Media kind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MediaKind {
    Audio = 1,
    Video = 2,
}

impl TryFrom<u8> for MediaKind {
    type Error = u8;
    fn try_from(v: u8) -> Result<Self, u8> {
        match v {
            1 => Ok(Self::Audio),
            2 => Ok(Self::Video),
            other => Err(other),
        }
    }
}

// ---------------------------------------------------------------------------
// MediaHeader (plaintext to the server / SFU)
// ---------------------------------------------------------------------------

/// Fixed-layout header prepended to every media datagram.
///
/// The server can read this to route/pace without decrypting the ciphertext.
#[derive(Debug, Clone)]
pub struct MediaHeader {
    pub magic: u16,
    pub version: u8,
    pub room_id: RoomId,
    pub sender_id: PeerId,
    pub kind: MediaKind,
    /// Stream identifier (allows multiple audio/video tracks per sender).
    pub ssrc: u32,
    /// Per-ssrc monotonic sequence number.
    pub seq: u16,
    /// Timestamp: 48 kHz ticks for audio, 90 kHz ticks for video.
    pub timestamp: u32,
    /// Which sender key to use for decryption.
    pub sender_key_id: SenderKeyId,
    /// Extra entropy folded into the AEAD nonce.
    pub nonce_salt: u16,
}

/// Fixed size of the serialized header in bytes (no ciphertext length yet).
pub const MEDIA_HEADER_SIZE: usize =
    2  // magic
    + 1  // version
    + 8  // room_id
    + 8  // sender_id
    + 1  // kind
    + 4  // ssrc
    + 2  // seq
    + 4  // timestamp
    + 4  // sender_key_id
    + 2; // nonce_salt
    // = 36 bytes

impl MediaHeader {
    /// Serialize the header to bytes (big-endian, fixed layout).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(MEDIA_HEADER_SIZE + 2); // +2 for ciphertext_len added later
        buf.extend_from_slice(&self.magic.to_be_bytes());
        buf.push(self.version);
        buf.extend_from_slice(&self.room_id.0.to_be_bytes());
        buf.extend_from_slice(&self.sender_id.0.to_be_bytes());
        buf.push(self.kind as u8);
        buf.extend_from_slice(&self.ssrc.to_be_bytes());
        buf.extend_from_slice(&self.seq.to_be_bytes());
        buf.extend_from_slice(&self.timestamp.to_be_bytes());
        buf.extend_from_slice(&self.sender_key_id.0.to_be_bytes());
        buf.extend_from_slice(&self.nonce_salt.to_be_bytes());
        buf
    }

    /// Parse a header from a byte slice. Returns the header and remaining bytes.
    pub fn from_bytes(data: &[u8]) -> Result<(Self, &[u8]), MediaParseError> {
        if data.len() < MEDIA_HEADER_SIZE + 2 {
            return Err(MediaParseError::TooShort);
        }
        let mut cursor = 0usize;

        let magic = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
        cursor += 2;
        if magic != MEDIA_MAGIC {
            return Err(MediaParseError::BadMagic(magic));
        }

        let version = data[cursor];
        cursor += 1;

        let room_id = RoomId(u64::from_be_bytes(
            data[cursor..cursor + 8].try_into().unwrap(),
        ));
        cursor += 8;

        let sender_id = PeerId(u64::from_be_bytes(
            data[cursor..cursor + 8].try_into().unwrap(),
        ));
        cursor += 8;

        let kind = MediaKind::try_from(data[cursor])
            .map_err(MediaParseError::UnknownKind)?;
        cursor += 1;

        let ssrc = u32::from_be_bytes(data[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;

        let seq = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
        cursor += 2;

        let timestamp = u32::from_be_bytes(data[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;

        let sender_key_id = SenderKeyId(u32::from_be_bytes(
            data[cursor..cursor + 4].try_into().unwrap(),
        ));
        cursor += 4;

        let nonce_salt = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
        cursor += 2;

        // ciphertext_len (u16) + ciphertext follow
        let ct_len = u16::from_be_bytes([data[cursor], data[cursor + 1]]) as usize;
        cursor += 2;

        if data.len() < cursor + ct_len {
            return Err(MediaParseError::TooShort);
        }

        let remaining = &data[cursor..cursor + ct_len];

        Ok((
            Self {
                magic,
                version,
                room_id,
                sender_id,
                kind,
                ssrc,
                seq,
                timestamp,
                sender_key_id,
                nonce_salt,
            },
            remaining,
        ))
    }
}

// ---------------------------------------------------------------------------
// Plaintext payload types (inside ciphertext)
// ---------------------------------------------------------------------------

/// Audio payload: raw Opus frame bytes.
#[derive(Debug, Clone)]
pub struct AudioPayload {
    pub opus_frame: Vec<u8>,
}

/// A single chunk of a video frame.
#[derive(Debug, Clone)]
pub struct VideoChunk {
    pub frame_id: u32,
    pub chunk_index: u16,
    pub chunk_count: u16,
    pub is_keyframe: bool,
    pub encoded_bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum MediaParseError {
    #[error("datagram too short to contain header")]
    TooShort,
    #[error("bad magic: expected 0xBEEF, got 0x{0:04X}")]
    BadMagic(u16),
    #[error("unknown media kind: {0}")]
    UnknownKind(u8),
}
