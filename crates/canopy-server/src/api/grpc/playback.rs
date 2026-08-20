use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use canopy_core::CanopyError;
use canopy_proto::playback_service_server::PlaybackService;
use canopy_proto::{PlaybackSource, ResolvePlaybackRequest};
use prost_types::Timestamp;
use tonic::{Request, Response, Status};

use super::{GrpcServices, extract_optional_track_scope};
use crate::api::to_status;

pub struct PlaybackGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl PlaybackService for PlaybackGrpc {
    async fn resolve_playback(
        &self,
        request: Request<ResolvePlaybackRequest>,
    ) -> Result<Response<PlaybackSource>, Status> {
        let metadata = request.metadata().clone();
        let track_id = request.into_inner().track_id;
        let scope = extract_optional_track_scope(&metadata, &self.0)
            .await
            .map_err(to_status)?;
        let source = self
            .0
            .resolver
            .resolve_at(&scope, &track_id, current_epoch_ms().map_err(to_status)?)
            .await
            .map_err(to_status)?;

        Ok(Response::new(PlaybackSource {
            track_id: source.track_id,
            stream_url: source.stream_url,
            content_type: source.content_type,
            codec: source.codec,
            duration_ms: source.duration_ms,
            expires_at: Some(
                timestamp_from_epoch_ms(source.expires_at_epoch_ms).map_err(to_status)?,
            ),
        }))
    }
}

fn current_epoch_ms() -> Result<u64, CanopyError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CanopyError::Internal(format!("system clock before Unix epoch: {error}")))?
        .as_millis();
    u64::try_from(millis).map_err(|_| CanopyError::Internal("epoch milliseconds overflow".into()))
}

fn timestamp_from_epoch_ms(epoch_ms: u64) -> Result<Timestamp, CanopyError> {
    let seconds = i64::try_from(epoch_ms / 1_000)
        .map_err(|_| CanopyError::Internal("playback expiry timestamp overflow".into()))?;
    let nanos = i32::try_from((epoch_ms % 1_000) * 1_000_000)
        .map_err(|_| CanopyError::Internal("playback expiry nanoseconds overflow".into()))?;
    Ok(Timestamp { seconds, nanos })
}

#[cfg(test)]
mod tests {
    use super::timestamp_from_epoch_ms;

    #[test]
    fn playback_expiry_preserves_millisecond_precision() {
        let timestamp = timestamp_from_epoch_ms(1_234).unwrap();

        assert_eq!(timestamp.seconds, 1);
        assert_eq!(timestamp.nanos, 234_000_000);
    }
}
