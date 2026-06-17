//! Provider adapter traits and implementations.
//!
//! A provider adapter fetches catalog content from an external source
//! (Musopen, Pixabay Music, Internet Archive, or a local test fixture),
//! normalizes it into the Canopy domain model, and hands it to the ingestion
//! service for persistence.

use async_trait::async_trait;
use canopy_core::{CanopyResult, ProviderTrack};

/// Fetches tracks from an external catalog source.
///
/// Each provider returns a stream (or batch) of [`ProviderTrack`] items, which
/// are then passed to the `IngestionService` for normalization and storage.
/// The adapter is not responsible for deduplication, conflict resolution, or
/// storage — those live in the `CatalogIngest` port.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Human-readable name of the provider (e.g., `musopen`, `pixabay`).
    fn name(&self) -> &str;

    /// Fetches all available tracks from the provider.
    ///
    /// The returned list may be empty if the provider has no new content or if
    /// the source is unavailable. Errors are provider-specific (network,
    /// parse, authentication).
    async fn fetch_catalog(&self) -> CanopyResult<Vec<ProviderTrack>>;
}
