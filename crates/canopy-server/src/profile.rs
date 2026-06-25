//! Logged-in profile service.
//!
//! Anonymous sessions are not treated as durable users. This service requires
//! an authenticated end-user identity before creating or updating profile-backed state.

use std::sync::Arc;

use canopy_core::{CanopyResult, ProfileRepository, UserIdentity, UserProfile};

/// Application service for real logged-in user profiles.
#[derive(Clone)]
pub struct ProfileService {
    profiles: Arc<dyn ProfileRepository>,
}

impl ProfileService {
    /// Creates a profile service over durable profile storage.
    pub fn new(profiles: Arc<dyn ProfileRepository>) -> Self {
        Self { profiles }
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

        self.profiles
            .upsert_profile(&identity.user_id, display_name, history_enabled)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::InMemoryProfileStore;
    use std::sync::Arc;

    #[tokio::test]
    async fn upsert_profile_persists_real_user_preferences() {
        let service = ProfileService::new(Arc::new(InMemoryProfileStore::default()));
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
