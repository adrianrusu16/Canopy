//! Request-principal classification for access-policy decisions.

use std::sync::Arc;

use canopy_core::{
    CanopyResult, InstanceSettingsRepository, ProfileRepository, TrackAccessScope, UserIdentity,
};

/// Resolves an optional verified identity into the current access class.
#[derive(Clone)]
pub struct PrincipalService {
    profiles: Arc<dyn ProfileRepository>,
    settings: Arc<dyn InstanceSettingsRepository>,
}

impl PrincipalService {
    /// Creates a principal classifier over profile and instance settings ports.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        settings: Arc<dyn InstanceSettingsRepository>,
    ) -> Self {
        Self { profiles, settings }
    }

    /// Derives the current track scope for an optional verified identity.
    pub async fn track_scope_for_identity(
        &self,
        identity: Option<&UserIdentity>,
    ) -> CanopyResult<TrackAccessScope> {
        let Some(identity) = identity else {
            return Ok(TrackAccessScope::Public);
        };
        let Some(profile) = self
            .profiles
            .get_by_external_user_id(&identity.user_id)
            .await?
        else {
            return Ok(TrackAccessScope::Public);
        };
        self.track_scope_for_profile(&profile.id).await
    }

    /// Derives the current track scope for a verified durable profile.
    pub async fn track_scope_for_profile(
        &self,
        profile_id: &str,
    ) -> CanopyResult<TrackAccessScope> {
        if self.settings.owner_profile_id().await?.as_deref() == Some(profile_id) {
            Ok(TrackAccessScope::Owner {
                profile_id: profile_id.to_string(),
            })
        } else {
            Ok(TrackAccessScope::Public)
        }
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use canopy_core::{
        InstanceSettingsRepository, ProfileRepository, TrackAccessScope, UserIdentity,
    };

    use super::*;
    use crate::jade_store::{InMemoryInstanceSettingsStore, InMemoryProfileStore};

    fn service_with_stores() -> (
        PrincipalService,
        Arc<InMemoryProfileStore>,
        Arc<InMemoryInstanceSettingsStore>,
    ) {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        let service = PrincipalService::new(profiles.clone(), settings.clone());
        (service, profiles, settings)
    }

    #[tokio::test]
    async fn missing_identity_receives_public_scope() {
        let (service, _, _) = service_with_stores();
        let principal = service.track_scope_for_identity(None).await.unwrap();
        assert_eq!(principal, TrackAccessScope::Public);
    }

    #[tokio::test]
    async fn identity_without_profile_receives_public_scope() {
        let (service, _, _) = service_with_stores();
        let identity = UserIdentity {
            user_id: "known-token-user".into(),
        };
        let principal = service
            .track_scope_for_identity(Some(&identity))
            .await
            .unwrap();
        assert_eq!(principal, TrackAccessScope::Public);
    }

    #[tokio::test]
    async fn non_owner_profile_receives_public_scope() {
        let (service, profiles, _) = service_with_stores();
        profiles
            .upsert_profile("listener", None, false)
            .await
            .unwrap();
        let identity = UserIdentity {
            user_id: "listener".into(),
        };
        let principal = service
            .track_scope_for_identity(Some(&identity))
            .await
            .unwrap();
        assert_eq!(principal, TrackAccessScope::Public);
    }

    #[tokio::test]
    async fn configured_profile_receives_owner_scope() {
        let (service, profiles, settings) = service_with_stores();
        let profile = profiles
            .upsert_profile("owner-user", Some("Owner"), true)
            .await
            .unwrap();
        settings.set_owner_profile_id(&profile.id).await.unwrap();
        let identity = UserIdentity {
            user_id: "owner-user".into(),
        };
        let principal = service
            .track_scope_for_identity(Some(&identity))
            .await
            .unwrap();
        assert_eq!(
            principal,
            TrackAccessScope::Owner {
                profile_id: profile.id,
            }
        );
    }

    #[tokio::test]
    async fn configured_owner_receives_owner_scope_until_ownership_changes() {
        let (service, profiles, settings) = service_with_stores();
        let owner = profiles
            .upsert_profile("owner-user", Some("Owner"), true)
            .await
            .unwrap();
        let replacement = profiles
            .upsert_profile("replacement-user", Some("Replacement"), true)
            .await
            .unwrap();
        settings.set_owner_profile_id(&owner.id).await.unwrap();

        assert_eq!(
            service.track_scope_for_profile(&owner.id).await.unwrap(),
            TrackAccessScope::Owner {
                profile_id: owner.id.clone(),
            }
        );

        settings
            .set_owner_profile_id(&replacement.id)
            .await
            .unwrap();
        assert_eq!(
            service.track_scope_for_profile(&owner.id).await.unwrap(),
            TrackAccessScope::Public
        );
    }
}
