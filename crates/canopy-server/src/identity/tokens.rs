use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use canopy_core::{CanopyError, CanopyResult};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessTokenConfig {
    pub issuer: String,
    pub audience: String,
    pub key_id: String,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct AccessTokenClaims {
    pub issuer: String,
    pub audience: String,
    pub subject: String,
    pub session_id: String,
    pub issued_at_epoch_seconds: u64,
    pub expires_at_epoch_seconds: u64,
    pub jwt_id: String,
    pub key_id: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IssuedAccessToken {
    pub token: String,
    pub expires_at_epoch_ms: u64,
}

#[derive(Clone)]
pub struct Ed25519AccessTokenIssuer {
    config: AccessTokenConfig,
    signing_key: Option<SigningKey>,
    verifying_key: VerifyingKey,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccessTokenHeader {
    alg: String,
    typ: String,
    kid: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AccessTokenPayload {
    iss: String,
    aud: String,
    sub: String,
    sid: String,
    iat: u64,
    exp: u64,
    jti: String,
    kid: String,
}

impl Ed25519AccessTokenIssuer {
    pub fn generate(config: AccessTokenConfig) -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self::from_signing_key(config, signing_key)
    }

    pub fn from_signing_key_base64(config: AccessTokenConfig, encoded: &str) -> CanopyResult<Self> {
        let decoded = STANDARD.decode(encoded).map_err(|_| {
            CanopyError::InvalidArgument("access-token signing key must be valid base64".into())
        })?;
        let signing_key_bytes: [u8; 32] = decoded.as_slice().try_into().map_err(|_| {
            CanopyError::InvalidArgument("access-token signing key must decode to 32 bytes".into())
        })?;
        Ok(Self::from_signing_key(
            config,
            SigningKey::from_bytes(&signing_key_bytes),
        ))
    }

    fn from_signing_key(config: AccessTokenConfig, signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        Self {
            config,
            signing_key: Some(signing_key),
            verifying_key,
        }
    }

    pub fn verifier(&self, config: AccessTokenConfig) -> Self {
        Self {
            config,
            signing_key: None,
            verifying_key: self.verifying_key,
        }
    }

    pub fn issue(
        &self,
        account_id: &str,
        session_id: &str,
        now_epoch_seconds: u64,
    ) -> CanopyResult<IssuedAccessToken> {
        let signing_key = self.signing_key.as_ref().ok_or_else(|| {
            CanopyError::Internal("access-token verifier cannot issue tokens".into())
        })?;
        let header = AccessTokenHeader {
            alg: "EdDSA".into(),
            typ: "JWT".into(),
            kid: self.config.key_id.clone(),
        };
        let expires_at_epoch_seconds = now_epoch_seconds + self.config.ttl_seconds;
        let payload = AccessTokenPayload {
            iss: self.config.issuer.clone(),
            aud: self.config.audience.clone(),
            sub: account_id.into(),
            sid: session_id.into(),
            iat: now_epoch_seconds,
            exp: expires_at_epoch_seconds,
            jti: OpaqueToken::generate().into_string(),
            kid: self.config.key_id.clone(),
        };

        let header = encode_json(&header)?;
        let payload = encode_json(&payload)?;
        let signing_input = format!("{header}.{payload}");
        let signature = signing_key.sign(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(signature.to_bytes());

        Ok(IssuedAccessToken {
            token: format!("{signing_input}.{signature}"),
            expires_at_epoch_ms: expires_at_epoch_seconds * 1_000,
        })
    }

    pub fn verify(&self, token: &str, now_epoch_seconds: u64) -> CanopyResult<AccessTokenClaims> {
        let mut parts = token.split('.');
        let Some(encoded_header) = parts.next() else {
            return Err(CanopyError::unauthenticated("malformed access token"));
        };
        let Some(encoded_payload) = parts.next() else {
            return Err(CanopyError::unauthenticated("malformed access token"));
        };
        let Some(encoded_signature) = parts.next() else {
            return Err(CanopyError::unauthenticated("malformed access token"));
        };
        if parts.next().is_some() {
            return Err(CanopyError::unauthenticated("malformed access token"));
        }

        let header: AccessTokenHeader = decode_json(encoded_header)?;
        if header.alg != "EdDSA" || header.typ != "JWT" || header.kid != self.config.key_id {
            return Err(CanopyError::unauthenticated(
                "unexpected access-token header",
            ));
        }

        let signing_input = format!("{encoded_header}.{encoded_payload}");
        let signature_bytes = URL_SAFE_NO_PAD
            .decode(encoded_signature)
            .map_err(|_| CanopyError::unauthenticated("invalid access-token signature"))?;
        let signature = Signature::try_from(signature_bytes.as_slice())
            .map_err(|_| CanopyError::unauthenticated("invalid access-token signature"))?;
        self.verifying_key
            .verify(signing_input.as_bytes(), &signature)
            .map_err(|_| CanopyError::unauthenticated("invalid access-token signature"))?;

        let payload: AccessTokenPayload = decode_json(encoded_payload)?;
        if payload.iss != self.config.issuer {
            return Err(CanopyError::unauthenticated(
                "unexpected access-token issuer",
            ));
        }
        if payload.aud != self.config.audience {
            return Err(CanopyError::unauthenticated(
                "unexpected access-token audience",
            ));
        }
        if payload.kid != self.config.key_id {
            return Err(CanopyError::unauthenticated("unexpected access-token key"));
        }
        if payload.exp < now_epoch_seconds {
            return Err(CanopyError::unauthenticated("expired access token"));
        }

        Ok(AccessTokenClaims {
            issuer: payload.iss,
            audience: payload.aud,
            subject: payload.sub,
            session_id: payload.sid,
            issued_at_epoch_seconds: payload.iat,
            expires_at_epoch_seconds: payload.exp,
            jwt_id: payload.jti,
            key_id: payload.kid,
        })
    }
}

fn encode_json<T: Serialize>(value: &T) -> CanopyResult<String> {
    let json = serde_json::to_vec(value)
        .map_err(|error| CanopyError::Internal(format!("failed to encode token JSON: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

fn decode_json<T: for<'de> Deserialize<'de>>(encoded: &str) -> CanopyResult<T> {
    let json = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| CanopyError::unauthenticated("invalid access-token encoding"))?;
    serde_json::from_slice(&json)
        .map_err(|_| CanopyError::unauthenticated("invalid access-token JSON"))
}

#[derive(Clone, Eq, PartialEq)]
pub struct OpaqueToken(String);

impl OpaqueToken {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for OpaqueToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TokenDigest([u8; 32]);

impl TokenDigest {
    pub fn from_secret(secret: &str) -> Self {
        let digest = Sha256::digest(secret.as_bytes());
        Self(digest.into())
    }

    pub fn from_token(token: &OpaqueToken) -> Self {
        let digest = Sha256::digest(token.as_str().as_bytes());
        Self(digest.into())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for TokenDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}
