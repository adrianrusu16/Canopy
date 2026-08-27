//! Transport-agnostic core of the Canopy backend.
//!
//! This crate holds the domain model, the error type, and the repository
//! ports (traits) that domain services depend on. It intentionally has no
//! dependency on `tonic`, `sqlx`, or any other transport / storage backend so
//! that it can be unit-tested in isolation and reused across adapters.

pub mod access;
pub mod auth_outbox_repository;
pub mod error;
pub mod identity;
pub mod identity_repository;
pub mod model;
pub mod repository;

pub use access::TrackAccessScope;
pub use auth_outbox_repository::{
    AuthOutboxFailureKind, AuthOutboxRepository, ClaimAuthOutboxBatch, ClaimedAuthOutbox,
    MarkAuthOutboxFailed,
};
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
    AudioAsset, AuthorizedArtworkAsset, AuthorizedStreamAsset, IngestStatus, LibraryItem,
    LikedTrackItem, LikedTrackPage, MediaArtwork, MediaItem, MediaPage, MediaVisibility, Page,
    PageTokenCodec, PendingImportOutcome, PendingMediaImport, PlayableAsset, PlaybackHistoryEntry,
    PlaybackHistoryEvent, PlaybackHistoryPage, PlaybackSource, Playlist, PlaylistPage,
    PlaylistTrack, PlaylistTrackItem, PlaylistTrackPage, ProfilePreferences, ProviderAudioAsset,
    ProviderLicense, ProviderTrack, SavedTrackItem, SavedTrackPage, StreamAudience, TrackLike,
    UserIdentity, UserProfile,
};
pub use repository::{
    AudioAssetRepository, CatalogIngest, CatalogRepository, DiscoveryRepository, IngestBatchResult,
    InstanceSettingsRepository, LibraryRepository, LikeRepository, MediaImportRepository,
    PlayableAssetRepository, PlaybackHistoryRepository, PlaylistRepository, PreferencesRepository,
    ProfileRepository,
};
#[cfg(test)]
mod auth_outbox_contract_tests {
    use super::{AuthOutboxFailureKind, ClaimedAuthOutbox};

    #[test]
    fn auth_outbox_failure_kinds_have_stable_database_values() {
        let values = [
            (AuthOutboxFailureKind::Configuration, "configuration"),
            (AuthOutboxFailureKind::Connection, "connection"),
            (AuthOutboxFailureKind::Timeout, "timeout"),
            (AuthOutboxFailureKind::Authentication, "authentication"),
            (AuthOutboxFailureKind::Rejected, "rejected"),
            (AuthOutboxFailureKind::Payload, "payload"),
            (AuthOutboxFailureKind::Internal, "internal"),
        ];

        for (kind, expected) in values {
            assert_eq!(kind.as_str(), expected);
        }
    }

    #[test]
    fn claimed_auth_outbox_debug_redacts_ciphertext() {
        let record = ClaimedAuthOutbox {
            id: "outbox-1".into(),
            kind: "email_verification".into(),
            encrypted_payload: b"secret-ciphertext".to_vec(),
            key_id: "key-1".into(),
            attempts: 1,
            lease_token: "lease-1".into(),
        };

        let debug = format!("{record:?}");
        assert!(!debug.contains("secret-ciphertext"));
        assert!(debug.contains("[REDACTED]"));
    }
}
