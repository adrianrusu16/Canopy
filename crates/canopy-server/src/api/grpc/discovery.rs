use std::sync::Arc;

use canopy_proto::discovery_service_server::DiscoveryService;
use canopy_proto::{GetDiscoveryFeedRequest, GetDiscoveryFeedResponse};
use tonic::{Request, Response, Status};

use super::{GrpcServices, page_from_request, page_info, to_track_summary};
use crate::api::to_status;

pub struct DiscoveryGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl DiscoveryService for DiscoveryGrpc {
    async fn get_discovery_feed(
        &self,
        request: Request<GetDiscoveryFeedRequest>,
    ) -> Result<Response<GetDiscoveryFeedResponse>, Status> {
        let request = request.into_inner();
        let page = page_from_request(request.page, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .discovery
            .feed(&request.exclude_track_ids, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;

        Ok(Response::new(GetDiscoveryFeedResponse {
            tracks: result.items.into_iter().map(to_track_summary).collect(),
            page_info: Some(page_info),
        }))
    }
}
