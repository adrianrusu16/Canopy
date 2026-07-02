//! JadeStore - the persistence layer.
//!
//! This module implements the [`canopy_core`] repository ports with in-memory
//! stores for isolated development and tests, plus PostgreSQL stores for durable
//! catalog, profile, policy, and managed-media metadata.

mod memory;

#[cfg(feature = "pg")]
mod pg;
#[cfg(feature = "pg")]
mod pg_media_import;
#[cfg(feature = "pg")]
mod pg_stream;

pub use memory::{
    InMemoryAudioAssetEntry, InMemoryAudioAssetStore, InMemoryCatalog, InMemoryCatalogEntry,
    InMemoryInstanceSettingsStore, InMemoryLibraryStore, InMemoryLikeStore,
    InMemoryPlaybackHistoryStore, InMemoryPlaylistStore, InMemoryPreferencesStore,
    InMemoryProfileStore, InMemorySessionStore,
};

#[cfg(feature = "pg")]
pub use pg::{
    PgAudioAssetRepository, PgCatalogRepository, PgInstanceSettingsRepository, PgLibraryRepository,
    PgLikeRepository, PgPlaybackHistoryRepository, PgPlaylistRepository, PgPreferencesRepository,
    PgProfileRepository, PgSessionRepository,
};
#[cfg(feature = "pg")]
pub use pg_stream::PgPlayableAssetRepository;

#[cfg(feature = "pg")]
pub use pg_media_import::PgMediaImportRepository;
