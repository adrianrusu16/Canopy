use std::sync::Arc;

use canopy_core::{CanopyError, Playlist as DomainPlaylist, PlaylistTrackItem};
use canopy_proto::playlist_service_server::PlaylistService;
use canopy_proto::{
    AddPlaylistTrackRequest, CreatePlaylistRequest, DeletePlaylistRequest, GetPlaylistRequest,
    ListPlaylistTracksRequest, ListPlaylistTracksResponse, ListPlaylistsRequest,
    ListPlaylistsResponse, Playlist, PlaylistTrack, RemovePlaylistTrackRequest,
    ReorderPlaylistTracksRequest, UpdatePlaylistRequest,
};
use prost_types::{FieldMask, Timestamp};
use tonic::{Request, Response, Status};

use super::{
    GrpcServices, extract_durable_principal, page_from_request, page_info, to_track_summary,
};
use crate::api::to_status;

pub struct PlaylistGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl PlaylistService for PlaylistGrpc {
    async fn create_playlist(
        &self,
        request: Request<CreatePlaylistRequest>,
    ) -> Result<Response<Playlist>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let playlist = self
            .0
            .playlists
            .create_playlist(
                &identity,
                &request.name,
                request.description.as_deref().unwrap_or_default(),
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(
            to_proto_playlist(playlist).map_err(to_status)?,
        ))
    }

    async fn get_playlist(
        &self,
        request: Request<GetPlaylistRequest>,
    ) -> Result<Response<Playlist>, Status> {
        let metadata = request.metadata().clone();
        let playlist_id = request.into_inner().playlist_id;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let playlist = self
            .0
            .playlists
            .get_playlist(&identity, &playlist_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(
            to_proto_playlist(playlist).map_err(to_status)?,
        ))
    }

    async fn update_playlist(
        &self,
        request: Request<UpdatePlaylistRequest>,
    ) -> Result<Response<Playlist>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        validate_playlist_update_mask(request.update_mask.as_ref()).map_err(to_status)?;
        let playlist = request
            .playlist
            .ok_or_else(|| Status::invalid_argument("playlist is required"))?;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let playlist = self
            .0
            .playlists
            .update_playlist(
                &identity,
                &playlist.id,
                &playlist.name,
                playlist.description.as_deref().unwrap_or_default(),
            )
            .await
            .map_err(to_status)?;
        Ok(Response::new(
            to_proto_playlist(playlist).map_err(to_status)?,
        ))
    }

    async fn delete_playlist(
        &self,
        request: Request<DeletePlaylistRequest>,
    ) -> Result<Response<()>, Status> {
        let metadata = request.metadata().clone();
        let playlist_id = request.into_inner().playlist_id;
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        self.0
            .playlists
            .delete_playlist(&identity, &playlist_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(()))
    }

    async fn list_playlists(
        &self,
        request: Request<ListPlaylistsRequest>,
    ) -> Result<Response<ListPlaylistsResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let page = page_from_request(request.page, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .playlists
            .list_playlists(&identity, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;
        let playlists = result
            .items
            .into_iter()
            .map(to_proto_playlist)
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_status)?;
        Ok(Response::new(ListPlaylistsResponse {
            playlists,
            page_info: Some(page_info),
        }))
    }

    async fn add_playlist_track(
        &self,
        request: Request<AddPlaylistTrackRequest>,
    ) -> Result<Response<PlaylistTrack>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let position = request
            .position
            .map(i32::try_from)
            .transpose()
            .map_err(|_| Status::invalid_argument("position exceeds supported range"))?;
        let track = self
            .0
            .playlists
            .add_track(&identity, &request.playlist_id, &request.track_id, position)
            .await
            .map_err(to_status)?;
        Ok(Response::new(
            to_proto_playlist_track(track).map_err(to_status)?,
        ))
    }

    async fn remove_playlist_track(
        &self,
        request: Request<RemovePlaylistTrackRequest>,
    ) -> Result<Response<()>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        self.0
            .playlists
            .remove_track(&identity, &request.playlist_id, &request.track_id)
            .await
            .map_err(to_status)?;
        Ok(Response::new(()))
    }

    async fn reorder_playlist_tracks(
        &self,
        request: Request<ReorderPlaylistTracksRequest>,
    ) -> Result<Response<Playlist>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let playlist = self
            .0
            .playlists
            .reorder_tracks(&identity, &request.playlist_id, request.track_ids)
            .await
            .map_err(to_status)?;
        Ok(Response::new(
            to_proto_playlist(playlist).map_err(to_status)?,
        ))
    }

    async fn list_playlist_tracks(
        &self,
        request: Request<ListPlaylistTracksRequest>,
    ) -> Result<Response<ListPlaylistTracksResponse>, Status> {
        let metadata = request.metadata().clone();
        let request = request.into_inner();
        let identity = extract_durable_principal(&metadata, &self.0)
            .await
            .map_err(to_status)?
            .user_identity();
        let page = page_from_request(request.page, &self.0.page_tokens).map_err(to_status)?;
        let result = self
            .0
            .playlists
            .list_tracks(&identity, &request.playlist_id, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.0.page_tokens,
        )
        .map_err(to_status)?;
        let tracks = result
            .items
            .into_iter()
            .map(to_proto_playlist_track)
            .collect::<Result<Vec<_>, _>>()
            .map_err(to_status)?;
        Ok(Response::new(ListPlaylistTracksResponse {
            tracks,
            page_info: Some(page_info),
        }))
    }
}

fn to_proto_playlist(playlist: DomainPlaylist) -> Result<Playlist, CanopyError> {
    Ok(Playlist {
        id: playlist.id,
        name: playlist.name,
        description: (!playlist.description.is_empty()).then_some(playlist.description),
        revision: playlist.updated_at_epoch_ms,
        created_at: Some(timestamp_from_epoch_ms(playlist.created_at_epoch_ms)?),
        updated_at: Some(timestamp_from_epoch_ms(playlist.updated_at_epoch_ms)?),
    })
}

fn to_proto_playlist_track(item: PlaylistTrackItem) -> Result<PlaylistTrack, CanopyError> {
    let position = u32::try_from(item.position)
        .map_err(|_| CanopyError::Internal("stored playlist position is negative".into()))?;
    Ok(PlaylistTrack {
        playlist_id: item.playlist_id,
        track: Some(to_track_summary(item.item)),
        position,
        added_at: Some(timestamp_from_epoch_ms(item.added_at_epoch_ms)?),
    })
}

fn validate_playlist_update_mask(mask: Option<&FieldMask>) -> Result<(), CanopyError> {
    let Some(mask) = mask else {
        return Err(CanopyError::InvalidArgument(
            "update_mask is required".into(),
        ));
    };
    if mask.paths.is_empty()
        || mask
            .paths
            .iter()
            .any(|path| path != "name" && path != "description")
    {
        return Err(CanopyError::InvalidArgument(
            "update_mask may contain only name and description".into(),
        ));
    }
    Ok(())
}

fn timestamp_from_epoch_ms(epoch_ms: u64) -> Result<Timestamp, CanopyError> {
    let seconds = i64::try_from(epoch_ms / 1_000)
        .map_err(|_| CanopyError::Internal("playlist timestamp overflow".into()))?;
    let nanos = i32::try_from((epoch_ms % 1_000) * 1_000_000)
        .map_err(|_| CanopyError::Internal("playlist timestamp overflow".into()))?;
    Ok(Timestamp { seconds, nanos })
}

#[cfg(test)]
mod tests {
    use canopy_core::MediaItem;

    use super::*;

    #[test]
    fn playlist_resource_maps_timestamps_and_revision() {
        let playlist = to_proto_playlist(DomainPlaylist {
            id: "playlist-1".into(),
            profile_id: "profile-1".into(),
            name: "Road Mix".into(),
            description: "For drives".into(),
            created_at_epoch_ms: 1_234,
            updated_at_epoch_ms: 2_345,
        })
        .unwrap();

        assert_eq!(playlist.id, "playlist-1");
        assert_eq!(playlist.description.as_deref(), Some("For drives"));
        assert_eq!(playlist.revision, 2_345);
        assert_eq!(playlist.created_at.unwrap().nanos, 234_000_000);
        assert_eq!(playlist.updated_at.unwrap().seconds, 2);
    }

    #[test]
    fn playlist_track_resource_maps_position_and_added_at() {
        let track = to_proto_playlist_track(PlaylistTrackItem {
            playlist_id: "playlist-1".into(),
            item: MediaItem {
                id: "track-1".into(),
                title: "Track".into(),
                ..MediaItem::default()
            },
            position: 42,
            added_at_epoch_ms: 3_456,
        })
        .unwrap();

        assert_eq!(track.playlist_id, "playlist-1");
        assert_eq!(track.track.unwrap().id, "track-1");
        assert_eq!(track.position, 42);
        assert_eq!(track.added_at.unwrap().nanos, 456_000_000);
    }
}
