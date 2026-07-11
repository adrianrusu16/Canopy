use std::collections::BTreeMap;
use std::env;
use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use canopy_core::{CanopyError, CanopyResult};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

const OUTBOX_KEY_ID: &str = "auth-outbox-aes256-gcm-v1";
const OUTBOX_KEY_ENV: &str = "CANOPY_AUTH_OUTBOX_SEALING_KEY";
const OUTBOX_FORMAT_VERSION: u8 = 1;
const OUTBOX_NONCE_LEN: usize = 12;
const OUTBOX_KEY_LEN: usize = 32;
const DEV_OUTBOX_SEALING_KEY: [u8; OUTBOX_KEY_LEN] = *b"canopy-dev-auth-outbox-key-v01!!";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmailOutboxPayload {
    pub email: String,
    pub purpose: String,
    pub expires_at_epoch_ms: u64,
    pub template_variables: BTreeMap<String, String>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct SealedOutboxPayload {
    bytes: Vec<u8>,
    key_id: String,
}

impl SealedOutboxPayload {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

impl fmt::Debug for SealedOutboxPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedOutboxPayload")
            .field("bytes", &"[REDACTED]")
            .field("key_id", &self.key_id)
            .finish()
    }
}

impl EmailOutboxPayload {
    pub fn email_verification(email: String, token: &str, expires_at_epoch_ms: u64) -> Self {
        let mut template_variables = BTreeMap::new();
        template_variables.insert("verification_token".into(), token.into());
        Self {
            email,
            purpose: "email_verification".into(),
            expires_at_epoch_ms,
            template_variables,
        }
    }

    pub fn password_reset(email: String, token: &str, expires_at_epoch_ms: u64) -> Self {
        let mut template_variables = BTreeMap::new();
        template_variables.insert("reset_token".into(), token.into());
        Self {
            email,
            purpose: "password_reset".into(),
            expires_at_epoch_ms,
            template_variables,
        }
    }

    pub fn seal(self) -> CanopyResult<SealedOutboxPayload> {
        let mut ciphertext = serde_json::to_vec(&self).map_err(|error| {
            CanopyError::Internal(format!("failed to encode auth outbox payload: {error}"))
        })?;
        let key = outbox_aead_key()?;
        let mut nonce_bytes = [0_u8; OUTBOX_NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| CanopyError::Internal("failed to generate auth outbox nonce".into()))?;

        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(OUTBOX_KEY_ID.as_bytes()),
            &mut ciphertext,
        )
        .map_err(|_| CanopyError::Internal("failed to seal auth outbox payload".into()))?;

        let mut bytes = Vec::with_capacity(1 + OUTBOX_NONCE_LEN + ciphertext.len());
        bytes.push(OUTBOX_FORMAT_VERSION);
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ciphertext);
        Ok(SealedOutboxPayload {
            bytes,
            key_id: OUTBOX_KEY_ID.into(),
        })
    }

    pub fn open(bytes: &[u8]) -> CanopyResult<Self> {
        let (version, rest) = bytes
            .split_first()
            .ok_or_else(|| CanopyError::InvalidArgument("auth outbox payload is empty".into()))?;
        if *version != OUTBOX_FORMAT_VERSION || rest.len() <= OUTBOX_NONCE_LEN {
            return Err(CanopyError::InvalidArgument(
                "unsupported auth outbox payload format".into(),
            ));
        }

        let mut nonce_bytes = [0_u8; OUTBOX_NONCE_LEN];
        nonce_bytes.copy_from_slice(&rest[..OUTBOX_NONCE_LEN]);
        let mut ciphertext = rest[OUTBOX_NONCE_LEN..].to_vec();
        let key = outbox_aead_key()?;
        let plaintext = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(OUTBOX_KEY_ID.as_bytes()),
                &mut ciphertext,
            )
            .map_err(|_| {
                CanopyError::InvalidArgument("auth outbox payload did not verify".into())
            })?;

        serde_json::from_slice(plaintext).map_err(|error| {
            CanopyError::Internal(format!("failed to decode auth outbox payload: {error}"))
        })
    }

    #[doc(hidden)]
    pub fn decode_for_test(bytes: &[u8]) -> CanopyResult<Self> {
        Self::open(bytes)
    }
}

fn outbox_aead_key() -> CanopyResult<LessSafeKey> {
    let key_bytes = outbox_sealing_key_bytes()?;
    let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes)
        .map_err(|_| CanopyError::Internal("invalid auth outbox sealing key".into()))?;
    Ok(LessSafeKey::new(unbound))
}

fn outbox_sealing_key_bytes() -> CanopyResult<[u8; OUTBOX_KEY_LEN]> {
    match env::var(OUTBOX_KEY_ENV) {
        Ok(encoded) if !encoded.trim().is_empty() => {
            let decoded = STANDARD.decode(encoded.trim()).map_err(|_| {
                CanopyError::Internal(format!(
                    "{OUTBOX_KEY_ENV} must be base64-encoded 32-byte key material"
                ))
            })?;
            decoded.try_into().map_err(|_| {
                CanopyError::Internal(format!(
                    "{OUTBOX_KEY_ENV} must be base64-encoded 32-byte key material"
                ))
            })
        }
        Ok(_) | Err(env::VarError::NotPresent) => Ok(DEV_OUTBOX_SEALING_KEY),
        Err(env::VarError::NotUnicode(_)) => Err(CanopyError::Internal(format!(
            "{OUTBOX_KEY_ENV} must be valid unicode"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_payload_hides_email_and_template_variables_but_opens() {
        let sealed =
            EmailOutboxPayload::email_verification("ada@example.test".into(), "secret-token", 42)
                .seal()
                .unwrap();
        let stored = String::from_utf8_lossy(sealed.bytes());

        assert!(!stored.contains("ada@example.test"));
        assert!(!stored.contains("verification_token"));
        assert!(!stored.contains("secret-token"));
        assert_eq!(sealed.key_id(), OUTBOX_KEY_ID);

        let opened = EmailOutboxPayload::open(sealed.bytes()).unwrap();
        assert_eq!(opened.email, "ada@example.test");
        assert_eq!(opened.purpose, "email_verification");
        assert_eq!(
            opened.template_variables["verification_token"],
            "secret-token"
        );
    }
}
