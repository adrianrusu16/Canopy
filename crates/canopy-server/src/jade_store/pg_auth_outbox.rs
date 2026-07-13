//! PostgreSQL-backed leased authentication email outbox.

use std::sync::Arc;

use async_trait::async_trait;
use canopy_core::{
    AuthOutboxRepository, CanopyError, CanopyResult, ClaimAuthOutboxBatch, ClaimedAuthOutbox,
    MarkAuthOutboxFailed,
};
use sqlx::Row;

/// PostgreSQL persistence adapter for the supervised authentication email worker.
#[derive(Clone)]
pub struct PgAuthOutboxRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgAuthOutboxRepository {
    /// Creates an outbox repository over the shared PostgreSQL pool.
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

#[async_trait]
impl AuthOutboxRepository for PgAuthOutboxRepository {
    async fn claim_auth_outbox_batch(
        &self,
        command: ClaimAuthOutboxBatch,
    ) -> CanopyResult<Vec<ClaimedAuthOutbox>> {
        if command.batch_size == 0 {
            return Err(CanopyError::InvalidArgument(
                "auth outbox batch_size must be positive".into(),
            ));
        }
        if command.lease_expires_at_epoch_ms <= command.now_epoch_ms {
            return Err(CanopyError::InvalidArgument(
                "auth outbox lease expiry must be after now".into(),
            ));
        }
        let lease_token = parse_uuid(&command.lease_token, "auth outbox lease_token")?;

        let rows = sqlx::query(
            r#"
                WITH candidates AS (
                    SELECT id
                    FROM auth_outbox
                    WHERE delivered_at IS NULL
                      AND failed_at IS NULL
                      AND available_at <= TO_TIMESTAMP($1)
                      AND (
                          lease_expires_at IS NULL
                          OR lease_expires_at <= TO_TIMESTAMP($1)
                      )
                    ORDER BY available_at, created_at
                    FOR UPDATE SKIP LOCKED
                    LIMIT $2
                )
                UPDATE auth_outbox AS outbox
                SET lease_token = $3,
                    lease_expires_at = TO_TIMESTAMP($4),
                    attempts = outbox.attempts + 1
                FROM candidates
                WHERE outbox.id = candidates.id
                RETURNING
                    outbox.id,
                    outbox.kind,
                    outbox.encrypted_payload,
                    outbox.key_id,
                    outbox.attempts,
                    outbox.lease_token
            "#,
        )
        .bind(timestamp_expr(command.now_epoch_ms))
        .bind(i64::from(command.batch_size))
        .bind(lease_token)
        .bind(timestamp_expr(command.lease_expires_at_epoch_ms))
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        rows.into_iter()
            .map(|row| {
                let encrypted_payload = row
                    .try_get::<Option<Vec<u8>>, _>("encrypted_payload")
                    .map_err(db_err)?
                    .ok_or_else(|| {
                        CanopyError::Storage(
                            "pending auth outbox row has no encrypted payload".into(),
                        )
                    })?;
                let attempts = row.try_get::<i32, _>("attempts").map_err(db_err)?;
                let attempts = u32::try_from(attempts).map_err(|_| {
                    CanopyError::Storage("auth outbox attempts is outside u32 range".into())
                })?;

                Ok(ClaimedAuthOutbox {
                    id: row
                        .try_get::<uuid::Uuid, _>("id")
                        .map_err(db_err)?
                        .to_string(),
                    kind: row.try_get("kind").map_err(db_err)?,
                    encrypted_payload,
                    key_id: row.try_get("key_id").map_err(db_err)?,
                    attempts,
                    lease_token: row
                        .try_get::<uuid::Uuid, _>("lease_token")
                        .map_err(db_err)?
                        .to_string(),
                })
            })
            .collect()
    }

    async fn mark_auth_outbox_delivered(
        &self,
        id: &str,
        lease_token: &str,
        delivered_at_epoch_ms: u64,
    ) -> CanopyResult<bool> {
        let id = parse_uuid(id, "auth outbox id")?;
        let lease_token = parse_uuid(lease_token, "auth outbox lease_token")?;
        let updated = sqlx::query_scalar::<_, bool>(
            r#"
                UPDATE auth_outbox
                SET delivered_at = TO_TIMESTAMP($3),
                    encrypted_payload = NULL,
                    lease_token = NULL,
                    lease_expires_at = NULL,
                    last_error_kind = NULL
                WHERE id = $1
                  AND lease_token = $2
                  AND delivered_at IS NULL
                  AND failed_at IS NULL
                RETURNING TRUE
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(timestamp_expr(delivered_at_epoch_ms))
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(updated.unwrap_or(false))
    }

    async fn mark_auth_outbox_failed(&self, command: MarkAuthOutboxFailed) -> CanopyResult<bool> {
        let id = parse_uuid(&command.id, "auth outbox id")?;
        let lease_token = parse_uuid(&command.lease_token, "auth outbox lease_token")?;
        let failed_at = command.failed_at_epoch_ms.map(timestamp_expr);
        let updated = sqlx::query_scalar::<_, bool>(
            r#"
                UPDATE auth_outbox
                SET available_at = TO_TIMESTAMP($4),
                    failed_at = CASE
                        WHEN $5::DOUBLE PRECISION IS NULL THEN NULL
                        ELSE TO_TIMESTAMP($5)
                    END,
                    last_error_kind = $3,
                    lease_token = NULL,
                    lease_expires_at = NULL
                WHERE id = $1
                  AND lease_token = $2
                  AND delivered_at IS NULL
                  AND failed_at IS NULL
                RETURNING TRUE
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(command.error_kind.as_str())
        .bind(timestamp_expr(command.available_at_epoch_ms))
        .bind(failed_at)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(updated.unwrap_or(false))
    }
}
