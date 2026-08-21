use std::sync::Arc;

use canopy_proto::discovery_service_server::DiscoveryService;
use canopy_proto::{
    GetDiscoveryFeedRequest, GetDiscoveryFeedResponse, GetForYouFeedRequest, GetForYouFeedResponse,
    GetRecommendationsRequest, GetRecommendationsResponse, PageInfo, PageRequest, TrackSummary,
};
use tonic::{Request, Response, Status};

use super::{
    GrpcServices, extract_optional_track_scope, page_from_request, page_info, to_track_summary,
};
use crate::api::to_status;

pub struct DiscoveryGrpc(pub Arc<GrpcServices>);

struct FeedPayload {
    tracks: Vec<TrackSummary>,
    page_info: Option<PageInfo>,
}

impl DiscoveryGrpc {
    async fn feed(
        &self,
        metadata: tonic::metadata::MetadataMap,
        exclude_track_ids: Vec<String>,
        page_request: Option<PageRequest>,
    ) -> Result<FeedPayload, Status> {
        let scope = extract_optional_track_scope(&metadata, &self.0)
            .await
            .map_err(to_status)?;
        let page = page_from_request(page_request, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .discovery
            .feed(&scope, &exclude_track_ids, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;

        Ok(FeedPayload {
            tracks: result.items.into_iter().map(to_track_summary).collect(),
            page_info: Some(page_info),
        })
    }
}

#[tonic::async_trait]
impl DiscoveryService for DiscoveryGrpc {
    async fn get_discovery_feed(
        &self,
        request: Request<GetDiscoveryFeedRequest>,
    ) -> Result<Response<GetDiscoveryFeedResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let feed = self
            .feed(metadata, request.exclude_track_ids, request.page)
            .await?;

        Ok(Response::new(GetDiscoveryFeedResponse {
            tracks: feed.tracks,
            page_info: feed.page_info,
        }))
    }

    async fn get_for_you_feed(
        &self,
        request: Request<GetForYouFeedRequest>,
    ) -> Result<Response<GetForYouFeedResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let feed = self
            .feed(metadata, request.exclude_track_ids, request.page)
            .await?;

        Ok(Response::new(GetForYouFeedResponse {
            tracks: feed.tracks,
            page_info: feed.page_info,
        }))
    }

    async fn get_recommendations(
        &self,
        request: Request<GetRecommendationsRequest>,
    ) -> Result<Response<GetRecommendationsResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let feed = self
            .feed(metadata, request.exclude_track_ids, request.page)
            .await?;

        Ok(Response::new(GetRecommendationsResponse {
            tracks: feed.tracks,
            page_info: feed.page_info,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopy_core::{CatalogRepository, MediaItem, PageTokenCodec};
    use canopy_proto::PageRequest;

    use crate::catalog::CatalogService as DomainCatalogService;
    use crate::discovery::DiscoveryService as DomainDiscoveryService;
    use crate::health::HealthService;
    use crate::history::HistoryService;
    use crate::jade_store::{
        InMemoryAudioAssetStore, InMemoryCatalog, InMemoryInstanceSettingsStore,
        InMemoryLibraryStore, InMemoryLikeStore, InMemoryPlaybackHistoryStore,
        InMemoryPlaylistStore, InMemoryPreferencesStore, InMemoryProfileStore,
    };
    use crate::library::LibraryService;
    use crate::likes::LikeService;
    use crate::playback::{ResolverConfig, ResolverService};
    use crate::playlists::PlaylistService;
    use crate::preferences::PreferencesService;
    use crate::principal::PrincipalService;
    use crate::profile::ProfileService;
    use crate::search::SearchService;
    use crate::stream::StreamTokenCodec;

    fn adapter() -> DiscoveryGrpc {
        let catalog = InMemoryCatalog::with_items(vec![
            MediaItem {
                id: "track-1".into(),
                title: "First".into(),
                artist: "Artist A".into(),
                ..MediaItem::default()
            },
            MediaItem {
                id: "track-2".into(),
                title: "Second".into(),
                artist: "Artist B".into(),
                ..MediaItem::default()
            },
        ]);
        let catalog_repo: Arc<dyn CatalogRepository> = Arc::new(catalog.clone());
        let profiles = Arc::new(InMemoryProfileStore::default());
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        let history_repo = Arc::new(InMemoryPlaybackHistoryStore::new(catalog_repo.clone()));
        let library_repo = Arc::new(InMemoryLibraryStore::new(catalog_repo.clone()));
        let like_repo = Arc::new(InMemoryLikeStore::new(catalog_repo.clone()));
        let preferences_repo = Arc::new(InMemoryPreferencesStore::default());
        let playlist_repo = Arc::new(InMemoryPlaylistStore::new(catalog_repo.clone()));
        let resolver = ResolverService::new(
            Arc::new(InMemoryAudioAssetStore::default().with_instance_settings(settings.clone())),
            Arc::new(StreamTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap()),
            ResolverConfig {
                public_base_url: "https://media.test".into(),
                ..ResolverConfig::default()
            },
        );
        let principal = PrincipalService::new(profiles.clone(), settings.clone());
        let services = GrpcServices {
            catalog: DomainCatalogService::new(catalog_repo.clone()),
            search: SearchService::new(catalog_repo.clone()),
            profile: ProfileService::new(profiles.clone(), history_repo.clone())
                .with_deletion_policy(settings.clone(), catalog_repo),
            history: HistoryService::new(profiles.clone(), history_repo, principal.clone()),
            library: LibraryService::new(profiles.clone(), library_repo, principal.clone()),
            likes: LikeService::new(profiles.clone(), like_repo, principal.clone()),
            preferences: PreferencesService::new(profiles.clone(), preferences_repo),
            playlists: PlaylistService::new(profiles.clone(), playlist_repo, principal.clone()),
            health: HealthService::new(),
            resolver,
            discovery: DomainDiscoveryService::new(Arc::new(catalog)),
            identity: None,
            principal,
            page_tokens: Arc::new(
                PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap(),
            ),
        };
        DiscoveryGrpc(Arc::new(services))
    }

    #[tokio::test]
    async fn named_feeds_match_discovery_for_equivalent_requests() {
        let grpc = adapter();
        let page = Some(PageRequest {
            page_size: 1,
            page_token: String::new(),
        });
        let excluded = vec!["track-2".to_string()];

        let discovery = grpc
            .get_discovery_feed(Request::new(GetDiscoveryFeedRequest {
                exclude_track_ids: excluded.clone(),
                page: page.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        let for_you = grpc
            .get_for_you_feed(Request::new(GetForYouFeedRequest {
                exclude_track_ids: excluded.clone(),
                page: page.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        let recommendations = grpc
            .get_recommendations(Request::new(GetRecommendationsRequest {
                exclude_track_ids: excluded,
                page,
            }))
            .await
            .unwrap()
            .into_inner();

        let discovery_ids: Vec<_> = discovery
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect();
        let for_you_ids: Vec<_> = for_you
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect();
        let recommendation_ids: Vec<_> = recommendations
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect();

        assert_eq!(discovery_ids, vec!["track-1"]);
        assert_eq!(for_you_ids, discovery_ids);
        assert_eq!(recommendation_ids, discovery_ids);
        assert_eq!(for_you.page_info, discovery.page_info);
        assert_eq!(recommendations.page_info, discovery.page_info);
    }
}
