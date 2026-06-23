//! PostgreSQL-backed implementations of the repository ports.
//!
//! This module uses `sqlx` to query the normalized schema described in the
//! architecture document. The domain model `MediaItem` is denormalized at
//! the adapter boundary via JOINs across `tracks`, `artists`, `albums`, and
//! `audio_assets`.
//!
//! # Production Notes
//!
//! * Connection pooling - configured via `PgPoolOptions` in `lib.rs` with
//!   sensible max-connections and acquire-timeout defaults.
//! * Materialized views - `mv_discovery_pool` and `mv_catalog_search` are
//!   refreshed periodically (e.g., via a cron job or a background task). The
//!   discovery service reads from `mv_discovery_pool` for O(1) shuffle selection.
//! * Indexes - the schema ships with strategic indexes: GIN trigram for
//!   search, BRIN for time-series playback history, covering indexes for the
//!   common browse query, and partial indexes for the explicit-content filter.
//! * Offline builds - this file uses `sqlx::query` (non-macro) to avoid the
//!   compile-time database dependency. For CI, switch to `sqlx::query!` after
//!   running `cargo sqlx prepare --workspace`.

use std::sync::Arc;

use async_trait::async_trait;

use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyError, CanopyResult, CatalogRepository,
    DiscoveryRepository, MediaItem, MediaPage, Page, Session, SessionRepository,
};
use sqlx::Row;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reads a PostgreSQL UUID column and converts it to the string ID used by the domain model.
fn uuid_string(row: &sqlx::postgres::PgRow, column: &str) -> String {
    row.try_get::<uuid::Uuid, _>(column)
        .map(|id| id.to_string())
        .unwrap_or_default()
}

/// Denormalizes a raw SQL row into a `MediaItem`.
fn media_item_from_row(row: &sqlx::postgres::PgRow) -> MediaItem {
    let track_duration_ms: i32 = row.try_get("track_duration_ms").unwrap_or(0);
    let asset_size_bytes: i64 = row.try_get("asset_size_bytes").unwrap_or(0);
    let bitrate_kbps = if track_duration_ms > 0 {
        ((asset_size_bytes as u64) * 8 / track_duration_ms as u64 / 1000).clamp(0, i32::MAX as u64)
            as i32
    } else {
        0
    };

    MediaItem {
        id: uuid_string(row, "track_id"),
        title: row.try_get("track_title").unwrap_or_default(),
        artist: row.try_get("artist_name").unwrap_or_default(),
        album: row.try_get("album_title").unwrap_or_default(),
        artwork_uri: format!(
            "content://com.adrianrusu.mediaapp.audio/artwork/{}",
            row.try_get::<Option<String>, _>("artwork_key")
                .unwrap_or_default()
                .unwrap_or_default()
        ),
        duration_ms: track_duration_ms as i64,
        bitrate_kbps,
        mime_type: row
            .try_get::<Option<String>, _>("asset_content_type")
            .unwrap_or_default()
            .unwrap_or_else(|| "audio/mpeg".to_string()),
        is_explicit: row.try_get("track_explicit").unwrap_or(false),
    }
}

/// Wraps a `sqlx::Error` into a `CanopyError::Storage`.
fn db_err(e: sqlx::Error) -> CanopyError {
    CanopyError::Storage(e.to_string())
}

// ---------------------------------------------------------------------------
// PgCatalogRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed catalog repository.
#[derive(Clone)]
pub struct PgCatalogRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgCatalogRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Shared query that denormalizes a track row into a `MediaItem`.
    async fn media_item_by_id(&self, id: &str) -> CanopyResult<Option<MediaItem>> {
        let track_uuid = uuid::Uuid::parse_str(id)
            .map_err(|e| CanopyError::Storage(format!("Invalid track ID: {e}")))?;

        let sql = r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes
            FROM tracks t
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            LEFT JOIN audio_assets aa ON aa.track_id = t.id
            WHERE t.id = $1
            ORDER BY aa.codec
            LIMIT 1
        "#;

        let row = sqlx::query(sql)
            .bind(track_uuid)
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(row.as_ref().map(media_item_from_row))
    }
}

#[async_trait]
impl CatalogRepository for PgCatalogRepository {
    async fn browse(
        &self,
        _parent_id: Option<&str>,
        _genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let sql = r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes
            FROM tracks t
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            LEFT JOIN audio_assets aa ON aa.track_id = t.id
            ORDER BY t.created_at
            LIMIT $1 OFFSET $2
        "#;

        let rows = sqlx::query(sql)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let total_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tracks")
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let items: Vec<MediaItem> = rows.iter().map(media_item_from_row).collect();
        let has_more = (page.offset + page.limit) < total_count as u32;

        Ok(MediaPage {
            items,
            total_count: total_count as i32,
            has_more,
        })
    }

    async fn search(&self, query: &str, page: Page) -> CanopyResult<MediaPage> {
        let sql = r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes,
                GREATEST(
                    similarity(t.title, $1),
                    similarity(a.name, $1),
                    similarity(al.title, $1)
                ) AS rank
            FROM tracks t
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            LEFT JOIN audio_assets aa ON aa.track_id = t.id
            WHERE t.title % $1
               OR a.name % $1
               OR al.title % $1
            ORDER BY rank DESC, t.title
            LIMIT $2 OFFSET $3
        "#;

        let rows = sqlx::query(sql)
            .bind(query)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let count_sql = r#"
            SELECT COUNT(DISTINCT t.id)
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            WHERE t.title % $1
               OR a.name % $1
               OR al.title % $1
        "#;

        let total_count: i64 = sqlx::query_scalar(count_sql)
            .bind(query)
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let items: Vec<MediaItem> = rows.iter().map(media_item_from_row).collect();
        let has_more = (page.offset + page.limit) < total_count as u32;

        Ok(MediaPage {
            items,
            total_count: total_count as i32,
            has_more,
        })
    }

    async fn get_media(&self, media_id: &str) -> CanopyResult<Option<MediaItem>> {
        self.media_item_by_id(media_id).await
    }
}

#[async_trait]
impl DiscoveryRepository for PgCatalogRepository {
    async fn shuffle_pool(&self) -> CanopyResult<Vec<MediaItem>> {
        let sql = r#"
            SELECT
                track_id,
                track_title,
                artist_name,
                album_title,
                track_duration_ms,
                track_explicit,
                artwork_key,
                asset_content_type,
                asset_size_bytes
            FROM mv_discovery_pool
            ORDER BY shuffle_rank
        "#;

        let rows = sqlx::query(sql)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        if rows.is_empty() {
            let sql = r#"
                SELECT
                    t.id             AS track_id,
                    t.title          AS track_title,
                    a.name           AS artist_name,
                    al.title         AS album_title,
                    t.duration_ms    AS track_duration_ms,
                    t.is_explicit    AS track_explicit,
                    COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                    aa.content_type  AS asset_content_type,
                    aa.size_bytes    AS asset_size_bytes
                FROM tracks t
                JOIN artists a     ON t.artist_id = a.id
                JOIN albums al     ON t.album_id = al.id
                LEFT JOIN audio_assets aa ON aa.track_id = t.id
                ORDER BY t.created_at
            "#;

            let rows = sqlx::query(sql)
                .fetch_all(self.pool.as_ref())
                .await
                .map_err(db_err)?;

            return Ok(rows.iter().map(media_item_from_row).collect());
        }

        Ok(rows.iter().map(media_item_from_row).collect())
    }
}

// ---------------------------------------------------------------------------
// PgAudioAssetRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed audio-asset repository.
#[derive(Clone)]
pub struct PgAudioAssetRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgAudioAssetRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl AudioAssetRepository for PgAudioAssetRepository {
    async fn assets_for_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>> {
        let track_uuid = uuid::Uuid::parse_str(track_id)
            .map_err(|e| CanopyError::Storage(format!("Invalid track ID: {e}")))?;

        let sql = r#"
            SELECT
                track_id,
                codec,
                content_type,
                object_key,
                size_bytes,
                checksum_sha256,
                duration_ms
            FROM audio_assets
            WHERE track_id = $1
            ORDER BY codec
        "#;

        let rows = sqlx::query(sql)
            .bind(track_uuid)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let assets = rows
            .iter()
            .map(|r| AudioAsset {
                track_id: uuid_string(r, "track_id"),
                codec: r.try_get("codec").unwrap_or_default(),
                content_type: r.try_get("content_type").unwrap_or_default(),
                object_key: r.try_get("object_key").unwrap_or_default(),
                size_bytes: r.try_get::<i64, _>("size_bytes").unwrap_or(0) as u64,
                checksum_sha256: r.try_get("checksum_sha256").unwrap_or_default(),
                duration_ms: r.try_get::<i64, _>("duration_ms").unwrap_or(0) as u64,
            })
            .collect();

        Ok(assets)
    }
}

// ---------------------------------------------------------------------------
// PgSessionRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed session repository.
#[derive(Clone)]
pub struct PgSessionRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgSessionRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl SessionRepository for PgSessionRepository {
    async fn create(&self) -> CanopyResult<String> {
        let id = uuid::Uuid::new_v4().to_string();

        let sql = r#"
            INSERT INTO sessions (id, current_media_id, position_ms, playback_speed, is_playing)
            VALUES ($1, NULL, 0, 1.0, FALSE)
        "#;

        sqlx::query(sql)
            .bind(&id)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(id)
    }

    async fn get(&self, id: &str) -> CanopyResult<Option<Session>> {
        let sql = r#"
            SELECT id, current_media_id, position_ms, playback_speed, is_playing
            FROM sessions
            WHERE id = $1
        "#;

        let row = sqlx::query(sql)
            .bind(id)
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(row.map(|r| Session {
            id: r.try_get("id").unwrap_or_default(),
            current_media_id: r.try_get("current_media_id").ok(),
            position_ms: r.try_get("position_ms").unwrap_or(0),
            playback_speed: r.try_get("playback_speed").unwrap_or(1.0),
            is_playing: r.try_get("is_playing").unwrap_or(false),
        }))
    }

    async fn update(&self, session: Session) -> CanopyResult<()> {
        let sql = r#"
            INSERT INTO sessions (id, current_media_id, position_ms, playback_speed, is_playing)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
                current_media_id = EXCLUDED.current_media_id,
                position_ms      = EXCLUDED.position_ms,
                playback_speed   = EXCLUDED.playback_speed,
                is_playing       = EXCLUDED.is_playing,
                updated_at       = NOW()
        "#;

        sqlx::query(sql)
            .bind(&session.id)
            .bind(&session.current_media_id)
            .bind(session.position_ms)
            .bind(session.playback_speed)
            .bind(session.is_playing)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(())
    }

    async fn delete(&self, id: &str) -> CanopyResult<()> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(id)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(())
    }
}
