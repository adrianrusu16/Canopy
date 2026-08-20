//! Repository ports (hexagonal "driven" interfaces).
//!
//! Domain services depend on these traits, never on a concrete backend. The
//! in-memory and PostgreSQL implementations are interchangeable behind these
//! abstractions. Managed media remains behind the same policy-oriented ports.

use async_trait::async_trait;

use crate::access::TrackAccessScope;
use crate::error::CanopyResult;
use crate::model::{
    AudioAsset, AuthorizedStreamAsset, LibraryItem, LikedTrackPage, MediaItem, MediaPage, Page,
    PendingImportOutcome, PendingMediaImport, PlayableAsset, PlaybackHistoryEvent,
    PlaybackHistoryPage, Playlist, PlaylistPage, PlaylistTrackItem, PlaylistTrackPage,
    ProfilePreferences, ProviderTrack, SavedTrackPage, StreamAudience, TrackLike, UserProfile,
};

/// Read access to public and owner-scoped catalog partitions.
#[async_trait]
pub trait CatalogRepository: Send + Sync {
    /// Returns ready catalog items accessible to the supplied scope.
    async fn browse(
        &self,
        scope: &TrackAccessScope,
        parent_id: Option<&str>,
        genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage>;
    /// Searches ready catalog items accessible to the supplied scope.
    async fn search(
        &self,
        scope: &TrackAccessScope,
        query: &str,
        page: Page,
    ) -> CanopyResult<MediaPage>;
    /// Fetches one ready catalog item when it is accessible to the supplied scope.
    async fn get_media(
        &self,
        scope: &TrackAccessScope,
        media_id: &str,
    ) -> CanopyResult<Option<MediaItem>>;
    /// Lists ready personal media owned by the profile.
    async fn list_personal(&self, owner_profile_id: &str, page: Page) -> CanopyResult<MediaPage>;
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
/// available in and which managed asset identity backs playback, without
/// reading media bytes itself.
#[async_trait]
pub trait AudioAssetRepository: Send + Sync {
    /// Returns assets for a release-safe, ready track.
    async fn assets_for_public_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>>;
    /// Returns assets for a ready personal track owned by the profile.
    async fn assets_for_personal_track(
        &self,
        owner_profile_id: &str,
        track_id: &str,
    ) -> CanopyResult<Vec<AudioAsset>>;
}

/// Playback asset access scoped to capability issuance and authorization.
#[async_trait]
pub trait PlayableAssetRepository: Send + Sync {
    /// Returns selectable assets for a ready personal track owned by the profile.
    async fn assets_for_personal_playback(
        &self,
        owner_profile_id: &str,
        track_id: &str,
    ) -> CanopyResult<Vec<PlayableAsset>>;
    /// Returns selectable assets for a release-safe, ready track.
    async fn assets_for_public_playback(&self, track_id: &str) -> CanopyResult<Vec<PlayableAsset>>;

    /// Re-evaluates current policy before exposing an asset's storage metadata.
    async fn authorize_stream_asset(
        &self,
        asset_id: &str,
        audience: StreamAudience,
    ) -> CanopyResult<Option<AuthorizedStreamAsset>>;
}

/// Persistence of durable logged-in user profiles.
#[async_trait]
pub trait ProfileRepository: Send + Sync {
    /// Creates or updates a profile by real external user identity.
    async fn upsert_profile(
        &self,
        external_user_id: &str,
        display_name: Option<&str>,
        history_enabled: bool,
    ) -> CanopyResult<UserProfile>;

    /// Fetches a profile by real external user identity, if present.
    async fn get_by_external_user_id(
        &self,
        external_user_id: &str,
    ) -> CanopyResult<Option<UserProfile>>;

    /// Deletes a profile by external identity after lifecycle policy checks.
    async fn delete_by_external_user_id(&self, external_user_id: &str) -> CanopyResult<()>;
}

/// Persistence of singleton settings for this Canopy installation.
#[async_trait]
pub trait InstanceSettingsRepository: Send + Sync {
    /// Assigns the profile that exclusively owns personal media.
    async fn set_owner_profile_id(&self, profile_id: &str) -> CanopyResult<()>;

    /// Returns the assigned owner profile, if ownership has been configured.
    async fn owner_profile_id(&self) -> CanopyResult<Option<String>>;
}

/// Persistence boundary for recoverable local-media imports.
#[async_trait]
pub trait MediaImportRepository: Send + Sync {
    /// Finds a track that already owns the case-insensitive audio checksum.
    async fn find_track_by_audio_checksum(
        &self,
        checksum_sha256: &str,
    ) -> CanopyResult<Option<String>>;

    /// Atomically persists one personal pending track and its MP3 asset.
    async fn insert_pending(
        &self,
        pending: &PendingMediaImport,
    ) -> CanopyResult<PendingImportOutcome>;

    /// Publishes a fully placed personal import to owner-scoped catalog reads.
    async fn mark_ready(&self, track_id: &str) -> CanopyResult<()>;
}

/// Persistence of durable playback history for logged-in profiles.
#[async_trait]
pub trait PlaybackHistoryRepository: Send + Sync {
    /// Records one playback-history event while profile consent remains enabled.
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<bool>;

    /// Lists profile-owned events newest first.
    async fn list(&self, profile_id: &str, page: Page) -> CanopyResult<PlaybackHistoryPage>;

    /// Deletes one owned event; unknown and foreign identifiers return false.
    async fn delete_entry(&self, profile_id: &str, history_id: &str) -> CanopyResult<bool>;

    /// Deletes every event owned by a profile and returns the deleted count.
    async fn clear(&self, profile_id: &str) -> CanopyResult<u64>;
}

/// Persistence of saved library items for logged-in profiles.
#[async_trait]
pub trait LibraryRepository: Send + Sync {
    /// Saves a track to a profile library. Re-saving is idempotent.
    async fn save_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<LibraryItem>;

    /// Removes a track from a profile library. Removing an absent item succeeds.
    async fn remove_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()>;

    /// Lists saved library items newest first.
    async fn list_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<SavedTrackPage>;

    /// Returns whether the profile has saved the track.
    async fn is_saved(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool>;
}

/// Persistence of profile-owned positive track likes.
#[async_trait]
pub trait LikeRepository: Send + Sync {
    /// Likes a track for a profile. Re-liking is idempotent.
    async fn like_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<TrackLike>;

    /// Removes a like for a profile. Removing an absent like succeeds.
    async fn unlike_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()>;

    /// Lists liked tracks newest first.
    async fn list_liked_tracks(&self, profile_id: &str, page: Page)
    -> CanopyResult<LikedTrackPage>;

    /// Returns whether the profile has liked the track.
    async fn is_liked(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool>;
}

/// Persistence of profile-scoped preferences.
#[async_trait]
pub trait PreferencesRepository: Send + Sync {
    /// Fetches preferences for a profile, returning an empty JSON document if absent.
    async fn get_preferences(&self, profile_id: &str) -> CanopyResult<ProfilePreferences>;

    /// Creates or replaces preferences for a profile.
    async fn upsert_preferences(
        &self,
        profile_id: &str,
        values_json: &str,
    ) -> CanopyResult<ProfilePreferences>;
}

/// Persistence of profile-owned playlists.
#[async_trait]
pub trait PlaylistRepository: Send + Sync {
    /// Creates a playlist for a real profile.
    async fn create_playlist(
        &self,
        profile_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist>;

    /// Fetches one playlist owned by a real profile.
    async fn get_playlist(
        &self,
        profile_id: &str,
        playlist_id: &str,
    ) -> CanopyResult<Option<Playlist>>;

    /// Updates playlist metadata for a real profile.
    async fn update_playlist(
        &self,
        profile_id: &str,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist>;

    /// Deletes a playlist owned by a real profile.
    async fn delete_playlist(&self, profile_id: &str, playlist_id: &str) -> CanopyResult<()>;

    /// Lists playlists for a real profile.
    async fn list_playlists(&self, profile_id: &str, page: Page) -> CanopyResult<PlaylistPage>;

    /// Adds a track to a playlist. Adding the same track is idempotent.
    async fn add_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
        position: Option<i32>,
    ) -> CanopyResult<PlaylistTrackItem>;

    /// Removes a track from a playlist. Removing an absent track succeeds.
    async fn remove_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
    ) -> CanopyResult<()>;

    /// Rewrites playlist order using a complete ordered list of current track IDs.
    async fn reorder_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_ids: &[String],
    ) -> CanopyResult<Playlist>;

    /// Lists playlist tracks as renderable media items in playlist order.
    async fn list_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        page: Page,
    ) -> CanopyResult<PlaylistTrackPage>;
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
