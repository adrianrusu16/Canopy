//! PostgreSQL-backed implementations of the repository ports.
//!
//! This module uses [`sqlx`] to query the normalized schema described in the
//! architecture document. The domain model [`MediaItem`] is denormalized at
//! the adapter boundary via JOINs across `tracks`, `artists`, `albums`, and
//! `audio_assets`.
//!
//! # Wiring
//!
//! A [`sqlx::PgPool`] is obtained from the application configuration (see
//! [`Config::database_url`](crate::config::Config)) and wrapped in an `Arc` so
//! it can be shared across domain services.
//!
//! ```ignore
//! let pool = sqlx::PgPool::connect(&config.database_url).await?;
//! let catalog_repo = Arc::new(PgCatalogRepository::new(pool.clone()));
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;

use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyError, CanopyResult, CatalogRepository,
    DiscoveryRepository, MediaItem, MediaPage, Page, Session, SessionRepository,
};

/// PostgreSQL-backed catalog repository.
///
/// Implements [`CatalogRepository`] and [`DiscoveryRepository`] by querying
/// the normalized `tracks`, `artists`, `albums`, and `audio_assets` tables.
///
/// This is a **stub** — `browse` and `get_media` are implemented end-to-end
/// against PostgreSQL, while `search` intentionally returns empty until the
/// `pg_trgm` integration is wired. The discovery shuffle pool returns the
/// catalog in insertion order (a real implementation will read from a
/// pre-shuffled materialized view).
#[derive(Clone)]
pub struct PgCatalogRepository {
    pool: Arc<PgPool>,
}

impl PgCatalogRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Shared query that denormalizes a track row into a [`MediaItem`].
    ///
    /// The query joins `tracks → artists → albums` and selects the *first*
    /// audio asset (ordered by codec) for MIME-type and bitrate metadata.
    /// This is sufficient for the initial prototype; a production path will
    /// select the best asset per client-declared codec preference.
    async fn media_item_by_id(&self, id: &str) -> CanopyResult<Option<MediaItem>> {
        let row = sqlx::query!(
            r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.codec         AS asset_codec,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes
            FROM tracks t
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            LEFT JOIN audio_assets aa ON aa.track_id = t.id
            WHERE t.id = $1
            ORDER BY aa.codec
            LIMIT 1
            "#,
            id
        )
        .fetch_optional(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        Ok(row.map(|r| MediaItem {
            id: r.track_id,
            title: r.track_title,
            artist: r.artist_name,
            album: r.album_title,
            artwork_uri: format!(
                "content://com.adrianrusu.mediaapp.audio/artwork/{}",
                r.artwork_key.unwrap_or_default()
            ),
            duration_ms: r.track_duration_ms,
            bitrate_kbps: (r.asset_size_bytes.unwrap_or(0) * 8 / r.track_duration_ms.max(1) as i64 / 1000)
                .clamp(0, i32::MAX as i64) as i32,
            mime_type: r.asset_content_type.unwrap_or_else(|| "audio/mpeg".to_string()),
            is_explicit: r.track_explicit,
        }))
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
        let rows = sqlx::query!(
            r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.codec         AS asset_codec,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes
            FROM tracks t
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            LEFT JOIN audio_assets aa ON aa.track_id = t.id
            ORDER BY t.created_at
            LIMIT $1 OFFSET $2
            "#,
            page.limit as i64,
            page.offset as i64
        )
        .fetch_all(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        let total_count = sqlx::query_scalar!("SELECT COUNT(*) FROM tracks")
            .fetch_one(&*self.pool)
            .await
            .map_err(|e| CanopyError::Storage(e.to_string()))?;

        let items: Vec<MediaItem> = rows
            .into_iter()
            .map(|r| MediaItem {
                id: r.track_id,
                title: r.track_title,
                artist: r.artist_name,
                album: r.album_title,
                artwork_uri: format!(
                    "content://com.adrianrusu.mediaapp.audio/artwork/{}",
                    r.artwork_key.unwrap_or_default()
                ),
                duration_ms: r.track_duration_ms,
                bitrate_kbps: (r.asset_size_bytes.unwrap_or(0) * 8
                    / r.track_duration_ms.max(1) as i64
                    / 1000)
                    .clamp(0, i32::MAX as i64) as i32,
                mime_type: r.asset_content_type.unwrap_or_else(|| "audio/mpeg".to_string()),
                is_explicit: r.track_explicit,
            })
            .collect();

        let has_more = (page.offset + page.limit) < total_count as u32;

        Ok(MediaPage {
            items,
            total_count: total_count as i32,
            has_more,
        })
    }

    async fn search(&self, query: &str, page: Page) -> CanopyResult<MediaPage> {
        // Use pg_trgm for fuzzy matching on track title, artist name, and album title
        // The query joins tracks → artists → albums and gets the first audio asset for MIME-type/bitrate
        let search_term = format!("%{}%", query); // For ILIKE fallback if needed

        // First get matching tracks with trigram similarity, ordered by similarity score
        let rows = sqlx::query!(
            r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.codec         AS asset_codec,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes,
                -- Calculate similarity score for ordering (higher = more similar)
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
            "#,
            query,
            page.limit as i64,
            page.offset as i64
        )
        .fetch_all(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        // Get total count for pagination
        let total_count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(DISTINCT t.id)
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            WHERE t.title % $1
                   OR a.name % $1
                   OR al.title % $1
            "#,
            query
        )
        .fetch_one(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        let items: Vec<MediaItem> = rows
            .into_iter()
            .map(|r| MediaItem {
                id: r.track_id,
                title: r.track_title,
                artist: r.artist_name,
                album: r.album_title,
                artwork_uri: format!(
                    "content://com.adrianrusu.mediaapp.audio/artwork/{}",
                    r.artwork_key.unwrap_or_default()
                ),
                duration_ms: r.track_duration_ms,
                bitrate_kbps: (r.asset_size_bytes.unwrap_or(0) * 8
                    / r.track_duration_ms.max(1) as i64
                    / 1000)
                    .clamp(0, i32::MAX as i64) as i32,
                mime_type: r.asset_content_type.unwrap_or_else(|| "audio/mpeg".to_string()),
                is_explicit: r.track_explicit,
            })
            .collect();

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
        // Stub: the production path reads a pre-shuffled materialized view.
        // For now, return all tracks in insertion order so the discovery
        // service can still apply diversity/exclusion logic on top.
        let rows = sqlx::query!(
            r#"
            SELECT
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_key, al.artwork_key) AS artwork_key,
                aa.codec         AS asset_codec,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes
            FROM tracks t
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            LEFT JOIN audio_assets aa ON aa.track_id = t.id
            ORDER BY t.created_at
            "#
        )
        .fetch_all(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| MediaItem {
                id: r.track_id,
                title: r.track_title,
                artist: r.artist_name,
                album: r.album_title,
                artwork_uri: format!(
                    "content://com.adrianrusu.mediaapp.audio/artwork/{}",
                    r.artwork_key.unwrap_or_default()
                ),
                duration_ms: r.track_duration_ms,
                bitrate_kbps: (r.asset_size_bytes.unwrap_or(0) * 8
                    / r.track_duration_ms.max(1) as i64
                    / 1000)
                    .clamp(0, i32::MAX as i64) as i32,
                mime_type: r.asset_content_type.unwrap_or_else(|| "audio/mpeg".to_string()),
                is_explicit: r.track_explicit,
            })
            .collect())
    }
}

/// PostgreSQL-backed audio-asset repository.
///
/// Implements [`AudioAssetRepository`] by reading from the `audio_assets`
/// table. This is a stub that compiles and demonstrates the port pattern;
/// it returns all assets for a track (ordered by codec).
#[derive(Clone)]
pub struct PgAudioAssetRepository {
    pool: Arc<PgPool>,
}

impl PgAudioAssetRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl AudioAssetRepository for PgAudioAssetRepository {
    async fn assets_for_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>> {
        let rows = sqlx::query!(
            r#"
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
            "#,
            track_id
        )
        .fetch_all(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| AudioAsset {
                track_id: r.track_id,
                codec: r.codec,
                content_type: r.content_type,
                object_key: r.object_key,
                size_bytes: r.size_bytes as u64,
                checksum_sha256: r.checksum_sha256,
                duration_ms: r.duration_ms as u64,
            })
            .collect())
    }
}

/// PostgreSQL-backed session repository.
///
/// Implements [`SessionRepository`] by reading from and writing to a
/// `sessions` table. This is a stub that demonstrates the port pattern.
#[derive(Clone)]
pub struct PgSessionRepository {
    pool: Arc<PgPool>,
}

impl PgSessionRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl SessionRepository for PgSessionRepository {
    async fn create(&self) -> CanopyResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query!(
            r#"
            INSERT INTO sessions (id, current_media_id, position_ms, playback_speed, is_playing)
            VALUES ($1, NULL, 0, 1.0, FALSE)
            "#,
            id
        )
        .execute(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;
        Ok(id)
    }

    async fn get(&self, id: &str) -> CanopyResult<Option<Session>> {
        let row = sqlx::query!(
            r#"
            SELECT id, current_media_id, position_ms, playback_speed, is_playing
            FROM sessions
            WHERE id = $1
            "#,
            id
        )
        .fetch_optional(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;

        Ok(row.map(|r| Session {
            id: r.id,
            current_media_id: r.current_media_id,
            position_ms: r.position_ms,
            playback_speed: r.playback_speed,
            is_playing: r.is_playing,
        }))
    }

    async fn update(&self, session: Session) -> CanopyResult<()> {
        sqlx::query!(
            r#"
            INSERT INTO sessions (id, current_media_id, position_ms, playback_speed, is_playing)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
                current_media_id = EXCLUDED.current_media_id,
                position_ms      = EXCLUDED.position_ms,
                playback_speed   = EXCLUDED.playback_speed,
                is_playing       = EXCLUDED.is_playing
            "#,
            session.id,
            session.current_media_id,
            session.position_ms,
            session.playback_speed,
            session.is_playing,
        )
        .execute(&*self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> CanopyResult<()> {
        sqlx::query!("DELETE FROM sessions WHERE id = $1", id)
            .execute(&*self.pool)
            .await
            .map_err(|e| CanopyError::Storage(e.to_string()))?;
        Ok(())
    }
}
