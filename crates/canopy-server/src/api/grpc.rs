//! gRPC adapter: implements the generated `Canopy` service over the domain
//! services, translating proto messages to and from the domain model.

use async_trait::async_trait;
use canopy_core::{
    CanopyError, CanopyResult, MediaItem as DomainMediaItem, MediaPage, Page, UserIdentity,
};
use canopy_proto::canopy_server::Canopy;
use canopy_proto::{
    BrowseRequest, BrowseResponse, DiscoveryRequest, DiscoveryTrack, EndSessionRequest,
    EndSessionResponse, GetMediaRequest, GetMediaResponse, GetSessionRequest, GetSessionResponse,
    HealthDependency, HealthRequest, HealthResponse, MediaItem as ProtoMediaItem, PauseRequest,
    PauseResponse, PlayRequest, PlayResponse, PlaybackRequest,
    PlaybackSource as ProtoPlaybackSource, RecordPlaybackHistoryRequest,
    RecordPlaybackHistoryResponse, SearchRequest, SearchResponse, SeekRequest, SeekResponse,
    SetPlaybackSpeedRequest, SetPlaybackSpeedResponse, StopRequest, StopResponse,
    UpdateSessionRequest, UpdateSessionResponse, UpsertProfileRequest, UpsertProfileResponse,
    UserProfile as ProtoUserProfile,
};
use tonic::{Request, Response, Status};

use crate::api::to_status;
use crate::auth::AuthService;
use crate::catalog::CatalogService;
use crate::discovery::DiscoveryService;
use crate::health::HealthService;
use crate::history::HistoryService;
use crate::playback::{PlaybackService, ResolverService};
use crate::profile::ProfileService;
use crate::search::SearchService;

/// Domain services exposed through the gRPC adapter.
pub struct GrpcServices {
    pub catalog: CatalogService,
    pub search: SearchService,
    pub playback: PlaybackService,
    pub profile: ProfileService,
    pub history: HistoryService,
    pub health: HealthService,
    pub resolver: ResolverService,
    pub discovery: DiscoveryService,
    pub auth: AuthService,
}

/// gRPC entry point wiring the wire contract to the domain services.
pub struct GrpcApi {
    catalog: CatalogService,
    search: SearchService,
    playback: PlaybackService,
    profile: ProfileService,
    history: HistoryService,
    health: HealthService,
    resolver: ResolverService,
    discovery: DiscoveryService,
    auth: AuthService,
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
            health: services.health,
            resolver: services.resolver,
            discovery: services.discovery,
            auth: services.auth,
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
}
