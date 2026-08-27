use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use canopy_core::{CanopyError, CanopyResult, StreamAudience};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const TOKEN_VERSION: u8 = 1;
const MIN_SECRET_BYTES: usize = 32;
const MAX_TOKEN_BYTES: usize = 2_048;
const INVALID_CAPABILITY: &str = "invalid stream capability";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamClaims {
    pub version: u8,
    pub asset_id: String,
    pub audience: StreamAudience,
    pub expires_at_epoch_ms: u64,
    pub nonce: String,
}

#[derive(Serialize, Deserialize)]
struct WireClaims {
    v: u8,
    aid: String,
    aud: String,
    exp: u64,
    nonce: String,
}

#[derive(Clone)]
pub struct StreamTokenCodec {
    secret: Vec<u8>,
}

impl StreamTokenCodec {
    pub fn new(secret: impl AsRef<[u8]>) -> CanopyResult<Self> {
        let secret = secret.as_ref();
        if secret.len() < MIN_SECRET_BYTES {
            return Err(CanopyError::InvalidArgument(
                "stream token secret must contain at least 32 bytes".into(),
            ));
        }
        Ok(Self {
            secret: secret.to_vec(),
        })
    }

    pub fn mint(
        &self,
        asset_id: &str,
        audience: StreamAudience,
        expires_at_epoch_ms: u64,
    ) -> CanopyResult<String> {
        Uuid::parse_str(asset_id)
            .map_err(|_| CanopyError::InvalidArgument("asset_id must be a UUID".into()))?;

        let claims = WireClaims {
            v: TOKEN_VERSION,
            aid: asset_id.to_owned(),
            aud: audience.as_str().to_owned(),
            exp: expires_at_epoch_ms,
            nonce: Uuid::new_v4().to_string(),
        };
        let payload = serde_json::to_vec(&claims)
            .map_err(|error| CanopyError::Internal(format!("encode stream capability: {error}")))?;
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let signature = self.sign(encoded_payload.as_bytes());
        Ok(format!(
            "{encoded_payload}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    pub fn verify(&self, token: &str, now_epoch_ms: u64) -> CanopyResult<StreamClaims> {
        self.verify_inner(token, now_epoch_ms)
            .map_err(|_| invalid_capability())
    }

    fn verify_inner(&self, token: &str, now_epoch_ms: u64) -> Result<StreamClaims, ()> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(());
        }
        let (payload, signature) = token.split_once('.').ok_or(())?;
        if signature.contains('.') {
            return Err(());
        }
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| ())?;
        let mut mac = HmacSha256::new_from_slice(&self.secret).map_err(|_| ())?;
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature).map_err(|_| ())?;

        let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| ())?;
        let claims: WireClaims = serde_json::from_slice(&payload).map_err(|_| ())?;
        if claims.v != TOKEN_VERSION || now_epoch_ms > claims.exp {
            return Err(());
        }
        Uuid::parse_str(&claims.aid).map_err(|_| ())?;
        Uuid::parse_str(&claims.nonce).map_err(|_| ())?;
        let audience = match claims.aud.as_str() {
            "public" => StreamAudience::Public,
            "personal" => StreamAudience::Personal,
            _ => return Err(()),
        };

        Ok(StreamClaims {
            version: claims.v,
            asset_id: claims.aid,
            audience,
            expires_at_epoch_ms: claims.exp,
            nonce: claims.nonce,
        })
    }

    fn sign(&self, payload: &[u8]) -> Vec<u8> {
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .expect("HMAC-SHA256 accepts keys of any length");
        mac.update(payload);
        mac.finalize().into_bytes().to_vec()
    }
}

fn invalid_capability() -> CanopyError {
    CanopyError::unauthenticated(INVALID_CAPABILITY)
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use canopy_core::CanopyError;
    use hmac::{Hmac, KeyInit, Mac};
    use serde_json::json;
    use sha2::Sha256;

    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    const ASSET_ID: &str = "018f0000-0000-7000-8000-000000000001";

    fn codec() -> StreamTokenCodec {
        StreamTokenCodec::new(SECRET).unwrap()
    }

    fn assert_generic_invalid(error: CanopyError) {
        assert_eq!(
            error.to_string(),
            "unauthenticated: invalid stream capability"
        );
    }

    fn sign_payload(payload: &[u8]) -> String {
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET).unwrap();
        mac.update(encoded_payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{encoded_payload}.{signature}")
    }

    #[test]
    fn token_round_trip_preserves_claims() {
        let token = codec()
            .mint(ASSET_ID, StreamAudience::Public, 5_000)
            .unwrap();

        let claims = codec().verify(&token, 4_000).unwrap();

        assert_eq!(claims.version, 1);
        assert_eq!(claims.asset_id, ASSET_ID);
        assert_eq!(claims.audience, StreamAudience::Public);
        assert_eq!(claims.expires_at_epoch_ms, 5_000);
        assert!(!claims.nonce.is_empty());
    }

    #[test]
    fn independently_minted_tokens_have_distinct_nonces() {
        let codec = codec();
        let first = codec.mint(ASSET_ID, StreamAudience::Public, 5_000).unwrap();
        let second = codec.mint(ASSET_ID, StreamAudience::Public, 5_000).unwrap();

        let first = codec.verify(&first, 4_000).unwrap();
        let second = codec.verify(&second, 4_000).unwrap();

        assert_ne!(first.nonce, second.nonce);
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let mut token = codec()
            .mint(ASSET_ID, StreamAudience::Public, 5_000)
            .unwrap();
        let replacement = if token.ends_with('A') { 'B' } else { 'A' };
        token.pop();
        token.push(replacement);

        assert_generic_invalid(codec().verify(&token, 4_000).unwrap_err());
    }

    #[test]
    fn verify_rejects_expired_token() {
        let token = codec()
            .mint(ASSET_ID, StreamAudience::Public, 5_000)
            .unwrap();

        assert_generic_invalid(codec().verify(&token, 5_001).unwrap_err());
    }

    #[test]
    fn verify_accepts_exact_expiry_boundary() {
        let token = codec()
            .mint(ASSET_ID, StreamAudience::Personal, 5_000)
            .unwrap();

        assert_eq!(
            codec().verify(&token, 5_000).unwrap().audience,
            StreamAudience::Personal
        );
    }

    #[test]
    fn verify_rejects_unknown_version_and_malformed_encoding() {
        let unknown = serde_json::to_vec(&json!({
            "v": 2,
            "aid": ASSET_ID,
            "aud": "public",
            "exp": 5_000,
            "nonce": "018f0000-0000-7000-8000-000000000099"
        }))
        .unwrap();

        assert_generic_invalid(codec().verify(&sign_payload(&unknown), 4_000).unwrap_err());
        assert_generic_invalid(codec().verify("not-a-capability", 4_000).unwrap_err());
    }

    #[test]
    fn codec_rejects_secrets_shorter_than_32_bytes() {
        assert!(matches!(
            StreamTokenCodec::new(b"short"),
            Err(CanopyError::InvalidArgument(_))
        ));
    }
}
