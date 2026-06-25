//! gRPC adapter: implements the generated `Canopy` service over the domain
//! services, translating proto messages to and from the domain model.

use async_trait::async_trait;
use canopy_core::{MediaItem as DomainMediaItem, MediaPage, Page};
use canopy_proto::canopy_server::Canopy;
use canopy_proto::{
    BrowseRequest, BrowseResponse, DiscoveryRequest, DiscoveryTrack, EndSessionRequest,
    EndSessionResponse, GetMediaRequest, GetMediaResponse, GetSessionRequest, GetSessionResponse,
    HealthDependency, HealthRequest, HealthResponse, MediaItem as ProtoMediaItem, PauseRequest,
    PauseResponse, PlayRequest, PlayResponse, PlaybackRequest,
    PlaybackSource as ProtoPlaybackSource, SearchRequest, SearchResponse, SeekRequest,
    SeekResponse, SetPlaybackSpeedRequest, SetPlaybackSpeedResponse, StopRequest, StopResponse,
    UpdateSessionRequest, UpdateSessionResponse, UpsertProfileRequest, UpsertProfileResponse,
    UserProfile as ProtoUserProfile,
};
use tonic::{Request, Response, Status};

use crate::api::to_status;
use crate::catalog::CatalogService;
use crate::discovery::DiscoveryService;
use crate::health::HealthService;
use crate::playback::{PlaybackService, ResolverService};
use crate::profile::ProfileService;
use crate::search::SearchService;

/// gRPC entry point wiring the wire contract to the domain services.
pub struct GrpcApi {
    catalog: CatalogService,
    search: SearchService,
    playback: PlaybackService,
    profile: ProfileService,
    health: HealthService,
    resolver: ResolverService,
    discovery: DiscoveryService,
}

impl GrpcApi {
    /// Creates a new gRPC adapter over the given domain services.
    pub fn new(
        catalog: CatalogService,
        search: SearchService,
        playback: PlaybackService,
        profile: ProfileService,
        health: HealthService,
        resolver: ResolverService,
        discovery: DiscoveryService,
    ) -> Self {
        Self {
            catalog,
            search,
            playback,
            profile,
            health,
            resolver,
            discovery,
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
            .browse(parent_id, &req.genres, page)
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
            .search(&req.query, page)
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
            .get_media(&req.media_id)
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
        let req = request.into_inner();
        let profile = self
            .profile
            .upsert_profile(&req.auth_token, &req.display_name, req.history_enabled)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UpsertProfileResponse {
            profile: Some(to_proto_profile(profile)),
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
        let req = request.into_inner();
        let source = self
            .resolver
            .resolve_for_session(
                &self.playback,
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
            .next(&req.recently_played, req.limit as u32)
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
