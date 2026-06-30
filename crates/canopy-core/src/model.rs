//! Domain model for Canopy.
//!
//! These types are deliberately independent of the wire (`canopy-proto`) and
//! of any storage backend. Adapters translate between these types and their
//! proto / database representations at the edges of the system.

/// A single browsable / playable catalog entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaItem {
    /// Stable backend-side identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Performing artist.
    pub artist: String,
    /// Album the track belongs to.
    pub album: String,
    /// Artwork URI the Android resolver understands.
    pub artwork_uri: String,
    /// Duration in milliseconds (`-1` if unknown).
    pub duration_ms: i64,
    /// Bitrate in kbps.
    pub bitrate_kbps: i32,
    /// MIME type, e.g. `audio/mp4`.
    pub mime_type: String,
    /// Whether the item is flagged explicit.
    pub is_explicit: bool,
}

/// A page of catalog items together with paging metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaPage {
    /// Items in the current page.
    pub items: Vec<MediaItem>,
    /// Total number of items matching the query.
    pub total_count: i32,
    /// Whether more items exist beyond this page.
    pub has_more: bool,
}

/// Catalog visibility enforced before items reach a client.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MediaVisibility {
    /// Media visible only to its owning profile.
    Personal,
    /// License-approved media available to every client.
    ReleaseSafe,
    /// Media unavailable to catalog and playback operations.
    #[default]
    Quarantined,
}

/// Recoverable lifecycle of a managed media import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IngestStatus {
    /// Metadata exists but file placement has not completed.
    Pending,
    /// Metadata and managed files are ready for use.
    Ready,
    /// The import is unavailable and requires review.
    #[default]
    Quarantined,
}

/// Paging parameters shared by browse and search queries.
#[derive(Clone, Copy, Debug)]
pub struct Page {
    /// Maximum number of items to return.
    pub limit: u32,
    /// Number of leading items to skip.
    pub offset: u32,
}

/// A single encoded representation of a track in Canopy-managed storage.
///
/// A track has one audio asset per codec it is available in; the playback
/// resolver chooses among them. Mirrors the `audio_assets` table in the
/// architecture document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioAsset {
    /// Identifier of the track this asset encodes.
    pub track_id: String,
    /// Codec short name, e.g. `mp3`, `opus`, `flac`.
    pub codec: String,
    /// MIME type served for the asset, e.g. `audio/mpeg`.
    pub content_type: String,
    /// Validated relative key within Canopy's managed media library.
    pub storage_key: String,
    /// Size of the encoded asset in bytes.
    pub size_bytes: u64,
    /// SHA-256 checksum of the asset contents (hex-encoded).
    pub checksum_sha256: String,
    /// Duration of the asset in milliseconds.
    pub duration_ms: u64,
}

/// A resolved, ready-to-stream source for a track.
///
/// This is the domain counterpart of the proto `PlaybackSource` contract: it
/// carries a presigned, time-limited URL that the player streams directly from
/// object storage, keeping Canopy out of the byte-serving path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackSource {
    /// Identifier of the resolved track.
    pub track_id: String,
    /// Presigned URL the player streams from (signature + expiry embedded).
    pub stream_url: String,
    /// MIME type of the stream, e.g. `audio/mpeg`.
    pub content_type: String,
    /// Codec short name of the chosen asset.
    pub codec: String,
    /// Duration of the track in milliseconds.
    pub duration_ms: u64,
    /// Wall-clock expiry of the URL, in epoch milliseconds.
    pub expires_at_epoch_ms: u64,
}

/// The authenticated end-user a request is acting on behalf of.
///
/// PandaWave users authenticate once and carry a session token through
/// PandaEngine to Canopy; verifying that token yields this identity, which is
/// what `playback_history` and personalization are scoped to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserIdentity {
    /// Stable backend-side identifier of the end user.
    pub user_id: String,
}

/// Durable profile for a real logged-in user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserProfile {
    /// Internal profile identifier.
    pub id: String,
    /// Stable identity from the login provider/token issuer.
    pub external_user_id: String,
    /// Optional display name supplied by the client/profile provider.
    pub display_name: Option<String>,
    /// Whether the user has opted into durable backend playback history.
    pub history_enabled: bool,
}

/// A durable playback-history event for a real logged-in profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackHistoryEvent {
    /// Internal profile identifier that owns the event.
    pub profile_id: String,
    /// Track identifier that was played.
    pub track_id: String,
    /// Duration listened, in milliseconds.
    pub duration_ms: i64,
    /// Completion percentage in the inclusive range `0.0..=1.0`.
    pub completion_pct: f32,
}

/// A renderable playback-history event owned by a real profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackHistoryEntry {
    /// Stable history-event identifier.
    pub id: String,
    /// Time the playback was recorded, in epoch milliseconds.
    pub played_at_epoch_ms: u64,
    /// Duration listened, in milliseconds.
    pub duration_ms: i64,
    /// Completion percentage in the inclusive range `0.0..=1.0`.
    pub completion_pct: f32,
    /// Catalog metadata required to render and replay the event.
    pub item: MediaItem,
}

/// A page of chronological playback-history events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackHistoryPage {
    /// Events in the current page, newest first.
    pub entries: Vec<PlaybackHistoryEntry>,
    /// Total number of events owned by the profile.
    pub total_count: i32,
    /// Whether more events exist beyond this page.
    pub has_more: bool,
}

/// A saved catalog item in a real logged-in profile's library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryItem {
    /// Internal profile identifier that owns the saved item.
    pub profile_id: String,
    /// Track identifier that was saved.
    pub track_id: String,
    /// Time the item was saved, in epoch milliseconds.
    pub added_at_epoch_ms: u64,
}

/// A positive track like from a real logged-in profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackLike {
    /// Internal profile identifier that owns the like.
    pub profile_id: String,
    /// Track identifier that was liked.
    pub track_id: String,
    /// Time the track was liked, in epoch milliseconds.
    pub liked_at_epoch_ms: u64,
}

/// Profile-scoped preferences stored as a JSON document at the repository boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfilePreferences {
    /// Internal profile identifier that owns the preferences.
    pub profile_id: String,
    /// JSON preference document. Services validate this before writing.
    pub values_json: String,
}

/// A private playlist owned by a real logged-in profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Playlist {
    /// Playlist identifier.
    pub id: String,
    /// Internal profile identifier that owns the playlist.
    pub profile_id: String,
    /// Human-readable playlist name.
    pub name: String,
    /// Optional playlist description.
    pub description: String,
    /// Creation time in epoch milliseconds.
    pub created_at_epoch_ms: u64,
    /// Last update time in epoch milliseconds.
    pub updated_at_epoch_ms: u64,
}

/// A track membership row in a profile-owned playlist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaylistTrack {
    /// Playlist identifier.
    pub playlist_id: String,
    /// Track identifier.
    pub track_id: String,
    /// Zero-based ordering position.
    pub position: i32,
    /// Time the track was added, in epoch milliseconds.
    pub added_at_epoch_ms: u64,
}

/// A page of playlists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaylistPage {
    /// Playlists in the current page.
    pub items: Vec<Playlist>,
    /// Total number of playlists for the profile.
    pub total_count: i32,
    /// Whether more playlists exist beyond this page.
    pub has_more: bool,
}
/// A lightweight playback session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    /// Opaque session identifier.
    pub id: String,
    /// Currently loaded media, if any.
    pub current_media_id: Option<String>,
    /// Playback position in milliseconds.
    pub position_ms: i64,
    /// Playback speed multiplier.
    pub playback_speed: f64,
    /// Whether playback is currently active.
    pub is_playing: bool,
}

// ---------------------------------------------------------------------------
// Local media import model
// ---------------------------------------------------------------------------

/// Metadata persisted while a managed local-media import is pending.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingMediaImport {
    /// Application-generated track identifier used by staging and PostgreSQL.
    pub track_id: String,
    /// Explicit instance-owner profile that owns this personal track.
    pub owner_profile_id: String,
    /// Display title extracted from tags or the source filename.
    pub title: String,
    /// Performing artist extracted from tags or the conservative fallback.
    pub artist: String,
    /// Album extracted from tags or the conservative fallback.
    pub album: String,
    /// Parsed audio duration in milliseconds.
    pub duration_ms: u64,
    /// Optional validated relative artwork key in managed storage.
    pub artwork_storage_key: Option<String>,
    /// Managed MP3 asset metadata.
    pub audio: AudioAsset,
}

/// Result of attempting to persist a pending local-media import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingImportOutcome {
    /// A new pending track and asset were inserted.
    Inserted,
    /// Another import already owns the same audio checksum.
    Duplicate {
        /// Existing track identifier selected by checksum.
        track_id: String,
    },
}

// ---------------------------------------------------------------------------
// Provider ingestion model
// ---------------------------------------------------------------------------

/// A provider license record attached to a track.
///
/// Every track ingested through a provider adapter carries a license record;
/// a track with no resolvable license is not added to the catalog.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderLicense {
    /// License type (e.g., `CC0`, `CC-BY`, `Public Domain`).
    pub license_type: String,
    /// URL to the canonical license text or source page.
    pub source_url: String,
    /// Attribution text required by the license.
    pub attribution_text: String,
}

/// A provider audio asset - a single encoded representation of a track.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderAudioAsset {
    /// Codec short name, e.g. `mp3`, `opus`, `flac`.
    pub codec: String,
    /// MIME type served for the asset, e.g. `audio/mpeg`.
    pub content_type: String,
    /// Validated relative key within Canopy's managed media library.
    #[serde(alias = "object_key")]
    pub storage_key: String,
    /// Size of the encoded asset in bytes.
    pub size_bytes: u64,
    /// SHA-256 checksum of the asset contents (hex-encoded).
    pub checksum_sha256: String,
    /// Duration of the asset in milliseconds.
    pub duration_ms: u64,
}

/// A track as delivered by a provider adapter before normalization into the
/// Canopy catalog schema.
///
/// This is the canonical ingestion unit: a provider adapter produces a
/// `ProviderTrack`, and the `CatalogIngest` port persists it (upserting
/// artist, license, album, track, and assets in a single transaction).
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderTrack {
    /// Provider-specific stable identifier (used for deduplication).
    pub provider_id: String,
    /// Provider name (e.g., `musopen`, `pixabay`, `archive`).
    pub provider: String,
    /// Human-readable track title.
    pub title: String,
    /// Performing artist name.
    pub artist: String,
    /// Album title (optional; may be synthesized from single-track providers).
    pub album: String,
    /// Album release year (optional).
    pub release_year: Option<i32>,
    /// Track duration in milliseconds.
    pub duration_ms: i64,
    /// Whether the track is flagged explicit.
    pub is_explicit: bool,
    /// License record for this track.
    pub license: ProviderLicense,
    /// Audio assets available for this track (one per codec).
    pub assets: Vec<ProviderAudioAsset>,
    /// Track artwork storage key (overrides album artwork if set).
    #[serde(alias = "artwork_key")]
    pub artwork_storage_key: Option<String>,
    /// Album artwork storage key.
    #[serde(alias = "album_artwork_key")]
    pub album_artwork_storage_key: Option<String>,
}
