use std::sync::Arc;

use canopy_proto::catalog_service_server::CatalogService;
use canopy_proto::{
    BrowseRequest, BrowseResponse, GetMediaRequest, SearchRequest, SearchResponse, Track,
};
use tonic::{Request, Response, Status};

use super::{GrpcServices, page_from_request, page_info, to_track, to_track_summary};
use crate::api::to_status;

pub struct CatalogGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl CatalogService for CatalogGrpc {
    async fn browse(
        &self,
        request: Request<BrowseRequest>,
    ) -> Result<Response<BrowseResponse>, Status> {
        let request = request.into_inner();
        let page = page_from_request(request.page, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .catalog
            .browse(request.parent_id.as_deref(), &request.genres, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;

        Ok(Response::new(BrowseResponse {
            tracks: result.items.into_iter().map(to_track_summary).collect(),
            page_info: Some(page_info),
        }))
    }

    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let request = request.into_inner();
        let page = page_from_request(request.page, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .search
            .search(&request.query, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;

        Ok(Response::new(SearchResponse {
            tracks: result.items.into_iter().map(to_track_summary).collect(),
            page_info: Some(page_info),
        }))
    }

    async fn get_media(
        &self,
        request: Request<GetMediaRequest>,
    ) -> Result<Response<Track>, Status> {
        let track_id = request.into_inner().track_id;
        let item = self
            .0
            .catalog
            .get_media(&track_id)
            .await
            .map_err(to_status)?
            .ok_or_else(|| Status::not_found(format!("track not found: {track_id}")))?;

        Ok(Response::new(to_track(item)))
    }
}
