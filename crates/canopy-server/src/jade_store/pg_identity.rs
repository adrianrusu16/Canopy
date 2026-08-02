//! PostgreSQL-backed authentication identity repository.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::identity::TokenDigest;
use async_trait::async_trait;
use canopy_core::{
    AccountRecord, AccountStatus, AuthSession, CanopyError, CanopyResult, ChangePasswordRecord,
    CompletePasswordResetRecord, ConsumeChallenge, ConsumedGoogleLoginChallenge,
    CreateEmailVerificationChallenge, CreateGoogleLinkChallenge, CreateGoogleLoginChallenge,
    CreatePasswordResetChallenge, CreateSessionRecord, ExternalIdentityRecord, IdentityRepository,
    PasswordCredentialRecord, PasswordLoginRecord, RateLimitBucket, RateLimitState,
    RegisterPasswordRecord, RotateRefreshTokenRecord, StoredAuthenticatedSession,
};
use sqlx::Row;

const RATE_LIMIT_WINDOW_MS: u64 = 15 * 60 * 1000;

#[derive(Clone)]
pub struct PgIdentityRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgIdentityRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

fn db_err(error: sqlx::Error) -> CanopyError {
    CanopyError::Storage(error.to_string())
}

fn parse_uuid(value: &str, field: &str) -> CanopyResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .map_err(|error| CanopyError::InvalidArgument(format!("invalid {field}: {error}")))
}

fn timestamp_expr(epoch_ms: u64) -> f64 {
    epoch_ms as f64 / 1000.0
}

fn validate_rate_limit_bucket(bucket: &RateLimitBucket) -> CanopyResult<()> {
    if bucket.operation.trim().is_empty() || bucket.operation.len() > 64 {
        return Err(CanopyError::InvalidArgument(
            "rate-limit operation must be between 1 and 64 bytes".into(),
        ));
    }
    if bucket.max_attempts == 0 {
        return Err(CanopyError::InvalidArgument(
            "rate-limit max_attempts must be positive".into(),
        ));
    }
    if bucket.window_end_epoch_ms <= bucket.window_start_epoch_ms {
        return Err(CanopyError::InvalidArgument(
            "rate-limit window_end must be after window_start".into(),
        ));
    }
    Ok(())
}

fn rate_limit_state(request_count: u32, max_attempts: u32) -> RateLimitState {
    RateLimitState {
        request_count,
        limited: request_count >= max_attempts,
    }
}

fn epoch_ms(row: &sqlx::postgres::PgRow, column: &str) -> u64 {
    row.try_get::<i64, _>(column).unwrap_or(0).max(0) as u64
}

fn account_status(value: &str) -> CanopyResult<AccountStatus> {
    match value {
        "pending_email_verification" => Ok(AccountStatus::PendingEmailVerification),
        "active" => Ok(AccountStatus::Active),
        "disabled" => Ok(AccountStatus::Disabled),
        "deletion_pending" => Ok(AccountStatus::DeletionPending),
        "deleted" => Ok(AccountStatus::Deleted),
        other => Err(CanopyError::Storage(format!(
            "unknown account status stored in database: {other}"
        ))),
    }
}

fn account_from_row(row: &sqlx::postgres::PgRow) -> CanopyResult<AccountRecord> {
    Ok(AccountRecord {
        id: row
            .try_get::<uuid::Uuid, _>("account_id")
            .map_err(db_err)?
            .to_string(),
        status: account_status(&row.try_get::<String, _>("account_status").map_err(db_err)?)?,
        primary_email: row
            .try_get::<Option<String>, _>("primary_email")
            .map_err(db_err)?,
        created_at_epoch_ms: epoch_ms(row, "created_at_epoch_ms"),
    })
}

fn session_from_row(row: &sqlx::postgres::PgRow) -> CanopyResult<AuthSession> {
    Ok(AuthSession {
        id: row
            .try_get::<uuid::Uuid, _>("session_id")
            .map_err(db_err)?
            .to_string(),
        account_id: row
            .try_get::<uuid::Uuid, _>("session_account_id")
            .map_err(db_err)?
            .to_string(),
        device_label: row.try_get("device_label").map_err(db_err)?,
        expires_at_epoch_ms: epoch_ms(row, "expires_at_epoch_ms"),
        created_at_epoch_ms: epoch_ms(row, "session_created_at_epoch_ms"),
        last_used_at_epoch_ms: epoch_ms(row, "last_used_at_epoch_ms"),
        revoked_at_epoch_ms: row
            .try_get::<Option<i64>, _>("revoked_at_epoch_ms")
            .map_err(db_err)?
            .map(|value| value.max(0) as u64),
    })
}

fn validate_google_identity(identity: &ExternalIdentityRecord) -> CanopyResult<()> {
    if identity.provider != "google" {
        return Err(CanopyError::InvalidArgument(
            "only google external identities are supported".into(),
        ));
    }
    if identity.provider_subject.trim().is_empty() {
        return Err(CanopyError::InvalidArgument(
            "external identity subject is required".into(),
        ));
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct StoredExternalIdentityPayload {
    provider: String,
    provider_subject: String,
    provider_email_at_link_time: Option<String>,
}

impl StoredExternalIdentityPayload {
    fn into_record(self) -> ExternalIdentityRecord {
        ExternalIdentityRecord {
            provider: self.provider,
            provider_subject: self.provider_subject,
            provider_email_at_link_time: self.provider_email_at_link_time,
        }
    }
}

#[async_trait]
impl IdentityRepository for PgIdentityRepository {
    async fn rate_limit_state(&self, bucket: RateLimitBucket) -> CanopyResult<RateLimitState> {
        validate_rate_limit_bucket(&bucket)?;
        let count: Option<i32> = sqlx::query_scalar(
            r#"
                SELECT request_count
                FROM auth_rate_limits
                WHERE operation = $1
                  AND subject_hash = $2
                  AND window_start = to_timestamp($3)
            "#,
        )
        .bind(bucket.operation)
        .bind(bucket.subject_hash.as_slice())
        .bind(timestamp_expr(bucket.window_start_epoch_ms))
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(rate_limit_state(
            count.unwrap_or(0).max(0) as u32,
            bucket.max_attempts,
        ))
    }

    async fn record_rate_limit_hit(&self, bucket: RateLimitBucket) -> CanopyResult<RateLimitState> {
        validate_rate_limit_bucket(&bucket)?;
        let count: i32 = sqlx::query_scalar(
            r#"
                INSERT INTO auth_rate_limits (
                    operation, subject_hash, window_start, window_end, request_count
                )
                VALUES ($1, $2, to_timestamp($3), to_timestamp($4), 1)
                ON CONFLICT (operation, subject_hash, window_start)
                DO UPDATE SET
                    request_count = auth_rate_limits.request_count + 1,
                    window_end = EXCLUDED.window_end
                RETURNING request_count
            "#,
        )
        .bind(bucket.operation)
        .bind(bucket.subject_hash.as_slice())
        .bind(timestamp_expr(bucket.window_start_epoch_ms))
        .bind(timestamp_expr(bucket.window_end_epoch_ms))
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(rate_limit_state(count.max(0) as u32, bucket.max_attempts))
    }

    async fn create_google_login_challenge(
        &self,
        record: CreateGoogleLoginChallenge,
    ) -> CanopyResult<String> {
        let challenge_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                INSERT INTO auth_challenges (
                    challenge_type, token_hash, encrypted_payload, expires_at
                )
                VALUES ('google_login_nonce'::auth_challenge_type, $1, $2, to_timestamp($3))
                RETURNING id
            "#,
        )
        .bind(record.nonce_hash.as_slice())
        .bind(record.encrypted_payload)
        .bind(timestamp_expr(record.expires_at_epoch_ms))
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(challenge_id.to_string())
    }

    async fn consume_google_login_challenge(
        &self,
        challenge_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<ConsumedGoogleLoginChallenge> {
        let challenge_id = parse_uuid(challenge_id, "challenge_id")?;
        let row = sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = to_timestamp($2), attempts = attempts + 1
                WHERE id = $1
                  AND challenge_type = 'google_login_nonce'::auth_challenge_type
                  AND consumed_at IS NULL
                  AND expires_at > to_timestamp($2)
                  AND attempts < max_attempts
                RETURNING encrypted_payload
            "#,
        )
        .bind(challenge_id)
        .bind(timestamp_expr(now_epoch_ms))
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        let Some(row) = row else {
            return Err(CanopyError::unauthenticated(
                "invalid or consumed google login challenge",
            ));
        };
        Ok(ConsumedGoogleLoginChallenge {
            encrypted_payload: row
                .try_get::<Option<Vec<u8>>, _>("encrypted_payload")
                .map_err(db_err)?
                .ok_or_else(|| {
                    CanopyError::Storage("google challenge payload is missing".into())
                })?,
        })
    }

    async fn register_password(&self, record: RegisterPasswordRecord) -> CanopyResult<()> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let account_id: uuid::Uuid =
            sqlx::query_scalar("INSERT INTO accounts DEFAULT VALUES RETURNING id")
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?;

        let email_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                INSERT INTO account_emails (account_id, normalized_email, is_primary)
                VALUES ($1, $2, TRUE)
                RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(&record.normalized_email)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO password_credentials (account_id, password_hash_phc, policy_version)
                VALUES ($1, $2, $3)
            "#,
        )
        .bind(account_id)
        .bind(&record.password_hash_phc)
        .bind(record.policy_version as i32)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_challenges (
                    account_id, email_id, challenge_type, token_hash, expires_at
                )
                VALUES ($1, $2, 'email_verification'::auth_challenge_type, $3, to_timestamp($4))
            "#,
        )
        .bind(account_id)
        .bind(email_id)
        .bind(record.verification_token_hash.as_slice())
        .bind(timestamp_expr(record.verification_expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_outbox (kind, encrypted_payload, key_id)
                VALUES ('email_verification', $1, $2)
            "#,
        )
        .bind(record.encrypted_outbox_payload)
        .bind(record.outbox_key_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn create_email_verification_challenge(
        &self,
        record: CreateEmailVerificationChallenge,
    ) -> CanopyResult<()> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    ae.id AS email_id,
                    a.status::text AS account_status
                FROM account_emails ae
                JOIN accounts a ON a.id = ae.account_id
                WHERE ae.normalized_email = $1
                  AND ae.deleted_at IS NULL
                  AND ae.is_primary
                FOR UPDATE OF a, ae
            "#,
        )
        .bind(&record.normalized_email)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let Some(row) = row else {
            tx.commit().await.map_err(db_err)?;
            return Ok(());
        };
        if account_status(&row.try_get::<String, _>("account_status").map_err(db_err)?)?
            != AccountStatus::PendingEmailVerification
        {
            tx.commit().await.map_err(db_err)?;
            return Ok(());
        }

        let account_id: uuid::Uuid = row.try_get("account_id").map_err(db_err)?;
        let email_id: uuid::Uuid = row.try_get("email_id").map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = COALESCE(consumed_at, NOW()),
                    attempts = max_attempts
                WHERE account_id = $1
                  AND email_id = $2
                  AND challenge_type = 'email_verification'::auth_challenge_type
                  AND consumed_at IS NULL
            "#,
        )
        .bind(account_id)
        .bind(email_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_challenges (
                    account_id, email_id, challenge_type, token_hash, expires_at
                )
                VALUES ($1, $2, 'email_verification'::auth_challenge_type, $3, to_timestamp($4))
            "#,
        )
        .bind(account_id)
        .bind(email_id)
        .bind(record.token_hash.as_slice())
        .bind(timestamp_expr(record.expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_outbox (kind, encrypted_payload, key_id)
                VALUES ('email_verification', $1, $2)
            "#,
        )
        .bind(record.encrypted_outbox_payload)
        .bind(record.outbox_key_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn create_password_reset_challenge(
        &self,
        record: CreatePasswordResetChallenge,
    ) -> CanopyResult<()> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status
                FROM account_emails ae
                JOIN accounts a ON a.id = ae.account_id
                JOIN password_credentials pc ON pc.account_id = a.id
                WHERE ae.normalized_email = $1
                  AND ae.deleted_at IS NULL
                  AND ae.is_primary
                FOR UPDATE OF a
            "#,
        )
        .bind(&record.normalized_email)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let Some(row) = row else {
            tx.commit().await.map_err(db_err)?;
            return Ok(());
        };
        if account_status(&row.try_get::<String, _>("account_status").map_err(db_err)?)?
            != AccountStatus::Active
        {
            tx.commit().await.map_err(db_err)?;
            return Ok(());
        }

        let account_id: uuid::Uuid = row.try_get("account_id").map_err(db_err)?;
        sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = COALESCE(consumed_at, NOW()),
                    attempts = max_attempts
                WHERE account_id = $1
                  AND challenge_type = 'password_reset'::auth_challenge_type
                  AND consumed_at IS NULL
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_challenges (
                    account_id, challenge_type, token_hash, expires_at
                )
                VALUES ($1, 'password_reset'::auth_challenge_type, $2, to_timestamp($3))
            "#,
        )
        .bind(account_id)
        .bind(record.token_hash.as_slice())
        .bind(timestamp_expr(record.expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_outbox (kind, encrypted_payload, key_id)
                VALUES ('password_reset', $1, $2)
            "#,
        )
        .bind(record.encrypted_outbox_payload)
        .bind(record.outbox_key_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn password_credential_for_account(
        &self,
        account_id: &str,
    ) -> CanopyResult<Option<PasswordCredentialRecord>> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms,
                    pc.password_hash_phc,
                    pc.policy_version
                FROM accounts a
                JOIN password_credentials pc ON pc.account_id = a.id
                LEFT JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                WHERE a.id = $1
            "#,
        )
        .bind(account_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        row.map(|row| {
            Ok(PasswordCredentialRecord {
                account: account_from_row(&row)?,
                password_hash_phc: row.try_get("password_hash_phc").map_err(db_err)?,
                policy_version: row.try_get::<i32, _>("policy_version").map_err(db_err)? as u32,
            })
        })
        .transpose()
    }

    async fn complete_password_reset(
        &self,
        record: CompletePasswordResetRecord,
    ) -> CanopyResult<()> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let now = timestamp_expr(record.now_epoch_ms);
        let row = sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = to_timestamp($1), attempts = attempts + 1
                WHERE token_hash = $2
                  AND challenge_type = 'password_reset'::auth_challenge_type
                  AND consumed_at IS NULL
                  AND expires_at > to_timestamp($1)
                  AND attempts < max_attempts
                RETURNING account_id
            "#,
        )
        .bind(now)
        .bind(record.token_hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let Some(row) = row else {
            return Err(CanopyError::unauthenticated(
                "invalid or consumed challenge",
            ));
        };
        let account_id = row
            .try_get::<Option<uuid::Uuid>, _>("account_id")
            .map_err(db_err)?
            .ok_or_else(|| {
                CanopyError::FailedPrecondition("challenge is not account-scoped".into())
            })?;

        sqlx::query(
            r#"
                UPDATE password_credentials
                SET password_hash_phc = $2,
                    policy_version = $3,
                    updated_at = to_timestamp($4),
                    password_changed_at = to_timestamp($4)
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .bind(record.password_hash_phc)
        .bind(record.policy_version as i32)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_sessions
                SET revoked_at = COALESCE(revoked_at, to_timestamp($2)),
                    revocation_reason = COALESCE(revocation_reason, 'password_reset')
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_session_tokens
                SET consumed_at = COALESCE(consumed_at, to_timestamp($2))
                WHERE session_id IN (SELECT id FROM auth_sessions WHERE account_id = $1)
            "#,
        )
        .bind(account_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn change_password(&self, record: ChangePasswordRecord) -> CanopyResult<()> {
        let account_id = parse_uuid(&record.account_id, "account_id")?;
        let current_session_id = parse_uuid(&record.current_session_id, "session_id")?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let active: bool = sqlx::query_scalar(
            r#"
                SELECT EXISTS (
                    SELECT 1
                    FROM auth_sessions s
                    JOIN accounts a ON a.id = s.account_id
                    WHERE s.id = $2
                      AND s.account_id = $1
                      AND a.status = 'active'::account_status
                      AND s.revoked_at IS NULL
                      AND s.expires_at > NOW()
                )
            "#,
        )
        .bind(account_id)
        .bind(current_session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        if !active {
            return Err(CanopyError::unauthenticated("session is not active"));
        }

        sqlx::query(
            r#"
                UPDATE password_credentials
                SET password_hash_phc = $2,
                    policy_version = $3,
                    updated_at = NOW(),
                    password_changed_at = NOW()
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .bind(record.password_hash_phc)
        .bind(record.policy_version as i32)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_sessions
                SET revoked_at = COALESCE(revoked_at, NOW()),
                    revocation_reason = COALESCE(revocation_reason, 'password_change')
                WHERE account_id = $1 AND id <> $2
            "#,
        )
        .bind(account_id)
        .bind(current_session_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_session_tokens
                SET consumed_at = COALESCE(consumed_at, NOW())
                WHERE session_id IN (
                    SELECT id FROM auth_sessions WHERE account_id = $1 AND id <> $2
                )
            "#,
        )
        .bind(account_id)
        .bind(current_session_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn password_login_record(
        &self,
        normalized_email: &str,
    ) -> CanopyResult<Option<PasswordLoginRecord>> {
        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms,
                    pc.password_hash_phc,
                    pc.policy_version
                FROM account_emails ae
                JOIN accounts a ON a.id = ae.account_id
                JOIN password_credentials pc ON pc.account_id = a.id
                WHERE ae.normalized_email = $1
                  AND ae.deleted_at IS NULL
                  AND ae.is_primary
            "#,
        )
        .bind(normalized_email)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        row.map(|row| {
            Ok(PasswordLoginRecord {
                account: account_from_row(&row)?,
                password_hash_phc: row.try_get("password_hash_phc").map_err(db_err)?,
                policy_version: row.try_get::<i32, _>("policy_version").map_err(db_err)? as u32,
            })
        })
        .transpose()
    }

    async fn update_password_hash(
        &self,
        account_id: &str,
        password_hash_phc: &str,
        policy_version: u32,
    ) -> CanopyResult<()> {
        let account_id = parse_uuid(account_id, "account_id")?;
        sqlx::query(
            r#"
                UPDATE password_credentials
                SET password_hash_phc = $2,
                    policy_version = $3,
                    updated_at = NOW(),
                    password_changed_at = NOW()
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .bind(password_hash_phc)
        .bind(policy_version as i32)
        .execute(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(())
    }

    async fn activate_email_and_create_session(
        &self,
        challenge: ConsumeChallenge,
        session: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let now = timestamp_expr(challenge.now_epoch_ms);

        let consumed = sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = to_timestamp($1), attempts = attempts + 1
                WHERE token_hash = $2
                  AND challenge_type = $3::auth_challenge_type
                  AND consumed_at IS NULL
                  AND expires_at > to_timestamp($1)
                  AND attempts < max_attempts
                RETURNING account_id, email_id
            "#,
        )
        .bind(now)
        .bind(challenge.token_hash.as_slice())
        .bind(challenge.challenge_type)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let Some(consumed) = consumed else {
            return Err(CanopyError::unauthenticated(
                "invalid or consumed challenge",
            ));
        };
        let account_id = consumed
            .try_get::<Option<uuid::Uuid>, _>("account_id")
            .map_err(db_err)?
            .ok_or_else(|| {
                CanopyError::FailedPrecondition("challenge is not account-scoped".into())
            })?;
        let email_id = consumed
            .try_get::<Option<uuid::Uuid>, _>("email_id")
            .map_err(db_err)?
            .ok_or_else(|| {
                CanopyError::FailedPrecondition("challenge is not email-scoped".into())
            })?;

        sqlx::query(
            r#"
                UPDATE account_emails
                SET verified_at = COALESCE(verified_at, to_timestamp($2))
                WHERE id = $1
            "#,
        )
        .bind(email_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE accounts
                SET status = 'active'::account_status,
                    activated_at = COALESCE(activated_at, to_timestamp($2))
                WHERE id = $1 AND status = 'pending_email_verification'::account_status
            "#,
        )
        .bind(account_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO profiles (external_user_id, history_enabled, account_id)
                VALUES ($1, FALSE, $2)
                ON CONFLICT (account_id) DO NOTHING
            "#,
        )
        .bind(account_id.to_string())
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let session_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                INSERT INTO auth_sessions (account_id, device_label, expires_at)
                VALUES ($1, $2, to_timestamp($3))
                RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(&session.device_label)
        .bind(timestamp_expr(session.expires_at_epoch_ms))
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_session_tokens (session_id, token_hash, expires_at)
                VALUES ($1, $2, to_timestamp($3))
            "#,
        )
        .bind(session_id)
        .bind(session.refresh_token_hash.as_slice())
        .bind(timestamp_expr(session.expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms,
                    s.id AS session_id,
                    s.account_id AS session_account_id,
                    s.device_label,
                    floor(extract(epoch from s.expires_at) * 1000)::bigint AS expires_at_epoch_ms,
                    floor(extract(epoch from s.created_at) * 1000)::bigint AS session_created_at_epoch_ms,
                    floor(extract(epoch from s.last_used_at) * 1000)::bigint AS last_used_at_epoch_ms,
                    CASE
                        WHEN s.revoked_at IS NULL THEN NULL
                        ELSE floor(extract(epoch from s.revoked_at) * 1000)::bigint
                    END AS revoked_at_epoch_ms
                FROM accounts a
                JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                JOIN auth_sessions s ON s.id = $2
                WHERE a.id = $1
            "#,
        )
        .bind(account_id)
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        let account = account_from_row(&row)?;
        let session = session_from_row(&row)?;
        tx.commit().await.map_err(db_err)?;

        Ok(StoredAuthenticatedSession { account, session })
    }

    async fn create_session(
        &self,
        record: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        let account_id = parse_uuid(&record.account_id, "account_id")?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let is_active: Option<bool> = sqlx::query_scalar(
            "SELECT status = 'active'::account_status FROM accounts WHERE id = $1 FOR UPDATE",
        )
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        if is_active != Some(true) {
            return Err(CanopyError::unauthenticated("account is not active"));
        }

        let session_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                INSERT INTO auth_sessions (account_id, device_label, expires_at)
                VALUES ($1, $2, to_timestamp($3))
                RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(&record.device_label)
        .bind(timestamp_expr(record.expires_at_epoch_ms))
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_session_tokens (session_id, token_hash, expires_at)
                VALUES ($1, $2, to_timestamp($3))
            "#,
        )
        .bind(session_id)
        .bind(record.refresh_token_hash.as_slice())
        .bind(timestamp_expr(record.expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms,
                    s.id AS session_id,
                    s.account_id AS session_account_id,
                    s.device_label,
                    floor(extract(epoch from s.expires_at) * 1000)::bigint AS expires_at_epoch_ms,
                    floor(extract(epoch from s.created_at) * 1000)::bigint AS session_created_at_epoch_ms,
                    floor(extract(epoch from s.last_used_at) * 1000)::bigint AS last_used_at_epoch_ms,
                    CASE
                        WHEN s.revoked_at IS NULL THEN NULL
                        ELSE floor(extract(epoch from s.revoked_at) * 1000)::bigint
                    END AS revoked_at_epoch_ms
                FROM auth_sessions s
                JOIN accounts a ON a.id = s.account_id
                JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                WHERE s.id = $1
            "#,
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        let account = account_from_row(&row)?;
        let session = session_from_row(&row)?;
        tx.commit().await.map_err(db_err)?;

        Ok(StoredAuthenticatedSession { account, session })
    }

    async fn rotate_refresh_token(
        &self,
        record: RotateRefreshTokenRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let now = timestamp_expr(record.now_epoch_ms);

        let token_row = sqlx::query(
            r#"
                SELECT
                    t.id AS token_id,
                    t.session_id,
                    t.consumed_at IS NOT NULL AS token_consumed,
                    t.expires_at <= to_timestamp($2) AS token_expired,
                    s.revoked_at IS NOT NULL AS session_revoked,
                    s.expires_at <= to_timestamp($2) AS session_expired
                FROM auth_session_tokens t
                JOIN auth_sessions s ON s.id = t.session_id
                WHERE t.token_hash = $1
                FOR UPDATE OF t, s
            "#,
        )
        .bind(record.presented_token_hash.as_slice())
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let Some(token_row) = token_row else {
            return Err(CanopyError::unauthenticated("invalid refresh token"));
        };

        let session_id: uuid::Uuid = token_row.try_get("session_id").map_err(db_err)?;
        let token_id: uuid::Uuid = token_row.try_get("token_id").map_err(db_err)?;
        let token_consumed: bool = token_row.try_get("token_consumed").map_err(db_err)?;
        let token_expired: bool = token_row.try_get("token_expired").map_err(db_err)?;
        let session_revoked: bool = token_row.try_get("session_revoked").map_err(db_err)?;
        let session_expired: bool = token_row.try_get("session_expired").map_err(db_err)?;

        if token_consumed {
            let window_start_epoch_ms =
                record.now_epoch_ms - (record.now_epoch_ms % RATE_LIMIT_WINDOW_MS);
            sqlx::query(
                r#"
                    INSERT INTO auth_rate_limits (
                        operation, subject_hash, window_start, window_end, request_count
                    )
                    VALUES (
                        'refresh_token_reuse', $1, to_timestamp($2), to_timestamp($3), 1
                    )
                    ON CONFLICT (operation, subject_hash, window_start)
                    DO UPDATE SET
                        request_count = auth_rate_limits.request_count + 1,
                        window_end = EXCLUDED.window_end
                "#,
            )
            .bind(record.presented_token_hash.as_slice())
            .bind(timestamp_expr(window_start_epoch_ms))
            .bind(timestamp_expr(window_start_epoch_ms + RATE_LIMIT_WINDOW_MS))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            sqlx::query(
                r#"
                    UPDATE auth_sessions
                    SET revoked_at = COALESCE(revoked_at, to_timestamp($2)),
                        revocation_reason = COALESCE(revocation_reason, 'refresh_reuse')
                    WHERE id = $1
                "#,
            )
            .bind(session_id)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            sqlx::query(
                r#"
                    UPDATE auth_session_tokens
                    SET consumed_at = COALESCE(consumed_at, to_timestamp($2))
                    WHERE session_id = $1
                "#,
            )
            .bind(session_id)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            tx.commit().await.map_err(db_err)?;
            return Err(CanopyError::unauthenticated("invalid refresh token"));
        }

        if token_expired || session_revoked || session_expired {
            return Err(CanopyError::unauthenticated("invalid refresh token"));
        }

        sqlx::query(
            r#"
                UPDATE auth_session_tokens
                SET consumed_at = to_timestamp($2)
                WHERE id = $1
            "#,
        )
        .bind(token_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_sessions
                SET last_used_at = to_timestamp($2)
                WHERE id = $1
            "#,
        )
        .bind(session_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_session_tokens (session_id, token_hash, expires_at)
                VALUES ($1, $2, to_timestamp($3))
            "#,
        )
        .bind(session_id)
        .bind(record.replacement_token_hash.as_slice())
        .bind(timestamp_expr(record.replacement_expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms,
                    s.id AS session_id,
                    s.account_id AS session_account_id,
                    s.device_label,
                    floor(extract(epoch from s.expires_at) * 1000)::bigint AS expires_at_epoch_ms,
                    floor(extract(epoch from s.created_at) * 1000)::bigint AS session_created_at_epoch_ms,
                    floor(extract(epoch from s.last_used_at) * 1000)::bigint AS last_used_at_epoch_ms,
                    CASE
                        WHEN s.revoked_at IS NULL THEN NULL
                        ELSE floor(extract(epoch from s.revoked_at) * 1000)::bigint
                    END AS revoked_at_epoch_ms
                FROM auth_sessions s
                JOIN accounts a ON a.id = s.account_id
                JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                WHERE s.id = $1
            "#,
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        let account = account_from_row(&row)?;
        let session = session_from_row(&row)?;
        tx.commit().await.map_err(db_err)?;

        Ok(StoredAuthenticatedSession { account, session })
    }

    async fn validate_active_session(
        &self,
        account_id: &str,
        session_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<()> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let session_id = parse_uuid(session_id, "session_id")?;
        let active: bool = sqlx::query_scalar(
            r#"
                SELECT EXISTS (
                    SELECT 1
                    FROM auth_sessions s
                    JOIN accounts a ON a.id = s.account_id
                    WHERE s.id = $1
                      AND s.account_id = $2
                      AND a.status = 'active'::account_status
                      AND s.revoked_at IS NULL
                      AND s.expires_at > to_timestamp($3)
                )
            "#,
        )
        .bind(session_id)
        .bind(account_id)
        .bind(timestamp_expr(now_epoch_ms))
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        if active {
            Ok(())
        } else {
            Err(CanopyError::unauthenticated("session is not active"))
        }
    }

    async fn account_by_id(&self, account_id: &str) -> CanopyResult<Option<AccountRecord>> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms
                FROM accounts a
                LEFT JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                WHERE a.id = $1
            "#,
        )
        .bind(account_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        row.map(|row| account_from_row(&row)).transpose()
    }

    async fn revoke_session(&self, account_id: &str, session_id: &str) -> CanopyResult<()> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let session_id = parse_uuid(session_id, "session_id")?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_sessions
                SET revoked_at = COALESCE(revoked_at, NOW()),
                    revocation_reason = COALESCE(revocation_reason, 'user_logout')
                WHERE id = $1 AND account_id = $2
            "#,
        )
        .bind(session_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_session_tokens
                SET consumed_at = COALESCE(consumed_at, NOW())
                WHERE session_id = $1
                  AND EXISTS (
                      SELECT 1
                      FROM auth_sessions s
                      WHERE s.id = auth_session_tokens.session_id
                        AND s.account_id = $2
                  )
            "#,
        )
        .bind(session_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn revoke_all_sessions(&self, account_id: &str, reason: &str) -> CanopyResult<()> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 64 {
            return Err(CanopyError::InvalidArgument(
                "revocation reason must be between 1 and 64 bytes".into(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_sessions
                SET revoked_at = COALESCE(revoked_at, NOW()),
                    revocation_reason = COALESCE(revocation_reason, $2)
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .bind(reason)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_session_tokens
                SET consumed_at = COALESCE(consumed_at, NOW())
                WHERE session_id IN (
                    SELECT id FROM auth_sessions WHERE account_id = $1
                )
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }

    async fn list_sessions(&self, account_id: &str) -> CanopyResult<Vec<AuthSession>> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let rows = sqlx::query(
            r#"
                SELECT
                    s.id AS session_id,
                    s.account_id AS session_account_id,
                    s.device_label,
                    floor(extract(epoch from s.expires_at) * 1000)::bigint AS expires_at_epoch_ms,
                    floor(extract(epoch from s.created_at) * 1000)::bigint AS session_created_at_epoch_ms,
                    floor(extract(epoch from s.last_used_at) * 1000)::bigint AS last_used_at_epoch_ms,
                    CASE
                        WHEN s.revoked_at IS NULL THEN NULL
                        ELSE floor(extract(epoch from s.revoked_at) * 1000)::bigint
                    END AS revoked_at_epoch_ms
                FROM auth_sessions s
                WHERE s.account_id = $1
                ORDER BY s.created_at DESC, s.id DESC
            "#,
        )
        .bind(account_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        rows.iter().map(session_from_row).collect()
    }

    async fn account_by_external_identity(
        &self,
        identity: &ExternalIdentityRecord,
    ) -> CanopyResult<Option<AccountRecord>> {
        validate_google_identity(identity)?;
        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms
                FROM external_identities ei
                JOIN accounts a ON a.id = ei.account_id
                LEFT JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                WHERE ei.provider = $1::auth_provider
                  AND ei.provider_subject = $2
                  AND a.status = 'active'::account_status
            "#,
        )
        .bind(&identity.provider)
        .bind(&identity.provider_subject)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        row.map(|row| account_from_row(&row)).transpose()
    }

    async fn account_by_primary_email(
        &self,
        normalized_email: &str,
    ) -> CanopyResult<Option<AccountRecord>> {
        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms
                FROM account_emails ae
                JOIN accounts a ON a.id = ae.account_id
                WHERE ae.normalized_email = $1
                  AND ae.deleted_at IS NULL
                  AND ae.is_primary
                  AND ae.verified_at IS NOT NULL
                  AND a.status = 'active'::account_status
            "#,
        )
        .bind(normalized_email)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        row.map(|row| account_from_row(&row)).transpose()
    }

    async fn create_external_identity_session(
        &self,
        identity: ExternalIdentityRecord,
        session: CreateSessionRecord,
    ) -> CanopyResult<StoredAuthenticatedSession> {
        validate_google_identity(&identity)?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let account_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                INSERT INTO accounts (status, activated_at)
                VALUES ('active'::account_status, NOW())
                RETURNING id
            "#,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        if let Some(email) = identity.provider_email_at_link_time.as_deref() {
            sqlx::query(
                r#"
                    INSERT INTO account_emails (
                        account_id, normalized_email, is_primary, verified_at
                    )
                    VALUES ($1, $2, TRUE, NOW())
                "#,
            )
            .bind(account_id)
            .bind(email)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }

        sqlx::query(
            r#"
                INSERT INTO external_identities (
                    account_id, provider, provider_subject, provider_email_at_link_time
                )
                VALUES ($1, $2::auth_provider, $3, $4)
            "#,
        )
        .bind(account_id)
        .bind(&identity.provider)
        .bind(&identity.provider_subject)
        .bind(&identity.provider_email_at_link_time)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO profiles (external_user_id, history_enabled, account_id)
                VALUES ($1, FALSE, $2)
                ON CONFLICT (account_id) DO NOTHING
            "#,
        )
        .bind(account_id.to_string())
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let session_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                INSERT INTO auth_sessions (account_id, device_label, expires_at)
                VALUES ($1, $2, to_timestamp($3))
                RETURNING id
            "#,
        )
        .bind(account_id)
        .bind(&session.device_label)
        .bind(timestamp_expr(session.expires_at_epoch_ms))
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                INSERT INTO auth_session_tokens (session_id, token_hash, expires_at)
                VALUES ($1, $2, to_timestamp($3))
            "#,
        )
        .bind(session_id)
        .bind(session.refresh_token_hash.as_slice())
        .bind(timestamp_expr(session.expires_at_epoch_ms))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let row = sqlx::query(
            r#"
                SELECT
                    a.id AS account_id,
                    a.status::text AS account_status,
                    ae.normalized_email AS primary_email,
                    floor(extract(epoch from a.created_at) * 1000)::bigint AS created_at_epoch_ms,
                    s.id AS session_id,
                    s.account_id AS session_account_id,
                    s.device_label,
                    floor(extract(epoch from s.expires_at) * 1000)::bigint AS expires_at_epoch_ms,
                    floor(extract(epoch from s.created_at) * 1000)::bigint AS session_created_at_epoch_ms,
                    floor(extract(epoch from s.last_used_at) * 1000)::bigint AS last_used_at_epoch_ms,
                    CASE
                        WHEN s.revoked_at IS NULL THEN NULL
                        ELSE floor(extract(epoch from s.revoked_at) * 1000)::bigint
                    END AS revoked_at_epoch_ms
                FROM auth_sessions s
                JOIN accounts a ON a.id = s.account_id
                LEFT JOIN account_emails ae ON ae.account_id = a.id AND ae.is_primary AND ae.deleted_at IS NULL
                WHERE s.id = $1
            "#,
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;

        let account = account_from_row(&row)?;
        let session = session_from_row(&row)?;
        tx.commit().await.map_err(db_err)?;

        Ok(StoredAuthenticatedSession { account, session })
    }

    async fn create_google_link_challenge(
        &self,
        record: CreateGoogleLinkChallenge,
    ) -> CanopyResult<()> {
        validate_google_identity(&record.identity)?;
        let account_id = parse_uuid(&record.account_id, "account_id")?;
        sqlx::query(
            r#"
                INSERT INTO auth_challenges (
                    account_id, challenge_type, token_hash, encrypted_payload, expires_at
                )
                VALUES ($1, 'google_link'::auth_challenge_type, $2, $3, to_timestamp($4))
            "#,
        )
        .bind(account_id)
        .bind(record.token_hash.as_slice())
        .bind(record.encrypted_payload)
        .bind(timestamp_expr(record.expires_at_epoch_ms))
        .execute(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(())
    }

    async fn link_external_identity(
        &self,
        account_id: &str,
        link_challenge_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<()> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let token_hash = *TokenDigest::from_secret(link_challenge_id).as_bytes();
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let row = sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = to_timestamp($3), attempts = attempts + 1
                WHERE token_hash = $1
                  AND account_id = $2
                  AND challenge_type = 'google_link'::auth_challenge_type
                  AND consumed_at IS NULL
                  AND expires_at > to_timestamp($3)
                  AND attempts < max_attempts
                RETURNING encrypted_payload
            "#,
        )
        .bind(token_hash.as_slice())
        .bind(account_id)
        .bind(timestamp_expr(now_epoch_ms))
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let Some(row) = row else {
            return Err(CanopyError::unauthenticated(
                "invalid or consumed google link challenge",
            ));
        };
        let payload = row
            .try_get::<Option<Vec<u8>>, _>("encrypted_payload")
            .map_err(db_err)?
            .ok_or_else(|| CanopyError::Storage("google link payload is missing".into()))?;
        let identity: StoredExternalIdentityPayload =
            serde_json::from_slice(&payload).map_err(|error| {
                CanopyError::Storage(format!("invalid google link payload: {error}"))
            })?;
        let identity = identity.into_record();
        validate_google_identity(&identity)?;

        sqlx::query(
            r#"
                INSERT INTO external_identities (
                    account_id, provider, provider_subject, provider_email_at_link_time
                )
                VALUES ($1, $2::auth_provider, $3, $4)
                ON CONFLICT (provider, provider_subject) DO NOTHING
            "#,
        )
        .bind(account_id)
        .bind(&identity.provider)
        .bind(&identity.provider_subject)
        .bind(&identity.provider_email_at_link_time)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let linked_account_id: uuid::Uuid = sqlx::query_scalar(
            r#"
                SELECT account_id
                FROM external_identities
                WHERE provider = $1::auth_provider AND provider_subject = $2
            "#,
        )
        .bind(&identity.provider)
        .bind(&identity.provider_subject)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        if linked_account_id != account_id {
            return Err(CanopyError::FailedPrecondition(
                "google identity is linked to another account".into(),
            ));
        }

        tx.commit().await.map_err(db_err)
    }

    async fn unlink_external_identity(&self, account_id: &str, provider: &str) -> CanopyResult<()> {
        if provider != "google" {
            return Err(CanopyError::InvalidArgument(
                "only google external identities are supported".into(),
            ));
        }
        let account_id = parse_uuid(account_id, "account_id")?;
        sqlx::query(
            "DELETE FROM external_identities WHERE account_id = $1 AND provider = $2::auth_provider",
        )
        .bind(account_id)
        .bind(provider)
        .execute(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_account(&self, account_id: &str) -> CanopyResult<()> {
        let account_id = parse_uuid(account_id, "account_id")?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let exists: Option<uuid::Uuid> =
            sqlx::query_scalar("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
                .bind(account_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?;
        if exists.is_none() {
            tx.commit().await.map_err(db_err)?;
            return Ok(());
        }

        sqlx::query(
            r#"
                UPDATE accounts
                SET status = 'deleted'::account_status,
                    deleted_at = COALESCE(deleted_at, NOW())
                WHERE id = $1
                  AND status <> 'deleted'::account_status
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE account_emails
                SET deleted_at = COALESCE(deleted_at, NOW()),
                    is_primary = FALSE
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_sessions
                SET revoked_at = COALESCE(revoked_at, NOW()),
                    revocation_reason = COALESCE(revocation_reason, 'account_deleted')
                WHERE account_id = $1
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_session_tokens
                SET consumed_at = COALESCE(consumed_at, NOW())
                WHERE session_id IN (SELECT id FROM auth_sessions WHERE account_id = $1)
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        sqlx::query(
            r#"
                UPDATE auth_challenges
                SET consumed_at = COALESCE(consumed_at, NOW()),
                    attempts = max_attempts
                WHERE account_id = $1
                  AND consumed_at IS NULL
            "#,
        )
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        tx.commit().await.map_err(db_err)
    }
}
