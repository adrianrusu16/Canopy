//! Domain model for Canopy.
//!
//! These types are deliberately independent of the wire (`canopy-proto`) and
//! of any storage backend. Adapters translate between these types and their
//! proto / database representations at the edges of the system.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::{CanopyError, CanopyResult};

type HmacSha256 = Hmac<Sha256>;

/// Opaque artwork identity projected to `canopy.v1.ArtworkRef`.
///
/// Does not include storage keys or platform URIs. Consumers derive display
/// URLs from their media origin plus `id` and `content_hash`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaArtwork {
    /// Stable artwork resource id (`ArtworkRef.id`).
    pub id: String,
    /// Lowercase SHA-256 hex of the artwork bytes (`ArtworkRef.content_hash`).
    pub content_hash: String,
}

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
    /// Optional artwork identity (track override, else album).
    pub artwork: Option<MediaArtwork>,
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

/// Encodes page offsets as opaque, authenticated continuation tokens.
#[derive(Clone)]
pub struct PageTokenCodec {
    signing_key: [u8; 32],
}

impl PageTokenCodec {
    const VERSION: u8 = 1;
    const MIN_SECRET_BYTES: usize = 32;
    const PAYLOAD_BYTES: usize = 5;
    const MAX_TOKEN_BYTES: usize = 128;
    const KEY_CONTEXT: &'static [u8] = b"canopy/page-token/v1";

    /// Derives a page-token signing key from the instance capability secret.
    pub fn new(secret: impl AsRef<[u8]>) -> CanopyResult<Self> {
        let secret = secret.as_ref();
        if secret.len() < Self::MIN_SECRET_BYTES {
            return Err(CanopyError::InvalidArgument(
                "page token secret must contain at least 32 bytes".into(),
            ));
        }

        let mut derivation =
            HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts keys of any length");
        derivation.update(Self::KEY_CONTEXT);
        let signing_key: [u8; 32] = derivation.finalize().into_bytes().into();
        Ok(Self { signing_key })
    }

    /// Encodes an offset without exposing it as client-editable state.
    pub fn encode(&self, offset: u32) -> CanopyResult<String> {
        let mut payload = [0_u8; Self::PAYLOAD_BYTES];
        payload[0] = Self::VERSION;
        payload[1..].copy_from_slice(&offset.to_be_bytes());
        let encoded_payload = URL_SAFE_NO_PAD.encode(payload);
        let signature = self.sign(encoded_payload.as_bytes());
        Ok(format!(
            "{encoded_payload}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    /// Verifies and decodes a continuation token.
    pub fn decode(&self, token: &str) -> CanopyResult<u32> {
        self.decode_inner(token)
            .map_err(|()| CanopyError::InvalidArgument("invalid page token".into()))
    }

    fn decode_inner(&self, token: &str) -> Result<u32, ()> {
        if token.len() > Self::MAX_TOKEN_BYTES {
            return Err(());
        }
        let (payload, signature) = token.split_once('.').ok_or(())?;
        if signature.contains('.') {
            return Err(());
        }
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| ())?;
        let mut mac = HmacSha256::new_from_slice(&self.signing_key).map_err(|_| ())?;
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature).map_err(|_| ())?;

        let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| ())?;
        let payload: [u8; Self::PAYLOAD_BYTES] = payload.try_into().map_err(|_| ())?;
        if payload[0] != Self::VERSION {
            return Err(());
        }
        Ok(u32::from_be_bytes(payload[1..].try_into().map_err(|_| ())?))
    }

    fn sign(&self, payload: &[u8]) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(&self.signing_key)
            .expect("HMAC-SHA256 accepts keys of any length");
        mac.update(payload);
        mac.finalize().into_bytes().into()
    }
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

/// Audience encoded into a stream capability and enforced at authorization time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamAudience {
    /// Release-safe media available without an authenticated profile.
    Public,
    /// Owner-scoped media available through a future authenticated flow.
    Personal,
}

impl StreamAudience {
    /// Stable wire representation used by signed stream claims.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Personal => "personal",
        }
    }
}

/// Public metadata required to select and mint a capability for an audio asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayableAsset {
    /// Stable audio asset identifier.
    pub asset_id: String,
    /// Track encoded by this asset.
    pub track_id: String,
    /// Codec short name, such as `mp3`.
    pub codec: String,
    /// MIME type served for the asset.
    pub content_type: String,
    /// Duration of the asset in milliseconds.
    pub duration_ms: u64,
}

/// Storage metadata returned only after current playback policy is authorized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedStreamAsset {
    /// Stable audio asset identifier.
    pub asset_id: String,
    /// Validated relative key within the managed media library.
    pub storage_key: String,
    /// MIME type served for the asset.
    pub content_type: String,
}

/// Storage metadata returned only after artwork identity + content hash match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedArtworkAsset {
    /// Stable artwork asset identifier.
    pub artwork_id: String,
    /// Validated relative key within the managed media library.
    pub storage_key: String,
    /// MIME type served for the artwork.
    pub content_type: String,
}

/// A resolved, ready-to-stream source for a track.
///
/// This is the domain counterpart of the proto `PlaybackSource` contract: it
/// carries a short-lived Canopy capability URL. Nginx authorizes the capability
/// through Canopy and serves managed bytes without proxying them through Rust.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackSource {
    /// Identifier of the resolved track.
    pub track_id: String,
    /// Short-lived opaque capability URL the player streams from.
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

/// A saved catalog item rendered with relationship metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedTrackItem {
    /// Renderable catalog item that was saved.
    pub item: MediaItem,
    /// Time the item was saved, in epoch milliseconds.
    pub saved_at_epoch_ms: u64,
}

/// A page of saved catalog items.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedTrackPage {
    /// Items in the current page, newest first.
    pub items: Vec<SavedTrackItem>,
    /// Total number of saved tracks owned by the profile.
    pub total_count: i32,
    /// Whether more saved tracks exist beyond this page.
    pub has_more: bool,
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

/// A liked catalog item rendered with relationship metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LikedTrackItem {
    /// Renderable catalog item that was liked.
    pub item: MediaItem,
    /// Time the track was liked, in epoch milliseconds.
    pub liked_at_epoch_ms: u64,
}

/// A page of liked catalog items.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LikedTrackPage {
    /// Items in the current page, newest first.
    pub items: Vec<LikedTrackItem>,
    /// Total number of liked tracks owned by the profile.
    pub total_count: i32,
    /// Whether more liked tracks exist beyond this page.
    pub has_more: bool,
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

/// A playlist track rendered with relationship metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistTrackItem {
    /// Playlist identifier.
    pub playlist_id: String,
    /// Renderable catalog item in the playlist.
    pub item: MediaItem,
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

/// A page of playlist tracks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistTrackPage {
    /// Tracks in the current page.
    pub items: Vec<PlaylistTrackItem>,
    /// Total number of tracks in the playlist.
    pub total_count: i32,
    /// Whether more tracks exist beyond this page.
    pub has_more: bool,
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
    /// Optional lowercase SHA-256 of artwork bytes (from staging when present).
    pub artwork_checksum_sha256: Option<String>,
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

#[cfg(test)]
mod page_token_tests {
    use crate::{CanopyError, PageTokenCodec};

    #[test]
    fn round_trips_offset_without_exposing_it() {
        let codec = PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap();
        let token = codec.encode(42).unwrap();

        assert!(!token.contains("42"));
        assert_eq!(codec.decode(&token).unwrap(), 42);
    }

    #[test]
    fn rejects_tampering() {
        let codec = PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap();

        assert!(matches!(
            codec.decode("tampered"),
            Err(CanopyError::InvalidArgument(_))
        ));
    }
}
