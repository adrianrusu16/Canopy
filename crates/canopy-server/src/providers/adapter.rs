//! Provider adapter traits and implementations.
//!
//! A provider adapter maps catalog source data into the Canopy domain model
//! and hands it to the ingestion service. The deterministic local fixture is
//! the supported implementation; adapters never become playback authorities.

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
    /// Human-readable stable source name.
    fn name(&self) -> &str;

    /// Fetches all available tracks from the provider.
    ///
    /// The returned list may be empty if the provider has no new content or if
    /// the source is unavailable. Errors are provider-specific (network,
    /// parse, authentication).
    async fn fetch_catalog(&self) -> CanopyResult<Vec<ProviderTrack>>;
}
