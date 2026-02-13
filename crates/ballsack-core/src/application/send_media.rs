//! SendMedia use case — captures, encrypts, and sends media datagrams.

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use bytes::Bytes;
use tracing::trace;

use crate::domain::identity::{PeerId, RoomId, SenderKeyId, SenderSecret};
use crate::domain::media::{MediaHeader, MediaKind, MEDIA_MAGIC, MEDIA_VERSION};

use super::ports::{E2eeKeystore, MediaCapture, Transport};

/// Reads from a capture source, encrypts, and sends datagrams.
pub struct SendMediaUseCase {
    transport: Arc<dyn Transport>,
    keystore: Arc<dyn E2eeKeystore>,
    room_id: RoomId,
    our_peer_id: PeerId,
    audio_ssrc: u32,
    video_ssrc: u32,
    audio_seq: AtomicU16,
    video_seq: AtomicU16,
}

impl SendMediaUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        keystore: Arc<dyn E2eeKeystore>,
        room_id: RoomId,
        our_peer_id: PeerId,
        audio_ssrc: u32,
        video_ssrc: u32,
    ) -> Self {
        Self {
            transport,
            keystore,
            room_id,
            our_peer_id,
            audio_ssrc,
            video_ssrc,
            audio_seq: AtomicU16::new(0),
            video_seq: AtomicU16::new(0),
        }
    }

    /// Run the send loop: pull frames from capture, encrypt, send.
    pub async fn run(
        &self,
        capture: &mut dyn MediaCapture,
        sender_key_id: SenderKeyId,
        sender_secret: SenderSecret,
    ) -> anyhow::Result<()> {
        loop {
            let frame = capture.next_frame().await?;

            let (kind, ssrc, seq) = if frame.is_audio {
                let seq = self.audio_seq.fetch_add(1, Ordering::Relaxed);
                (MediaKind::Audio, self.audio_ssrc, seq)
            } else {
                let seq = self.video_seq.fetch_add(1, Ordering::Relaxed);
                (MediaKind::Video, self.video_ssrc, seq)
            };

            let nonce_salt: u16 = rand::random();

            let header = MediaHeader {
                magic: MEDIA_MAGIC,
                version: MEDIA_VERSION,
                room_id: self.room_id,
                sender_id: self.our_peer_id,
                kind,
                ssrc,
                seq,
                timestamp: frame.timestamp,
                sender_key_id,
                nonce_salt,
            };

            let ciphertext =
                self.keystore
                    .encrypt_media(&sender_secret, &header, &frame.data)?;

            // Build final datagram: header bytes + ciphertext_len(u16) + ciphertext
            let mut datagram = header.to_bytes();
            datagram.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
            datagram.extend_from_slice(&ciphertext);

            self.transport
                .send_datagram(Bytes::from(datagram))
                .await?;

            trace!(?kind, seq, ts = frame.timestamp, ct_len = ciphertext.len(), "Sent media");
        }
    }
}
