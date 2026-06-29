//! Transport-agnostic core of the Canopy backend.
//!
//! This crate holds the domain model, the error type, and the repository
//! ports (traits) that domain services depend on. It intentionally has no
//! dependency on `tonic`, `sqlx`, or any other transport / storage backend so
//! that it can be unit-tested in isolation and reused across adapters.

pub mod error;
pub mod model;
pub mod repository;
pub mod signing;

pub use error::{CanopyError, CanopyResult};
pub use model::{
    AudioAsset, LibraryItem, MediaItem, MediaPage, Page, PlaybackHistoryEntry,
    PlaybackHistoryEvent, PlaybackHistoryPage, PlaybackSource, Playlist, PlaylistPage,
    PlaylistTrack, ProfilePreferences, ProviderAudioAsset, ProviderLicense, ProviderTrack, Session,
    TrackLike, UserIdentity, UserProfile,
};
pub use repository::{
    AudioAssetRepository, CatalogIngest, CatalogRepository, DiscoveryRepository, IngestBatchResult,
    LibraryRepository, LikeRepository, PlaybackHistoryRepository, PlaylistRepository,
    PreferencesRepository, ProfileRepository, SessionRepository,
};

pub use signing::UrlSigner;
