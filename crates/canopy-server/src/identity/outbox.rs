use std::collections::BTreeMap;
use std::fmt;

use canopy_core::{CanopyError, CanopyResult};
use serde::{Deserialize, Serialize};

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
        let bytes = serde_json::to_vec(&self).map_err(|error| {
            CanopyError::Internal(format!("failed to encode auth outbox payload: {error}"))
        })?;
        Ok(SealedOutboxPayload {
            bytes,
            key_id: "auth-outbox-json-v1".into(),
        })
    }

    #[cfg(test)]
    pub fn decode_for_test(bytes: &[u8]) -> CanopyResult<Self> {
        serde_json::from_slice(bytes).map_err(|error| {
            CanopyError::Internal(format!("failed to decode auth outbox payload: {error}"))
        })
    }
}
