use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use canopy_core::{
    AccountRecord, AccountStatus, AuthSession, CanopyError, CanopyResult, ChangePasswordRecord,
    CompletePasswordResetRecord, ConsumeChallenge, CreateEmailVerificationChallenge,
    CreateGoogleLinkChallenge, CreateGoogleLoginChallenge, CreatePasswordResetChallenge,
    CreateSessionRecord, ExternalIdentityRecord, IdentityRepository, RateLimitBucket,
    RegisterPasswordRecord, RotateRefreshTokenRecord,
};

use super::{
    AccessTokenClaims, Ed25519AccessTokenIssuer, EmailOutboxPayload, NoopOidcVerifier,
    OidcVerifier, OpaqueToken, PasswordHasher, TokenDigest,
};

const PASSWORD_POLICY_VERSION: u32 = 2;
const VERIFICATION_TOKEN_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const REFRESH_TOKEN_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const GOOGLE_CHALLENGE_TTL_MS: u64 = 10 * 60 * 1000;
const RATE_LIMIT_WINDOW_MS: u64 = 15 * 60 * 1000;
const LOGIN_FAILURE_LIMIT: u32 = 5;
const REGISTER_REQUEST_LIMIT: u32 = 5;
const GENERIC_EMAIL_REQUEST_LIMIT: u32 = 3;

const PASSWORD_WORK_LIMIT: usize = 4;
const PASSWORD_WORK_QUEUE_TIMEOUT: Duration = Duration::from_secs(1);
const EMAIL_MAX_UTF8_BYTES: usize = 254;
const EMAIL_LOCAL_MAX_UTF8_BYTES: usize = 64;
const EMAIL_DOMAIN_MAX_BYTES: usize = 253;
static PASSWORD_WORK: Semaphore = Semaphore::const_new(PASSWORD_WORK_LIMIT);

pub trait Clock: Send + Sync {
    fn now_epoch_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_epoch_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[derive(Debug)]
pub struct FixedClock {
    now_epoch_ms: u64,
}

impl FixedClock {
    pub fn new(now_epoch_ms: u64) -> Self {
        Self { now_epoch_ms }
    }
}

impl Clock for FixedClock {
    fn now_epoch_ms(&self) -> u64 {
        self.now_epoch_ms
    }
}

async fn run_bounded_password_work<T, Work>(
    semaphore: &Semaphore,
    queue_timeout: Duration,
    work: Work,
) -> CanopyResult<T>
where
    T: Send + 'static,
    Work: FnOnce() -> CanopyResult<T> + Send + 'static,
{
    let permit = tokio::time::timeout(queue_timeout, semaphore.acquire())
        .await
        .map_err(|_| {
            CanopyError::RateLimited("password processing is temporarily saturated".into())
        })?
        .map_err(|_| CanopyError::Internal("password processing is unavailable".into()))?;
    let result = tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| CanopyError::Internal(format!("password task failed: {error}")))?;
    drop(permit);
    result
}

async fn run_password_work<T, Work>(work: Work) -> CanopyResult<T>
where
    T: Send + 'static,
    Work: FnOnce() -> CanopyResult<T> + Send + 'static,
{
    run_bounded_password_work(&PASSWORD_WORK, PASSWORD_WORK_QUEUE_TIMEOUT, work).await
}

pub struct RegisterPasswordCommand {
    pub email: String,
    pub password: String,
}

pub struct ResendVerificationCommand {
    pub email: String,
}

pub struct RequestPasswordResetCommand {
    pub email: String,
}

pub struct CompletePasswordResetCommand {
    pub reset_token: String,
    pub new_password: String,
}

pub struct ChangePasswordCommand {
    pub principal: AuthenticatedPrincipal,
    pub current_password: String,
    pub new_password: String,
}

pub struct BeginGoogleLoginCommand;

pub struct CompleteGoogleLoginCommand {
    pub challenge_id: String,
    pub id_token: String,
    pub device_label: String,
}

pub struct LinkGoogleCommand {
    pub principal: AuthenticatedPrincipal,
    pub link_challenge_id: String,
}

pub struct UnlinkGoogleCommand {
    pub principal: AuthenticatedPrincipal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoogleLoginChallenge {
    pub challenge_id: String,
    pub nonce: String,
    pub expires_at_epoch_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoogleLoginOutcome {
    Session(Box<SessionEnvelope>),
    AccountLinkRequired { link_challenge_id: String },
}

pub struct VerifyEmailCommand {
    pub verification_token: String,
    pub device_label: String,
}

pub struct LoginPasswordCommand {
    pub email: String,
    pub password: String,
    pub device_label: String,
}

pub struct RefreshSessionCommand {
    pub refresh_token: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedPrincipal {
    pub account_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionEnvelope {
    pub account: AccountRecord,
    pub session: AuthSession,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at_epoch_ms: u64,
}

pub struct IdentityService {
    repository: Arc<dyn IdentityRepository>,
    password_hasher: Arc<dyn PasswordHasher>,
    access_tokens: Ed25519AccessTokenIssuer,
    clock: Arc<dyn Clock>,
    oidc_verifier: Arc<dyn OidcVerifier>,
}

impl IdentityService {
    pub fn new(
        repository: Arc<dyn IdentityRepository>,
        password_hasher: Arc<dyn PasswordHasher>,
        access_tokens: Ed25519AccessTokenIssuer,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            password_hasher,
            access_tokens,
            clock,
            oidc_verifier: Arc::new(NoopOidcVerifier),
        }
    }

    pub fn with_oidc_verifier(mut self, oidc_verifier: Arc<dyn OidcVerifier>) -> Self {
        self.oidc_verifier = oidc_verifier;
        self
    }

    fn rate_limit_bucket(
        &self,
        operation: &'static str,
        subject: &str,
        now_epoch_ms: u64,
        max_attempts: u32,
    ) -> RateLimitBucket {
        let window_start_epoch_ms = now_epoch_ms - (now_epoch_ms % RATE_LIMIT_WINDOW_MS);
        RateLimitBucket {
            operation,
            subject_hash: *TokenDigest::from_secret(&format!("{operation}:{subject}")).as_bytes(),
            window_start_epoch_ms,
            window_end_epoch_ms: window_start_epoch_ms + RATE_LIMIT_WINDOW_MS,
            max_attempts,
        }
    }

    async fn ensure_not_rate_limited(&self, bucket: &RateLimitBucket) -> CanopyResult<()> {
        let state = self.repository.rate_limit_state(bucket.clone()).await?;
        if state.limited {
            Err(CanopyError::RateLimited(
                "too many authentication attempts; try again later".into(),
            ))
        } else {
            Ok(())
        }
    }

    async fn record_rate_limit_hit(&self, bucket: RateLimitBucket) -> CanopyResult<()> {
        self.repository.record_rate_limit_hit(bucket).await?;
        Ok(())
    }

    pub async fn register_password(&self, command: RegisterPasswordCommand) -> CanopyResult<()> {
        let normalized_email = normalize_email(&command.email)?;
        let now = self.clock.now_epoch_ms();
        let bucket = self.rate_limit_bucket(
            "register_password",
            &normalized_email,
            now,
            REGISTER_REQUEST_LIMIT,
        );
        self.ensure_not_rate_limited(&bucket).await?;
        self.record_rate_limit_hit(bucket).await?;

        let password_hasher = self.password_hasher.clone();
        let password = command.password;
        let password_hash_phc = run_password_work(move || password_hasher.hash(&password)).await?;

        let verification_token = OpaqueToken::generate();
        let verification_token_hash = *TokenDigest::from_token(&verification_token).as_bytes();
        let verification_expires_at_epoch_ms = now + VERIFICATION_TOKEN_TTL_MS;
        let outbox = verification_outbox(
            normalized_email.clone(),
            verification_token.as_str(),
            verification_expires_at_epoch_ms,
        )?;
        let outbox_key_id = outbox.key_id().to_owned();

        self.repository
            .register_password(RegisterPasswordRecord {
                normalized_email,
                password_hash_phc,
                policy_version: PASSWORD_POLICY_VERSION,
                verification_token_hash,
                verification_expires_at_epoch_ms,
                encrypted_outbox_payload: outbox.into_bytes(),
                outbox_key_id,
            })
            .await
    }

    pub async fn resend_verification(
        &self,
        command: ResendVerificationCommand,
    ) -> CanopyResult<()> {
        let normalized_email = normalize_email(&command.email)?;
        let now = self.clock.now_epoch_ms();
        let bucket = self.rate_limit_bucket(
            "resend_verification",
            &normalized_email,
            now,
            GENERIC_EMAIL_REQUEST_LIMIT,
        );
        if self
            .repository
            .rate_limit_state(bucket.clone())
            .await?
            .limited
        {
            return Ok(());
        }
        self.record_rate_limit_hit(bucket).await?;

        let verification_token = OpaqueToken::generate();
        let verification_token_hash = *TokenDigest::from_token(&verification_token).as_bytes();
        let expires_at_epoch_ms = now + VERIFICATION_TOKEN_TTL_MS;
        let outbox = verification_outbox(
            normalized_email.clone(),
            verification_token.as_str(),
            expires_at_epoch_ms,
        )?;
        let outbox_key_id = outbox.key_id().to_owned();

        self.repository
            .create_email_verification_challenge(CreateEmailVerificationChallenge {
                normalized_email,
                token_hash: verification_token_hash,
                expires_at_epoch_ms,
                encrypted_outbox_payload: outbox.into_bytes(),
                outbox_key_id,
            })
            .await
    }

    pub async fn request_password_reset(
        &self,
        command: RequestPasswordResetCommand,
    ) -> CanopyResult<()> {
        let Ok(normalized_email) = normalize_email(&command.email) else {
            return Ok(());
        };
        let now = self.clock.now_epoch_ms();
        let bucket = self.rate_limit_bucket(
            "password_reset_request",
            &normalized_email,
            now,
            GENERIC_EMAIL_REQUEST_LIMIT,
        );
        if self
            .repository
            .rate_limit_state(bucket.clone())
            .await?
            .limited
        {
            return Ok(());
        }
        self.record_rate_limit_hit(bucket).await?;

        let reset_token = OpaqueToken::generate();
        let token_hash = *TokenDigest::from_token(&reset_token).as_bytes();
        let expires_at_epoch_ms = now + VERIFICATION_TOKEN_TTL_MS;
        let outbox = password_reset_outbox(
            normalized_email.clone(),
            reset_token.as_str(),
            expires_at_epoch_ms,
        )?;
        let outbox_key_id = outbox.key_id().to_owned();

        self.repository
            .create_password_reset_challenge(CreatePasswordResetChallenge {
                normalized_email,
                token_hash,
                expires_at_epoch_ms,
                encrypted_outbox_payload: outbox.into_bytes(),
                outbox_key_id,
            })
            .await
    }

    pub async fn complete_password_reset(
        &self,
        command: CompletePasswordResetCommand,
    ) -> CanopyResult<()> {
        let password_hasher = self.password_hasher.clone();
        let new_password = command.new_password;
        let password_hash_phc =
            run_password_work(move || password_hasher.hash(&new_password)).await?;
        let now = self.clock.now_epoch_ms();
        self.repository
            .complete_password_reset(CompletePasswordResetRecord {
                token_hash: *TokenDigest::from_secret(&command.reset_token).as_bytes(),
                password_hash_phc,
                policy_version: PASSWORD_POLICY_VERSION,
                now_epoch_ms: now,
            })
            .await
    }

    pub async fn change_password(&self, command: ChangePasswordCommand) -> CanopyResult<()> {
        let Some(credential) = self
            .repository
            .password_credential_for_account(&command.principal.account_id)
            .await?
        else {
            return Err(CanopyError::unauthenticated("invalid current password"));
        };
        if credential.account.status != AccountStatus::Active {
            return Err(CanopyError::unauthenticated("invalid current password"));
        }

        let password_hasher = self.password_hasher.clone();
        let current_password = command.current_password;
        let current_hash = credential.password_hash_phc.clone();
        let verification =
            run_password_work(move || password_hasher.verify(&current_password, &current_hash))
                .await?;
        if !verification.valid {
            return Err(CanopyError::unauthenticated("invalid current password"));
        }

        let password_hasher = self.password_hasher.clone();
        let new_password = command.new_password;
        let password_hash_phc =
            run_password_work(move || password_hasher.hash(&new_password)).await?;

        self.repository
            .change_password(ChangePasswordRecord {
                account_id: command.principal.account_id,
                current_session_id: command.principal.session_id,
                password_hash_phc,
                policy_version: PASSWORD_POLICY_VERSION,
            })
            .await
    }
    pub async fn begin_google_login(
        &self,
        _command: BeginGoogleLoginCommand,
    ) -> CanopyResult<GoogleLoginChallenge> {
        let nonce = OpaqueToken::generate();
        let now = self.clock.now_epoch_ms();
        let expires_at_epoch_ms = now + GOOGLE_CHALLENGE_TTL_MS;
        let payload = GoogleLoginChallengePayload {
            nonce: nonce.as_str().into(),
        }
        .seal()?;
        let challenge_id = self
            .repository
            .create_google_login_challenge(CreateGoogleLoginChallenge {
                nonce_hash: *TokenDigest::from_token(&nonce).as_bytes(),
                expires_at_epoch_ms,
                encrypted_payload: payload,
            })
            .await?;

        Ok(GoogleLoginChallenge {
            challenge_id,
            nonce: nonce.into_string(),
            expires_at_epoch_ms,
        })
    }

    pub async fn complete_google_login(
        &self,
        command: CompleteGoogleLoginCommand,
    ) -> CanopyResult<GoogleLoginOutcome> {
        let now = self.clock.now_epoch_ms();
        let consumed = self
            .repository
            .consume_google_login_challenge(&command.challenge_id, now)
            .await?;
        let challenge = GoogleLoginChallengePayload::open(&consumed.encrypted_payload)?;
        let google = self
            .oidc_verifier
            .verify_google_id_token(&command.id_token, &challenge.nonce)
            .await?;
        let verified_email = if google.email_verified {
            google.email.as_deref().map(normalize_email).transpose()?
        } else {
            None
        };
        let identity = ExternalIdentityRecord {
            provider: "google".into(),
            provider_subject: google.subject,
            provider_email_at_link_time: verified_email.clone(),
        };

        if let Some(account) = self
            .repository
            .account_by_external_identity(&identity)
            .await?
        {
            let refresh_token = OpaqueToken::generate();
            let stored = self
                .repository
                .create_session(CreateSessionRecord {
                    account_id: account.id,
                    device_label: command.device_label,
                    refresh_token_hash: *TokenDigest::from_token(&refresh_token).as_bytes(),
                    expires_at_epoch_ms: now + REFRESH_TOKEN_TTL_MS,
                })
                .await?;
            return Ok(GoogleLoginOutcome::Session(Box::new(
                self.session_envelope(stored, refresh_token, now)?,
            )));
        }

        if let Some(email) = verified_email.as_deref()
            && let Some(account) = self.repository.account_by_primary_email(email).await?
        {
            let link_payload = GoogleLinkChallengePayload::from_identity(&identity).seal()?;
            let link_token = OpaqueToken::generate();
            self.repository
                .create_google_link_challenge(CreateGoogleLinkChallenge {
                    account_id: account.id,
                    token_hash: *TokenDigest::from_token(&link_token).as_bytes(),
                    identity,
                    expires_at_epoch_ms: now + GOOGLE_CHALLENGE_TTL_MS,
                    encrypted_payload: link_payload,
                })
                .await?;
            return Ok(GoogleLoginOutcome::AccountLinkRequired {
                link_challenge_id: link_token.into_string(),
            });
        }

        let refresh_token = OpaqueToken::generate();
        let stored = self
            .repository
            .create_external_identity_session(
                identity,
                CreateSessionRecord {
                    account_id: String::new(),
                    device_label: command.device_label,
                    refresh_token_hash: *TokenDigest::from_token(&refresh_token).as_bytes(),
                    expires_at_epoch_ms: now + REFRESH_TOKEN_TTL_MS,
                },
            )
            .await?;
        Ok(GoogleLoginOutcome::Session(Box::new(
            self.session_envelope(stored, refresh_token, now)?,
        )))
    }

    pub async fn link_google(&self, command: LinkGoogleCommand) -> CanopyResult<()> {
        let now = self.clock.now_epoch_ms();
        self.repository
            .link_external_identity(
                &command.principal.account_id,
                &command.link_challenge_id,
                now,
            )
            .await
    }

    pub async fn unlink_google(&self, command: UnlinkGoogleCommand) -> CanopyResult<()> {
        self.repository
            .unlink_external_identity(&command.principal.account_id, "google")
            .await
    }
    pub async fn login_password(
        &self,
        command: LoginPasswordCommand,
    ) -> CanopyResult<SessionEnvelope> {
        let normalized_email = normalize_email(&command.email)?;
        let now = self.clock.now_epoch_ms();
        let bucket = self.rate_limit_bucket(
            "login_password_failure",
            &normalized_email,
            now,
            LOGIN_FAILURE_LIMIT,
        );
        self.ensure_not_rate_limited(&bucket).await?;

        let Some(login) = self
            .repository
            .password_login_record(&normalized_email)
            .await?
        else {
            self.record_rate_limit_hit(bucket).await?;
            return Err(CanopyError::unauthenticated("invalid email or password"));
        };

        if login.account.status != AccountStatus::Active {
            self.record_rate_limit_hit(bucket).await?;
            return Err(CanopyError::unauthenticated("invalid email or password"));
        }

        let password_hasher = self.password_hasher.clone();
        let password = command.password;
        let password_for_verify = password.clone();
        let password_hash_phc = login.password_hash_phc.clone();
        let verification = run_password_work(move || {
            password_hasher.verify(&password_for_verify, &password_hash_phc)
        })
        .await?;

        if !verification.valid {
            self.record_rate_limit_hit(bucket).await?;
            return Err(CanopyError::unauthenticated("invalid email or password"));
        }

        if verification.needs_rehash {
            let password_hasher = self.password_hasher.clone();
            match run_password_work(move || password_hasher.hash(&password)).await {
                Ok(password_hash_phc) => {
                    self.repository
                        .update_password_hash(
                            &login.account.id,
                            &password_hash_phc,
                            PASSWORD_POLICY_VERSION,
                        )
                        .await?;
                }
                // Existing credentials remain usable even when their password no longer meets
                // the policy for newly created or replacement passwords.
                Err(CanopyError::InvalidArgument(_)) => {}
                Err(error) => return Err(error),
            }
        }

        let refresh_token = OpaqueToken::generate();
        let stored = self
            .repository
            .create_session(CreateSessionRecord {
                account_id: login.account.id,
                device_label: command.device_label,
                refresh_token_hash: *TokenDigest::from_token(&refresh_token).as_bytes(),
                expires_at_epoch_ms: now + REFRESH_TOKEN_TTL_MS,
            })
            .await?;

        self.session_envelope(stored, refresh_token, now)
    }

    pub async fn authenticate_access_token(
        &self,
        access_token: &str,
    ) -> CanopyResult<AuthenticatedPrincipal> {
        let now = self.clock.now_epoch_ms();
        let claims: AccessTokenClaims = self.access_tokens.verify(access_token, now / 1000)?;
        self.repository
            .validate_active_session(&claims.subject, &claims.session_id, now)
            .await?;
        Ok(AuthenticatedPrincipal {
            account_id: claims.subject,
            session_id: claims.session_id,
        })
    }

    pub async fn get_account(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> CanopyResult<AccountRecord> {
        self.repository
            .account_by_id(&principal.account_id)
            .await?
            .ok_or_else(|| CanopyError::unauthenticated("account is not active"))
    }

    pub async fn delete_account(&self, principal: &AuthenticatedPrincipal) -> CanopyResult<()> {
        self.repository.delete_account(&principal.account_id).await
    }
    pub async fn logout(&self, principal: &AuthenticatedPrincipal) -> CanopyResult<()> {
        self.repository
            .revoke_session(&principal.account_id, &principal.session_id)
            .await
    }

    pub async fn logout_all(&self, principal: &AuthenticatedPrincipal) -> CanopyResult<()> {
        self.repository
            .revoke_all_sessions(&principal.account_id, "logout_all")
            .await
    }

    pub async fn revoke_session(
        &self,
        principal: &AuthenticatedPrincipal,
        session_id: &str,
    ) -> CanopyResult<()> {
        self.repository
            .revoke_session(&principal.account_id, session_id)
            .await
    }

    pub async fn list_sessions(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> CanopyResult<Vec<AuthSession>> {
        self.repository.list_sessions(&principal.account_id).await
    }

    pub async fn refresh_session(
        &self,
        command: RefreshSessionCommand,
    ) -> CanopyResult<SessionEnvelope> {
        let replacement = OpaqueToken::generate();
        let now = self.clock.now_epoch_ms();
        let stored = self
            .repository
            .rotate_refresh_token(RotateRefreshTokenRecord {
                presented_token_hash: *TokenDigest::from_secret(&command.refresh_token).as_bytes(),
                replacement_token_hash: *TokenDigest::from_token(&replacement).as_bytes(),
                replacement_expires_at_epoch_ms: now + REFRESH_TOKEN_TTL_MS,
                now_epoch_ms: now,
            })
            .await?;

        self.session_envelope(stored, replacement, now)
    }

    fn session_envelope(
        &self,
        stored: canopy_core::StoredAuthenticatedSession,
        refresh_token: OpaqueToken,
        now_epoch_ms: u64,
    ) -> CanopyResult<SessionEnvelope> {
        let issued_access_token = self.access_tokens.issue(
            &stored.account.id,
            &stored.session.id,
            now_epoch_ms / 1000,
        )?;
        Ok(SessionEnvelope {
            account: stored.account,
            session: stored.session,
            access_token: issued_access_token.token,
            access_expires_at_epoch_ms: issued_access_token.expires_at_epoch_ms,
            refresh_token: refresh_token.into_string(),
        })
    }
    pub async fn verify_email(&self, command: VerifyEmailCommand) -> CanopyResult<SessionEnvelope> {
        let refresh_token = OpaqueToken::generate();
        let refresh_token_hash = *TokenDigest::from_token(&refresh_token).as_bytes();
        let now = self.clock.now_epoch_ms();
        let stored = self
            .repository
            .activate_email_and_create_session(
                ConsumeChallenge {
                    token_hash: *TokenDigest::from_secret(&command.verification_token).as_bytes(),
                    challenge_type: "email_verification",
                    now_epoch_ms: now,
                },
                CreateSessionRecord {
                    account_id: String::new(),
                    device_label: command.device_label,
                    refresh_token_hash,
                    expires_at_epoch_ms: now + REFRESH_TOKEN_TTL_MS,
                },
            )
            .await?;

        let issued_access_token =
            self.access_tokens
                .issue(&stored.account.id, &stored.session.id, now / 1000)?;

        Ok(SessionEnvelope {
            account: stored.account,
            session: stored.session,
            access_token: issued_access_token.token,
            access_expires_at_epoch_ms: issued_access_token.expires_at_epoch_ms,
            refresh_token: refresh_token.into_string(),
        })
    }
}

fn verification_outbox(
    normalized_email: String,
    verification_token: &str,
    expires_at_epoch_ms: u64,
) -> CanopyResult<super::SealedOutboxPayload> {
    EmailOutboxPayload::email_verification(
        normalized_email,
        verification_token,
        expires_at_epoch_ms,
    )
    .seal()
}

fn password_reset_outbox(
    normalized_email: String,
    reset_token: &str,
    expires_at_epoch_ms: u64,
) -> CanopyResult<super::SealedOutboxPayload> {
    EmailOutboxPayload::password_reset(normalized_email, reset_token, expires_at_epoch_ms).seal()
}

#[derive(Deserialize, Serialize)]
struct GoogleLoginChallengePayload {
    nonce: String,
}

impl GoogleLoginChallengePayload {
    fn seal(&self) -> CanopyResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|error| {
            CanopyError::Internal(format!("failed to encode google login challenge: {error}"))
        })
    }

    fn open(payload: &[u8]) -> CanopyResult<Self> {
        serde_json::from_slice(payload).map_err(|error| {
            CanopyError::Internal(format!("failed to decode google login challenge: {error}"))
        })
    }
}

#[derive(Deserialize, Serialize)]
struct GoogleLinkChallengePayload {
    provider: String,
    provider_subject: String,
    provider_email_at_link_time: Option<String>,
}

impl GoogleLinkChallengePayload {
    fn from_identity(identity: &ExternalIdentityRecord) -> Self {
        Self {
            provider: identity.provider.clone(),
            provider_subject: identity.provider_subject.clone(),
            provider_email_at_link_time: identity.provider_email_at_link_time.clone(),
        }
    }

    fn seal(&self) -> CanopyResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|error| {
            CanopyError::Internal(format!("failed to encode google link challenge: {error}"))
        })
    }
}

fn normalize_email(email: &str) -> CanopyResult<String> {
    let normalized = email.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized.len() > EMAIL_MAX_UTF8_BYTES {
        return Err(CanopyError::InvalidArgument(
            "valid email is required".into(),
        ));
    }

    let mut parts = normalized.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || local.is_empty()
        || domain.is_empty()
        || local.len() > EMAIL_LOCAL_MAX_UTF8_BYTES
        || domain.len() > EMAIL_DOMAIN_MAX_BYTES
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local.bytes().all(valid_email_local_byte)
        || !domain.split('.').all(valid_email_domain_label)
    {
        return Err(CanopyError::InvalidArgument(
            "valid email is required".into(),
        ));
    }
    Ok(normalized)
}

fn valid_email_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte)
}

fn valid_email_domain_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && label
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod input_and_password_work_tests {
    use std::time::Duration;

    use canopy_core::CanopyError;
    use tokio::sync::Semaphore;

    use super::{normalize_email, run_bounded_password_work};

    #[test]
    fn email_policy_accepts_common_trimmed_addresses() {
        assert_eq!(
            normalize_email(" Driver+Car@Example.COM ").unwrap(),
            "driver+car@example.com"
        );
        assert_eq!(
            normalize_email("driver@canopy.test").unwrap(),
            "driver@canopy.test"
        );
    }

    #[test]
    fn email_policy_rejects_malformed_and_oversized_addresses() {
        for invalid in [
            "",
            "foo",
            "a@@example.com",
            "a b@example.com",
            ".a@example.com",
            "a@example..com",
        ] {
            assert!(
                matches!(
                    normalize_email(invalid),
                    Err(CanopyError::InvalidArgument(_))
                ),
                "expected invalid email: {invalid}"
            );
        }
        let oversized = format!("{}@example.com", "a".repeat(245));
        assert!(matches!(
            normalize_email(&oversized),
            Err(CanopyError::InvalidArgument(_))
        ));
    }

    #[tokio::test]
    async fn saturated_password_work_fails_with_rate_limit() {
        let semaphore = Semaphore::new(1);
        let _permit = semaphore.acquire().await.unwrap();

        let error = run_bounded_password_work(&semaphore, Duration::from_millis(1), || {
            Ok::<_, CanopyError>(())
        })
        .await
        .unwrap_err();

        assert!(matches!(error, CanopyError::RateLimited(_)));
    }
}
