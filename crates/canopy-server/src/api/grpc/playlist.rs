use std::sync::Arc;

use canopy_proto::playlist_service_server::PlaylistService;
use canopy_proto::{
    AddPlaylistTrackRequest, CreatePlaylistRequest, DeletePlaylistRequest, GetPlaylistRequest,
    ListPlaylistTracksRequest, ListPlaylistTracksResponse, ListPlaylistsRequest,
    ListPlaylistsResponse, Playlist, PlaylistTrack, RemovePlaylistTrackRequest,
    ReorderPlaylistTracksRequest, UpdatePlaylistRequest,
};
use tonic::{Request, Response, Status};

use super::{GrpcServices, not_implemented};

pub struct PlaylistGrpc(pub Arc<GrpcServices>);

#[tonic::async_trait]
impl PlaylistService for PlaylistGrpc {
    async fn create_playlist(
        &self,
        _: Request<CreatePlaylistRequest>,
    ) -> Result<Response<Playlist>, Status> {
        Err(not_implemented("PlaylistService.CreatePlaylist"))
    }
    async fn get_playlist(
        &self,
        _: Request<GetPlaylistRequest>,
    ) -> Result<Response<Playlist>, Status> {
        Err(not_implemented("PlaylistService.GetPlaylist"))
    }
    async fn update_playlist(
        &self,
        _: Request<UpdatePlaylistRequest>,
    ) -> Result<Response<Playlist>, Status> {
        Err(not_implemented("PlaylistService.UpdatePlaylist"))
    }
    async fn delete_playlist(
        &self,
        _: Request<DeletePlaylistRequest>,
    ) -> Result<Response<()>, Status> {
        Err(not_implemented("PlaylistService.DeletePlaylist"))
    }
    async fn list_playlists(
        &self,
        _: Request<ListPlaylistsRequest>,
    ) -> Result<Response<ListPlaylistsResponse>, Status> {
        Err(not_implemented("PlaylistService.ListPlaylists"))
    }
    async fn add_playlist_track(
        &self,
        _: Request<AddPlaylistTrackRequest>,
    ) -> Result<Response<PlaylistTrack>, Status> {
        Err(not_implemented("PlaylistService.AddPlaylistTrack"))
    }
    async fn remove_playlist_track(
        &self,
        _: Request<RemovePlaylistTrackRequest>,
    ) -> Result<Response<()>, Status> {
        Err(not_implemented("PlaylistService.RemovePlaylistTrack"))
    }
    async fn reorder_playlist_tracks(
        &self,
        _: Request<ReorderPlaylistTracksRequest>,
    ) -> Result<Response<Playlist>, Status> {
        Err(not_implemented("PlaylistService.ReorderPlaylistTracks"))
    }
    async fn list_playlist_tracks(
        &self,
        _: Request<ListPlaylistTracksRequest>,
    ) -> Result<Response<ListPlaylistTracksResponse>, Status> {
        Err(not_implemented("PlaylistService.ListPlaylistTracks"))
    }
}
