//! Presigned-URL signing port.
//!
//! The playback resolver embeds a signature and an expiry directly into the
//! stream URL so that object storage can validate an in-flight request
//! statelessly — no database lookup in the hot path. This trait abstracts the
//! signing scheme (HMAC today) from the resolver so the algorithm can evolve
//! without touching domain logic.

/// Stateless signer for presigned object-storage URLs.
pub trait UrlSigner: Send + Sync {
    /// Returns the signature binding `object_key` to its `expires_at_epoch_ms`.
    ///
    /// The signature must depend on both inputs so a URL cannot be replayed
    /// against a different object or have its expiry tampered with.
    fn sign(&self, object_key: &str, expires_at_epoch_ms: u64) -> String;

    /// Validates a presented `signature` for `object_key`/`expires_at_epoch_ms`.
    ///
    /// Returns `true` only when the signature matches and the URL has not yet
    /// expired relative to `now_epoch_ms`.
    fn verify(
        &self,
        object_key: &str,
        expires_at_epoch_ms: u64,
        signature: &str,
        now_epoch_ms: u64,
    ) -> bool;
}
