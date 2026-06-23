//! Auth service.
//!
//! Canopy distinguishes two independent identities on every request:
//!
//! * **End-user identity** — PandaWave users authenticate once and carry a
//!   session token through PandaEngine to Canopy. This is what
//!   `playback_history` and personalization are scoped to.
//! * **Service identity** — PandaEngine authenticates to Canopy as a trusted
//!   client via mTLS or a service credential. This is a transport-layer
//!   concern and is established at the TLS edge, not here.
//!
//! This module implements the **end-user** half: stateless verification of the
//! session token. The token carries the user identity and an expiry, signed
//! with HMAC-SHA256 so it can be validated in memory without a database lookup
//! — the same stateless-validation property the playback resolver relies on.
//! Minting lives here too so tests (and a future login endpoint) can issue
//! tokens; in production the token originates from PandaWave.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canopy_core::{CanopyError, CanopyResult, UserIdentity};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Field separator inside the token. The user id must not contain it, which is
/// enforced when a token is minted.
const SEP: char = '.';

/// Stateless verifier (and minter) of end-user session tokens.
///
/// A token has the form `user_id.expires_at_epoch_ms.signature`, where the
/// signature is the lowercase-hex HMAC-SHA256 of `user_id.expires_at_epoch_ms`.
/// Verification recomputes the signature, compares it in constant time, and
/// rejects expired tokens — all without touching any backing store.
#[derive(Clone)]
pub struct AuthService {
    secret: Vec<u8>,
}

impl AuthService {
    /// Creates a service from the shared signing secret (any length accepted).
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// Mints a token for `user_id` valid for `ttl` from now.
    ///
    /// Returns [`CanopyError::InvalidArgument`] if the user id is empty or
    /// contains the reserved separator (which would make the token ambiguous).
    pub fn mint(&self, user_id: &str, ttl: Duration) -> CanopyResult<String> {
        self.mint_at(user_id, ttl, now_epoch_ms())
    }

    /// Mints a token using an explicit `now` (epoch ms); the expiry is computed
    /// relative to it. Split out so token lifetimes are deterministically
    /// testable.
    pub fn mint_at(&self, user_id: &str, ttl: Duration, now_epoch_ms: u64) -> CanopyResult<String> {
        if user_id.is_empty() {
            return Err(CanopyError::InvalidArgument("empty user id".into()));
        }
        if user_id.contains(SEP) {
            return Err(CanopyError::InvalidArgument(format!(
                "user id must not contain '{SEP}'"
            )));
        }
        let expires_at = now_epoch_ms + ttl.as_millis() as u64;
        let signature = self.compute(user_id, expires_at);
        Ok(format!("{user_id}{SEP}{expires_at}{SEP}{signature}"))
    }

    /// Verifies `token` and returns the [`UserIdentity`] it carries.
    ///
    /// Fails with [`CanopyError::Unauthenticated`] if the token is malformed,
    /// improperly signed, or expired relative to the current time.
    pub fn verify(&self, token: &str) -> CanopyResult<UserIdentity> {
        self.verify_at(token, now_epoch_ms())
    }

    /// Verifies `token` against an explicit `now` (epoch ms). Split out so the
    /// expiry check is deterministically testable.
    pub fn verify_at(&self, token: &str, now_epoch_ms: u64) -> CanopyResult<UserIdentity> {
        let mut parts = token.split(SEP);
        let (user_id, expires_raw, signature) = match (parts.next(), parts.next(), parts.next()) {
            (Some(u), Some(e), Some(s)) if parts.next().is_none() => (u, e, s),
            _ => return Err(CanopyError::unauthenticated("malformed token")),
        };

        let expires_at: u64 = expires_raw
            .parse()
            .map_err(|_| CanopyError::unauthenticated("malformed expiry"))?;

        let expected = self.compute(user_id, expires_at);
        if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
            return Err(CanopyError::unauthenticated("bad signature"));
        }
        if now_epoch_ms > expires_at {
            return Err(CanopyError::unauthenticated("expired token"));
        }

        Ok(UserIdentity {
            user_id: user_id.to_string(),
        })
    }

    /// Computes the lowercase-hex HMAC over the canonical signing string.
    fn compute(&self, user_id: &str, expires_at_epoch_ms: u64) -> String {
        // `new_from_slice` only errors on key lengths HMAC cannot accept, and
        // HMAC accepts keys of any length, so this never fails in practice.
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts keys of any length");
        mac.update(user_id.as_bytes());
        mac.update(SEP.to_string().as_bytes());
        mac.update(expires_at_epoch_ms.to_string().as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

/// Compares two byte slices in time independent of where they first differ, to
/// avoid leaking the signature through timing.
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

/// Current wall-clock time in epoch milliseconds.
fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    #[test]
    fn mint_then_verify_round_trips_identity() {
        let auth = AuthService::new("secret");
        let token = auth.mint_at("user-1", TTL, 1_000).unwrap();
        let identity = auth.verify_at(&token, 2_000).unwrap();
        assert_eq!(identity.user_id, "user-1");
    }

    #[test]
    fn verify_rejects_expired_token() {
        let auth = AuthService::new("secret");
        let token = auth
            .mint_at("user-1", Duration::from_millis(500), 1_000)
            .unwrap();
        // 1_000 + 500 = 1_500 expiry; one ms past must be rejected.
        let err = auth.verify_at(&token, 1_501).unwrap_err();
        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[test]
    fn verify_rejects_tampered_or_foreign_signature() {
        let auth = AuthService::new("secret");
        let token = auth.mint_at("user-1", TTL, 1_000).unwrap();

        // Swapping the user id but keeping the signature must fail.
        let (_, rest) = token.split_once(SEP).unwrap();
        let forged = format!("user-2{SEP}{rest}");
        assert!(auth.verify_at(&forged, 2_000).is_err());

        // A token minted with a different secret must not validate.
        let other = AuthService::new("other-secret");
        let foreign = other.mint_at("user-1", TTL, 1_000).unwrap();
        assert!(auth.verify_at(&foreign, 2_000).is_err());
    }

    #[test]
    fn verify_rejects_malformed_tokens() {
        let auth = AuthService::new("secret");
        assert!(auth.verify_at("", 1).is_err());
        assert!(auth.verify_at("only.two", 1).is_err());
        assert!(auth.verify_at("a.b.c.d", 1).is_err());
        assert!(auth.verify_at("user.not-a-number.deadbeef", 1).is_err());
    }

    #[test]
    fn mint_rejects_invalid_user_ids() {
        let auth = AuthService::new("secret");
        assert!(auth.mint("", TTL).is_err());
        assert!(auth.mint("has.separator", TTL).is_err());
    }
}
