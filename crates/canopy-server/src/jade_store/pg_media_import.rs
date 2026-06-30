//! PostgreSQL persistence for recoverable owner-scoped media imports.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::Row;

use canopy_core::{
    CanopyError, CanopyResult, MediaImportRepository, PendingImportOutcome, PendingMediaImport,
};

#[derive(Clone)]
pub struct PgMediaImportRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgMediaImportRepository {
    /// Creates a local-media import repository backed by the given pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    async fn duplicate_track_id(&self, checksum_sha256: &str) -> CanopyResult<Option<String>> {
        let row = sqlx::query(
            r#"
                SELECT t.id
                FROM audio_assets aa
                JOIN tracks t ON t.id = aa.track_id
                WHERE LOWER(aa.checksum_sha256) = LOWER($1)
                LIMIT 1
            "#,
        )
        .bind(checksum_sha256)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(row.map(|row| row.get::<uuid::Uuid, _>("id").to_string()))
    }
}

#[async_trait]
impl MediaImportRepository for PgMediaImportRepository {
    async fn find_track_by_audio_checksum(
        &self,
        checksum_sha256: &str,
    ) -> CanopyResult<Option<String>> {
        self.duplicate_track_id(checksum_sha256).await
    }

    async fn insert_pending(
        &self,
        pending: &PendingMediaImport,
    ) -> CanopyResult<PendingImportOutcome> {
        let track_id = parse_uuid(&pending.track_id, "track_id")?;
        let owner_profile_id = parse_uuid(&pending.owner_profile_id, "owner_profile_id")?;
        if pending.audio.track_id != pending.track_id {
            return Err(CanopyError::InvalidArgument(
                "audio track_id must match pending track_id".into(),
            ));
        }
        let duration_ms = i32::try_from(pending.duration_ms).map_err(|_| {
            CanopyError::InvalidArgument("duration_ms exceeds PostgreSQL INTEGER range".into())
        })?;
        let audio_duration_ms = i64::try_from(pending.audio.duration_ms).map_err(|_| {
            CanopyError::InvalidArgument("audio duration_ms exceeds PostgreSQL BIGINT range".into())
        })?;
        let size_bytes = i64::try_from(pending.audio.size_bytes).map_err(|_| {
            CanopyError::InvalidArgument("audio size_bytes exceeds PostgreSQL BIGINT range".into())
        })?;

        if let Some(existing) = self
            .duplicate_track_id(&pending.audio.checksum_sha256)
            .await?
        {
            return Ok(PendingImportOutcome::Duplicate { track_id: existing });
        }

        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let artist_id = match sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT id FROM artists WHERE LOWER(name) = LOWER($1) ORDER BY created_at, id LIMIT 1",
        )
        .bind(&pending.artist)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        {
            Some(id) => id,
            None => sqlx::query_scalar::<_, uuid::Uuid>(
                "INSERT INTO artists (name, sort_name) VALUES ($1, $1) RETURNING id",
            )
            .bind(&pending.artist)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?,
        };
        let album_id = match sqlx::query_scalar::<_, uuid::Uuid>(
            r#"
                SELECT id FROM albums
                WHERE artist_id = $1 AND LOWER(title) = LOWER($2)
                ORDER BY created_at, id
                LIMIT 1
            "#,
        )
        .bind(artist_id)
        .bind(&pending.album)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        {
            Some(id) => id,
            None => sqlx::query_scalar::<_, uuid::Uuid>(
                "INSERT INTO albums (title, artist_id) VALUES ($1, $2) RETURNING id",
            )
            .bind(&pending.album)
            .bind(artist_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?,
        };

        sqlx::query(
            r#"
                INSERT INTO tracks (
                    id, title, artist_id, album_id, duration_ms,
                    artwork_storage_key, visibility, ingest_status,
                    owner_profile_id, ingest_source
                )
                VALUES ($1, $2, $3, $4, $5, $6, 'personal', 'pending', $7, 'local_admin')
            "#,
        )
        .bind(track_id)
        .bind(&pending.title)
        .bind(artist_id)
        .bind(album_id)
        .bind(duration_ms)
        .bind(&pending.artwork_storage_key)
        .bind(owner_profile_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        let asset_result = sqlx::query(
            r#"
                INSERT INTO audio_assets (
                    track_id, codec, content_type, storage_key,
                    size_bytes, checksum_sha256, duration_ms
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(track_id)
        .bind(&pending.audio.codec)
        .bind(&pending.audio.content_type)
        .bind(&pending.audio.storage_key)
        .bind(size_bytes)
        .bind(&pending.audio.checksum_sha256)
        .bind(audio_duration_ms)
        .execute(&mut *tx)
        .await;

        if let Err(error) = asset_result {
            let checksum_race = is_checksum_unique_violation(&error);
            tx.rollback().await.map_err(db_err)?;
            if checksum_race
                && let Some(existing) = self
                    .duplicate_track_id(&pending.audio.checksum_sha256)
                    .await?
            {
                return Ok(PendingImportOutcome::Duplicate { track_id: existing });
            }
            return Err(db_err(error));
        }

        tx.commit().await.map_err(db_err)?;
        Ok(PendingImportOutcome::Inserted)
    }

    async fn mark_ready(&self, track_id: &str) -> CanopyResult<()> {
        let track_uuid = parse_uuid(track_id, "track_id")?;
        let result = sqlx::query(
            r#"
                UPDATE tracks
                SET ingest_status = 'ready', updated_at = NOW()
                WHERE id = $1
                  AND visibility = 'personal'
                  AND ingest_source = 'local_admin'
                  AND ingest_status = 'pending'
            "#,
        )
        .bind(track_uuid)
        .execute(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        if result.rows_affected() != 1 {
            return Err(CanopyError::not_found("pending local media", track_id));
        }
        Ok(())
    }
}

fn parse_uuid(value: &str, field: &str) -> CanopyResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .map_err(|error| CanopyError::InvalidArgument(format!("invalid {field}: {error}")))
}

fn is_checksum_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database)
            if database.constraint() == Some("uq_audio_assets_checksum_sha256")
    )
}

fn db_err(error: sqlx::Error) -> CanopyError {
    CanopyError::Storage(error.to_string())
}
