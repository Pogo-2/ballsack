//! ReceiveMedia use case — receives datagrams, decrypts, and pushes to playback.

use std::sync::Arc;

use tracing::{trace, warn};

use crate::domain::media::{MediaHeader, MediaKind};

use super::ports::{E2eeKeystore, MediaPlayback, Transport};

/// Receives media datagrams, decrypts, and feeds into playback buffers.
pub struct ReceiveMediaUseCase {
    transport: Arc<dyn Transport>,
    keystore: Arc<dyn E2eeKeystore>,
    playback: Arc<dyn MediaPlayback>,
}

impl ReceiveMediaUseCase {
    pub fn new(
        transport: Arc<dyn Transport>,
        keystore: Arc<dyn E2eeKeystore>,
        playback: Arc<dyn MediaPlayback>,
    ) -> Self {
        Self {
            transport,
            keystore,
            playback,
        }
    }

    /// Run the receive loop forever.
    pub async fn run(&self) -> anyhow::Result<()> {
        loop {
            let datagram = self.transport.recv_datagram().await?;
            if let Err(e) = self.process_datagram(&datagram).await {
                warn!("Failed to process datagram: {e}");
            }
        }
    }

    async fn process_datagram(&self, data: &[u8]) -> anyhow::Result<()> {
        let (header, ciphertext) = MediaHeader::from_bytes(data)?;

        // Look up sender key
        let secret = self
            .keystore
            .get_peer_sender_key(header.sender_id, header.sender_key_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No sender key for peer {:?} key_id {:?}",
                    header.sender_id,
                    header.sender_key_id
                )
            })?;

        // Decrypt
        let plaintext = self
            .keystore
            .decrypt_media(&secret, &header, ciphertext)?;

        // Dispatch to playback
        match header.kind {
            MediaKind::Audio => {
                self.playback
                    .push_audio(
                        header.sender_id,
                        header.ssrc,
                        header.seq,
                        header.timestamp,
                        &plaintext,
                    )
                    .await?;
            }
            MediaKind::Video => {
                // For video, the plaintext contains chunk metadata + encoded bytes.
                // Parse the chunk header:
                //   frame_id: u32, chunk_index: u16, chunk_count: u16, is_keyframe: u8
                if plaintext.len() < 9 {
                    anyhow::bail!("Video plaintext too short for chunk header");
                }
                let frame_id = u32::from_be_bytes(plaintext[0..4].try_into()?);
                let chunk_index = u16::from_be_bytes(plaintext[4..6].try_into()?);
                let chunk_count = u16::from_be_bytes(plaintext[6..8].try_into()?);
                let is_keyframe = plaintext[8] != 0;
                let encoded_bytes = &plaintext[9..];

                self.playback
                    .push_video(
                        header.sender_id,
                        header.ssrc,
                        header.seq,
                        header.timestamp,
                        frame_id,
                        chunk_index,
                        chunk_count,
                        is_keyframe,
                        encoded_bytes,
                    )
                    .await?;
            }
        }

        trace!(
            peer = ?header.sender_id,
            kind = ?header.kind,
            seq = header.seq,
            "Received & decrypted media"
        );

        Ok(())
    }
}
