use std::sync::Arc;

use canopy_core::PageTokenCodec;
use canopy_proto::discovery_service_server::DiscoveryService;
use canopy_proto::{
    GetDiscoveryFeedRequest, GetDiscoveryFeedResponse, GetForYouFeedRequest, GetForYouFeedResponse,
    GetRecommendationsRequest, GetRecommendationsResponse, PageInfo, PageRequest, TrackSummary,
};
use tonic::{Request, Response, Status};

use super::{page_from_request, page_info, to_track_summary};
use crate::api::to_status;

pub struct DiscoveryGrpc {
    discovery: crate::discovery::DiscoveryService,
    page_tokens: Arc<PageTokenCodec>,
}

struct FeedPayload {
    tracks: Vec<TrackSummary>,
    page_info: Option<PageInfo>,
}

impl DiscoveryGrpc {
    pub fn new(
        discovery: crate::discovery::DiscoveryService,
        page_tokens: Arc<PageTokenCodec>,
    ) -> Self {
        Self {
            discovery,
            page_tokens,
        }
    }

    async fn feed(
        &self,
        exclude_track_ids: Vec<String>,
        page_request: Option<PageRequest>,
    ) -> Result<FeedPayload, Status> {
        let page = page_from_request(page_request, &self.page_tokens).map_err(to_status)?;
        let result = self
            .discovery
            .feed(&exclude_track_ids, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(page, result.items.len(), result.has_more, &self.page_tokens)
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
        let request = request.into_inner();
        let feed = self.feed(request.exclude_track_ids, request.page).await?;

        Ok(Response::new(GetDiscoveryFeedResponse {
            tracks: feed.tracks,
            page_info: feed.page_info,
        }))
    }

    async fn get_for_you_feed(
        &self,
        request: Request<GetForYouFeedRequest>,
    ) -> Result<Response<GetForYouFeedResponse>, Status> {
        let request = request.into_inner();
        let feed = self.feed(request.exclude_track_ids, request.page).await?;

        Ok(Response::new(GetForYouFeedResponse {
            tracks: feed.tracks,
            page_info: feed.page_info,
        }))
    }

    async fn get_recommendations(
        &self,
        request: Request<GetRecommendationsRequest>,
    ) -> Result<Response<GetRecommendationsResponse>, Status> {
        let request = request.into_inner();
        let feed = self.feed(request.exclude_track_ids, request.page).await?;

        Ok(Response::new(GetRecommendationsResponse {
            tracks: feed.tracks,
            page_info: feed.page_info,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canopy_core::{MediaItem, PageTokenCodec};
    use canopy_proto::PageRequest;

    use crate::jade_store::InMemoryCatalog;

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
        DiscoveryGrpc::new(
            crate::discovery::DiscoveryService::new(Arc::new(catalog)),
            Arc::new(PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap()),
        )
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
