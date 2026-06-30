//! JadeStore - the persistence layer.
//!
//! This module is the home of all storage-backend implementations of the
//! [`canopy_core`] repository ports. Today it ships in-memory
//! implementations used by the prototype; the future PostgreSQL (`sqlx`) and
//! RustFS backends will live alongside them and be selected at wiring time.

mod memory;

#[cfg(feature = "pg")]
mod pg;
#[cfg(feature = "pg")]
mod pg_media_import;

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
pub use pg_media_import::PgMediaImportRepository;
