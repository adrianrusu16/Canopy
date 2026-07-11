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

/// Password material for authenticated password-change verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PasswordCredentialRecord {
    /// Account being authenticated.
    pub account: AccountRecord,
    /// Canonical Argon2 PHC string.
    pub password_hash_phc: String,
    /// Password policy used to create the hash.
    pub policy_version: u32,
}

/// Request to create a generic password-reset challenge for an active account.
pub struct CreatePasswordResetChallenge {
    /// Normalized primary email to target.
    pub normalized_email: String,
    /// Digest of the password-reset token.
    pub token_hash: [u8; 32],
    /// Expiry of the reset challenge.
    pub expires_at_epoch_ms: u64,
    /// Authenticated ciphertext for post-commit email delivery.
    pub encrypted_outbox_payload: Vec<u8>,
    /// Encryption key identifier used by the outbox worker.
    pub outbox_key_id: String,
}

/// Atomic password reset input.
pub struct CompletePasswordResetRecord {
    /// Digest of the presented reset token.
    pub token_hash: [u8; 32],
    /// Replacement password hash.
    pub password_hash_phc: String,
    /// Password-policy version used to create the hash.
    pub policy_version: u32,
    /// Current time used for challenge expiry.
    pub now_epoch_ms: u64,
}

/// Atomic authenticated password-change input.
pub struct ChangePasswordRecord {
    /// Account changing its password.
    pub account_id: String,
    /// Current session to keep active after the password change.
    pub current_session_id: String,
    /// Replacement password hash.
    pub password_hash_phc: String,
    /// Password-policy version used to create the hash.
    pub policy_version: u32,
}

/// Request to rotate an email-verification challenge for a pending account.
pub struct CreateEmailVerificationChallenge {
    /// Normalized primary email to target.
    pub normalized_email: String,
    /// Digest of the replacement email-verification token.
    pub token_hash: [u8; 32],
    /// Expiry of the replacement verification challenge.
    pub expires_at_epoch_ms: u64,
    /// Authenticated ciphertext for post-commit email delivery.
    pub encrypted_outbox_payload: Vec<u8>,
    /// Encryption key identifier used by the outbox worker.
    pub outbox_key_id: String,
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

/// Fixed-window abuse-control bucket keyed by a hashed subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitBucket {
    /// Stable operation name, such as login_password_failure.
    pub operation: &'static str,
    /// SHA-256 digest of the operation-specific subject. Never store raw email, IP, or token values.
    pub subject_hash: [u8; 32],
    /// Inclusive fixed-window start in Unix epoch milliseconds.
    pub window_start_epoch_ms: u64,
    /// Exclusive fixed-window end in Unix epoch milliseconds.
    pub window_end_epoch_ms: u64,
    /// Maximum permitted hits within this window before requests are throttled.
    pub max_attempts: u32,
}

/// Current fixed-window rate-limit state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RateLimitState {
    /// Number of hits recorded in the current window.
    pub request_count: u32,
    /// Whether this bucket has reached its configured limit.
    pub limited: bool,
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
    /// Verified email observed when the identity was linked, when available.
    pub provider_email_at_link_time: Option<String>,
}

/// Request to create a Google login nonce challenge.
pub struct CreateGoogleLoginChallenge {
    /// Digest of the nonce sent to the OIDC client.
    pub nonce_hash: [u8; 32],
    /// Expiry of the nonce challenge.
    pub expires_at_epoch_ms: u64,
    /// Authenticated ciphertext carrying verifier-only challenge context.
    pub encrypted_payload: Vec<u8>,
}

/// Result returned after atomically consuming a Google login challenge.
pub struct ConsumedGoogleLoginChallenge {
    /// Authenticated ciphertext created with the challenge.
    pub encrypted_payload: Vec<u8>,
}

/// Request to create a Google link challenge for an existing account.
pub struct CreateGoogleLinkChallenge {
    /// Existing account that must authenticate before linking.
    pub account_id: String,
    /// Digest of the explicit link challenge token.
    pub token_hash: [u8; 32],
    /// Proven external identity to link after challenge consumption.
    pub identity: ExternalIdentityRecord,
    /// Expiry of the explicit link challenge.
    pub expires_at_epoch_ms: u64,
    /// Authenticated ciphertext carrying verifier-only link context.
    pub encrypted_payload: Vec<u8>,
}

/// Transaction-oriented persistence operations required by `IdentityService`.
#[async_trait]
pub trait IdentityRepository: Send + Sync {
    /// Returns the current state for a fixed-window abuse-control bucket.
    async fn rate_limit_state(&self, bucket: RateLimitBucket) -> CanopyResult<RateLimitState>;

    /// Atomically records one hit in a fixed-window abuse-control bucket.
    async fn record_rate_limit_hit(&self, bucket: RateLimitBucket) -> CanopyResult<RateLimitState>;

    /// Registers a pending native account and queues its verification email.
    async fn register_password(&self, record: RegisterPasswordRecord) -> CanopyResult<()>;

    /// Creates a Google OIDC nonce challenge and returns its public challenge id.
    async fn create_google_login_challenge(
        &self,
        record: CreateGoogleLoginChallenge,
    ) -> CanopyResult<String>;

    /// Atomically consumes a Google OIDC nonce challenge by public challenge id.
    async fn consume_google_login_challenge(
        &self,
        challenge_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<ConsumedGoogleLoginChallenge>;

    /// Creates a password-reset challenge for an active native account.
    ///
    /// Implementations must return `Ok(())` for missing or non-active accounts
    /// so public reset responses do not reveal account state.
    async fn create_password_reset_challenge(
        &self,
        record: CreatePasswordResetChallenge,
    ) -> CanopyResult<()>;

    /// Loads password material by account for authenticated password changes.
    async fn password_credential_for_account(
        &self,
        account_id: &str,
    ) -> CanopyResult<Option<PasswordCredentialRecord>>;

    /// Atomically consumes a password-reset challenge, changes the password,
    /// and revokes all sessions for the account.
    async fn complete_password_reset(
        &self,
        record: CompletePasswordResetRecord,
    ) -> CanopyResult<()>;

    /// Atomically changes a password and revokes all other sessions.
    async fn change_password(&self, record: ChangePasswordRecord) -> CanopyResult<()>;

    /// Creates a replacement email-verification challenge for a pending account.
    ///
    /// Implementations must return `Ok(())` for missing, active, disabled, or
    /// deleted accounts so public resend responses do not reveal account state.
    async fn create_email_verification_challenge(
        &self,
        record: CreateEmailVerificationChallenge,
    ) -> CanopyResult<()>;

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

    /// Loads an account by stable account id.
    async fn account_by_id(&self, account_id: &str) -> CanopyResult<Option<AccountRecord>>;

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

    /// Finds an active account by verified primary email without exposing credentials.
    async fn account_by_primary_email(
        &self,
        normalized_email: &str,
    ) -> CanopyResult<Option<AccountRecord>>;

    /// Creates a new active external-identity account and its initial session atomically.
    async fn create_external_identity_session(
        &self,
        identity: ExternalIdentityRecord,
        session: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession>;

    /// Creates an explicit Google link challenge for an existing account.
    async fn create_google_link_challenge(
        &self,
        record: CreateGoogleLinkChallenge,
    ) -> CanopyResult<()>;

    /// Atomically consumes a link challenge and attaches the external identity.
    async fn link_external_identity(
        &self,
        account_id: &str,
        link_challenge_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<()>;

    /// Idempotently unlinks one external provider from an account.
    async fn unlink_external_identity(&self, account_id: &str, provider: &str) -> CanopyResult<()>;

    /// Idempotently transitions an account into deletion and purges profile data.
    async fn delete_account(&self, account_id: &str) -> CanopyResult<()>;
}
