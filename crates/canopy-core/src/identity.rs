//! Authentication identity and session domain types.

use crate::{CanopyError, CanopyResult};

/// Lifecycle state of a Canopy account.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountStatus {
    /// Native registration exists but its primary email is not verified.
    PendingEmailVerification,
    /// The account may authenticate and use durable features.
    Active,
    /// An administrator or security workflow disabled the account.
    Disabled,
    /// Deletion has begun and authentication is permanently unavailable.
    DeletionPending,
    /// The account has completed deletion.
    Deleted,
}

impl AccountStatus {
    /// Activates a newly verified pending account.
    pub fn activate(self) -> CanopyResult<Self> {
        match self {
            Self::PendingEmailVerification => Ok(Self::Active),
            _ => Err(CanopyError::FailedPrecondition(
                "account cannot be activated from its current state".into(),
            )),
        }
    }
}

/// Per-device authentication session used for revocation decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthSession {
    /// Stable session identifier carried by access tokens.
    pub id: String,
    /// Account that owns the session.
    pub account_id: String,
    /// User-visible device label.
    pub device_label: String,
    /// Absolute session expiry in Unix epoch milliseconds.
    pub expires_at_epoch_ms: u64,
    /// Revocation timestamp when the session is no longer valid.
    pub revoked_at_epoch_ms: Option<u64>,
}

impl AuthSession {
    /// Returns whether this session is unrevoked and unexpired at `now_epoch_ms`.
    pub fn is_active_at(&self, now_epoch_ms: u64) -> bool {
        self.revoked_at_epoch_ms.is_none() && now_epoch_ms <= self.expires_at_epoch_ms
    }
}
