//! Logged-in track-like service.
//!
//! Likes are profile-owned durable state and are unavailable to anonymous users.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, LikeRepository, MediaPage, Page, ProfileRepository, TrackLike,
    UserIdentity,
};

/// Application service for profile-owned positive track likes.
#[derive(Clone)]
pub struct LikeService {
    profiles: Arc<dyn ProfileRepository>,
    likes: Arc<dyn LikeRepository>,
}

impl LikeService {
    /// Creates a like service over profile storage and like storage.
    pub fn new(profiles: Arc<dyn ProfileRepository>, likes: Arc<dyn LikeRepository>) -> Self {
        Self { profiles, likes }
    }

    /// Likes a track for the authenticated profile.
    pub async fn like_track(
        &self,
        identity: &UserIdentity,
        track_id: &str,
    ) -> CanopyResult<TrackLike> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        self.likes.like_track(&profile_id, track_id).await
    }

    /// Removes a track like for the authenticated profile.
    pub async fn unlike_track(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<()> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        self.likes.unlike_track(&profile_id, track_id).await
    }

    /// Lists liked tracks for the authenticated profile.
    pub async fn list_liked_tracks(
        &self,
        identity: &UserIdentity,
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let profile_id = self.profile_id(identity).await?;
        self.likes.list_liked_tracks(&profile_id, page).await
    }

    /// Returns whether the authenticated profile has liked the track.
    pub async fn is_liked(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<bool> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        self.likes.is_liked(&profile_id, track_id).await
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
    use crate::jade_store::{InMemoryLikeStore, InMemoryProfileStore};

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(profiles: Arc<InMemoryProfileStore>, likes: Arc<InMemoryLikeStore>) -> LikeService {
        LikeService::new(profiles, likes)
    }

    #[tokio::test]
    async fn like_track_rejects_missing_profile() {
        let service = service(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryLikeStore::default()),
        );

        let err = service
            .like_track(&identity(), "track-1")
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn like_track_rejects_empty_track_id() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryLikeStore::default()));

        let err = service.like_track(&identity(), " ").await.unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn like_unlike_and_list_are_idempotent() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let profile = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let likes = Arc::new(InMemoryLikeStore::default());
        let service = service(profiles, likes);

        let liked = service.like_track(&identity(), "track-1").await.unwrap();
        let liked_again = service.like_track(&identity(), "track-1").await.unwrap();

        assert_eq!(liked.profile_id, profile.id);
        assert_eq!(liked_again.track_id, "track-1");
        assert!(service.is_liked(&identity(), "track-1").await.unwrap());
        assert_eq!(
            service
                .list_liked_tracks(
                    &identity(),
                    Page {
                        limit: 10,
                        offset: 0,
                    },
                )
                .await
                .unwrap()
                .items
                .len(),
            1
        );

        service.unlike_track(&identity(), "track-1").await.unwrap();
        service.unlike_track(&identity(), "track-1").await.unwrap();

        assert!(!service.is_liked(&identity(), "track-1").await.unwrap());
    }
}
