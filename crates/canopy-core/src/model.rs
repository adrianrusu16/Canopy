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

/// Paging parameters shared by browse and search queries.
#[derive(Clone, Copy, Debug)]
pub struct Page {
    /// Maximum number of items to return.
    pub limit: u32,
    /// Number of leading items to skip.
    pub offset: u32,
}

/// A single encoded representation of a track as stored in object storage.
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
    /// Object-storage key (path within the media bucket).
    pub object_key: String,
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

/// A provider audio asset — a single encoded representation of a track.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderAudioAsset {
    /// Codec short name, e.g. `mp3`, `opus`, `flac`.
    pub codec: String,
    /// MIME type served for the asset, e.g. `audio/mpeg`.
    pub content_type: String,
    /// Object-storage key (path within the media bucket, e.g. `audio/tracks/musopen/trk_001.mp3`).
    pub object_key: String,
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
    /// Artwork object key (overrides album artwork if set).
    pub artwork_key: Option<String>,
    /// Album artwork object key.
    pub album_artwork_key: Option<String>,
}
