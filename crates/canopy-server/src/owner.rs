//! Explicit instance-owner assignment and authorization.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, InstanceSettingsRepository, ProfileRepository, UserProfile,
};

/// Assigns and verifies the one profile allowed to access personal media.
#[derive(Clone)]
pub struct OwnerService {
    profiles: Arc<dyn ProfileRepository>,
    settings: Arc<dyn InstanceSettingsRepository>,
}

impl OwnerService {
    /// Creates an owner service over profile and installation-settings ports.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        settings: Arc<dyn InstanceSettingsRepository>,
    ) -> Self {
        Self { profiles, settings }
    }

    /// Assigns an existing external user profile as the instance owner.
    pub async fn set_owner(&self, external_user_id: &str) -> CanopyResult<UserProfile> {
        let profile = self
            .profiles
            .get_by_external_user_id(external_user_id)
            .await?
            .ok_or_else(|| CanopyError::not_found("profile", external_user_id))?;

        self.settings.set_owner_profile_id(&profile.id).await?;
        Ok(profile)
    }

    /// Returns whether the profile is the explicitly assigned instance owner.
    pub async fn is_owner(&self, profile_id: &str) -> CanopyResult<bool> {
        Ok(self.settings.owner_profile_id().await?.as_deref() == Some(profile_id))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use canopy_core::{CanopyError, ProfileRepository};

    use super::*;
    use crate::jade_store::{InMemoryInstanceSettingsStore, InMemoryProfileStore};

    #[tokio::test]
    async fn owner_assignment_requires_existing_profile() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        let service = OwnerService::new(profiles, settings);

        let err = service.set_owner("missing-user").await.unwrap_err();

        assert!(matches!(err, CanopyError::NotFound { .. }));
    }

    #[tokio::test]
    async fn owner_assignment_and_checks_round_trip() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        let service = OwnerService::new(profiles.clone(), settings);
        let profile = profiles
            .upsert_profile("owner-user", None, false)
            .await
            .unwrap();

        assert_eq!(service.set_owner("owner-user").await.unwrap(), profile);
        assert!(service.is_owner(&profile.id).await.unwrap());
        assert!(
            !service
                .is_owner("00000000-0000-0000-0000-000000000000")
                .await
                .unwrap()
        );
    }
}
