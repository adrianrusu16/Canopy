//! PostgreSQL-backed authentication identity repository.

use std::sync::Arc;

use async_trait::async_trait;
use canopy_core::{
    AccountRecord, AccountStatus, AuthSession, CanopyError, CanopyResult, ConsumeChallenge,
    CreateSessionRecord, ExternalIdentityRecord, IdentityRepository, PasswordLoginRecord,
    RegisterPasswordRecord, RotateRefreshTokenRecord, StoredAuthenticatedSession,
};
use sqlx::Row;

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

fn unsupported(operation: &str) -> CanopyError {
    CanopyError::Internal(format!(
        "identity repository operation not implemented yet: {operation}"
    ))
}

fn parse_uuid(value: &str, field: &str) -> CanopyResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .map_err(|error| CanopyError::InvalidArgument(format!("invalid {field}: {error}")))
}

fn timestamp_expr(epoch_ms: u64) -> f64 {
    epoch_ms as f64 / 1000.0
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
        revoked_at_epoch_ms: row
            .try_get::<Option<i64>, _>("revoked_at_epoch_ms")
            .map_err(db_err)?
            .map(|value| value.max(0) as u64),
    })
}

#[async_trait]
impl IdentityRepository for PgIdentityRepository {
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
        _identity: &ExternalIdentityRecord,
    ) -> CanopyResult<Option<AccountRecord>> {
        Err(unsupported("account_by_external_identity"))
    }

    async fn delete_account(&self, account_id: &str) -> CanopyResult<()> {
        let _ = parse_uuid(account_id, "account_id")?;
        Err(unsupported("delete_account"))
    }
}
