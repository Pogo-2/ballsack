//! Ed25519 identity + X25519 key agreement + fingerprints.

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};

use crate::domain::identity::{Ed25519PublicKey, KeyFingerprint, X25519PublicKey};

/// Holds the local participant's long-lived identity keys.
pub struct IdentityKeyPair {
    pub ed25519_signing: SigningKey,
    pub ed25519_verifying: VerifyingKey,
    pub x25519_secret: X25519Secret,
    pub x25519_public: X25519Public,
}

impl IdentityKeyPair {
    /// Generate a fresh identity (random).
    pub fn generate() -> Self {
        let ed_signing = SigningKey::generate(&mut OsRng);
        let ed_verifying = ed_signing.verifying_key();

        let x_secret = X25519Secret::random_from_rng(OsRng);
        let x_public = X25519Public::from(&x_secret);

        Self {
            ed25519_signing: ed_signing,
            ed25519_verifying: ed_verifying,
            x25519_secret: x_secret,
            x25519_public: x_public,
        }
    }

    /// Domain-typed Ed25519 public key.
    pub fn ed25519_pub_domain(&self) -> Ed25519PublicKey {
        Ed25519PublicKey(self.ed25519_verifying.to_bytes())
    }

    /// Domain-typed X25519 public key.
    pub fn x25519_pub_domain(&self) -> X25519PublicKey {
        X25519PublicKey(self.x25519_public.to_bytes())
    }

    /// Compute a fingerprint: SHA256(Ed25519_pub || X25519_pub).
    pub fn compute_fingerprint(
        ed25519_pub: &Ed25519PublicKey,
        x25519_pub: &X25519PublicKey,
    ) -> KeyFingerprint {
        let mut hasher = Sha256::new();
        hasher.update(&ed25519_pub.0);
        hasher.update(&x25519_pub.0);
        let result = hasher.finalize();
        KeyFingerprint(result.into())
    }
}
