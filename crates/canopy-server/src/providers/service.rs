//! Ingestion service — coordinates provider adapters with the catalog write port.
//!
//! The ingestion service is the domain boundary between "what the provider
//! delivers" and "how the catalog stores it". It owns normalization rules
//! (e.g., defaulting missing album artwork, synthesizing album titles from
//! single-track providers) and batch tracking.

use std::sync::Arc;

use canopy_core::{
    CanopyResult, CatalogIngest, IngestBatchResult, ProviderAudioAsset, ProviderLicense,
    ProviderTrack,
};

use crate::providers::adapter::ProviderAdapter;

/// Coordinates provider → catalog ingestion.
pub struct IngestionService {
    ingest: Arc<dyn CatalogIngest>,
}

impl IngestionService {
    /// Creates a new ingestion service over the given write port.
    pub fn new(ingest: Arc<dyn CatalogIngest>) -> Self {
        Self { ingest }
    }

    /// Ingests the full catalog from a provider adapter.
    ///
    /// Normalizes each [`ProviderTrack`] (fills optional fields, ensures
    /// license is present) and delegates to the [`CatalogIngest`] port.
    pub async fn ingest_from_provider(
        &self,
        provider: &dyn ProviderAdapter,
    ) -> CanopyResult<IngestBatchResult> {
        let tracks = provider.fetch_catalog().await?;
        let normalized: Vec<ProviderTrack> = tracks.into_iter().map(normalize).collect();
        self.ingest.ingest_batch(normalized).await
    }
}

/// Normalizes a provider track before ingestion.
///
/// - Synthesizes an album title from the track title if the album is empty.
/// - Ensures the license is present (returns a default "Unknown" license if
///   missing, which the catalog ingest may reject).
/// - Synthesizes artwork keys from the track ID if none are provided.
fn normalize(mut track: ProviderTrack) -> ProviderTrack {
    if track.album.is_empty() {
        track.album = format!("{} — Single", track.title);
    }
    if track.license.license_type.is_empty() {
        track.license = ProviderLicense {
            license_type: "Unknown".into(),
            source_url: String::new(),
            attribution_text: String::new(),
        };
    }
    if track.assets.is_empty() {
        // A track with no assets is ingestable but unplayable. The catalog
        // ingest stores the track metadata anyway; the resolver will return
        // NotFound when asked for playback.
        track.assets.push(ProviderAudioAsset::default());
    }
    track
}
