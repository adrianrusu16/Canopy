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
    DiscoveryRepository, IngestBatchResult, InstanceSettingsRepository, LibraryItem,
    LibraryRepository, LikeRepository, LikedTrackItem, LikedTrackPage, MediaItem, MediaPage, Page,
    PlaybackHistoryEntry, PlaybackHistoryEvent, PlaybackHistoryPage, PlaybackHistoryRepository,
    Playlist, PlaylistPage, PlaylistRepository, PlaylistTrackItem, PlaylistTrackPage,
    PreferencesRepository, ProfilePreferences, ProfileRepository, ProviderTrack, SavedTrackItem,
    SavedTrackPage, TrackLike, UserProfile,
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
            row.try_get::<Option<String>, _>("artwork_storage_key")
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

fn media_page(rows: Vec<sqlx::postgres::PgRow>, page: Page, total_count: i64) -> MediaPage {
    let items = rows.iter().map(media_item_from_row).collect();
    MediaPage {
        items,
        total_count: total_count as i32,
        has_more: (page.offset + page.limit) < total_count as u32,
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
            || db.constraint() == Some("profile_track_likes_track_id_fkey")
            || db.constraint() == Some("profile_playlist_tracks_track_id_fkey")
            || db.constraint() == Some("playback_history_track_id_fkey"))
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
        require_non_empty(&asset.storage_key, "asset.storage_key")?;
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
                    artwork_storage_key = COALESCE($3, artwork_storage_key)
                WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(track.release_year)
        .bind(&track.album_artwork_storage_key)
        .execute(&mut **tx)
        .await?;

        return Ok(id);
    }

    sqlx::query_scalar::<_, uuid::Uuid>(
        r#"
            INSERT INTO albums (title, artist_id, release_year, artwork_storage_key)
            VALUES ($1, $2, $3, $4)
            RETURNING id
        "#,
    )
    .bind(&track.album)
    .bind(artist_id)
    .bind(track.release_year)
    .bind(&track.album_artwork_storage_key)
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

    async fn public_media_item_by_id(&self, id: &str) -> CanopyResult<Option<MediaItem>> {
        let track_uuid = parse_uuid_arg(id, "media_id")?;
        let sql = format!(
            r#"
            SELECT t.id AS track_id, t.title AS track_title, a.name AS artist_name,
                al.title AS album_title, t.duration_ms AS track_duration_ms,
                t.is_explicit AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type AS asset_content_type, aa.size_bytes AS asset_size_bytes
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.id = $1
              AND t.visibility = 'release_safe'
              AND t.ingest_status = 'ready'
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

    async fn personal_media_item_by_id(
        &self,
        owner_profile_id: &str,
        id: &str,
    ) -> CanopyResult<Option<MediaItem>> {
        let owner_uuid = parse_uuid_arg(owner_profile_id, "owner_profile_id")?;
        let track_uuid = parse_uuid_arg(id, "media_id")?;
        let sql = format!(
            r#"
            SELECT t.id AS track_id, t.title AS track_title, a.name AS artist_name,
                al.title AS album_title, t.duration_ms AS track_duration_ms,
                t.is_explicit AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type AS asset_content_type, aa.size_bytes AS asset_size_bytes
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.id = $1
              AND t.owner_profile_id = $2
              AND t.visibility = 'personal'
              AND t.ingest_status = 'ready'
            LIMIT 1
        "#
        );
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(track_uuid)
            .bind(owner_uuid)
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(media_item_from_row))
    }
}

#[async_trait]
impl CatalogRepository for PgCatalogRepository {
    async fn browse_public(
        &self,
        _parent_id: Option<&str>,
        _genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let sql = format!(
            r#"
            SELECT t.id AS track_id, t.title AS track_title, a.name AS artist_name,
                al.title AS album_title, t.duration_ms AS track_duration_ms,
                t.is_explicit AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type AS asset_content_type, aa.size_bytes AS asset_size_bytes
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.visibility = 'release_safe' AND t.ingest_status = 'ready'
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
        let total_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tracks WHERE visibility = 'release_safe' AND ingest_status = 'ready'"
        ).fetch_one(self.pool.as_ref()).await.map_err(db_err)?;
        Ok(media_page(rows, page, total_count))
    }

    async fn search_public(&self, query: &str, page: Page) -> CanopyResult<MediaPage> {
        let sql = format!(
            r#"
            SELECT t.id AS track_id, t.title AS track_title, a.name AS artist_name,
                al.title AS album_title, t.duration_ms AS track_duration_ms,
                t.is_explicit AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type AS asset_content_type, aa.size_bytes AS asset_size_bytes,
                GREATEST(similarity(t.title, $1), similarity(a.name, $1), similarity(al.title, $1)) AS rank
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE (t.title % $1 OR a.name % $1 OR al.title % $1)
              AND t.visibility = 'release_safe'
              AND t.ingest_status = 'ready'
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
        let total_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(DISTINCT t.id) FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            WHERE (t.title % $1 OR a.name % $1 OR al.title % $1)
              AND t.visibility = 'release_safe' AND t.ingest_status = 'ready'
        "#,
        )
        .bind(query)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(media_page(rows, page, total_count))
    }

    async fn get_public_media(&self, media_id: &str) -> CanopyResult<Option<MediaItem>> {
        self.public_media_item_by_id(media_id).await
    }

    async fn list_personal(&self, owner_profile_id: &str, page: Page) -> CanopyResult<MediaPage> {
        let owner_uuid = parse_uuid_arg(owner_profile_id, "owner_profile_id")?;
        let sql = format!(
            r#"
            SELECT t.id AS track_id, t.title AS track_title, a.name AS artist_name,
                al.title AS album_title, t.duration_ms AS track_duration_ms,
                t.is_explicit AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type AS asset_content_type, aa.size_bytes AS asset_size_bytes
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.owner_profile_id = $1
              AND t.visibility = 'personal' AND t.ingest_status = 'ready'
            ORDER BY t.created_at
            LIMIT $2 OFFSET $3
        "#
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(owner_uuid)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        let total_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tracks WHERE owner_profile_id = $1 AND visibility = 'personal' AND ingest_status = 'ready'"
        ).bind(owner_uuid).fetch_one(self.pool.as_ref()).await.map_err(db_err)?;
        Ok(media_page(rows, page, total_count))
    }

    async fn search_personal(
        &self,
        owner_profile_id: &str,
        query: &str,
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let owner_uuid = parse_uuid_arg(owner_profile_id, "owner_profile_id")?;
        let sql = format!(
            r#"
            SELECT t.id AS track_id, t.title AS track_title, a.name AS artist_name,
                al.title AS album_title, t.duration_ms AS track_duration_ms,
                t.is_explicit AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type AS asset_content_type, aa.size_bytes AS asset_size_bytes,
                GREATEST(similarity(t.title, $2), similarity(a.name, $2), similarity(al.title, $2)) AS rank
            FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE t.owner_profile_id = $1
              AND (t.title % $2 OR a.name % $2 OR al.title % $2)
              AND t.visibility = 'personal' AND t.ingest_status = 'ready'
            ORDER BY rank DESC, t.title
            LIMIT $3 OFFSET $4
        "#
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(owner_uuid)
            .bind(query)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        let total_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(DISTINCT t.id) FROM tracks t
            JOIN artists a ON t.artist_id = a.id
            JOIN albums al ON t.album_id = al.id
            WHERE t.owner_profile_id = $1
              AND (t.title % $2 OR a.name % $2 OR al.title % $2)
              AND t.visibility = 'personal' AND t.ingest_status = 'ready'
        "#,
        )
        .bind(owner_uuid)
        .bind(query)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(media_page(rows, page, total_count))
    }

    async fn get_personal_media(
        &self,
        owner_profile_id: &str,
        media_id: &str,
    ) -> CanopyResult<Option<MediaItem>> {
        self.personal_media_item_by_id(owner_profile_id, media_id)
            .await
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
                        artwork_storage_key = $8,
                        visibility = 'quarantined',
                        ingest_status = 'quarantined',
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
            .bind(&track.artwork_storage_key)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

            track_id
        } else {
            let track_id = sqlx::query_scalar::<_, uuid::Uuid>(
                r#"
                    INSERT INTO tracks (
                        title, artist_id, album_id, duration_ms,
                        license_id, is_explicit, artwork_storage_key,
                        visibility, ingest_status
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7, 'quarantined', 'quarantined')
                    RETURNING id
                "#,
            )
            .bind(&track.title)
            .bind(artist_id)
            .bind(album_id)
            .bind(track.duration_ms as i32)
            .bind(license_id)
            .bind(track.is_explicit)
            .bind(&track.artwork_storage_key)
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
                        track_id, codec, content_type, storage_key,
                        size_bytes, checksum_sha256, duration_ms
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (track_id, codec) DO UPDATE SET
                        content_type = EXCLUDED.content_type,
                        storage_key = EXCLUDED.storage_key,
                        size_bytes = EXCLUDED.size_bytes,
                        checksum_sha256 = EXCLUDED.checksum_sha256,
                        duration_ms = EXCLUDED.duration_ms,
                        updated_at = NOW()
                "#,
            )
            .bind(track_id)
            .bind(&asset.codec)
            .bind(&asset.content_type)
            .bind(&asset.storage_key)
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
                artwork_storage_key,
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
                    COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                    aa.content_type  AS asset_content_type,
                    aa.size_bytes    AS asset_size_bytes
                FROM tracks t
                JOIN artists a     ON t.artist_id = a.id
                JOIN albums al     ON t.album_id = al.id
                {REPRESENTATIVE_ASSET_JOIN}
                WHERE t.is_explicit = FALSE
                  AND t.visibility = 'release_safe'
                  AND t.ingest_status = 'ready'
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
    async fn assets_for_public_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>> {
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        let rows = sqlx::query(
            r#"
            SELECT aa.track_id, aa.codec, aa.content_type, aa.storage_key,
                aa.size_bytes, aa.checksum_sha256, aa.duration_ms
            FROM audio_assets aa
            JOIN tracks t ON t.id = aa.track_id
            WHERE aa.track_id = $1
              AND t.visibility = 'release_safe'
              AND t.ingest_status = 'ready'
            ORDER BY aa.codec
        "#,
        )
        .bind(track_uuid)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(audio_assets_from_rows(&rows))
    }

    async fn assets_for_personal_track(
        &self,
        owner_profile_id: &str,
        track_id: &str,
    ) -> CanopyResult<Vec<AudioAsset>> {
        let owner_uuid = parse_uuid_arg(owner_profile_id, "owner_profile_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        let rows = sqlx::query(
            r#"
            SELECT aa.track_id, aa.codec, aa.content_type, aa.storage_key,
                aa.size_bytes, aa.checksum_sha256, aa.duration_ms
            FROM audio_assets aa
            JOIN tracks t ON t.id = aa.track_id
            WHERE aa.track_id = $1 AND t.owner_profile_id = $2
              AND t.visibility = 'personal'
              AND t.ingest_status = 'ready'
            ORDER BY aa.codec
        "#,
        )
        .bind(track_uuid)
        .bind(owner_uuid)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;
        Ok(audio_assets_from_rows(&rows))
    }
}

fn audio_assets_from_rows(rows: &[sqlx::postgres::PgRow]) -> Vec<AudioAsset> {
    rows.iter()
        .map(|row| AudioAsset {
            track_id: uuid_string(row, "track_id"),
            codec: row.try_get("codec").unwrap_or_default(),
            content_type: row.try_get("content_type").unwrap_or_default(),
            storage_key: row.try_get("storage_key").unwrap_or_default(),
            size_bytes: row.try_get::<i64, _>("size_bytes").unwrap_or(0).max(0) as u64,
            checksum_sha256: row.try_get("checksum_sha256").unwrap_or_default(),
            duration_ms: row.try_get::<i64, _>("duration_ms").unwrap_or(0).max(0) as u64,
        })
        .collect()
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

    async fn delete_by_external_user_id(&self, external_user_id: &str) -> CanopyResult<()> {
        sqlx::query("DELETE FROM profiles WHERE external_user_id = $1")
            .bind(external_user_id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|error| {
                if error
                    .as_database_error()
                    .and_then(|db| db.code())
                    .as_deref()
                    == Some("23503")
                {
                    CanopyError::FailedPrecondition(
                        "profile is still referenced by protected resources".into(),
                    )
                } else {
                    db_err(error)
                }
            })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PgInstanceSettingsRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed singleton installation settings.
#[derive(Clone)]
pub struct PgInstanceSettingsRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgInstanceSettingsRepository {
    /// Creates a settings repository backed by the given pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }
}

#[async_trait]
impl InstanceSettingsRepository for PgInstanceSettingsRepository {
    async fn set_owner_profile_id(&self, profile_id: &str) -> CanopyResult<()> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let updated = sqlx::query_scalar::<_, bool>(
            r#"
                UPDATE instance_settings
                SET owner_profile_id = $1, updated_at = NOW()
                WHERE singleton = TRUE
                RETURNING singleton
            "#,
        )
        .bind(profile_uuid)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        if updated.is_none() {
            return Err(CanopyError::Storage(
                "instance settings singleton is missing".to_string(),
            ));
        }

        Ok(())
    }

    async fn owner_profile_id(&self) -> CanopyResult<Option<String>> {
        let owner = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
            "SELECT owner_profile_id FROM instance_settings WHERE singleton = TRUE",
        )
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?
        .ok_or_else(|| {
            CanopyError::Storage("instance settings singleton is missing".to_string())
        })?;

        Ok(owner.map(|id| id.to_string()))
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
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<bool> {
        let profile_uuid = parse_uuid_arg(&event.profile_id, "profile_id")?;
        let track_uuid = parse_uuid_arg(&event.track_id, "track_id")?;
        let duration_ms = i32::try_from(event.duration_ms).map_err(|_| {
            CanopyError::InvalidArgument("duration_ms exceeds database range".into())
        })?;

        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let enabled: Option<bool> =
            sqlx::query_scalar("SELECT history_enabled FROM profiles WHERE id = $1 FOR UPDATE")
                .bind(profile_uuid)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?;
        let Some(enabled) = enabled else {
            return Err(CanopyError::not_found("profile", &event.profile_id));
        };
        if !enabled {
            tx.commit().await.map_err(db_err)?;
            return Ok(false);
        }

        sqlx::query(
            r#"
                INSERT INTO playback_history (profile_id, track_id, duration_ms, completion_pct)
                VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(profile_uuid)
        .bind(track_uuid)
        .bind(duration_ms)
        .bind(event.completion_pct)
        .execute(&mut *tx)
        .await
        .map_err(|err| map_track_write_err(err, &event.track_id))?;

        tx.commit().await.map_err(db_err)?;
        Ok(true)
    }

    async fn list(&self, profile_id: &str, page: Page) -> CanopyResult<PlaybackHistoryPage> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let sql = format!(
            r#"
                SELECT
                    ph.id AS history_id,
                    (EXTRACT(EPOCH FROM ph.played_at) * 1000)::bigint AS played_at_epoch_ms,
                    ph.duration_ms,
                    ph.completion_pct,
                    t.id AS track_id,
                    t.title AS track_title,
                    a.name AS artist_name,
                    al.title AS album_title,
                    t.duration_ms AS track_duration_ms,
                    t.is_explicit AS track_explicit,
                    COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                    aa.content_type AS asset_content_type,
                    aa.size_bytes AS asset_size_bytes
                FROM playback_history ph
                JOIN tracks t ON ph.track_id = t.id
                JOIN artists a ON t.artist_id = a.id
                JOIN albums al ON t.album_id = al.id
                {REPRESENTATIVE_ASSET_JOIN}
                WHERE ph.profile_id = $1
                ORDER BY ph.played_at DESC, ph.id DESC
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
            sqlx::query_scalar("SELECT COUNT(*) FROM playback_history WHERE profile_id = $1")
                .bind(profile_uuid)
                .fetch_one(self.pool.as_ref())
                .await
                .map_err(db_err)?;

        let entries = rows
            .iter()
            .map(|row| PlaybackHistoryEntry {
                id: uuid_string(row, "history_id"),
                played_at_epoch_ms: epoch_ms_i64(row, "played_at_epoch_ms"),
                duration_ms: row.try_get::<i32, _>("duration_ms").unwrap_or(0) as i64,
                completion_pct: row.try_get("completion_pct").unwrap_or(0.0),
                item: media_item_from_row(row),
            })
            .collect();
        let page_end = u64::from(page.offset) + u64::from(page.limit);

        Ok(PlaybackHistoryPage {
            entries,
            total_count: total_count.min(i64::from(i32::MAX)) as i32,
            has_more: page_end < total_count.max(0) as u64,
        })
    }

    async fn delete_entry(&self, profile_id: &str, history_id: &str) -> CanopyResult<bool> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let history_uuid = parse_uuid_arg(history_id, "history_id")?;
        let result = sqlx::query("DELETE FROM playback_history WHERE id = $1 AND profile_id = $2")
            .bind(history_uuid)
            .bind(profile_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn clear(&self, profile_id: &str) -> CanopyResult<u64> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let result = sqlx::query("DELETE FROM playback_history WHERE profile_id = $1")
            .bind(profile_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected())
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

    async fn list_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<SavedTrackPage> {
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
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes,
                (EXTRACT(EPOCH FROM pli.added_at) * 1000)::bigint AS saved_at_epoch_ms
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
        Ok(SavedTrackPage {
            items: rows
                .iter()
                .map(|row| SavedTrackItem {
                    item: media_item_from_row(row),
                    saved_at_epoch_ms: epoch_ms_i64(row, "saved_at_epoch_ms"),
                })
                .collect(),
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

    async fn list_liked_tracks(
        &self,
        profile_id: &str,
        page: Page,
    ) -> CanopyResult<LikedTrackPage> {
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
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes,
                (EXTRACT(EPOCH FROM ptl.liked_at) * 1000)::bigint AS liked_at_epoch_ms
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
        Ok(LikedTrackPage {
            items: rows
                .iter()
                .map(|row| LikedTrackItem {
                    item: media_item_from_row(row),
                    liked_at_epoch_ms: epoch_ms_i64(row, "liked_at_epoch_ms"),
                })
                .collect(),
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
// PgPlaylistRepository
// ---------------------------------------------------------------------------

/// PostgreSQL-backed profile playlist repository.
#[derive(Clone)]
pub struct PgPlaylistRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgPlaylistRepository {
    /// Creates a new repository backed by the given connection pool.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    async fn ensure_owned_playlist(
        &self,
        profile_uuid: uuid::Uuid,
        playlist_uuid: uuid::Uuid,
        playlist_id: &str,
    ) -> CanopyResult<()> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM profile_playlists WHERE id = $1 AND profile_id = $2)",
        )
        .bind(playlist_uuid)
        .bind(profile_uuid)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        if exists {
            Ok(())
        } else {
            Err(CanopyError::not_found("playlist", playlist_id))
        }
    }
}

fn playlist_from_row(row: &sqlx::postgres::PgRow) -> Playlist {
    Playlist {
        id: row.try_get("id").unwrap_or_default(),
        profile_id: row.try_get("profile_id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        description: row.try_get("description").unwrap_or_default(),
        created_at_epoch_ms: epoch_ms_i64(row, "created_at_epoch_ms"),
        updated_at_epoch_ms: epoch_ms_i64(row, "updated_at_epoch_ms"),
    }
}

fn playlist_track_item_from_row(row: &sqlx::postgres::PgRow) -> PlaylistTrackItem {
    PlaylistTrackItem {
        playlist_id: row.try_get("playlist_id").unwrap_or_default(),
        item: media_item_from_row(row),
        position: row.try_get("position").unwrap_or_default(),
        added_at_epoch_ms: epoch_ms_i64(row, "added_at_epoch_ms"),
    }
}

#[async_trait]
impl PlaylistRepository for PgPlaylistRepository {
    async fn create_playlist(
        &self,
        profile_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let row = sqlx::query(
            r#"
                INSERT INTO profile_playlists (profile_id, name, description)
                VALUES ($1, $2, $3)
                RETURNING
                    id::text,
                    profile_id::text,
                    name,
                    description,
                    (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_epoch_ms,
                    (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_epoch_ms
            "#,
        )
        .bind(profile_uuid)
        .bind(name)
        .bind(description)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(playlist_from_row(&row))
    }

    async fn get_playlist(
        &self,
        profile_id: &str,
        playlist_id: &str,
    ) -> CanopyResult<Option<Playlist>> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        let row = sqlx::query(
            r#"
                SELECT
                    id::text,
                    profile_id::text,
                    name,
                    description,
                    (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_epoch_ms,
                    (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_epoch_ms
                FROM profile_playlists
                WHERE id = $1 AND profile_id = $2
            "#,
        )
        .bind(playlist_uuid)
        .bind(profile_uuid)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(row.as_ref().map(playlist_from_row))
    }

    async fn update_playlist(
        &self,
        profile_id: &str,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        let row = sqlx::query(
            r#"
                UPDATE profile_playlists
                SET name = $3,
                    description = $4,
                    updated_at = NOW()
                WHERE id = $1 AND profile_id = $2
                RETURNING
                    id::text,
                    profile_id::text,
                    name,
                    description,
                    (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_epoch_ms,
                    (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_epoch_ms
            "#,
        )
        .bind(playlist_uuid)
        .bind(profile_uuid)
        .bind(name)
        .bind(description)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        row.as_ref()
            .map(playlist_from_row)
            .ok_or_else(|| CanopyError::not_found("playlist", playlist_id))
    }

    async fn delete_playlist(&self, profile_id: &str, playlist_id: &str) -> CanopyResult<()> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        let result = sqlx::query("DELETE FROM profile_playlists WHERE id = $1 AND profile_id = $2")
            .bind(playlist_uuid)
            .bind(profile_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        if result.rows_affected() == 0 {
            Err(CanopyError::not_found("playlist", playlist_id))
        } else {
            Ok(())
        }
    }

    async fn list_playlists(&self, profile_id: &str, page: Page) -> CanopyResult<PlaylistPage> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let rows = sqlx::query(
            r#"
                SELECT
                    id::text,
                    profile_id::text,
                    name,
                    description,
                    (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_epoch_ms,
                    (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_epoch_ms
                FROM profile_playlists
                WHERE profile_id = $1
                ORDER BY updated_at DESC, name
                LIMIT $2 OFFSET $3
            "#,
        )
        .bind(profile_uuid)
        .bind(page.limit as i64)
        .bind(page.offset as i64)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        let total_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM profile_playlists WHERE profile_id = $1")
                .bind(profile_uuid)
                .fetch_one(self.pool.as_ref())
                .await
                .map_err(db_err)?;

        Ok(PlaylistPage {
            items: rows.iter().map(playlist_from_row).collect(),
            total_count: total_count as i32,
            has_more: (page.offset + page.limit) < total_count as u32,
        })
    }

    async fn add_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
        position: Option<i32>,
    ) -> CanopyResult<PlaylistTrackItem> {
        if let Some(position) = position
            && position < 0
        {
            return Err(CanopyError::InvalidArgument(
                "position must be non-negative".into(),
            ));
        }
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        self.ensure_owned_playlist(profile_uuid, playlist_uuid, playlist_id)
            .await?;

        let position = if let Some(position) = position {
            position
        } else {
            sqlx::query_scalar::<_, Option<i32>>(
                "SELECT MAX(position) + 1 FROM profile_playlist_tracks WHERE playlist_id = $1",
            )
            .bind(playlist_uuid)
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(db_err)?
            .unwrap_or(0)
        };

        sqlx::query(
            r#"
                INSERT INTO profile_playlist_tracks (playlist_id, track_id, position)
                VALUES ($1, $2, $3)
                ON CONFLICT (playlist_id, track_id) DO UPDATE SET
                    position = EXCLUDED.position,
                    updated_at = NOW()
            "#,
        )
        .bind(playlist_uuid)
        .bind(track_uuid)
        .bind(position)
        .execute(self.pool.as_ref())
        .await
        .map_err(|err| map_track_write_err(err, track_id))?;

        sqlx::query("UPDATE profile_playlists SET updated_at = NOW() WHERE id = $1")
            .bind(playlist_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let sql = format!(
            r#"
            SELECT
                ppt.playlist_id::text AS playlist_id,
                ppt.position          AS position,
                (EXTRACT(EPOCH FROM ppt.added_at) * 1000)::bigint AS added_at_epoch_ms,
                t.id                  AS track_id,
                t.title               AS track_title,
                a.name                AS artist_name,
                al.title              AS album_title,
                t.duration_ms         AS track_duration_ms,
                t.is_explicit         AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type       AS asset_content_type,
                aa.size_bytes         AS asset_size_bytes
            FROM profile_playlist_tracks ppt
            JOIN tracks t      ON ppt.track_id = t.id
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE ppt.playlist_id = $1 AND ppt.track_id = $2
            "#
        );
        let row = sqlx::query(AssertSqlSafe(sql))
            .bind(playlist_uuid)
            .bind(track_uuid)
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(playlist_track_item_from_row(&row))
    }

    async fn remove_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
    ) -> CanopyResult<()> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        let track_uuid = parse_uuid_arg(track_id, "track_id")?;
        self.ensure_owned_playlist(profile_uuid, playlist_uuid, playlist_id)
            .await?;

        sqlx::query("DELETE FROM profile_playlist_tracks WHERE playlist_id = $1 AND track_id = $2")
            .bind(playlist_uuid)
            .bind(track_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        sqlx::query("UPDATE profile_playlists SET updated_at = NOW() WHERE id = $1")
            .bind(playlist_uuid)
            .execute(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        Ok(())
    }

    async fn reorder_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_ids: &[String],
    ) -> CanopyResult<Playlist> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        let mut requested = Vec::with_capacity(track_ids.len());
        let mut seen = std::collections::HashSet::new();
        for track_id in track_ids {
            if !seen.insert(track_id.clone()) {
                return Err(CanopyError::InvalidArgument(
                    "reorder track_ids must be unique".into(),
                ));
            }
            requested.push(parse_uuid_arg(track_id, "track_id")?);
        }

        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM profile_playlists WHERE id = $1 AND profile_id = $2)",
        )
        .bind(playlist_uuid)
        .bind(profile_uuid)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        if !owned {
            return Err(CanopyError::not_found("playlist", playlist_id));
        }

        let current: Vec<uuid::Uuid> = sqlx::query_scalar(
            "SELECT track_id FROM profile_playlist_tracks WHERE playlist_id = $1 ORDER BY position",
        )
        .bind(playlist_uuid)
        .fetch_all(&mut *tx)
        .await
        .map_err(db_err)?;

        let mut current_sorted = current.clone();
        current_sorted.sort();
        let mut requested_sorted = requested.clone();
        requested_sorted.sort();
        if current_sorted != requested_sorted {
            return Err(CanopyError::InvalidArgument(
                "reorder must include exactly the playlist track_ids".into(),
            ));
        }

        for (position, track_uuid) in requested.iter().enumerate() {
            let position = i32::try_from(position)
                .map_err(|_| CanopyError::InvalidArgument("playlist is too large".into()))?;
            sqlx::query(
                "UPDATE profile_playlist_tracks SET position = $3, updated_at = NOW() WHERE playlist_id = $1 AND track_id = $2",
            )
            .bind(playlist_uuid)
            .bind(track_uuid)
            .bind(position)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }

        let playlist_row = sqlx::query(
            r#"
                UPDATE profile_playlists
                SET updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id::text,
                    profile_id::text,
                    name,
                    description,
                    (EXTRACT(EPOCH FROM created_at) * 1000)::bigint AS created_at_epoch_ms,
                    (EXTRACT(EPOCH FROM updated_at) * 1000)::bigint AS updated_at_epoch_ms
            "#,
        )
        .bind(playlist_uuid)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        let playlist = playlist_from_row(&playlist_row);

        tx.commit().await.map_err(db_err)?;
        Ok(playlist)
    }

    async fn list_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        page: Page,
    ) -> CanopyResult<PlaylistTrackPage> {
        let profile_uuid = parse_uuid_arg(profile_id, "profile_id")?;
        let playlist_uuid = parse_uuid_arg(playlist_id, "playlist_id")?;
        self.ensure_owned_playlist(profile_uuid, playlist_uuid, playlist_id)
            .await?;

        let sql = format!(
            r#"
            SELECT
                ppt.playlist_id::text AS playlist_id,
                ppt.position          AS position,
                (EXTRACT(EPOCH FROM ppt.added_at) * 1000)::bigint AS added_at_epoch_ms,
                t.id             AS track_id,
                t.title          AS track_title,
                a.name           AS artist_name,
                al.title         AS album_title,
                t.duration_ms    AS track_duration_ms,
                t.is_explicit    AS track_explicit,
                COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
                aa.content_type  AS asset_content_type,
                aa.size_bytes    AS asset_size_bytes
            FROM profile_playlist_tracks ppt
            JOIN tracks t      ON ppt.track_id = t.id
            JOIN artists a     ON t.artist_id = a.id
            JOIN albums al     ON t.album_id = al.id
            {REPRESENTATIVE_ASSET_JOIN}
            WHERE ppt.playlist_id = $1
            ORDER BY ppt.position, ppt.added_at, t.title
            LIMIT $2 OFFSET $3
        "#
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(playlist_uuid)
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(db_err)?;

        let total_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM profile_playlist_tracks WHERE playlist_id = $1",
        )
        .bind(playlist_uuid)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(PlaylistTrackPage {
            items: rows.iter().map(playlist_track_item_from_row).collect(),
            total_count: total_count as i32,
            has_more: (page.offset + page.limit) < total_count as u32,
        })
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
