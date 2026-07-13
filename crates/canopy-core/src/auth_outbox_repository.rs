//! Persistence boundary for leased authentication email delivery.

use std::fmt;

use async_trait::async_trait;

use crate::CanopyResult;

/// Sanitized failure categories safe to persist and report in telemetry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthOutboxFailureKind {
    /// Runtime or deployment configuration is invalid.
    Configuration,
    /// The SMTP endpoint could not be reached.
    Connection,
    /// The delivery attempt exceeded its timeout.
    Timeout,
    /// The SMTP endpoint rejected configured credentials.
    Authentication,
    /// The SMTP endpoint permanently rejected the message.
    Rejected,
    /// The persisted payload cannot be validated or rendered.
    Payload,
    /// An unexpected failure occurred without safe provider detail.
    Internal,
}

impl AuthOutboxFailureKind {
    /// Stable database value for this sanitized failure category.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::Connection => "connection",
            Self::Timeout => "timeout",
            Self::Authentication => "authentication",
            Self::Rejected => "rejected",
            Self::Payload => "payload",
            Self::Internal => "internal",
        }
    }
}

/// Input for one atomic, leased outbox claim.
pub struct ClaimAuthOutboxBatch {
    /// Current wall-clock time in Unix epoch milliseconds.
    pub now_epoch_ms: u64,
    /// Exclusive expiry assigned to the new lease.
    pub lease_expires_at_epoch_ms: u64,
    /// Maximum number of rows to claim.
    pub batch_size: u32,
    /// Opaque UUID token proving ownership of the claimed rows.
    pub lease_token: String,
}

/// One claimed authentication email with ciphertext redacted from debug output.
#[derive(Clone, Eq, PartialEq)]
pub struct ClaimedAuthOutbox {
    /// Stable outbox row identifier.
    pub id: String,
    /// Fixed email purpose stored with the row.
    pub kind: String,
    /// Authenticated ciphertext opened only in worker memory.
    pub encrypted_payload: Vec<u8>,
    /// Identifier of the key required to open the ciphertext.
    pub key_id: String,
    /// One-based delivery attempt count after this claim.
    pub attempts: u32,
    /// Lease token that must guard every state transition.
    pub lease_token: String,
}

impl fmt::Debug for ClaimedAuthOutbox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimedAuthOutbox")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("encrypted_payload", &"[REDACTED]")
            .field("key_id", &self.key_id)
            .field("attempts", &self.attempts)
            .field("lease_token", &self.lease_token)
            .finish()
    }
}

/// Conditional retry or terminal-failure update for one active lease.
pub struct MarkAuthOutboxFailed {
    /// Stable outbox row identifier.
    pub id: String,
    /// Token proving ownership of the active lease.
    pub lease_token: String,
    /// Sanitized failure category.
    pub error_kind: AuthOutboxFailureKind,
    /// Next retry availability, even when the row becomes terminal.
    pub available_at_epoch_ms: u64,
    /// Terminal failure time; None schedules another retry.
    pub failed_at_epoch_ms: Option<u64>,
}

/// Transactional persistence operations used by the auth email worker.
#[async_trait]
pub trait AuthOutboxRepository: Send + Sync {
    /// Atomically claims a bounded batch of available rows.
    async fn claim_auth_outbox_batch(
        &self,
        command: ClaimAuthOutboxBatch,
    ) -> CanopyResult<Vec<ClaimedAuthOutbox>>;

    /// Marks a matching active lease delivered and clears its ciphertext.
    async fn mark_auth_outbox_delivered(
        &self,
        id: &str,
        lease_token: &str,
        delivered_at_epoch_ms: u64,
    ) -> CanopyResult<bool>;

    /// Reschedules or exhausts a matching active lease.
    async fn mark_auth_outbox_failed(&self, command: MarkAuthOutboxFailed) -> CanopyResult<bool>;
}
