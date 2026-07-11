//! Bounded gRPC adapters for the audited `canopy.v1` contract.

use std::sync::Arc;

use canopy_core::{CanopyError, CanopyResult, MediaItem, Page, PageTokenCodec, UserIdentity};
use canopy_proto::{
    AlbumSummary, ArtistSummary, ArtworkRef, PageInfo, PageRequest, Track, TrackSummary,
};

use crate::auth::AuthService;
use crate::catalog::CatalogService;
use crate::discovery::DiscoveryService;
use crate::health::HealthService;
use crate::history::HistoryService;
use crate::identity::IdentityService;
use crate::library::LibraryService;
use crate::likes::LikeService;
use crate::playback::ResolverService;
use crate::playlists::PlaylistService;
use crate::preferences::PreferencesService;
use crate::principal::PrincipalService;
use crate::profile::ProfileService;
use crate::search::SearchService;

mod auth;
mod catalog;
mod discovery;
mod history;
mod library;
mod playback;
mod playlist;
mod profile;
mod system;

pub use auth::AuthGrpc;
pub use catalog::CatalogGrpc;
pub use discovery::DiscoveryGrpc;
pub use history::HistoryGrpc;
pub use library::LibraryGrpc;
pub use playback::PlaybackGrpc;
pub use playlist::PlaylistGrpc;
pub use profile::ProfileGrpc;
pub use system::SystemGrpc;

/// Domain services shared by the bounded transport adapters.
pub struct GrpcServices {
    pub catalog: CatalogService,
    pub search: SearchService,
    pub profile: ProfileService,
    pub history: HistoryService,
    pub library: LibraryService,
    pub likes: LikeService,
    pub preferences: PreferencesService,
    pub playlists: PlaylistService,
    pub health: HealthService,
    pub resolver: ResolverService,
    pub discovery: DiscoveryService,
    pub auth: AuthService,
    pub identity: Option<Arc<IdentityService>>,
    pub principal: PrincipalService,
    pub page_tokens: Arc<PageTokenCodec>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DurablePrincipal {
    pub account_id: String,
    pub session_id: String,
    pub profile_id: String,
}

impl DurablePrincipal {
    pub(crate) fn user_identity(&self) -> UserIdentity {
        UserIdentity {
            user_id: self.account_id.clone(),
        }
    }
}

pub(crate) async fn extract_durable_principal(
    metadata: &tonic::metadata::MetadataMap,
    services: &GrpcServices,
) -> CanopyResult<DurablePrincipal> {
    let access_token = extract_bearer_token(metadata)?;
    let identity_service = services
        .identity
        .as_ref()
        .ok_or_else(|| CanopyError::unauthenticated("native identity is not configured"))?;
    let principal = identity_service
        .authenticate_access_token(access_token)
        .await?;
    let identity = UserIdentity {
        user_id: principal.account_id.clone(),
    };
    let profile = services.profile.get_profile(&identity).await?;
    Ok(DurablePrincipal {
        account_id: principal.account_id,
        session_id: principal.session_id,
        profile_id: profile.id,
    })
}

const DEFAULT_PAGE_SIZE: u32 = 20;
const MAX_PAGE_SIZE: u32 = 100;

pub(crate) fn to_track_summary(item: MediaItem) -> TrackSummary {
    let album = (!item.album.is_empty()).then(|| AlbumSummary {
        id: String::new(),
        title: item.album,
    });
    let artwork = (!item.artwork_uri.is_empty()).then_some(ArtworkRef {
        id: item.artwork_uri,
    });

    TrackSummary {
        id: item.id,
        title: item.title,
        artist: Some(ArtistSummary {
            id: String::new(),
            name: item.artist,
        }),
        album,
        duration_ms: u64::try_from(item.duration_ms).unwrap_or(0),
        explicit: item.is_explicit,
        artwork,
    }
}

pub(crate) fn to_track(item: MediaItem) -> Track {
    Track {
        summary: Some(to_track_summary(item)),
        genres: Vec::new(),
    }
}

pub(crate) fn page_from_request(
    request: Option<PageRequest>,
    codec: &PageTokenCodec,
) -> CanopyResult<Page> {
    let request = request.unwrap_or_default();
    let limit = match request.page_size {
        0 => DEFAULT_PAGE_SIZE,
        size => size.min(MAX_PAGE_SIZE),
    };
    let offset = if request.page_token.is_empty() {
        0
    } else {
        codec.decode(&request.page_token)?
    };
    Ok(Page { limit, offset })
}

pub(crate) fn page_info(
    page: Page,
    returned: usize,
    has_more: bool,
    codec: &PageTokenCodec,
) -> CanopyResult<PageInfo> {
    let next_page_token = if has_more {
        let returned = u32::try_from(returned)
            .map_err(|_| CanopyError::Internal("page result count overflow".into()))?;
        let offset = page
            .offset
            .checked_add(returned)
            .ok_or_else(|| CanopyError::Internal("page offset overflow".into()))?;
        codec.encode(offset)?
    } else {
        String::new()
    };
    Ok(PageInfo { next_page_token })
}

fn extract_bearer_token(metadata: &tonic::metadata::MetadataMap) -> CanopyResult<&str> {
    let raw = metadata
        .get("authorization")
        .ok_or_else(|| CanopyError::unauthenticated("missing authorization metadata"))?;
    let value = raw
        .to_str()
        .map_err(|_| CanopyError::unauthenticated("invalid authorization metadata"))?;
    let token = value
        .strip_prefix("Bearer ")
        .ok_or_else(|| CanopyError::unauthenticated("authorization must use Bearer token"))?
        .trim();
    if token.is_empty() {
        return Err(CanopyError::unauthenticated("authorization token is empty"));
    }
    Ok(token)
}

pub(crate) fn extract_metadata_identity(
    metadata: &tonic::metadata::MetadataMap,
    auth: &AuthService,
) -> CanopyResult<UserIdentity> {
    if metadata.get("authorization").is_some() {
        return auth.verify(extract_bearer_token(metadata)?);
    }

    if let Some(raw) = metadata.get("x-canopy-auth-token") {
        let token = raw
            .to_str()
            .map_err(|_| CanopyError::unauthenticated("invalid x-canopy-auth-token metadata"))?;
        return auth.verify(token.trim());
    }

    Err(CanopyError::unauthenticated("missing auth token"))
}

pub(crate) fn extract_optional_metadata_identity(
    metadata: &tonic::metadata::MetadataMap,
    auth: &AuthService,
) -> CanopyResult<Option<UserIdentity>> {
    if metadata.get("authorization").is_none() && metadata.get("x-canopy-auth-token").is_none() {
        return Ok(None);
    }

    extract_metadata_identity(metadata, auth).map(Some)
}

// Kept out of the module tree while behavior is migrated service by service.
// Remove after every bounded adapter reaches parity.
#[allow(dead_code)]
const LEGACY_ADAPTER_PATH: &str = "legacy.rs";

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use canopy_core::MediaItem;
    use canopy_proto::PageRequest;

    use super::*;

    fn codec() -> PageTokenCodec {
        PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap()
    }

    #[test]
    fn track_summary_uses_platform_neutral_artwork_and_nonnegative_duration() {
        let summary = to_track_summary(MediaItem {
            id: "trk_1".into(),
            title: "Track".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            artwork_uri: "artwork/track.jpg".into(),
            duration_ms: -1,
            is_explicit: true,
            ..MediaItem::default()
        });

        assert_eq!(summary.duration_ms, 0);
        assert_eq!(summary.artwork.unwrap().id, "artwork/track.jpg");
    }

    #[test]
    fn page_request_decodes_opaque_offset_and_mints_continuation() {
        let codec = codec();
        let token = codec.encode(40).unwrap();
        let page = page_from_request(
            Some(PageRequest {
                page_size: 20,
                page_token: token,
            }),
            &codec,
        )
        .unwrap();
        let info = page_info(page, 20, true, &codec).unwrap();

        assert_eq!(page.offset, 40);
        assert_eq!(codec.decode(&info.next_page_token).unwrap(), 60);
    }

    #[test]
    fn optional_identity_is_anonymous_only_when_metadata_is_absent() {
        let auth = AuthService::new("secret");
        let metadata = tonic::metadata::MetadataMap::new();

        assert!(
            extract_optional_metadata_identity(&metadata, &auth)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn optional_identity_verifies_bearer_metadata() {
        let auth = AuthService::new("secret");
        let token = auth.mint("owner-user", Duration::from_secs(60)).unwrap();
        let mut request = tonic::Request::new(());
        request.metadata_mut().insert(
            "authorization",
            tonic::metadata::MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
        );

        let identity = extract_optional_metadata_identity(request.metadata(), &auth)
            .unwrap()
            .unwrap();
        assert_eq!(identity.user_id, "owner-user");
    }

    #[test]
    fn optional_identity_rejects_malformed_present_metadata() {
        let auth = AuthService::new("secret");
        let mut request = tonic::Request::new(());
        request.metadata_mut().insert(
            "authorization",
            tonic::metadata::MetadataValue::from_static("not-bearer"),
        );

        assert!(matches!(
            extract_optional_metadata_identity(request.metadata(), &auth),
            Err(CanopyError::Unauthenticated(_))
        ));
    }

    #[test]
    fn bearer_token_rejects_blank_token() {
        let mut request = tonic::Request::new(());
        request.metadata_mut().insert(
            "authorization",
            tonic::metadata::MetadataValue::from_static("Bearer "),
        );

        assert!(matches!(
            extract_bearer_token(request.metadata()),
            Err(CanopyError::Unauthenticated(_))
        ));
    }
}
