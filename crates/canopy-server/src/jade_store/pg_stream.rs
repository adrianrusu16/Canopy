//! PostgreSQL policy adapter for capability issuance and stream authorization.

use std::sync::Arc;

use async_trait::async_trait;
use canopy_core::{
    AuthorizedArtworkAsset, AuthorizedStreamAsset, CanopyError, CanopyResult, PlayableAsset,
    PlayableAssetRepository, StreamAudience,
};

#[derive(Clone)]
pub struct PgPlayableAssetRepository {
    pool: Arc<sqlx::PgPool>,
}

impl PgPlayableAssetRepository {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    async fn authorize_public(
        &self,
        asset_id: uuid::Uuid,
    ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
        let row = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
            r#"
                SELECT aa.id, aa.storage_key, aa.content_type
                FROM audio_assets aa
                JOIN tracks t ON t.id = aa.track_id
                WHERE aa.id = $1
                  AND t.visibility = 'release_safe'
                  AND t.ingest_status = 'ready'
            "#,
        )
        .bind(asset_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(row.map(authorized_asset))
    }

    async fn authorize_personal(
        &self,
        asset_id: uuid::Uuid,
    ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
        let row = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
            r#"
                SELECT aa.id, aa.storage_key, aa.content_type
                FROM audio_assets aa
                JOIN tracks t ON t.id = aa.track_id
                WHERE aa.id = $1
                  AND t.visibility = 'personal'
                  AND t.ingest_status = 'ready'
                  AND t.owner_profile_id = (
                      SELECT owner_profile_id
                      FROM instance_settings
                      WHERE singleton = TRUE
                  )
            "#,
        )
        .bind(asset_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(row.map(authorized_asset))
    }
}

#[async_trait]
impl PlayableAssetRepository for PgPlayableAssetRepository {
    async fn assets_for_personal_playback(
        &self,
        owner_profile_id: &str,
        track_id: &str,
    ) -> CanopyResult<Vec<PlayableAsset>> {
        let owner_profile_id = parse_uuid(owner_profile_id, "owner_profile_id")?;
        let track_id = parse_uuid(track_id, "track_id")?;
        let rows = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, i64)>(
            r#"
                SELECT aa.id, aa.track_id, aa.codec, aa.content_type, aa.duration_ms
                FROM audio_assets aa
                JOIN tracks t ON t.id = aa.track_id
                WHERE t.id = $1
                  AND t.owner_profile_id = $2
                  AND t.visibility = 'personal'
                  AND t.ingest_status = 'ready'
                ORDER BY aa.codec, aa.id
            "#,
        )
        .bind(track_id)
        .bind(owner_profile_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(
                |(asset_id, track_id, codec, content_type, duration_ms)| PlayableAsset {
                    asset_id: asset_id.to_string(),
                    track_id: track_id.to_string(),
                    codec,
                    content_type,
                    duration_ms: duration_ms.max(0) as u64,
                },
            )
            .collect())
    }
    async fn assets_for_public_playback(&self, track_id: &str) -> CanopyResult<Vec<PlayableAsset>> {
        let track_id = parse_uuid(track_id, "track_id")?;
        let rows = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, i64)>(
            r#"
                SELECT aa.id, aa.track_id, aa.codec, aa.content_type, aa.duration_ms
                FROM audio_assets aa
                JOIN tracks t ON t.id = aa.track_id
                WHERE t.id = $1
                  AND t.visibility = 'release_safe'
                  AND t.ingest_status = 'ready'
                ORDER BY aa.codec, aa.id
            "#,
        )
        .bind(track_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(
                |(asset_id, track_id, codec, content_type, duration_ms)| PlayableAsset {
                    asset_id: asset_id.to_string(),
                    track_id: track_id.to_string(),
                    codec,
                    content_type,
                    duration_ms: duration_ms.max(0) as u64,
                },
            )
            .collect())
    }

    async fn authorize_stream_asset(
        &self,
        asset_id: &str,
        audience: StreamAudience,
    ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
        let asset_id = parse_uuid(asset_id, "asset_id")?;
        match audience {
            StreamAudience::Public => self.authorize_public(asset_id).await,
            StreamAudience::Personal => self.authorize_personal(asset_id).await,
        }
    }

    async fn authorize_artwork(
        &self,
        artwork_id: &str,
        content_hash: &str,
    ) -> CanopyResult<Option<AuthorizedArtworkAsset>> {
        let artwork_id = parse_uuid(artwork_id, "artwork_id")?;
        let content_hash = content_hash.to_ascii_lowercase();
        if content_hash.len() != 64 || !content_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(None);
        }

        // v1: any artwork_assets row matching id + checksum is authorized.
        let row = sqlx::query_as::<_, (uuid::Uuid, String, String)>(
            r#"
                SELECT id, storage_key, content_type
                FROM artwork_assets
                WHERE id = $1
                  AND checksum_sha256 = $2
            "#,
        )
        .bind(artwork_id)
        .bind(&content_hash)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(db_err)?;

        Ok(row.map(|(id, storage_key, content_type)| AuthorizedArtworkAsset {
            artwork_id: id.to_string(),
            storage_key,
            content_type,
        }))
    }
}

fn authorized_asset(
    (asset_id, storage_key, content_type): (uuid::Uuid, String, String),
) -> AuthorizedStreamAsset {
    AuthorizedStreamAsset {
        asset_id: asset_id.to_string(),
        storage_key,
        content_type,
    }
}

fn parse_uuid(value: &str, field: &str) -> CanopyResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .map_err(|error| CanopyError::InvalidArgument(format!("invalid {field}: {error}")))
}

fn db_err(error: sqlx::Error) -> CanopyError {
    CanopyError::Storage(error.to_string())
}
