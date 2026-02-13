//! E2EE keystore adapter: sender key management + AEAD media encrypt/decrypt.
//!
//! Implements the [`E2eeKeystore`] port trait.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};

use crate::application::ports::E2eeKeystore;
use crate::domain::identity::{
    Ed25519PublicKey, KeyFingerprint, PeerId, SenderKeyId, SenderSecret, X25519PublicKey,
};
use crate::domain::media::MediaHeader;

use super::identity::IdentityKeyPair;

// ---------------------------------------------------------------------------
// Keystore implementation
// ---------------------------------------------------------------------------

/// Concrete [`E2eeKeystore`] backed by in-memory storage.
pub struct InMemoryE2eeKeystore {
    identity: IdentityKeyPair,
    /// Counter for generating sender key IDs.
    next_key_id: AtomicU32,
    /// Peer sender keys: (peer_id, sender_key_id) -> SenderSecret.
    /// Keeps last K keys per peer for handling reordered packets.
    peer_keys: Mutex<HashMap<(u64, u32), SenderSecret>>,
}

impl InMemoryE2eeKeystore {
    pub fn new(identity: IdentityKeyPair) -> Self {
        Self {
            identity,
            next_key_id: AtomicU32::new(1),
            peer_keys: Mutex::new(HashMap::new()),
        }
    }
}

impl E2eeKeystore for InMemoryE2eeKeystore {
    // -- Identity --

    fn our_ed25519_pub(&self) -> Ed25519PublicKey {
        self.identity.ed25519_pub_domain()
    }

    fn our_x25519_pub(&self) -> X25519PublicKey {
        self.identity.x25519_pub_domain()
    }

    fn fingerprint(
        &self,
        ed25519_pub: &Ed25519PublicKey,
        x25519_pub: &X25519PublicKey,
    ) -> KeyFingerprint {
        IdentityKeyPair::compute_fingerprint(ed25519_pub, x25519_pub)
    }

    // -- Sender key management --

    fn generate_sender_key(&self) -> (SenderKeyId, SenderSecret) {
        let id = SenderKeyId(self.next_key_id.fetch_add(1, Ordering::Relaxed));
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        (id, SenderSecret(secret))
    }

    fn seal_sender_secret(
        &self,
        recipient_x25519_pub: &X25519PublicKey,
        secret: &SenderSecret,
    ) -> anyhow::Result<Vec<u8>> {
        // Simple "sealed box": ephemeral X25519 + ChaCha20-Poly1305
        // 1. Generate ephemeral keypair
        let eph_secret = X25519Secret::random_from_rng(rand::thread_rng());
        let eph_public = X25519Public::from(&eph_secret);

        // 2. Derive shared secret
        let recipient_pub = X25519Public::from(recipient_x25519_pub.0);
        let shared = eph_secret.diffie_hellman(&recipient_pub);

        // 3. Derive a symmetric key from the shared secret
        let sym_key = Sha256::digest(shared.as_bytes());

        // 4. Encrypt the sender secret
        let cipher = ChaCha20Poly1305::new_from_slice(&sym_key)?;
        let nonce = Nonce::default(); // zeroed nonce is fine for a one-time ephemeral key
        let ciphertext = cipher
            .encrypt(&nonce, secret.0.as_slice())
            .map_err(|e| anyhow::anyhow!("AEAD seal error: {e}"))?;

        // 5. Prepend ephemeral public key (32 bytes) + ciphertext
        let mut sealed = Vec::with_capacity(32 + ciphertext.len());
        sealed.extend_from_slice(eph_public.as_bytes());
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    fn unseal_sender_secret(&self, sealed: &[u8]) -> anyhow::Result<SenderSecret> {
        if sealed.len() < 32 {
            anyhow::bail!("Sealed data too short");
        }

        // 1. Extract ephemeral public key
        let mut eph_pub_bytes = [0u8; 32];
        eph_pub_bytes.copy_from_slice(&sealed[..32]);
        let eph_public = X25519Public::from(eph_pub_bytes);

        // 2. Derive shared secret using our static X25519 secret
        let shared = self.identity.x25519_secret.diffie_hellman(&eph_public);

        // 3. Derive symmetric key
        let sym_key = Sha256::digest(shared.as_bytes());

        // 4. Decrypt
        let cipher = ChaCha20Poly1305::new_from_slice(&sym_key)?;
        let nonce = Nonce::default();
        let plaintext = cipher
            .decrypt(&nonce, &sealed[32..])
            .map_err(|e| anyhow::anyhow!("AEAD unseal error: {e}"))?;

        if plaintext.len() != 32 {
            anyhow::bail!("Unsealed secret has wrong length: {}", plaintext.len());
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&plaintext);
        Ok(SenderSecret(secret))
    }

    fn store_peer_sender_key(
        &self,
        peer_id: PeerId,
        key_id: SenderKeyId,
        secret: SenderSecret,
    ) {
        let mut keys = self.peer_keys.lock().unwrap();
        keys.insert((peer_id.0, key_id.0), secret);

        // Evict old keys if we keep too many per peer (keep last 5)
        let peer_entries: Vec<u32> = keys
            .keys()
            .filter(|(p, _)| *p == peer_id.0)
            .map(|(_, k)| *k)
            .collect();
        if peer_entries.len() > 5 {
            let mut sorted = peer_entries;
            sorted.sort();
            for old_kid in &sorted[..sorted.len() - 5] {
                keys.remove(&(peer_id.0, *old_kid));
            }
        }
    }

    fn get_peer_sender_key(
        &self,
        peer_id: PeerId,
        key_id: SenderKeyId,
    ) -> Option<SenderSecret> {
        let keys = self.peer_keys.lock().unwrap();
        keys.get(&(peer_id.0, key_id.0)).cloned()
    }

    // -- Media AEAD --

    fn encrypt_media(
        &self,
        secret: &SenderSecret,
        header: &MediaHeader,
        plaintext: &[u8],
    ) -> anyhow::Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(&secret.0)?;
        let nonce = derive_media_nonce(header);
        let aad = header.to_bytes();

        // Use encrypt_in_place_detached for AAD support isn't directly available
        // in the simple API; use the lower-level approach.
        use chacha20poly1305::aead::Payload;
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|e| anyhow::anyhow!("Media encrypt error: {e}"))?;

        Ok(ciphertext)
    }

    fn decrypt_media(
        &self,
        secret: &SenderSecret,
        header: &MediaHeader,
        ciphertext: &[u8],
    ) -> anyhow::Result<Vec<u8>> {
        let cipher = ChaCha20Poly1305::new_from_slice(&secret.0)?;
        let nonce = derive_media_nonce(header);
        let aad = header.to_bytes();

        use chacha20poly1305::aead::Payload;
        let plaintext = cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|e| anyhow::anyhow!("Media decrypt error: {e}"))?;

        Ok(plaintext)
    }
}

// ---------------------------------------------------------------------------
// Nonce derivation
// ---------------------------------------------------------------------------

/// Derive a 96-bit (12-byte) nonce from header fields:
/// `nonce = SHA256(room_id || sender_id || ssrc || seq || timestamp || nonce_salt)[..12]`
fn derive_media_nonce(header: &MediaHeader) -> Nonce {
    let mut hasher = Sha256::new();
    hasher.update(header.room_id.0.to_be_bytes());
    hasher.update(header.sender_id.0.to_be_bytes());
    hasher.update(header.ssrc.to_be_bytes());
    hasher.update(header.seq.to_be_bytes());
    hasher.update(header.timestamp.to_be_bytes());
    hasher.update(header.nonce_salt.to_be_bytes());
    let hash = hasher.finalize();

    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&hash[..12]);
    Nonce::from(nonce_bytes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_unseal_round_trip() {
        let alice = IdentityKeyPair::generate();
        let bob = IdentityKeyPair::generate();

        let alice_store = InMemoryE2eeKeystore::new(alice);
        let bob_store = InMemoryE2eeKeystore::new(bob);

        let (_key_id, secret) = alice_store.generate_sender_key();

        let sealed = alice_store
            .seal_sender_secret(&bob_store.our_x25519_pub(), &secret)
            .unwrap();

        let unsealed = bob_store.unseal_sender_secret(&sealed).unwrap();
        assert_eq!(secret.0, unsealed.0);
    }

    #[test]
    fn media_encrypt_decrypt_round_trip() {
        use crate::domain::identity::{PeerId, RoomId};
        use crate::domain::media::{MediaKind, MEDIA_MAGIC, MEDIA_VERSION};

        let identity = IdentityKeyPair::generate();
        let store = InMemoryE2eeKeystore::new(identity);

        let (key_id, secret) = store.generate_sender_key();

        let header = MediaHeader {
            magic: MEDIA_MAGIC,
            version: MEDIA_VERSION,
            room_id: RoomId(42),
            sender_id: PeerId(1),
            kind: MediaKind::Audio,
            ssrc: 100,
            seq: 0,
            timestamp: 960,
            sender_key_id: key_id,
            nonce_salt: 0x1234,
        };

        let plaintext = b"hello opus frame";
        let ciphertext = store.encrypt_media(&secret, &header, plaintext).unwrap();
        let decrypted = store.decrypt_media(&secret, &header, &ciphertext).unwrap();
        assert_eq!(plaintext.as_slice(), decrypted.as_slice());
    }
}
