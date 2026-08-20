//! gRPC adapter: implements the generated `Canopy` service over the domain
//! services, translating proto messages to and from the domain model.

use async_trait::async_trait;
use canopy_core::{
    CanopyError, CanopyResult, MediaItem as DomainMediaItem, MediaPage, Page, UserIdentity,
    TrackAccessScope,
};
use canopy_proto::canopy_server::Canopy;
use canopy_proto::{
    AddPlaylistTrackRequest, AddPlaylistTrackResponse, CreatePlaylistRequest,
    CreatePlaylistResponse, DeletePlaylistRequest, DeletePlaylistResponse,
    ListPlaylistTracksRequest, ListPlaylistTracksResponse, ListPlaylistsRequest,
    ListPlaylistsResponse, Playlist as ProtoPlaylist, RemovePlaylistTrackRequest,
    RemovePlaylistTrackResponse, ReorderPlaylistTracksRequest, ReorderPlaylistTracksResponse,
    UpdatePlaylistRequest, UpdatePlaylistResponse,
};
use canopy_proto::{
    BrowseRequest, BrowseResponse, DiscoveryRequest, DiscoveryTrack, EndSessionRequest,
    EndSessionResponse, GetMediaRequest, GetMediaResponse, GetPreferencesRequest,
    GetPreferencesResponse, GetSessionRequest, GetSessionResponse, HealthDependency, HealthRequest,
    HealthResponse, LikeTrackRequest, LikeTrackResponse, ListLibraryItemsRequest,
    ListLibraryItemsResponse, ListLikedTracksRequest, ListLikedTracksResponse,
    MediaItem as ProtoMediaItem, PauseRequest, PauseResponse, PlayRequest, PlayResponse,
    PlaybackRequest, PlaybackSource as ProtoPlaybackSource, RecordPlaybackHistoryRequest,
    RecordPlaybackHistoryResponse, RemoveLibraryItemRequest, RemoveLibraryItemResponse,
    SaveLibraryItemRequest, SaveLibraryItemResponse, SearchRequest, SearchResponse, SeekRequest,
    SeekResponse, SetPlaybackSpeedRequest, SetPlaybackSpeedResponse, StopRequest, StopResponse,
    UnlikeTrackRequest, UnlikeTrackResponse, UpdatePreferencesRequest, UpdatePreferencesResponse,
    UpdateSessionRequest, UpdateSessionResponse, UpsertProfileRequest, UpsertProfileResponse,
    UserProfile as ProtoUserProfile,
};
use canopy_proto::{
    ClearPlaybackHistoryRequest, ClearPlaybackHistoryResponse, DeletePlaybackHistoryEntryRequest,
    DeletePlaybackHistoryEntryResponse, ListPlaybackHistoryRequest, ListPlaybackHistoryResponse,
    PlaybackHistoryEntry as ProtoPlaybackHistoryEntry,
};
use tonic::{Request, Response, Status};

use crate::api::to_status;
use crate::auth::AuthService;
use crate::catalog::CatalogService;
use crate::discovery::DiscoveryService;
use crate::health::HealthService;
use crate::history::HistoryService;
use crate::library::LibraryService;
use crate::likes::LikeService;
use crate::playback::{PlaybackService, ResolverService};
use crate::playlists::PlaylistService;
use crate::preferences::PreferencesService;
use crate::principal::PrincipalService;
use crate::profile::ProfileService;
use crate::search::SearchService;

/// Domain services exposed through the gRPC adapter.
pub struct GrpcServices {
    pub catalog: CatalogService,
    pub search: SearchService,
    pub playback: PlaybackService,
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
    pub principal: PrincipalService,
}

/// gRPC entry point wiring the wire contract to the domain services.
pub struct GrpcApi {
    catalog: CatalogService,
    search: SearchService,
    playback: PlaybackService,
    profile: ProfileService,
    history: HistoryService,
    library: LibraryService,
    likes: LikeService,
    preferences: PreferencesService,
    playlists: PlaylistService,
    health: HealthService,
    resolver: ResolverService,
    discovery: DiscoveryService,
    auth: AuthService,
    principal: PrincipalService,
}

impl GrpcApi {
    /// Creates a new gRPC adapter over the given domain services.
    pub fn new(services: GrpcServices) -> Self {
        Self {
            catalog: services.catalog,
            search: services.search,
            playback: services.playback,
            profile: services.profile,
            history: services.history,
            library: services.library,
            likes: services.likes,
            preferences: services.preferences,
            playlists: services.playlists,
            health: services.health,
            resolver: services.resolver,
            discovery: services.discovery,
            auth: services.auth,
            principal: services.principal,
        }
    }
}

fn to_proto_item(item: DomainMediaItem) -> ProtoMediaItem {
    ProtoMediaItem {
        id: item.id,
        title: item.title,
        artist: item.artist,
        album: item.album,
        artwork_uri: item.artwork_uri,
        duration_ms: item.duration_ms,
        bitrate_kbps: item.bitrate_kbps,
        mime_type: item.mime_type,
        is_explicit: item.is_explicit,
    }
}

fn to_proto_items(page: MediaPage) -> Vec<ProtoMediaItem> {
    page.items.into_iter().map(to_proto_item).collect()
}

fn to_proto_profile(profile: canopy_core::UserProfile) -> ProtoUserProfile {
    ProtoUserProfile {
        profile_id: profile.id,
        external_user_id: profile.external_user_id,
        display_name: profile.display_name.unwrap_or_default(),
        history_enabled: profile.history_enabled,
    }
}

fn to_proto_playlist(playlist: canopy_core::Playlist) -> ProtoPlaylist {
    ProtoPlaylist {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        created_at_epoch_ms: playlist.created_at_epoch_ms,
        updated_at_epoch_ms: playlist.updated_at_epoch_ms,
    }
}

fn to_proto_history_entry(entry: canopy_core::PlaybackHistoryEntry) -> ProtoPlaybackHistoryEntry {
    ProtoPlaybackHistoryEntry {
        history_id: entry.id,
        played_at_epoch_ms: entry.played_at_epoch_ms,
        duration_ms: entry.duration_ms,
        completion_pct: entry.completion_pct,
        item: Some(to_proto_item(entry.item)),
    }
}

fn history_page(limit: i32, offset: i32) -> CanopyResult<Page> {
    let limit = u32::try_from(limit)
        .map_err(|_| CanopyError::InvalidArgument("limit must be non-negative".into()))?;
    let offset = u32::try_from(offset)
        .map_err(|_| CanopyError::InvalidArgument("offset must be non-negative".into()))?;
    Ok(Page { limit, offset })
}

fn playlist_page(limit: i32, offset: i32) -> CanopyResult<Page> {
    let limit = u32::try_from(limit)
        .map_err(|_| CanopyError::InvalidArgument("limit must be non-negative".into()))?;
    let offset = u32::try_from(offset)
        .map_err(|_| CanopyError::InvalidArgument("offset must be non-negative".into()))?;
    Ok(Page { limit, offset })
}

fn extract_identity(
    metadata: &tonic::metadata::MetadataMap,
    body_token: &str,
    auth: &AuthService,
) -> CanopyResult<UserIdentity> {
    if let Some(raw) = metadata.get("authorization") {
        let value = raw
            .to_str()
            .map_err(|_| CanopyError::unauthenticated("invalid authorization metadata"))?;
        let Some(token) = value.strip_prefix("Bearer ") else {
            return Err(CanopyError::unauthenticated(
                "authorization must use Bearer token",
            ));
        };
        return auth.verify(token.trim());
    }

    if let Some(raw) = metadata.get("x-canopy-auth-token") {
        let token = raw
            .to_str()
            .map_err(|_| CanopyError::unauthenticated("invalid x-canopy-auth-token metadata"))?;
        return auth.verify(token.trim());
    }

    let body_token = body_token.trim();
    if !body_token.is_empty() {
        return auth.verify(body_token);
    }

    Err(CanopyError::unauthenticated("missing auth token"))
}

fn extract_metadata_identity(
    metadata: &tonic::metadata::MetadataMap,
    auth: &AuthService,
) -> CanopyResult<UserIdentity> {
    extract_identity(metadata, "", auth)
}

fn extract_optional_metadata_identity(
    metadata: &tonic::metadata::MetadataMap,
    auth: &AuthService,
) -> CanopyResult<Option<UserIdentity>> {
    if metadata.get("authorization").is_none() && metadata.get("x-canopy-auth-token").is_none() {
        return Ok(None);
    }

    extract_metadata_identity(metadata, auth).map(Some)
}

fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[async_trait]
impl Canopy for GrpcApi {
    async fn browse(
        &self,
        request: Request<BrowseRequest>,
    ) -> Result<Response<BrowseResponse>, Status> {
        let req = request.into_inner();
        let parent_id = if req.parent_id.is_empty() {
            None
        } else {
            Some(req.parent_id.as_str())
        };
        let page = Page {
            limit: req.limit as u32,
            offset: req.offset as u32,
        };
        let result = self
            .catalog
            .browse(&TrackAccessScope::Public, parent_id, &req.genres, page)
            .await
            .map_err(to_status)?;
        Ok(Response::new(BrowseResponse {
            total_count: result.total_count,
            has_more: result.has_more,
            items: to_proto_items(result),
        }))
    }

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let req = request.into_inner();
        let page = Page {
            limit: req.limit as u32,
            offset: req.offset as u32,
        };
        let result = self
            .search
            .search(&TrackAccessScope::Public, &req.query, page)
            .await
            .map_err(to_status)?;
        Ok(Response::new(SearchResponse {
            total_count: result.total_count,
            items: to_proto_items(result),
        }))
    }

    async fn get_media(
        &self,
        request: Request<GetMediaRequest>,
    ) -> Result<Response<GetMediaResponse>, Status> {
        let req = request.into_inner();
        let item = self
            .catalog
            .get_media(&TrackAccessScope::Public, &req.media_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(GetMediaResponse {
            item: item.map(to_proto_item),
        }))
    }

    async fn play(&self, request: Request<PlayRequest>) -> Result<Response<PlayResponse>, Status> {
        let req = request.into_inner();
        let session_id = self
            .playback
            .play(&req.session_id, req.media_id, req.start_pos_ms)
            .await
            .map_err(to_status)?;
        Ok(Response::new(PlayResponse {
            success: true,
            error_msg: String::new(),
            session_id,
        }))
    }

    async fn pause(
        &self,
        request: Request<PauseRequest>,
    ) -> Result<Response<PauseResponse>, Status> {
        let req = request.into_inner();
        self.playback
            .pause(&req.session_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(PauseResponse { success: true }))
    }

    async fn seek(&self, request: Request<SeekRequest>) -> Result<Response<SeekResponse>, Status> {
        let req = request.into_inner();
        self.playback
            .seek(&req.session_id, req.position_ms)
            .await
            .map_err(to_status)?;
        Ok(Response::new(SeekResponse { success: true }))
    }

    async fn set_playback_speed(
        &self,
        request: Request<SetPlaybackSpeedRequest>,
    ) -> Result<Response<SetPlaybackSpeedResponse>, Status> {
        let req = request.into_inner();
        self.playback
            .set_playback_speed(&req.session_id, req.speed)
            .await
            .map_err(to_status)?;
        Ok(Response::new(SetPlaybackSpeedResponse { success: true }))
    }

    async fn stop(&self, request: Request<StopRequest>) -> Result<Response<StopResponse>, Status> {
        let req = request.into_inner();
        self.playback
            .stop(&req.session_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(StopResponse { success: true }))
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .playback
            .get_session(&req.session_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(GetSessionResponse {
            session_id: req.session_id,
            current_media_id: session.current_media_id.unwrap_or_default(),
            position_ms: session.position_ms,
            playback_speed: session.playback_speed,
            is_playing: session.is_playing,
        }))
    }

    async fn update_session(
        &self,
        request: Request<UpdateSessionRequest>,
    ) -> Result<Response<UpdateSessionResponse>, Status> {
        let req = request.into_inner();
        let media_id = if req.media_id.is_empty() {
            None
        } else {
            Some(req.media_id)
        };
        let position_ms = if req.position_ms != 0 {
            Some(req.position_ms)
        } else {
            None
        };
        self.playback
            .update_session(&req.session_id, media_id, position_ms)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UpdateSessionResponse { success: true }))
    }

    async fn end_session(
        &self,
        request: Request<EndSessionRequest>,
    ) -> Result<Response<EndSessionResponse>, Status> {
        let req = request.into_inner();
        self.playback
            .end_session(&req.session_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(EndSessionResponse { success: true }))
    }

    async fn upsert_profile(
        &self,
        request: Request<UpsertProfileRequest>,
    ) -> Result<Response<UpsertProfileResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity =
            extract_identity(&metadata, &req.auth_token, &self.auth).map_err(to_status)?;
        let profile = self
            .profile
            .upsert_profile(&identity, &req.display_name, req.history_enabled)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UpsertProfileResponse {
            profile: Some(to_proto_profile(profile)),
        }))
    }

    async fn record_playback_history(
        &self,
        request: Request<RecordPlaybackHistoryRequest>,
    ) -> Result<Response<RecordPlaybackHistoryResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity =
            extract_identity(&metadata, &req.auth_token, &self.auth).map_err(to_status)?;
        let recorded = self
            .history
            .record_playback(
                &identity,
                &req.track_id,
                req.duration_ms,
                req.completion_pct,
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(RecordPlaybackHistoryResponse { recorded }))
    }

    async fn list_playback_history(
        &self,
        request: Request<ListPlaybackHistoryRequest>,
    ) -> Result<Response<ListPlaybackHistoryResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let request_page = history_page(req.limit, req.offset).map_err(to_status)?;
        let page = self
            .history
            .list_history(&identity, request_page)
            .await
            .map_err(to_status)?;

        Ok(Response::new(ListPlaybackHistoryResponse {
            entries: page
                .entries
                .into_iter()
                .map(to_proto_history_entry)
                .collect(),
            total_count: page.total_count,
            has_more: page.has_more,
        }))
    }

    async fn delete_playback_history_entry(
        &self,
        request: Request<DeletePlaybackHistoryEntryRequest>,
    ) -> Result<Response<DeletePlaybackHistoryEntryResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let deleted = self
            .history
            .delete_entry(&identity, &req.history_id)
            .await
            .map_err(to_status)?;

        Ok(Response::new(DeletePlaybackHistoryEntryResponse {
            deleted,
        }))
    }

    async fn clear_playback_history(
        &self,
        request: Request<ClearPlaybackHistoryRequest>,
    ) -> Result<Response<ClearPlaybackHistoryResponse>, Status> {
        let metadata = request.metadata().clone();
        let _req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let deleted_count = self
            .history
            .clear_history(&identity)
            .await
            .map_err(to_status)?;

        Ok(Response::new(ClearPlaybackHistoryResponse {
            deleted_count,
        }))
    }

    async fn save_library_item(
        &self,
        request: Request<SaveLibraryItemRequest>,
    ) -> Result<Response<SaveLibraryItemResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.library
            .save_track(&identity, &req.track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(SaveLibraryItemResponse { saved: true }))
    }

    async fn remove_library_item(
        &self,
        request: Request<RemoveLibraryItemRequest>,
    ) -> Result<Response<RemoveLibraryItemResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.library
            .remove_track(&identity, &req.track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(RemoveLibraryItemResponse { removed: true }))
    }

    async fn list_library_items(
        &self,
        request: Request<ListLibraryItemsRequest>,
    ) -> Result<Response<ListLibraryItemsResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let page = self
            .library
            .list_tracks(
                &identity,
                Page {
                    limit: req.limit as u32,
                    offset: req.offset as u32,
                },
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(ListLibraryItemsResponse {
            total_count: page.total_count,
            has_more: page.has_more,
            items: to_proto_items(page),
        }))
    }

    async fn like_track(
        &self,
        request: Request<LikeTrackRequest>,
    ) -> Result<Response<LikeTrackResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.likes
            .like_track(&identity, &req.track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(LikeTrackResponse { liked: true }))
    }

    async fn unlike_track(
        &self,
        request: Request<UnlikeTrackRequest>,
    ) -> Result<Response<UnlikeTrackResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.likes
            .unlike_track(&identity, &req.track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UnlikeTrackResponse { unliked: true }))
    }

    async fn list_liked_tracks(
        &self,
        request: Request<ListLikedTracksRequest>,
    ) -> Result<Response<ListLikedTracksResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let page = self
            .likes
            .list_liked_tracks(
                &identity,
                Page {
                    limit: req.limit as u32,
                    offset: req.offset as u32,
                },
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(ListLikedTracksResponse {
            total_count: page.total_count,
            has_more: page.has_more,
            items: to_proto_items(page),
        }))
    }

    async fn get_preferences(
        &self,
        request: Request<GetPreferencesRequest>,
    ) -> Result<Response<GetPreferencesResponse>, Status> {
        let metadata = request.metadata().clone();
        let _req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let preferences = self
            .preferences
            .get_preferences(&identity)
            .await
            .map_err(to_status)?;
        Ok(Response::new(GetPreferencesResponse {
            preferences_json: preferences.values_json,
        }))
    }

    async fn update_preferences(
        &self,
        request: Request<UpdatePreferencesRequest>,
    ) -> Result<Response<UpdatePreferencesResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let preferences = self
            .preferences
            .update_preferences(&identity, &req.preferences_json)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UpdatePreferencesResponse {
            preferences_json: preferences.values_json,
        }))
    }

    async fn create_playlist(
        &self,
        request: Request<CreatePlaylistRequest>,
    ) -> Result<Response<CreatePlaylistResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let playlist = self
            .playlists
            .create_playlist(&identity, &req.name, &req.description)
            .await
            .map_err(to_status)?;
        Ok(Response::new(CreatePlaylistResponse {
            playlist: Some(to_proto_playlist(playlist)),
        }))
    }

    async fn update_playlist(
        &self,
        request: Request<UpdatePlaylistRequest>,
    ) -> Result<Response<UpdatePlaylistResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let playlist = self
            .playlists
            .update_playlist(&identity, &req.playlist_id, &req.name, &req.description)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UpdatePlaylistResponse {
            playlist: Some(to_proto_playlist(playlist)),
        }))
    }

    async fn delete_playlist(
        &self,
        request: Request<DeletePlaylistRequest>,
    ) -> Result<Response<DeletePlaylistResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.playlists
            .delete_playlist(&identity, &req.playlist_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(DeletePlaylistResponse { success: true }))
    }

    async fn list_playlists(
        &self,
        request: Request<ListPlaylistsRequest>,
    ) -> Result<Response<ListPlaylistsResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let request_page = playlist_page(req.limit, req.offset).map_err(to_status)?;
        let page = self
            .playlists
            .list_playlists(&identity, request_page)
            .await
            .map_err(to_status)?;
        Ok(Response::new(ListPlaylistsResponse {
            playlists: page.items.into_iter().map(to_proto_playlist).collect(),
            total_count: page.total_count,
            has_more: page.has_more,
        }))
    }

    async fn add_playlist_track(
        &self,
        request: Request<AddPlaylistTrackRequest>,
    ) -> Result<Response<AddPlaylistTrackResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.playlists
            .add_track(
                &identity,
                &req.playlist_id,
                &req.track_id,
                req.has_position.then_some(req.position),
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(AddPlaylistTrackResponse { success: true }))
    }

    async fn remove_playlist_track(
        &self,
        request: Request<RemovePlaylistTrackRequest>,
    ) -> Result<Response<RemovePlaylistTrackResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.playlists
            .remove_track(&identity, &req.playlist_id, &req.track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(RemovePlaylistTrackResponse { success: true }))
    }

    async fn reorder_playlist_tracks(
        &self,
        request: Request<ReorderPlaylistTracksRequest>,
    ) -> Result<Response<ReorderPlaylistTracksResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        self.playlists
            .reorder_tracks(&identity, &req.playlist_id, req.track_ids)
            .await
            .map_err(to_status)?;
        Ok(Response::new(ReorderPlaylistTracksResponse {
            success: true,
        }))
    }

    async fn list_playlist_tracks(
        &self,
        request: Request<ListPlaylistTracksRequest>,
    ) -> Result<Response<ListPlaylistTracksResponse>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let request_page = playlist_page(req.limit, req.offset).map_err(to_status)?;
        let page = self
            .playlists
            .list_tracks(&identity, &req.playlist_id, request_page)
            .await
            .map_err(to_status)?;
        Ok(Response::new(ListPlaylistTracksResponse {
            total_count: page.total_count,
            has_more: page.has_more,
            items: to_proto_items(page),
        }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let status = self.health.check().await;
        Ok(Response::new(HealthResponse {
            healthy: status.healthy,
            version: status.version,
            status: status.status.as_str().to_string(),
            dependencies: status
                .dependencies
                .into_iter()
                .map(|dep| HealthDependency {
                    name: dep.name,
                    status: dep.status.as_str().to_string(),
                    message: dep.message,
                })
                .collect(),
        }))
    }

    async fn resolve_playback(
        &self,
        request: Request<PlaybackRequest>,
    ) -> Result<Response<ProtoPlaybackSource>, Status> {
        let metadata = request.metadata().clone();
        let req = request.into_inner();
        let identity =
            extract_optional_metadata_identity(&metadata, &self.auth).map_err(to_status)?;
        let principal = self
            .principal
            .classify(identity.as_ref())
            .await
            .map_err(to_status)?;
        let source = self
            .resolver
            .resolve_for_session(
                &self.playback,
                &principal,
                &req.session_id,
                &req.track_id,
                current_epoch_ms(),
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(ProtoPlaybackSource {
            track_id: source.track_id,
            stream_url: source.stream_url,
            content_type: source.content_type,
            codec: source.codec,
            duration_ms: source.duration_ms,
            expires_at_epoch_ms: source.expires_at_epoch_ms,
        }))
    }

    async fn discovery_next(
        &self,
        request: Request<DiscoveryRequest>,
    ) -> Result<Response<DiscoveryTrack>, Status> {
        let req = request.into_inner();
        let page = self
            .discovery
            .next(
                &TrackAccessScope::Public,
                &req.recently_played,
                req.limit as u32,
            )
            .await
            .map_err(to_status)?;
        let item = page
            .items
            .into_iter()
            .next()
            .ok_or_else(|| Status::not_found("no discovery tracks available"))?;
        Ok(Response::new(DiscoveryTrack {
            item: Some(to_proto_item(item)),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use tonic::metadata::MetadataValue;

    fn auth() -> AuthService {
        AuthService::new("secret")
    }

    fn token_for(user_id: &str) -> String {
        auth()
            .mint(user_id, std::time::Duration::from_secs(3600))
            .unwrap()
    }

    #[test]
    fn extract_identity_reads_bearer_authorization_metadata() {
        let mut request = Request::new(());
        let token = token_for("user-1");
        request.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
        );

        let identity = extract_identity(request.metadata(), "", &auth()).unwrap();

        assert_eq!(identity.user_id, "user-1");
    }

    #[test]
    fn extract_identity_reads_direct_token_metadata() {
        let mut request = Request::new(());
        let token = token_for("user-2");
        request.metadata_mut().insert(
            "x-canopy-auth-token",
            MetadataValue::try_from(token).unwrap(),
        );

        let identity = extract_identity(request.metadata(), "", &auth()).unwrap();

        assert_eq!(identity.user_id, "user-2");
    }

    #[test]
    fn extract_identity_prefers_authorization_over_direct_token_metadata() {
        let mut request = Request::new(());
        let bearer = token_for("bearer-user");
        let direct = token_for("direct-user");
        request.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {bearer}")).unwrap(),
        );
        request.metadata_mut().insert(
            "x-canopy-auth-token",
            MetadataValue::try_from(direct).unwrap(),
        );

        let identity = extract_identity(request.metadata(), "", &auth()).unwrap();

        assert_eq!(identity.user_id, "bearer-user");
    }

    #[test]
    fn extract_identity_uses_body_fallback_when_metadata_is_absent() {
        let token = token_for("fallback-user");

        let identity =
            extract_identity(&tonic::metadata::MetadataMap::new(), &token, &auth()).unwrap();

        assert_eq!(identity.user_id, "fallback-user");
    }

    #[test]
    fn extract_identity_rejects_malformed_bearer_metadata() {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::from_static("Token abc"));

        let err = extract_identity(request.metadata(), "", &auth()).unwrap_err();

        assert!(matches!(err, canopy_core::CanopyError::Unauthenticated(_)));
    }

    #[test]
    fn extract_identity_rejects_missing_token() {
        let err = extract_identity(&tonic::metadata::MetadataMap::new(), "", &auth()).unwrap_err();

        assert!(matches!(err, canopy_core::CanopyError::Unauthenticated(_)));
    }
    #[test]
    fn optional_identity_treats_absent_metadata_as_anonymous() {
        let identity =
            extract_optional_metadata_identity(&tonic::metadata::MetadataMap::new(), &auth())
                .unwrap();

        assert_eq!(identity, None);
    }

    #[test]
    fn optional_identity_verifies_supplied_bearer_token() {
        let mut request = Request::new(());
        let token = token_for("optional-user");
        request.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {token}")).unwrap(),
        );

        let identity = extract_optional_metadata_identity(request.metadata(), &auth()).unwrap();

        assert_eq!(
            identity,
            Some(UserIdentity {
                user_id: "optional-user".into(),
            })
        );
    }

    #[test]
    fn optional_identity_rejects_invalid_supplied_metadata() {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::from_static("Token abc"));

        let error = extract_optional_metadata_identity(request.metadata(), &auth()).unwrap_err();

        assert!(matches!(error, CanopyError::Unauthenticated(_)));
    }

    #[test]
    fn history_page_rejects_negative_values() {
        let negative_limit = history_page(-1, 0).unwrap_err();
        let negative_offset = history_page(10, -1).unwrap_err();

        assert!(matches!(negative_limit, CanopyError::InvalidArgument(_)));
        assert!(matches!(negative_offset, CanopyError::InvalidArgument(_)));
    }

    #[test]
    fn playlist_page_rejects_negative_values() {
        let negative_limit = playlist_page(-1, 0).unwrap_err();
        let negative_offset = playlist_page(10, -1).unwrap_err();

        assert!(matches!(negative_limit, CanopyError::InvalidArgument(_)));
        assert!(matches!(negative_offset, CanopyError::InvalidArgument(_)));
    }
}
