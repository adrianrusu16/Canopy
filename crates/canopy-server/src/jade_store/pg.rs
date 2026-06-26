//! PostgreSQL-backed implementations of the repository ports.
//!
//! This module uses `sqlx` to query the normalized schema described in the
//! architecture document. The domain model `MediaItem` is denormalized at
//! the adapter boundary via JOINs across `tracks`, `artists`, `albums`, and
//! a single representative `audio_assets` row per track.
//!
//! # Production Notes
//!
//! * Connection pooling - configured via `PgPoolOptions` in `lib.rs` with
//!   sensible max-connections and acquire-timeout defaults.
//! * Materialized views - `mv_discovery_pool` and `mv_catalog_search` are
//!   refreshed after ingestion for now. A background refresher can replace
//!   that once provider sync becomes high-volume.
//! * Indexes - the schema ships with strategic indexes: GIN trigram for
//!   search, BRIN for time-series playback history, covering indexes for the
//!   common browse query, and partial indexes for the explicit-content filter.
//! * Offline builds - this file uses `sqlx::query` (non-macro) to avoid the
//!   compile-time database dependency. For CI, switch to `sqlx::query!` after
//!   running `cargo sqlx prepare --workspace`.

use std::sync::Arc;

use async_trait::async_trait;

use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyError, CanopyResult, CatalogIngest, CatalogRepository,
    DiscoveryRepository, IngestBatchResult, LibraryItem, LibraryRepository, LikeRepository,
    MediaItem, MediaPage, Page, PlaybackHistoryEvent, PlaybackHistoryRepository,
    PreferencesRepository, ProfilePreferences, ProfileRepository, ProviderTrack, Session,
    SessionRepository, TrackLike, UserProfile,
};
use sqlx::{AssertSqlSafe, Row, Transaction};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const REPRESENTATIVE_ASSET_JOIN: &str = r#"
    LEFT JOIN LATERAL (
        SELECT codec, content_type, size_bytes
        FROM audio_assets
        WHERE track_id = t.id
        ORDER BY
            CASE codec
                WHEN 'opus' THEN 0
                WHEN 'mp4' THEN 1
                WHEN 'mp3' THEN 2
                WHEN 'flac' THEN 3
                ELSE 4
            END,
            codec
        LIMIT 1
    ) aa ON TRUE
"#;

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

fn invalid_arg(message: impl Into<String>) -> CanopyError {
    CanopyError::InvalidArgument(message.into())
}

fn parse_uuid_arg(value: &str, field: &str) -> CanopyResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .map_err(|e| CanopyError::InvalidArgument(format!("invalid {field}: {e}")))
}

fn epoch_ms_i64(row: &sqlx::postgres::PgRow, column: &str) -> u64 {
    row.try_get::<i64, _>(column).unwrap_or(0).max(0) as u64
}

fn map_track_write_err(err: sqlx::Error, track_id: &str) -> CanopyError {
    if let sqlx::Error::Database(db) = &err
        && (db.constraint() == Some("profile_library_items_track_id_fkey")
            || db.constraint() == Some("profile_track_likes_track_id_fkey"))
    {
        return CanopyError::not_found("track", track_id);
    }

    db_err(err)
}

fn require_non_empty(value: &str, field: &str) -> CanopyResult<()> {
    if value.trim().is_empty() {
        return Err(invalid_arg(format!("provider track {field} is required")));
    }

    Ok(())
}

fn u64_to_i64(value: u64, field: &str) -> CanopyResult<i64> {
    i64::try_from(value)
        .map_err(|_| invalid_arg(format!("{field} exceeds PostgreSQL BIGINT range")))
}

fn validate_provider_track(track: &ProviderTrack) -> CanopyResult<()> {
    require_non_empty(&track.provider, "provider")?;
    require_non_empty(&track.provider_id, "provider_id")?;
    require_non_empty(&track.title, "title")?;
    require_non_empty(&track.artist, "artist")?;
    require_non_empty(&track.album, "album")?;
    require_non_empty(&track.license.license_type, "license.license_type")?;
    require_non_empty(&track.license.source_url, "license.source_url")?;

    if track.duration_ms < 0 {
        return Err(invalid_arg(
            "provider track duration_ms must be non-negative",
        ));
    }

    for asset in &track.assets {
        require_non_empty(&asset.codec, "asset.codec")?;
        require_non_empty(&asset.content_type, "asset.content_type")?;
        require_non_empty(&asset.object_key, "asset.object_key")?;
        require_non_empty(&asset.checksum_sha256, "asset.checksum_sha256")?;
        if asset.checksum_sha256.len() != 64 {
            return Err(invalid_arg(
                "asset.checksum_sha256 must be 64 hex characters",
            ));
        }
        let _ = u64_to_i64(asset.size_bytes, "asset.size_bytes")?;
        let _ = u64_to_i64(asset.duration_ms, "asset.duration_ms")?;
    }

    Ok(())
}

fn sort_name(name: &str) -> String {
    name.trim().to_string()
}

async fn find_or_insert_artist(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    name: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT id FROM artists WHERE lower(name) = lower($1) ORDER BY created_at LIMIT 1",
    )
    .bind(name)
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }

    sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO artists (name, sort_name) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(sort_name(name))
    .fetch_one(&mut **tx)
    .await
}

async fn find_or_insert_license(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    track: &ProviderTrack,
) -> Result<uuid::Uuid, sqlx::Error> {
    if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT id FROM licenses WHERE source_url = $1 ORDER BY created_at LIMIT 1",
    )
    .bind(&track.license.source_url)
    .fetch_optional(&mut **tx)
    .await?
    {
        sqlx::query(
            r#"
                UPDATE licenses
                SET license_type = $2,
                    attribution_text = $3
                WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(&track.license.license_type)
        .bind(&track.license.attribution_text)
        .execute(&mut **tx)
        .await?;

        return Ok(id);
    }

    sqlx::query_scalar::<_, uuid::Uuid>(
        r#"
            INSERT INTO licenses (license_type, source_url, attribution_text)
            VALUES ($1, $2, $3)
            RETURNING id
        "#,
    )
    .bind(&track.license.license_type)
    .bind(&track.license.source_url)
    .bind(&track.license.attribution_text)
    .fetch_one(&mut **tx)
    .await
}

async fn find_or_insert_album(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    artist_id: uuid::Uuid,
    track: &ProviderTrack,
) -> Result<uuid::Uuid, sqlx::Error> {
    if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
        r#"
            SELECT id
            FROM albums
            WHERE artist_id = $1 AND lower(title) = lower($2)
            ORDER BY created_at
            LIMIT 1
        "#,
    )
    .bind(artist_id)
    .bind(&track.album)
    .fetch_optional(&mut **tx)
    .await?
    {
        sqlx::query(
            r#"
                UPDATE albums
                SET release_year = COALESCE($2, release_year),
                    artwork_key = COALESCE($3, artwork_key)
                WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(track.release_year)
        .bind(&track.album_artwork_key)
        .execute(&mut **tx)
        .await?;

        return Ok(id);
    }

    sqlx::query_scalar::<_, uuid::Uuid>(
        r#"
            INSERT INTO albums (title, artist_id, release_year, artwork_key)
            VALUES ($1, $2, $3, $4)
            RETURNING id
        "#,
    )
    .bind(&track.album)
    .bind(artist_id)
    .bind(track.release_year)
    .bind(&track.album_artwork_key)
    .fetch_one(&mut **tx)
    .await
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

    async fn refresh_catalog_views(&self) -> CanopyResult<()> {
        sqlx::query("REFRESH MATERIALIZED VIEW mv_discovery_pool")
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        sqlx::query("REFRESH MATERIALIZED VIEW mv_catalog_search")
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// Shared query that denormalizes a track row into a `MediaItem`.
    async fn media_item_by_id(&self, id: &str) -> CanopyResult<Option<MediaItem>> {
        let track_uuid = uuid::Uuid::parse_str(id)
            .map_err(|e| CanopyError::Storage(format!("Invalid track ID: {e}")))?;

        let sql = format!(
            r#"
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
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.id = $1
            LIMIT 1
        "#
        );

        let row = sqlx::query(AssertSqlSafe(sql))
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
        let sql = format!(
            r#"
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
            {REPRESENTATIVE_ASSET_JOIN}
            ORDER BY t.created_at
            LIMIT $1 OFFSET $2
        "#
        );

        let rows = sqlx::query(AssertSqlSafe(sql))
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
        let sql = format!(
            r#"
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
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.title % $1
               OR a.name % $1
               OR al.title % $1
            ORDER BY rank DESC, t.title
            LIMIT $2 OFFSET $3
        "#
        );

        let rows = sqlx::query(AssertSqlSafe(sql))
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
impl CatalogIngest for PgCatalogRepository {
    async fn ingest(&self, track: ProviderTrack) -> CanopyResult<String> {
        validate_provider_track(&track)?;

        let raw_metadata = serde_json::to_value(&track)
            .map_err(|e| CanopyError::Internal(format!("serialize provider metadata: {e}")))?;

        let mut tx = self.pool.begin().await.map_err(db_err)?;

        let artist_id = find_or_insert_artist(&mut tx, &track.artist)
            .await
            .map_err(db_err)?;
        let license_id = find_or_insert_license(&mut tx, &track)
            .await
            .map_err(db_err)?;
        let album_id = find_or_insert_album(&mut tx, artist_id, &track)
            .await
            .map_err(db_err)?;

        let existing_track_id = sqlx::query_scalar::<_, uuid::Uuid>(
            r#"
                SELECT track_id
                FROM provider_tracks
                WHERE provider = $1 AND provider_track_id = $2
                FOR UPDATE
            "#,
        )
        .bind(&track.provider)
        .bind(&track.provider_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;

        let track_id = if let Some(track_id) = existing_track_id {
            sqlx::query(
                r#"
                    UPDATE tracks
                    SET title = $2,
                        artist_id = $3,
                        album_id = $4,
                        duration_ms = $5,
                        license_id = $6,
                        is_explicit = $7,
                        artwork_key = $8,
                        updated_at = NOW()
                    WHERE id = $1
                "#,
            )
            .bind(track_id)
            .bind(&track.title)
            .bind(artist_id)
            .bind(album_id)
            .bind(track.duration_ms as i32)
            .bind(license_id)
            .bind(track.is_explicit)
            .bind(&track.artwork_key)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            track_id
        } else {
            let track_id = sqlx::query_scalar::<_, uuid::Uuid>(
                r#"
                    INSERT INTO tracks (
                        title, artist_id, album_id, duration_ms,
                        license_id, is_explicit, artwork_key
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    RETURNING id
                "#,
            )
            .bind(&track.title)
            .bind(artist_id)
            .bind(album_id)
            .bind(track.duration_ms as i32)
            .bind(license_id)
            .bind(track.is_explicit)
            .bind(&track.artwork_key)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;

            sqlx::query(
                r#"
                    INSERT INTO provider_tracks (track_id, provider, provider_track_id, raw_metadata)
                    VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(track_id)
            .bind(&track.provider)
            .bind(&track.provider_id)
            .bind(&raw_metadata)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            track_id
        };

        sqlx::query(
            r#"
                UPDATE provider_tracks
                SET raw_metadata = $3,
                    fetched_at = NOW(),
                    updated_at = NOW()
                WHERE provider = $1 AND provider_track_id = $2
            "#,
        )
        .bind(&track.provider)
        .bind(&track.provider_id)
        .bind(&raw_metadata)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

        for asset in &track.assets {
            sqlx::query(
                r#"
                    INSERT INTO audio_assets (
                        track_id, codec, content_type, object_key,
                        size_bytes, checksum_sha256, duration_ms
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (track_id, codec) DO UPDATE SET
                        content_type = EXCLUDED.content_type,
                        object_key = EXCLUDED.object_key,
                        size_bytes = EXCLUDED.size_bytes,
                        checksum_sha256 = EXCLUDED.checksum_sha256,
                        duration_ms = EXCLUDED.duration_ms,
                        updated_at = NOW()
                "#,
            )
            .bind(track_id)
            .bind(&asset.codec)
            .bind(&asset.content_type)
            .bind(&asset.object_key)
            .bind(u64_to_i64(asset.size_bytes, "asset.size_bytes")?)
            .bind(&asset.checksum_sha256)
            .bind(u64_to_i64(asset.duration_ms, "asset.duration_ms")?)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }

        tx.commit().await.map_err(db_err)?;
        self.refresh_catalog_views().await?;

        Ok(track_id.to_string())
    }

    async fn ingest_batch(&self, tracks: Vec<ProviderTrack>) -> CanopyResult<IngestBatchResult> {
        let mut result = IngestBatchResult::default();

        for track in tracks {
            let label = format!("{}/{}", track.provider, track.provider_id);
            match self.ingest(track).await {
                Ok(_) => result.succeeded += 1,
                Err(err) => {
                    result.failed += 1;
                    result.failures.push(format!("{label}: {err}"));
                }
            }
        }

        Ok(result)
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
            let sql = format!(
                r#"
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
                {REPRESENTATIVE_ASSET_JOIN}
                ORDER BY t.created_at
            "#
            );

            let rows = sqlx::query(AssertSqlSafe(sql))
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
// PgProfileRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed logged-in profile repository.
#[derive(Clone)]
pub struct PgProfileRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgProfileRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl ProfileRepository for PgProfileRepository {
    async fn upsert_profile(
        &self,
        external_user_id: &str,
        display_name: Option<&str>,
        history_enabled: bool,
    ) -> CanopyResult<UserProfile> {
        let row = sqlx::query(
            r#"
                INSERT INTO profiles (external_user_id, display_name, history_enabled)
                VALUES ($1, $2, $3)
                ON CONFLICT (external_user_id) DO UPDATE SET
                    display_name = EXCLUDED.display_name,
                    history_enabled = EXCLUDED.history_enabled,
                    updated_at = NOW()
                RETURNING id, external_user_id, display_name, history_enabled
            "#,
        )
        .bind(external_user_id)
        .bind(display_name)
        .bind(history_enabled)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(profile_from_row(&row))
    }

    async fn get_by_external_user_id(
        &self,
        external_user_id: &str,
    ) -> CanopyResult<Option<UserProfile>> {
        let row = sqlx::query(
            r#"
                SELECT id, external_user_id, display_name, history_enabled
                FROM profiles
                WHERE external_user_id = $1
            "#,
        )
        .bind(external_user_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(row.as_ref().map(profile_from_row))
    }
}

fn profile_from_row(row: &sqlx::postgres::PgRow) -> UserProfile {
    UserProfile {
        id: uuid_string(row, "id"),
        external_user_id: row.try_get("external_user_id").unwrap_or_default(),
        display_name: row.try_get("display_name").unwrap_or_default(),
        history_enabled: row.try_get("history_enabled").unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// PgPlaybackHistoryRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed durable playback-history repository.
#[derive(Clone)]
pub struct PgPlaybackHistoryRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgPlaybackHistoryRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl PlaybackHistoryRepository for PgPlaybackHistoryRepository {
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<()> {
        let duration_ms = i32::try_from(event.duration_ms).map_err(|_| {
            CanopyError::InvalidArgument("duration_ms exceeds database range".into())
        })?;

        sqlx::query(
            r#"
                INSERT INTO playback_history (profile_id, track_id, duration_ms, completion_pct)
                VALUES ($1::uuid, $2::uuid, $3, $4)
            "#,
        )
        .bind(&event.profile_id)
        .bind(&event.track_id)
        .bind(duration_ms)
        .bind(event.completion_pct)
        .execute(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PgLibraryRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed profile library repository.
#[derive(Clone)]
pub struct PgLibraryRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgLibraryRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl LibraryRepository for PgLibraryRepository {
    async fn save_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<LibraryItem> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        let row = sqlx::query(
            r#"
                INSERT INTO profile_library_items (profile_id, track_id)
                VALUES ($1, $2)
                ON CONFLICT (profile_id, track_id) DO UPDATE SET updated_at = NOW()
                RETURNING
                    profile_id::text,
                    track_id::text,
                    (EXTRACT(EPOCH FROM added_at) * 1000)::bigint AS added_at_epoch_ms
            "#,
        )
        .bind(profile_uuid)
        .bind(track_uuid)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|err| map_track_write_err(err, track_id))?;

        Ok(LibraryItem {
            profile_id: row.try_get("profile_id").unwrap_or_default(),
            track_id: row.try_get("track_id").unwrap_or_default(),
            added_at_epoch_ms: epoch_ms_i64(&row, "added_at_epoch_ms"),
        })
    }

    async fn remove_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        sqlx::query("DELETE FROM profile_library_items WHERE profile_id = $1 AND track_id = $2")
            .bind(profile_uuid)
            .bind(track_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn list_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<MediaPage> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let sql = format!(
            r#"
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
            FROM profile_library_items pli
            JOIN tracks t      ON pli.track_id = t.id
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE pli.profile_id = $1
            ORDER BY pli.added_at DESC, t.title
            LIMIT $2 OFFSET $3
        "#
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(profile_uuid)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        let total_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM profile_library_items WHERE profile_id = $1")
                .bind(profile_uuid)
                .fetch_one(self.pool.as_ref())
                .await
                .map_err(db_err)?;
        Ok(MediaPage {
            items: rows.iter().map(media_item_from_row).collect(),
            total_count: total_count as i32,
            has_more: (page.offset + page.limit) < total_count as u32,
        })
    }

    async fn is_saved(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM profile_library_items WHERE profile_id = $1 AND track_id = $2)",
        )
        .bind(profile_uuid)
        .bind(track_uuid)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(exists)
    }
}

// ---------------------------------------------------------------------------
// PgLikeRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed profile track-like repository.
#[derive(Clone)]
pub struct PgLikeRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgLikeRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl LikeRepository for PgLikeRepository {
    async fn like_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<TrackLike> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        let row = sqlx::query(
            r#"
                INSERT INTO profile_track_likes (profile_id, track_id)
                VALUES ($1, $2)
                ON CONFLICT (profile_id, track_id) DO UPDATE SET updated_at = NOW()
                RETURNING
                    profile_id::text,
                    track_id::text,
                    (EXTRACT(EPOCH FROM liked_at) * 1000)::bigint AS liked_at_epoch_ms
            "#,
        )
        .bind(profile_uuid)
        .bind(track_uuid)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|err| map_track_write_err(err, track_id))?;

        Ok(TrackLike {
            profile_id: row.try_get("profile_id").unwrap_or_default(),
            track_id: row.try_get("track_id").unwrap_or_default(),
            liked_at_epoch_ms: epoch_ms_i64(&row, "liked_at_epoch_ms"),
        })
    }

    async fn unlike_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        sqlx::query("DELETE FROM profile_track_likes WHERE profile_id = $1 AND track_id = $2")
            .bind(profile_uuid)
            .bind(track_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn list_liked_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<MediaPage> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let sql = format!(
            r#"
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
            FROM profile_track_likes ptl
            JOIN tracks t      ON ptl.track_id = t.id
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE ptl.profile_id = $1
            ORDER BY ptl.liked_at DESC, t.title
            LIMIT $2 OFFSET $3
        "#
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(profile_uuid)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        let total_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM profile_track_likes WHERE profile_id = $1")
                .bind(profile_uuid)
                .fetch_one(self.pool.as_ref())
                .await
                .map_err(db_err)?;
        Ok(MediaPage {
            items: rows.iter().map(media_item_from_row).collect(),
            total_count: total_count as i32,
            has_more: (page.offset + page.limit) < total_count as u32,
        })
    }

    async fn is_liked(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM profile_track_likes WHERE profile_id = $1 AND track_id = $2)",
        )
        .bind(profile_uuid)
        .bind(track_uuid)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(exists)
    }
}

// ---------------------------------------------------------------------------
// PgPreferencesRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed profile preferences repository.
#[derive(Clone)]
pub struct PgPreferencesRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgPreferencesRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl PreferencesRepository for PgPreferencesRepository {
    async fn get_preferences(&self, profile_id: &str) -> CanopyResult<ProfilePreferences> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let row = sqlx::query(
            "SELECT profile_id::text, preferences::text AS preferences FROM profile_preferences WHERE profile_id = $1",
        )
        .bind(profile_uuid)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(row
            .as_ref()
            .map(|row| ProfilePreferences {
                profile_id: row.try_get("profile_id").unwrap_or_default(),
                values_json: row
                    .try_get("preferences")
                    .unwrap_or_else(|_| "{}".to_string()),
            })
            .unwrap_or_else(|| ProfilePreferences {
                profile_id: profile_id.to_string(),
                values_json: "{}".to_string(),
            }))
    }

    async fn upsert_preferences(
        &self,
        profile_id: &str,
        values_json: &str,
    ) -> CanopyResult<ProfilePreferences> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let row = sqlx::query(
            r#"
                INSERT INTO profile_preferences (profile_id, preferences)
                VALUES ($1, $2::jsonb)
                ON CONFLICT (profile_id) DO UPDATE SET
                    preferences = EXCLUDED.preferences,
                    updated_at = NOW()
                RETURNING profile_id::text, preferences::text AS preferences
            "#,
        )
        .bind(profile_uuid)
        .bind(values_json)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(ProfilePreferences {
            profile_id: row.try_get("profile_id").unwrap_or_default(),
            values_json: row
                .try_get("preferences")
                .unwrap_or_else(|_| "{}".to_string()),
        })
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
