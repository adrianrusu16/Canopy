//! Logged-in profile playlist service.
//!
//! Anonymous sessions are not durable users. This service manages playlists
//! only after authenticated profile lookup.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, MediaPage, Page, Playlist, PlaylistPage, PlaylistRepository,
    ProfileRepository, UserIdentity,
};

/// Application service for profile-owned playlists.
#[derive(Clone)]
pub struct PlaylistService {
    profiles: Arc<dyn ProfileRepository>,
    playlists: Arc<dyn PlaylistRepository>,
}

impl PlaylistService {
    /// Creates a playlist service over profile storage and playlist storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        playlists: Arc<dyn PlaylistRepository>,
    ) -> Self {
        Self {
            profiles,
            playlists,
        }
    }

    /// Creates a playlist for the authenticated profile.
    pub async fn create_playlist(
        &self,
        identity: &UserIdentity,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist> {
        let profile_id = self.profile_id(identity).await?;
        let name = validate_name(name)?;
        let description = description.trim();
        self.playlists
            .create_playlist(&profile_id, name, description)
            .await
    }

    /// Updates playlist metadata for the authenticated profile.
    pub async fn update_playlist(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let name = validate_name(name)?;
        let description = description.trim();
        self.playlists
            .update_playlist(&profile_id, playlist_id, name, description)
            .await
    }

    /// Deletes a playlist for the authenticated profile.
    pub async fn delete_playlist(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
    ) -> CanopyResult<()> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        self.playlists
            .delete_playlist(&profile_id, playlist_id)
            .await
    }

    /// Lists playlists for the authenticated profile.
    pub async fn list_playlists(
        &self,
        identity: &UserIdentity,
        page: Page,
    ) -> CanopyResult<PlaylistPage> {
        let profile_id = self.profile_id(identity).await?;
        self.playlists.list_playlists(&profile_id, page).await
    }

    /// Adds a track to a playlist for the authenticated profile.
    pub async fn add_track(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
        track_id: &str,
        position: Option<i32>,
    ) -> CanopyResult<()> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let track_id = validate_track_id(track_id)?;
        self.playlists
            .add_track(&profile_id, playlist_id, track_id, position)
            .await
    }

    /// Removes a track from a playlist for the authenticated profile.
    pub async fn remove_track(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
        track_id: &str,
    ) -> CanopyResult<()> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let track_id = validate_track_id(track_id)?;
        self.playlists
            .remove_track(&profile_id, playlist_id, track_id)
            .await
    }

    /// Reorders tracks in a playlist for the authenticated profile.
    pub async fn reorder_tracks(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
        track_ids: Vec<String>,
    ) -> CanopyResult<()> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let track_ids = track_ids
            .into_iter()
            .map(|track_id| validate_track_id(&track_id).map(str::to_string))
            .collect::<CanopyResult<Vec<_>>>()?;
        self.playlists
            .reorder_tracks(&profile_id, playlist_id, &track_ids)
            .await
    }

    /// Lists playlist tracks as renderable media items for the authenticated profile.
    pub async fn list_tracks(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        self.playlists
            .list_tracks(&profile_id, playlist_id, page)
            .await
    }

    async fn profile_id(&self, identity: &UserIdentity) -> CanopyResult<String> {
        let profile = self
            .profiles
            .get_by_external_user_id(&identity.user_id)
            .await?
            .ok_or_else(|| CanopyError::unauthenticated("profile not found"))?;
        Ok(profile.id)
    }
}

fn validate_name(name: &str) -> CanopyResult<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CanopyError::InvalidArgument(
            "playlist name is required".into(),
        ));
    }
    Ok(name)
}

fn validate_playlist_id(playlist_id: &str) -> CanopyResult<&str> {
    let playlist_id = playlist_id.trim();
    if playlist_id.is_empty() {
        return Err(CanopyError::InvalidArgument(
            "playlist_id is required".into(),
        ));
    }
    Ok(playlist_id)
}

fn validate_track_id(track_id: &str) -> CanopyResult<&str> {
    let track_id = track_id.trim();
    if track_id.is_empty() {
        return Err(CanopyError::InvalidArgument("track_id is required".into()));
    }
    Ok(track_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::{InMemoryPlaylistStore, InMemoryProfileStore};

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(
        profiles: Arc<InMemoryProfileStore>,
        playlists: Arc<InMemoryPlaylistStore>,
    ) -> PlaylistService {
        PlaylistService::new(profiles, playlists)
    }

    async fn profile_store() -> Arc<InMemoryProfileStore> {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        profiles
    }

    #[tokio::test]
    async fn create_playlist_rejects_missing_profile() {
        let service = service(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaylistStore::default()),
        );

        let err = service
            .create_playlist(&identity(), "Road Mix", "")
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn create_playlist_rejects_empty_name() {
        let service = service(
            profile_store().await,
            Arc::new(InMemoryPlaylistStore::default()),
        );

        let err = service
            .create_playlist(&identity(), " ", "")
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn create_update_list_and_delete_playlist() {
        let service = service(
            profile_store().await,
            Arc::new(InMemoryPlaylistStore::default()),
        );

        let playlist = service
            .create_playlist(&identity(), " Road Mix ", " For drives ")
            .await
            .unwrap();
        assert_eq!(playlist.name, "Road Mix");
        assert_eq!(playlist.description, "For drives");

        let updated = service
            .update_playlist(&identity(), &playlist.id, "Night Drive", "Late routes")
            .await
            .unwrap();
        assert_eq!(updated.name, "Night Drive");

        let page = service
            .list_playlists(
                &identity(),
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.total_count, 1);

        service
            .delete_playlist(&identity(), &playlist.id)
            .await
            .unwrap();
        let err = service
            .delete_playlist(&identity(), &playlist.id)
            .await
            .unwrap_err();
        assert!(matches!(err, CanopyError::NotFound { .. }));
    }

    #[tokio::test]
    async fn tracks_are_added_removed_reordered_and_listed() {
        let service = service(
            profile_store().await,
            Arc::new(InMemoryPlaylistStore::default()),
        );
        let playlist = service
            .create_playlist(&identity(), "Road Mix", "")
            .await
            .unwrap();

        service
            .add_track(&identity(), &playlist.id, "track-1", None)
            .await
            .unwrap();
        service
            .add_track(&identity(), &playlist.id, "track-2", Some(0))
            .await
            .unwrap();
        service
            .reorder_tracks(
                &identity(),
                &playlist.id,
                vec!["track-1".into(), "track-2".into()],
            )
            .await
            .unwrap();

        let page = service
            .list_tracks(
                &identity(),
                &playlist.id,
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.items[0].id, "track-1");
        assert_eq!(page.total_count, 2);

        service
            .remove_track(&identity(), &playlist.id, "track-2")
            .await
            .unwrap();
        service
            .remove_track(&identity(), &playlist.id, "track-2")
            .await
            .unwrap();
        assert_eq!(
            service
                .list_tracks(
                    &identity(),
                    &playlist.id,
                    Page {
                        limit: 10,
                        offset: 0,
                    },
                )
                .await
                .unwrap()
                .total_count,
            1
        );
    }

    #[tokio::test]
    async fn reorder_rejects_duplicate_track_ids() {
        let service = service(
            profile_store().await,
            Arc::new(InMemoryPlaylistStore::default()),
        );
        let playlist = service
            .create_playlist(&identity(), "Road Mix", "")
            .await
            .unwrap();
        service
            .add_track(&identity(), &playlist.id, "track-1", None)
            .await
            .unwrap();

        let err = service
            .reorder_tracks(
                &identity(),
                &playlist.id,
                vec!["track-1".into(), "track-1".into()],
            )
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }
}
