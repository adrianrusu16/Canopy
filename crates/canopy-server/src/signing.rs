//! HMAC-SHA256 implementation of the [`UrlSigner`] port.
//!
//! The signature binds the object key to its expiry, so a presented URL cannot
//! be replayed against a different object or have its expiry extended. Object
//! storage (RustFS in production) can recompute and compare the signature with
//! only the shared secret, making validation stateless on the streaming hot
//! path.

use canopy_core::UrlSigner;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Signs presigned URLs with HMAC-SHA256 over `storage_key` and expiry.
#[derive(Clone)]
pub struct HmacUrlSigner {
    secret: Vec<u8>,
}

impl HmacUrlSigner {
    /// Creates a signer from the shared secret key (any length is accepted).
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// Computes the lowercase-hex HMAC of the canonical signing string.
    fn compute(&self, storage_key: &str, expires_at_epoch_ms: u64) -> String {
        // `new_from_slice` only errors on key lengths HMAC cannot accept, and
        // HMAC accepts keys of any length, so this never fails in practice.
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts keys of any length");
        // A newline separator keeps the two fields unambiguous so that
        // (`ab`, `c`) and (`a`, `bc`) cannot collide.
        mac.update(storage_key.as_bytes());
        mac.update(b"\n");
        mac.update(expires_at_epoch_ms.to_string().as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

impl UrlSigner for HmacUrlSigner {
    fn sign(&self, storage_key: &str, expires_at_epoch_ms: u64) -> String {
        self.compute(storage_key, expires_at_epoch_ms)
    }

    fn verify(
        &self,
        storage_key: &str,
        expires_at_epoch_ms: u64,
        signature: &str,
        now_epoch_ms: u64,
    ) -> bool {
        if now_epoch_ms > expires_at_epoch_ms {
            return false;
        }
        let expected = self.compute(storage_key, expires_at_epoch_ms);
        constant_time_eq(expected.as_bytes(), signature.as_bytes())
    }
}

/// Compares two byte slices in time independent of the position of the first
/// differing byte, to avoid leaking the signature through timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_is_deterministic_and_input_dependent() {
        let signer = HmacUrlSigner::new("secret");
        let a = signer.sign("audio/tracks/trk_1.mp3", 1_000);
        assert_eq!(a, signer.sign("audio/tracks/trk_1.mp3", 1_000));
        // Changing either field changes the signature.
        assert_ne!(a, signer.sign("audio/tracks/trk_2.mp3", 1_000));
        assert_ne!(a, signer.sign("audio/tracks/trk_1.mp3", 1_001));
    }

    #[test]
    fn verify_accepts_valid_unexpired_signature() {
        let signer = HmacUrlSigner::new("secret");
        let sig = signer.sign("k", 5_000);
        assert!(signer.verify("k", 5_000, &sig, 4_000));
    }

    #[test]
    fn verify_rejects_expired_url() {
        let signer = HmacUrlSigner::new("secret");
        let sig = signer.sign("k", 5_000);
        assert!(!signer.verify("k", 5_000, &sig, 5_001));
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let signer = HmacUrlSigner::new("secret");
        assert!(!signer.verify("k", 5_000, "deadbeef", 1_000));
        // A signature minted with a different key must not validate.
        let other = HmacUrlSigner::new("other-secret");
        let forged = other.sign("k", 5_000);
        assert!(!signer.verify("k", 5_000, &forged, 1_000));
    }
}
