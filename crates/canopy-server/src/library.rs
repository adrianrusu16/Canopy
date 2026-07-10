//! Logged-in profile library service.
//!
//! Anonymous sessions are not durable users. This service saves library items
//! only after authenticated profile lookup.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, LibraryItem, LibraryRepository, Page, ProfileRepository,
    SavedTrackPage, UserIdentity,
};

/// Application service for profile-owned saved library items.
#[derive(Clone)]
pub struct LibraryService {
    profiles: Arc<dyn ProfileRepository>,
    library: Arc<dyn LibraryRepository>,
}

impl LibraryService {
    /// Creates a library service over profile storage and library storage.
    pub fn new(profiles: Arc<dyn ProfileRepository>, library: Arc<dyn LibraryRepository>) -> Self {
        Self { profiles, library }
    }

    /// Saves a track in the authenticated profile's library.
    pub async fn save_track(
        &self,
        identity: &UserIdentity,
        track_id: &str,
    ) -> CanopyResult<LibraryItem> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        self.library.save_track(&profile_id, track_id).await
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
        self.library.list_tracks(&profile_id, page).await
    }

    /// Returns whether the authenticated profile has saved the track.
    pub async fn is_saved(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<bool> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        self.library.is_saved(&profile_id, track_id).await
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
    use crate::jade_store::{InMemoryLibraryStore, InMemoryProfileStore};

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(
        profiles: Arc<InMemoryProfileStore>,
        library: Arc<InMemoryLibraryStore>,
    ) -> LibraryService {
        LibraryService::new(profiles, library)
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
}
