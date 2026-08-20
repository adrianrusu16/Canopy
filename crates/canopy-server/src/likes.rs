//! Logged-in track-like service.
//!
//! Likes are profile-owned durable state and are unavailable to anonymous users.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, LikeRepository, LikedTrackPage, Page, ProfileRepository, TrackLike,
    UserIdentity,
};

use crate::principal::PrincipalService;

/// Application service for profile-owned positive track likes.
#[derive(Clone)]
pub struct LikeService {
    profiles: Arc<dyn ProfileRepository>,
    likes: Arc<dyn LikeRepository>,
    principal: PrincipalService,
}

impl LikeService {
    /// Creates a like service over profile storage and like storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        likes: Arc<dyn LikeRepository>,
        principal: PrincipalService,
    ) -> Self {
        Self {
            profiles,
            likes,
            principal,
        }
    }

    /// Likes a track for the authenticated profile.
    pub async fn like_track(
        &self,
        identity: &UserIdentity,
        track_id: &str,
    ) -> CanopyResult<TrackLike> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.likes.like_track(&profile_id, track_id, &scope).await
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
    ) -> CanopyResult<LikedTrackPage> {
        let profile_id = self.profile_id(identity).await?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.likes
            .list_liked_tracks(&profile_id, &scope, page)
            .await
    }

    /// Returns whether the authenticated profile has liked the track.
    pub async fn is_liked(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<bool> {
        let profile_id = self.profile_id(identity).await?;
        let track_id = validate_track_id(track_id)?;
        let scope = self.principal.track_scope_for_profile(&profile_id).await?;
        self.likes.is_liked(&profile_id, track_id, &scope).await
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
        InMemoryCatalog, InMemoryCatalogEntry, InMemoryInstanceSettingsStore, InMemoryLikeStore,
        InMemoryProfileStore,
    };
    use canopy_core::{IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility};

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(profiles: Arc<InMemoryProfileStore>, likes: Arc<InMemoryLikeStore>) -> LikeService {
        let principal = PrincipalService::new(
            profiles.clone(),
            Arc::new(InMemoryInstanceSettingsStore::default()),
        );
        LikeService::new(profiles, likes, principal)
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
        let page = service
            .list_liked_tracks(
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
        assert_eq!(page.items[0].liked_at_epoch_ms, liked.liked_at_epoch_ms);

        service.unlike_track(&identity(), "track-1").await.unwrap();
        service.unlike_track(&identity(), "track-1").await.unwrap();

        assert!(!service.is_liked(&identity(), "track-1").await.unwrap());
    }

    #[tokio::test]
    async fn ownership_transfer_hides_liked_personal_track_but_allows_unlike() {
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
        let likes = Arc::new(InMemoryLikeStore::new(catalog));
        let principal = PrincipalService::new(profiles.clone(), settings.clone());
        let service = LikeService::new(profiles, likes, principal);

        service
            .like_track(&identity(), "personal-track")
            .await
            .unwrap();
        settings
            .set_owner_profile_id(&replacement.id)
            .await
            .unwrap();

        assert!(
            !service
                .is_liked(&identity(), "personal-track")
                .await
                .unwrap()
        );
        let page = service
            .list_liked_tracks(
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
            service.like_track(&identity(), "personal-track").await,
            Err(CanopyError::NotFound { .. })
        ));

        service
            .unlike_track(&identity(), "personal-track")
            .await
            .unwrap();
        settings.set_owner_profile_id(&owner.id).await.unwrap();
        assert!(
            !service
                .is_liked(&identity(), "personal-track")
                .await
                .unwrap()
        );
    }
}
