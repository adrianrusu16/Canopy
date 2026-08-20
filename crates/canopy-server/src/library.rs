//! Logged-in profile library service.
//!
//! Anonymous sessions are not durable users. This service saves library items
//! only after authenticated profile lookup.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, LibraryItem, LibraryRepository, Page, ProfileRepository,
    SavedTrackPage, UserIdentity,
};

use crate::principal::PrincipalService;

/// Application service for profile-owned saved library items.
#[derive(Clone)]
pub struct LibraryService {
    profiles: Arc<dyn ProfileRepository>,
    library: Arc<dyn LibraryRepository>,
    principal: PrincipalService,
}

impl LibraryService {
    /// Creates a library service over profile storage and library storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        library: Arc<dyn LibraryRepository>,
        principal: PrincipalService,
    ) -> Self {
        Self {
            profiles,
            library,
            principal,
        }
    }

    /// Saves a track in the authenticated profile's library.
    pub async fn save_track(
        &self,
        identity: &UserIdentity,
        track_id: &str,
    ) -> CanopyResult<LibraryItem> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.library.save_track(&profile_id, track_id, &scope).await
    }

    /// Removes a track from the authenticated profile's library.
    pub async fn remove_track(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<()> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        self.library.remove_track(&profile_id, track_id).await
    }

    /// Lists saved tracks for the authenticated profile.
    pub async fn list_tracks(
        &self,
        identity: &UserIdentity,
        page: Page,
    ) -> CanopyResult<SavedTrackPage> {
        let profile_id = self.profile_id(identity).await?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.library.list_tracks(&profile_id, &scope, page).await
    }

    /// Returns whether the authenticated profile has saved the track.
    pub async fn is_saved(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<bool> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.library.is_saved(&profile_id, track_id, &scope).await
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
        InMemoryCatalog, InMemoryCatalogEntry, InMemoryInstanceSettingsStore, InMemoryLibraryStore,
        InMemoryProfileStore,
    };
    use canopy_core::{IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility};

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(
        profiles: Arc<InMemoryProfileStore>,
        library: Arc<InMemoryLibraryStore>,
    ) -> LibraryService {
        let principal = PrincipalService::new(
            profiles.clone(),
            Arc::new(InMemoryInstanceSettingsStore::default()),
        );
        LibraryService::new(profiles, library, principal)
    }

    #[tokio::test]
    async fn save_track_rejects_missing_profile() {
        let service = service(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryLibraryStore::default()),
        );

        let err = service
            .save_track(&identity(), "track-1")
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn save_track_rejects_empty_track_id() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryLibraryStore::default()));

        let err = service.save_track(&identity(), " ").await.unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn save_remove_and_list_are_idempotent() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let profile = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let library = Arc::new(InMemoryLibraryStore::default());
        let service = service(profiles, library);

        let saved = service.save_track(&identity(), "track-1").await.unwrap();
        let saved_again = service.save_track(&identity(), "track-1").await.unwrap();

        assert_eq!(saved.profile_id, profile.id);
        assert_eq!(saved_again.track_id, "track-1");
        assert!(service.is_saved(&identity(), "track-1").await.unwrap());
        let page = service
            .list_tracks(
                &identity(),
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].item.id, "track-1");
        assert_eq!(page.items[0].saved_at_epoch_ms, saved.added_at_epoch_ms);

        service.remove_track(&identity(), "track-1").await.unwrap();
        service.remove_track(&identity(), "track-1").await.unwrap();

        assert!(!service.is_saved(&identity(), "track-1").await.unwrap());
    }

    #[tokio::test]
    async fn ownership_transfer_hides_saved_personal_track_but_allows_removal() {
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
                id: "personal-track".into(),
                ..MediaItem::default()
            },
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some(owner.id.clone()),
        }]));
        let library = Arc::new(InMemoryLibraryStore::new(catalog));
        let principal = PrincipalService::new(profiles.clone(), settings.clone());
        let service = LibraryService::new(profiles, library, principal);

        service
            .save_track(&identity(), "personal-track")
            .await
            .unwrap();
        settings
            .set_owner_profile_id(&replacement.id)
            .await
            .unwrap();

        assert!(
            !service
                .is_saved(&identity(), "personal-track")
                .await
                .unwrap()
        );
        let page = service
            .list_tracks(
                &identity(),
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.total_count, 0);
        assert!(matches!(
            service.save_track(&identity(), "personal-track").await,
            Err(CanopyError::NotFound { .. })
        ));

        service
            .remove_track(&identity(), "personal-track")
            .await
            .unwrap();
        settings.set_owner_profile_id(&owner.id).await.unwrap();
        assert!(
            !service
                .is_saved(&identity(), "personal-track")
                .await
                .unwrap()
        );
    }
}
