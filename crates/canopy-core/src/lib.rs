//! Transport-agnostic core of the Canopy backend.
//!
//! This crate holds the domain model, the error type, and the repository
//! ports (traits) that domain services depend on. It intentionally has no
//! dependency on `tonic`, `sqlx`, or any other transport / storage backend so
//! that it can be unit-tested in isolation and reused across adapters.

pub mod error;
pub mod identity;
pub mod identity_repository;
pub mod model;
pub mod repository;

pub use error::{CanopyError, CanopyResult};
pub use identity::{AccountStatus, AuthSession};
pub use identity_repository::{
    AccountRecord, ChangePasswordRecord, CompletePasswordResetRecord, ConsumeChallenge,
    ConsumedGoogleLoginChallenge, CreateEmailVerificationChallenge, CreateGoogleLinkChallenge,
    CreateGoogleLoginChallenge, CreatePasswordResetChallenge, CreateSessionRecord,
    ExternalIdentityRecord, IdentityRepository, PasswordCredentialRecord, PasswordLoginRecord,
    RateLimitBucket, RateLimitState, RegisterPasswordRecord, RotateRefreshTokenRecord,
    StoredAuthenticatedSession,
};
pub use model::{
    AudioAsset, AuthorizedStreamAsset, IngestStatus, LibraryItem, LikedTrackItem, LikedTrackPage,
    MediaItem, MediaPage, MediaVisibility, Page, PageTokenCodec, PendingImportOutcome,
    PendingMediaImport, PlayableAsset, PlaybackHistoryEntry, PlaybackHistoryEvent,
    PlaybackHistoryPage, PlaybackSource, Playlist, PlaylistPage, PlaylistTrack, PlaylistTrackItem,
    PlaylistTrackPage, ProfilePreferences, ProviderAudioAsset, ProviderLicense, ProviderTrack,
    SavedTrackItem, SavedTrackPage, StreamAudience, TrackLike, UserIdentity, UserProfile,
};
pub use repository::{
    AudioAssetRepository, CatalogIngest, CatalogRepository, DiscoveryRepository, IngestBatchResult,
    InstanceSettingsRepository, LibraryRepository, LikeRepository, MediaImportRepository,
    PlayableAssetRepository, PlaybackHistoryRepository, PlaylistRepository, PreferencesRepository,
    ProfileRepository,
};
