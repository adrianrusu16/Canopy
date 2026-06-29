//! Logged-in profile service.
//!
//! Anonymous sessions are not treated as durable users. This service requires
//! an authenticated end-user identity before creating or updating profile-backed state.

use std::sync::Arc;

use canopy_core::{
    CanopyResult, PlaybackHistoryRepository, ProfileRepository, UserIdentity, UserProfile,
};

/// Application service for real logged-in user profiles.
#[derive(Clone)]
pub struct ProfileService {
    profiles: Arc<dyn ProfileRepository>,
    history: Arc<dyn PlaybackHistoryRepository>,
}

impl ProfileService {
    /// Creates a profile service over durable profile and history storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        history: Arc<dyn PlaybackHistoryRepository>,
    ) -> Self {
        Self { profiles, history }
    }

    /// Creates or updates the authenticated user's profile.
    pub async fn upsert_profile(
        &self,
        identity: &UserIdentity,
        display_name: &str,
        history_enabled: bool,
    ) -> CanopyResult<UserProfile> {
        let display_name = display_name.trim();
        let display_name = if display_name.is_empty() {
            None
        } else {
            Some(display_name)
        };

        let profile = self
            .profiles
            .upsert_profile(&identity.user_id, display_name, history_enabled)
            .await?;

        if !history_enabled {
            self.history.clear(&profile.id).await?;
        }

        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::{InMemoryPlaybackHistoryStore, InMemoryProfileStore};
    use canopy_core::{PlaybackHistoryEvent, PlaybackHistoryRepository};
    use std::sync::Arc;

    #[tokio::test]
    async fn disabling_history_purges_in_memory_events() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = ProfileService::new(profiles.clone(), history.clone());
        let identity = UserIdentity {
            user_id: "privacy-user".into(),
        };
        let profile = service
            .upsert_profile(&identity, "Ada", true)
            .await
            .unwrap();
        history
            .record(PlaybackHistoryEvent {
                profile_id: profile.id,
                track_id: "track-1".into(),
                duration_ms: 1000,
                completion_pct: 0.5,
            })
            .await
            .unwrap();

        service
            .upsert_profile(&identity, "Ada", false)
            .await
            .unwrap();

        assert!(history.events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn upsert_profile_persists_real_user_preferences() {
        let service = ProfileService::new(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaybackHistoryStore::default()),
        );
        let identity = UserIdentity {
            user_id: "user-123".into(),
        };

        let profile = service
            .upsert_profile(&identity, "Ada", true)
            .await
            .unwrap();

        assert_eq!(profile.external_user_id, "user-123");
        assert_eq!(profile.display_name.as_deref(), Some("Ada"));
        assert!(profile.history_enabled);
    }
}
