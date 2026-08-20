//! Logged-in profile playlist service.
//!
//! Anonymous sessions are not durable users. This service manages playlists
//! only after authenticated profile lookup.

use std::sync::Arc;

use crate::principal::PrincipalService;
use canopy_core::{
    CanopyError, CanopyResult, Page, Playlist, PlaylistPage, PlaylistRepository, PlaylistTrackItem,
    PlaylistTrackPage, ProfileRepository, UserIdentity,
};

/// Application service for profile-owned playlists.
#[derive(Clone)]
pub struct PlaylistService {
    profiles: Arc<dyn ProfileRepository>,
    playlists: Arc<dyn PlaylistRepository>,
    principal: PrincipalService,
}

impl PlaylistService {
    /// Creates a playlist service over profile storage and playlist storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        playlists: Arc<dyn PlaylistRepository>,
        principal: PrincipalService,
    ) -> Self {
        Self {
            profiles,
            playlists,
            principal,
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

    /// Returns one playlist for the authenticated profile.
    pub async fn get_playlist(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
    ) -> CanopyResult<Playlist> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        self.playlists
            .get_playlist(&profile_id, playlist_id)
            .await?
            .ok_or_else(|| CanopyError::not_found("playlist", playlist_id))
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
    ) -> CanopyResult<PlaylistTrackItem> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let track_id = validate_track_id(track_id)?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.playlists
            .add_track(&profile_id, playlist_id, track_id, position, &scope)
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
    ) -> CanopyResult<Playlist> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let track_ids = track_ids
            .into_iter()
            .map(|track_id| validate_track_id(&track_id).map(str::to_string))
            .collect::<CanopyResult<Vec<_>>>()?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.playlists
            .reorder_tracks(&profile_id, playlist_id, &track_ids, &scope)
            .await
    }

    /// Lists playlist tracks as renderable media items for the authenticated profile.
    pub async fn list_tracks(
        &self,
        identity: &UserIdentity,
        playlist_id: &str,
        page: Page,
    ) -> CanopyResult<PlaylistTrackPage> {
        let profile_id = self.profile_id(identity).await?;
        let playlist_id = validate_playlist_id(playlist_id)?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.playlists
            .list_tracks(&profile_id, playlist_id, &scope, page)
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
    use crate::jade_store::{
        InMemoryCatalog, InMemoryCatalogEntry, InMemoryInstanceSettingsStore,
        InMemoryPlaylistStore, InMemoryProfileStore,
    };
    use canopy_core::{
        IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility, TrackAccessScope,
    };

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(
        profiles: Arc<InMemoryProfileStore>,
        playlists: Arc<InMemoryPlaylistStore>,
    ) -> PlaylistService {
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        let principal = PrincipalService::new(profiles.clone(), settings);
        PlaylistService::new(profiles, playlists, principal)
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

        let fetched = service
            .get_playlist(&identity(), &playlist.id)
            .await
            .unwrap();
        assert_eq!(fetched.id, playlist.id);

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

        let added = service
            .add_track(&identity(), &playlist.id, "track-1", None)
            .await
            .unwrap();
        assert_eq!(added.playlist_id, playlist.id);
        assert_eq!(added.item.id, "track-1");
        assert_eq!(added.position, 0);
        assert!(added.added_at_epoch_ms > 0);

        let inserted = service
            .add_track(&identity(), &playlist.id, "track-2", Some(0))
            .await
            .unwrap();
        assert_eq!(inserted.item.id, "track-2");
        assert_eq!(inserted.position, 0);

        let reordered = service
            .reorder_tracks(
                &identity(),
                &playlist.id,
                vec!["track-1".into(), "track-2".into()],
            )
            .await
            .unwrap();
        assert_eq!(reordered.id, playlist.id);

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
        assert_eq!(page.items[0].item.id, "track-1");
        assert_eq!(page.items[0].position, 0);
        assert!(page.items[0].added_at_epoch_ms > 0);
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

    #[tokio::test]
    async fn playlist_access_revocation_hides_but_does_not_block_cleanup() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let owner = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let replacement = profiles
            .upsert_profile("replacement", Some("Grace"), true)
            .await
            .unwrap();
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        settings.set_owner_profile_id(&owner.id).await.unwrap();
        let catalog = Arc::new(InMemoryCatalog::from_entries(vec![InMemoryCatalogEntry {
            item: MediaItem {
                id: "owner-ready".into(),
                ..MediaItem::default()
            },
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some(owner.id.clone()),
        }]));
        let playlists = Arc::new(InMemoryPlaylistStore::new(catalog));
        let principal = PrincipalService::new(profiles.clone(), settings.clone());
        let service = PlaylistService::new(profiles, playlists, principal);
        let playlist = service
            .create_playlist(&identity(), "Private Mix", "")
            .await
            .unwrap();

        service
            .add_track(&identity(), &playlist.id, "owner-ready", None)
            .await
            .unwrap();
        settings
            .set_owner_profile_id(&replacement.id)
            .await
            .unwrap();

        let add_error = service
            .add_track(&identity(), &playlist.id, "owner-ready", None)
            .await
            .unwrap_err();
        assert!(matches!(add_error, CanopyError::NotFound { .. }));

        let visible = service
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
        assert!(visible.items.is_empty());
        assert_eq!(visible.total_count, 0);
        service
            .reorder_tracks(&identity(), &playlist.id, vec![])
            .await
            .unwrap();
        service
            .remove_track(&identity(), &playlist.id, "owner-ready")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn reorder_requires_only_visible_membership() {
        let catalog = Arc::new(InMemoryCatalog::from_entries(vec![
            InMemoryCatalogEntry {
                item: MediaItem {
                    id: "public-a".into(),
                    ..MediaItem::default()
                },
                visibility: MediaVisibility::ReleaseSafe,
                ingest_status: IngestStatus::Ready,
                owner_profile_id: None,
            },
            InMemoryCatalogEntry {
                item: MediaItem {
                    id: "public-b".into(),
                    ..MediaItem::default()
                },
                visibility: MediaVisibility::ReleaseSafe,
                ingest_status: IngestStatus::Ready,
                owner_profile_id: None,
            },
            InMemoryCatalogEntry {
                item: MediaItem {
                    id: "owner-ready".into(),
                    ..MediaItem::default()
                },
                visibility: MediaVisibility::Personal,
                ingest_status: IngestStatus::Ready,
                owner_profile_id: Some("owner".into()),
            },
        ]));
        let store = InMemoryPlaylistStore::new(catalog);
        let owner_scope = TrackAccessScope::Owner {
            profile_id: "owner".into(),
        };
        let playlist = store
            .create_playlist("profile", "Mixed Mix", "")
            .await
            .unwrap();
        for track_id in ["public-a", "public-b", "owner-ready"] {
            store
                .add_track("profile", &playlist.id, track_id, None, &owner_scope)
                .await
                .unwrap();
        }

        store
            .reorder_tracks(
                "profile",
                &playlist.id,
                &["public-b".into(), "public-a".into()],
                &TrackAccessScope::Public,
            )
            .await
            .unwrap();
        let visible = store
            .list_tracks(
                "profile",
                &playlist.id,
                &TrackAccessScope::Public,
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            visible
                .items
                .iter()
                .map(|track| track.item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["public-b", "public-a"]
        );

        let restored = store
            .list_tracks(
                "profile",
                &playlist.id,
                &owner_scope,
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            restored
                .items
                .iter()
                .map(|track| (track.item.id.as_str(), track.position))
                .collect::<Vec<_>>(),
            vec![("public-b", 0), ("public-a", 1), ("owner-ready", 2)]
        );
    }
}
