//! Transaction-oriented persistence port for authentication identity state.

use async_trait::async_trait;

use crate::{AccountStatus, AuthSession, CanopyResult};

/// Public account facts needed by authentication responses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountRecord {
    /// Stable account identifier.
    pub id: String,
    /// Current lifecycle state.
    pub status: AccountStatus,
    /// Verified primary email when available.
    pub primary_email: Option<String>,
    /// Creation time in Unix epoch milliseconds.
    pub created_at_epoch_ms: u64,
}

/// Password-login material loaded without exposing any raw credential.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PasswordLoginRecord {
    /// Account being authenticated.
    pub account: AccountRecord,
    /// Canonical Argon2 PHC string.
    pub password_hash_phc: String,
    /// Password policy used to create the hash.
    pub policy_version: u32,
}

/// Hashed and encrypted values persisted during native registration.
pub struct RegisterPasswordRecord {
    /// Normalized primary email.
    pub normalized_email: String,
    /// Canonical Argon2 PHC string.
    pub password_hash_phc: String,
    /// Password-policy version.
    pub policy_version: u32,
    /// Digest of the email-verification token.
    pub verification_token_hash: [u8; 32],
    /// Expiry of the verification challenge.
    pub verification_expires_at_epoch_ms: u64,
    /// Authenticated ciphertext for post-commit email delivery.
    pub encrypted_outbox_payload: Vec<u8>,
    /// Encryption key identifier used by the outbox worker.
    pub outbox_key_id: String,
}

/// Input used to consume one typed, single-use challenge.
pub struct ConsumeChallenge {
    /// Digest of the presented high-entropy token.
    pub token_hash: [u8; 32],
    /// Expected challenge type as a stable database value.
    pub challenge_type: &'static str,
    /// Current time used for deterministic expiry checks.
    pub now_epoch_ms: u64,
}

/// Hashed refresh token and device facts used to create a session.
pub struct CreateSessionRecord {
    /// Account that owns the device session.
    pub account_id: String,
    /// User-visible device label.
    pub device_label: String,
    /// Digest of the initial refresh token.
    pub refresh_token_hash: [u8; 32],
    /// Absolute refresh/session expiry.
    pub expires_at_epoch_ms: u64,
}

/// Atomic refresh-token rotation input.
pub struct RotateRefreshTokenRecord {
    /// Digest of the presented refresh token.
    pub presented_token_hash: [u8; 32],
    /// Digest of the replacement refresh token.
    pub replacement_token_hash: [u8; 32],
    /// Expiry assigned to the replacement token.
    pub replacement_expires_at_epoch_ms: u64,
    /// Current time for expiry and last-use checks.
    pub now_epoch_ms: u64,
}

/// Account and session returned by successful activation/login/refresh work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredAuthenticatedSession {
    /// Active account.
    pub account: AccountRecord,
    /// Active device session.
    pub session: AuthSession,
}

/// Immutable external identity proven by an OIDC provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalIdentityRecord {
    /// Provider name, initially `google`.
    pub provider: String,
    /// Immutable provider subject (`sub`).
    pub provider_subject: String,
    /// Verified email observed when the identity was linked.
    pub provider_email_at_link_time: String,
}

/// Transaction-oriented persistence operations required by `IdentityService`.
#[async_trait]
pub trait IdentityRepository: Send + Sync {
    /// Registers a pending native account and queues its verification email.
    async fn register_password(&self, record: RegisterPasswordRecord) -> CanopyResult<()>;

    /// Loads password verification material by normalized email.
    async fn password_login_record(
        &self,
        normalized_email: &str,
    ) -> CanopyResult<Option<PasswordLoginRecord>>;

    /// Replaces an outdated password hash after successful verification.
    async fn update_password_hash(
        &self,
        account_id: &str,
        password_hash_phc: &str,
        policy_version: u32,
    ) -> CanopyResult<()>;

    /// Atomically consumes email verification, activates the account/profile,
    /// and creates the initial device session.
    async fn activate_email_and_create_session(
        &self,
        challenge: ConsumeChallenge,
        session: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession>;

    /// Creates a device session for an already active account.
    async fn create_session(
        &self,
        record: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession>;

    /// Atomically consumes one refresh token and inserts its replacement.
    async fn rotate_refresh_token(
        &self,
        record: RotateRefreshTokenRecord,
    ) -> CanopyResult<StoredAuthenticatedSession>;

    /// Confirms that an account and session remain active for a protected call.
    async fn validate_active_session(
        &self,
        account_id: &str,
        session_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<()>;

    /// Idempotently revokes one account-owned session.
    async fn revoke_session(&self, account_id: &str, session_id: &str) -> CanopyResult<()>;

    /// Idempotently revokes all sessions belonging to an account.
    async fn revoke_all_sessions(&self, account_id: &str, reason: &str) -> CanopyResult<()>;

    /// Lists sessions scoped to the authenticated account.
    async fn list_sessions(&self, account_id: &str) -> CanopyResult<Vec<AuthSession>>;

    /// Finds an account by immutable external provider identity.
    async fn account_by_external_identity(
        &self,
        identity: &ExternalIdentityRecord,
    ) -> CanopyResult<Option<AccountRecord>>;

    /// Idempotently transitions an account into deletion and purges profile data.
    async fn delete_account(&self, account_id: &str) -> CanopyResult<()>;
}
