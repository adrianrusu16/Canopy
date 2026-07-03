use std::sync::Arc;

use canopy_core::{CanopyError, PlaybackHistoryEntry};
use canopy_proto::history_service_server::HistoryService;
use canopy_proto::{
    ClearHistoryRequest, ClearHistoryResponse, DeleteHistoryEntryRequest,
    GetHistorySettingsRequest, HistoryEntry, HistorySettings, ListHistoryRequest,
    ListHistoryResponse, RecordPlaybackRequest, RecordPlaybackResponse,
    UpdateHistorySettingsRequest, UpdateHistorySettingsResponse,
};
use prost_types::Timestamp;
use tonic::{Request, Response, Status};

use super::{
    GrpcServices, extract_metadata_identity, page_from_request, page_info, to_track_summary,
};
use crate::api::to_status;

pub struct HistoryGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl HistoryService for HistoryGrpc {
    async fn get_history_settings(
        &self,
        request: Request<GetHistorySettingsRequest>,
    ) -> Result<Response<HistorySettings>, Status> {
        let identity =
            extract_metadata_identity(request.metadata(), &self.0.auth).map_err(to_status)?;
        let profile = self
            .0
            .profile
            .get_profile(&identity)
            .await
            .map_err(to_status)?;
        Ok(Response::new(HistorySettings {
            enabled: profile.history_enabled,
        }))
    }

    async fn update_history_settings(
        &self,
        request: Request<UpdateHistorySettingsRequest>,
    ) -> Result<Response<UpdateHistorySettingsResponse>, Status> {
        let metadata = request.metadata().clone();
        let enabled = request.into_inner().enabled;
        let identity = extract_metadata_identity(&metadata, &self.0.auth).map_err(to_status)?;
        let (profile, deleted_count) = self
            .0
            .profile
            .set_history_enabled(&identity, enabled)
            .await
            .map_err(to_status)?;
        Ok(Response::new(UpdateHistorySettingsResponse {
            settings: Some(HistorySettings {
                enabled: profile.history_enabled,
            }),
            deleted_count,
        }))
    }

    async fn record_playback(
        &self,
        request: Request<RecordPlaybackRequest>,
    ) -> Result<Response<RecordPlaybackResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.0.auth).map_err(to_status)?;
        let duration_ms = i64::try_from(request.duration_ms)
            .map_err(|_| Status::invalid_argument("duration_ms exceeds supported range"))?;
        let recorded = self
            .0
            .history
            .record_playback(
                &identity,
                &request.track_id,
                duration_ms,
                request.completion_ratio,
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(RecordPlaybackResponse { recorded }))
    }

    async fn list_history(
        &self,
        request: Request<ListHistoryRequest>,
    ) -> Result<Response<ListHistoryResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_metadata_identity(&metadata, &self.0.auth).map_err(to_status)?;
        let page = page_from_request(request.page, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .history
            .list_history(&identity, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.entries.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;
        let entries = result
            .entries
            .into_iter()
            .map(to_proto_history_entry)
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_status)?;
        Ok(Response::new(ListHistoryResponse {
            entries,
            page_info: Some(page_info),
        }))
    }

    async fn delete_history_entry(
        &self,
        request: Request<DeleteHistoryEntryRequest>,
    ) -> Result<Response<()>, Status> {
        let metadata = request.metadata().clone();
        let history_id = request.into_inner().history_id;
        let identity = extract_metadata_identity(&metadata, &self.0.auth).map_err(to_status)?;
        self.0
            .history
            .delete_entry(&identity, &history_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(()))
    }

    async fn clear_history(
        &self,
        request: Request<ClearHistoryRequest>,
    ) -> Result<Response<ClearHistoryResponse>, Status> {
        let identity =
            extract_metadata_identity(request.metadata(), &self.0.auth).map_err(to_status)?;
        let deleted_count = self
            .0
            .history
            .clear_history(&identity)
            .await
            .map_err(to_status)?;
        Ok(Response::new(ClearHistoryResponse { deleted_count }))
    }
}

fn to_proto_history_entry(entry: PlaybackHistoryEntry) -> Result<HistoryEntry, CanopyError> {
    let duration_ms = u64::try_from(entry.duration_ms)
        .map_err(|_| CanopyError::Internal("stored history duration is negative".into()))?;
    Ok(HistoryEntry {
        id: entry.id,
        played_at: Some(timestamp_from_epoch_ms(entry.played_at_epoch_ms)?),
        duration_ms,
        completion_ratio: entry.completion_pct,
        track: Some(to_track_summary(entry.item)),
    })
}

fn timestamp_from_epoch_ms(epoch_ms: u64) -> Result<Timestamp, CanopyError> {
    let seconds = i64::try_from(epoch_ms / 1_000)
        .map_err(|_| CanopyError::Internal("history timestamp overflow".into()))?;
    let nanos = i32::try_from((epoch_ms % 1_000) * 1_000_000)
        .map_err(|_| CanopyError::Internal("history nanoseconds overflow".into()))?;
    Ok(Timestamp { seconds, nanos })
}
