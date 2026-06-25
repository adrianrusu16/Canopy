//! Logged-in profile service.
//!
//! Anonymous sessions are not treated as durable users. This service requires
//! a valid end-user token before creating or updating profile-backed state.

use std::sync::Arc;

use canopy_core::{CanopyResult, ProfileRepository, UserProfile};

use crate::auth::AuthService;

/// Application service for real logged-in user profiles.
#[derive(Clone)]
pub struct ProfileService {
    profiles: Arc<dyn ProfileRepository>,
    auth: AuthService,
}

impl ProfileService {
    /// Creates a profile service over durable profile storage and token auth.
    pub fn new(profiles: Arc<dyn ProfileRepository>, auth: AuthService) -> Self {
        Self { profiles, auth }
    }

    /// Verifies the login token and creates or updates the user's profile.
    pub async fn upsert_profile(
        &self,
        auth_token: &str,
        display_name: &str,
        history_enabled: bool,
    ) -> CanopyResult<UserProfile> {
        let identity = self.auth.verify(auth_token)?;
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
    use crate::auth::AuthService;
    use crate::jade_store::InMemoryProfileStore;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn upsert_profile_requires_valid_login_token() {
        let auth = AuthService::new("secret");
        let service = ProfileService::new(Arc::new(InMemoryProfileStore::default()), auth);

        let err = service
            .upsert_profile("not-a-token", "Ada", true)
            .await
            .unwrap_err();

        assert!(matches!(err, canopy_core::CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn upsert_profile_persists_real_user_preferences() {
        let auth = AuthService::new("secret");
        let token = auth.mint("user-123", Duration::from_secs(3600)).unwrap();
        let service = ProfileService::new(Arc::new(InMemoryProfileStore::default()), auth);

        let profile = service.upsert_profile(&token, "Ada", true).await.unwrap();

        assert_eq!(profile.external_user_id, "user-123");
        assert_eq!(profile.display_name.as_deref(), Some("Ada"));
        assert!(profile.history_enabled);
    }
}
