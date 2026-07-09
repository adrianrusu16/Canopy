use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use canopy_core::{
    AccountRecord, AccountStatus, AuthSession, CanopyError, CanopyResult, ConsumeChallenge,
    CreateSessionRecord, IdentityRepository, RegisterPasswordRecord, RotateRefreshTokenRecord,
};

use super::{
    AccessTokenClaims, Ed25519AccessTokenIssuer, OpaqueToken, PasswordHasher, TokenDigest,
};

const PASSWORD_POLICY_VERSION: u32 = 1;
const VERIFICATION_TOKEN_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const REFRESH_TOKEN_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

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

pub struct RegisterPasswordCommand {
    pub email: String,
    pub password: String,
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
}

pub struct IdentityService {
    repository: Arc<dyn IdentityRepository>,
    password_hasher: Arc<dyn PasswordHasher>,
    access_tokens: Ed25519AccessTokenIssuer,
    clock: Arc<dyn Clock>,
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
        }
    }

    pub async fn register_password(&self, command: RegisterPasswordCommand) -> CanopyResult<()> {
        let normalized_email = normalize_email(&command.email)?;
        let password_hasher = self.password_hasher.clone();
        let password = command.password;
        let password_hash_phc =
            tokio::task::spawn_blocking(move || password_hasher.hash(&password))
                .await
                .map_err(|error| {
                    CanopyError::Internal(format!("password task failed: {error}"))
                })??;

        let verification_token = OpaqueToken::generate();
        let verification_token_hash = *TokenDigest::from_token(&verification_token).as_bytes();
        let now = self.clock.now_epoch_ms();

        self.repository
            .register_password(RegisterPasswordRecord {
                normalized_email,
                password_hash_phc,
                policy_version: PASSWORD_POLICY_VERSION,
                verification_token_hash,
                verification_expires_at_epoch_ms: now + VERIFICATION_TOKEN_TTL_MS,
                encrypted_outbox_payload: vec![0; 32],
                outbox_key_id: "pending-outbox-encryption".into(),
            })
            .await
    }

    pub async fn login_password(
        &self,
        command: LoginPasswordCommand,
    ) -> CanopyResult<SessionEnvelope> {
        let normalized_email = normalize_email(&command.email)?;
        let Some(login) = self
            .repository
            .password_login_record(&normalized_email)
            .await?
        else {
            return Err(CanopyError::unauthenticated("invalid email or password"));
        };

        if login.account.status != AccountStatus::Active {
            return Err(CanopyError::unauthenticated("invalid email or password"));
        }

        let password_hasher = self.password_hasher.clone();
        let password = command.password;
        let password_for_verify = password.clone();
        let password_hash_phc = login.password_hash_phc.clone();
        let verification = tokio::task::spawn_blocking(move || {
            password_hasher.verify(&password_for_verify, &password_hash_phc)
        })
        .await
        .map_err(|error| CanopyError::Internal(format!("password task failed: {error}")))??;

        if !verification.valid {
            return Err(CanopyError::unauthenticated("invalid email or password"));
        }

        if verification.needs_rehash {
            let password_hasher = self.password_hasher.clone();
            let password_hash_phc =
                tokio::task::spawn_blocking(move || password_hasher.hash(&password))
                    .await
                    .map_err(|error| {
                        CanopyError::Internal(format!("password task failed: {error}"))
                    })??;
            self.repository
                .update_password_hash(
                    &login.account.id,
                    &password_hash_phc,
                    PASSWORD_POLICY_VERSION,
                )
                .await?;
        }

        let refresh_token = OpaqueToken::generate();
        let now = self.clock.now_epoch_ms();
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
        let access_token = self.access_tokens.issue(
            &stored.account.id,
            &stored.session.id,
            now_epoch_ms / 1000,
        )?;
        Ok(SessionEnvelope {
            account: stored.account,
            session: stored.session,
            access_token,
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

        let access_token =
            self.access_tokens
                .issue(&stored.account.id, &stored.session.id, now / 1000)?;

        Ok(SessionEnvelope {
            account: stored.account,
            session: stored.session,
            access_token,
            refresh_token: refresh_token.into_string(),
        })
    }
}

fn normalize_email(email: &str) -> CanopyResult<String> {
    let normalized = email.trim().to_ascii_lowercase();
    if normalized.is_empty() || !normalized.contains('@') {
        return Err(CanopyError::InvalidArgument(
            "valid email is required".into(),
        ));
    }
    Ok(normalized)
}
