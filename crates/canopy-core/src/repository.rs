//! Repository ports (hexagonal "driven" interfaces).
//!
//! Domain services depend on these traits, never on a concrete backend. The
//! in-memory implementation used by the prototype and the future PostgreSQL /
//! RustFS implementations are interchangeable behind these abstractions.

use async_trait::async_trait;

use crate::error::CanopyResult;
use crate::model::{AudioAsset, MediaItem, MediaPage, Page, ProviderTrack, Session};

/// Read access to the catalog (artists, albums, tracks, playlists).
#[async_trait]
pub trait CatalogRepository: Send + Sync {
    /// Returns a hierarchical browse page under `parent_id` (root if `None`),
    /// optionally filtered by `genres`.
    async fn browse(
        &self,
        parent_id: Option<&str>,
        genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage>;

    /// Full-text / trigram search over the catalog.
    async fn search(&self, query: &str, page: Page) -> CanopyResult<MediaPage>;

    /// Fetches a single item by identifier, if present.
    async fn get_media(&self, media_id: &str) -> CanopyResult<Option<MediaItem>>;
}

/// Source of the discovery shuffle channel.
///
/// Models the pre-shuffled materialized view described in the architecture:
/// the backend maintains a diversified, randomized ordering offline so the hot
/// path never pays for `ORDER BY random()`. The domain service layers
/// recently-played exclusion and diversity filtering on top of this pool.
#[async_trait]
pub trait DiscoveryRepository: Send + Sync {
    /// Returns the current shuffle pool in materialized-view order.
    async fn shuffle_pool(&self) -> CanopyResult<Vec<MediaItem>>;
}

/// Read access to the encoded audio assets backing a track.
///
/// The playback resolver uses this to discover which codecs a track is
/// available in and where the bytes live in object storage, without itself
/// touching the storage backend.
#[async_trait]
pub trait AudioAssetRepository: Send + Sync {
    /// Returns every audio asset available for `track_id` (one per codec).
    async fn assets_for_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>>;
}

/// Persistence of lightweight playback sessions.
#[async_trait]
pub trait SessionRepository: Send + Sync {
    /// Creates a fresh session and returns its identifier.
    async fn create(&self) -> CanopyResult<String>;

    /// Fetches a session by identifier, if present.
    async fn get(&self, id: &str) -> CanopyResult<Option<Session>>;

    /// Persists the given session state.
    async fn update(&self, session: Session) -> CanopyResult<()>;

    /// Removes a session by identifier.
    async fn delete(&self, id: &str) -> CanopyResult<()>;
}

/// Write access to the catalog for provider ingestion.
///
/// A provider adapter produces a [`ProviderTrack`] and calls `ingest` on this
/// port. The implementation is responsible for upserting artist, license,
/// album, track, and audio_assets rows in a single transaction, using
/// provider-specific identifiers for deduplication.
#[async_trait]
pub trait CatalogIngest: Send + Sync {
    /// Ingests a single provider track into the catalog.
    ///
    /// The implementation upserts the artist (by name), license (by source_url),
    /// album (by artist_id + title), track (by provider_id), and audio assets
    /// (by track_id + codec). If the track already exists, metadata is updated;
    /// new assets are appended.
    async fn ingest(&self, track: ProviderTrack) -> CanopyResult<String>;

    /// Ingests a batch of provider tracks, returning the number successfully
    /// written and a list of failures.
    async fn ingest_batch(&self, tracks: Vec<ProviderTrack>) -> CanopyResult<IngestBatchResult>;
}

/// Result of a batch ingestion operation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IngestBatchResult {
    /// Number of tracks successfully ingested.
    pub succeeded: usize,
    /// Number of tracks that failed.
    pub failed: usize,
    /// Human-readable failure summaries (e.g., "license missing: trk_123").
    pub failures: Vec<String>,
}
