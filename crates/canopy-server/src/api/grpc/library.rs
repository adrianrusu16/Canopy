use std::sync::Arc;

use canopy_core::{CanopyError, LibraryItem, MediaItem, TrackLike};
use canopy_proto::library_service_server::LibraryService;
use canopy_proto::{
    LikeTrackRequest, LikedTrack, ListLikedTracksRequest, ListLikedTracksResponse,
    ListSavedTracksRequest, ListSavedTracksResponse, RemoveSavedTrackRequest, SaveTrackRequest,
    SavedTrack, UnlikeTrackRequest,
};
use prost_types::Timestamp;
use tonic::{Request, Response, Status};

use super::{GrpcServices, extract_durable_principal, not_implemented, to_track_summary};
use crate::api::to_status;

pub struct LibraryGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl LibraryService for LibraryGrpc {
    async fn save_track(
        &self,
        request: Request<SaveTrackRequest>,
    ) -> Result<Response<SavedTrack>, Status> {
        let metadata = request.metadata().clone();
        let track_id = request.into_inner().track_id;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let saved = self
            .0
            .library
            .save_track(&identity, &track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_saved_track(saved).map_err(to_status)?))
    }

    async fn remove_saved_track(
        &self,
        request: Request<RemoveSavedTrackRequest>,
    ) -> Result<Response<()>, Status> {
        let metadata = request.metadata().clone();
        let track_id = request.into_inner().track_id;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        self.0
            .library
            .remove_track(&identity, &track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(()))
    }

    async fn list_saved_tracks(
        &self,
        _: Request<ListSavedTracksRequest>,
    ) -> Result<Response<ListSavedTracksResponse>, Status> {
        Err(not_implemented("LibraryService.ListSavedTracks"))
    }

    async fn like_track(
        &self,
        request: Request<LikeTrackRequest>,
    ) -> Result<Response<LikedTrack>, Status> {
        let metadata = request.metadata().clone();
        let track_id = request.into_inner().track_id;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let liked = self
            .0
            .likes
            .like_track(&identity, &track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(to_liked_track(liked).map_err(to_status)?))
    }

    async fn unlike_track(
        &self,
        request: Request<UnlikeTrackRequest>,
    ) -> Result<Response<()>, Status> {
        let metadata = request.metadata().clone();
        let track_id = request.into_inner().track_id;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        self.0
            .likes
            .unlike_track(&identity, &track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(()))
    }

    async fn list_liked_tracks(
        &self,
        _: Request<ListLikedTracksRequest>,
    ) -> Result<Response<ListLikedTracksResponse>, Status> {
        Err(not_implemented("LibraryService.ListLikedTracks"))
    }
}

fn to_saved_track(item: LibraryItem) -> Result<SavedTrack, CanopyError> {
    Ok(SavedTrack {
        track: Some(to_track_summary(MediaItem {
            id: item.track_id,
            ..MediaItem::default()
        })),
        saved_at: Some(timestamp_from_epoch_ms(item.added_at_epoch_ms)?),
    })
}

fn to_liked_track(item: TrackLike) -> Result<LikedTrack, CanopyError> {
    Ok(LikedTrack {
        track: Some(to_track_summary(MediaItem {
            id: item.track_id,
            ..MediaItem::default()
        })),
        liked_at: Some(timestamp_from_epoch_ms(item.liked_at_epoch_ms)?),
    })
}

fn timestamp_from_epoch_ms(epoch_ms: u64) -> Result<Timestamp, CanopyError> {
    let seconds = i64::try_from(epoch_ms / 1_000)
        .map_err(|_| CanopyError::Internal("relationship timestamp overflow".into()))?;
    let nanos = i32::try_from((epoch_ms % 1_000) * 1_000_000)
        .map_err(|_| CanopyError::Internal("relationship nanoseconds overflow".into()))?;
    Ok(Timestamp { seconds, nanos })
}

#[cfg(test)]
mod tests {
    use canopy_core::{LibraryItem, TrackLike};

    use super::*;

    #[test]
    fn saved_track_resource_preserves_track_and_timestamp() {
        let saved = to_saved_track(LibraryItem {
            profile_id: "profile-1".into(),
            track_id: "track-1".into(),
            added_at_epoch_ms: 1_234,
        })
        .unwrap();

        assert_eq!(saved.track.unwrap().id, "track-1");
        assert_eq!(saved.saved_at.unwrap().nanos, 234_000_000);
    }

    #[test]
    fn liked_track_resource_preserves_track_and_timestamp() {
        let liked = to_liked_track(TrackLike {
            profile_id: "profile-1".into(),
            track_id: "track-2".into(),
            liked_at_epoch_ms: 2_345,
        })
        .unwrap();

        assert_eq!(liked.track.unwrap().id, "track-2");
        assert_eq!(liked.liked_at.unwrap().seconds, 2);
    }
}
