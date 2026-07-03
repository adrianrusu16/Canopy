//! Logged-in profile service.
//!
//! Anonymous sessions are not treated as durable users. This service requires
//! an authenticated end-user identity before creating or updating profile-backed state.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, CatalogRepository, InstanceSettingsRepository, Page,
    PlaybackHistoryRepository, ProfileRepository, UserIdentity, UserProfile,
};

/// Application service for real logged-in user profiles.
#[derive(Clone)]
pub struct ProfileService {
    profiles: Arc<dyn ProfileRepository>,
    history: Arc<dyn PlaybackHistoryRepository>,
    settings: Option<Arc<dyn InstanceSettingsRepository>>,
    catalog: Option<Arc<dyn CatalogRepository>>,
}

impl ProfileService {
    /// Creates a profile service over durable profile and history storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        history: Arc<dyn PlaybackHistoryRepository>,
    ) -> Self {
        Self {
            profiles,
            history,
            settings: None,
            catalog: None,
        }
    }

    /// Adds lifecycle policy dependencies required for profile deletion.
    pub fn with_deletion_policy(
        mut self,
        settings: Arc<dyn InstanceSettingsRepository>,
        catalog: Arc<dyn CatalogRepository>,
    ) -> Self {
        self.settings = Some(settings);
        self.catalog = Some(catalog);
        self
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

    /// Creates or updates contract-owned profile fields without changing consent.
    pub async fn upsert_from_contract(
        &self,
        identity: &UserIdentity,
        display_name: Option<&str>,
    ) -> CanopyResult<UserProfile> {
        let existing = self
            .profiles
            .get_by_external_user_id(&identity.user_id)
            .await?;
        let history_enabled = existing
            .as_ref()
            .is_some_and(|profile| profile.history_enabled);
        let display_name = display_name
            .map(str::trim)
            .filter(|display_name| !display_name.is_empty());
        self.profiles
            .upsert_profile(&identity.user_id, display_name, history_enabled)
            .await
    }

    /// Returns the authenticated user's existing profile.
    pub async fn get_profile(&self, identity: &UserIdentity) -> CanopyResult<UserProfile> {
        self.profiles
            .get_by_external_user_id(&identity.user_id)
            .await?
            .ok_or_else(|| CanopyError::unauthenticated("profile not found"))
    }

    /// Updates history consent and returns the number of events purged.
    pub async fn set_history_enabled(
        &self,
        identity: &UserIdentity,
        enabled: bool,
    ) -> CanopyResult<(UserProfile, u64)> {
        let profile = self.get_profile(identity).await?;
        let profile = self
            .profiles
            .upsert_profile(&identity.user_id, profile.display_name.as_deref(), enabled)
            .await?;
        let deleted_count = if enabled {
            0
        } else {
            self.history.clear(&profile.id).await?
        };
        Ok((profile, deleted_count))
    }

    /// Deletes an ordinary profile after enforcing protected ownership rules.
    pub async fn delete_profile(&self, identity: &UserIdentity) -> CanopyResult<()> {
        let profile = self.get_profile(identity).await?;
        let settings = self.settings.as_ref().ok_or_else(|| {
            CanopyError::Internal("profile deletion policy is not configured".into())
        })?;
        let catalog = self.catalog.as_ref().ok_or_else(|| {
            CanopyError::Internal("profile deletion policy is not configured".into())
        })?;

        if settings.owner_profile_id().await?.as_deref() == Some(profile.id.as_str()) {
            return Err(CanopyError::FailedPrecondition(
                "instance owner profile cannot be deleted".into(),
            ));
        }
        if !catalog
            .list_personal(
                &profile.id,
                Page {
                    limit: 1,
                    offset: 0,
                },
            )
            .await?
            .items
            .is_empty()
        {
            return Err(CanopyError::FailedPrecondition(
                "profile owns personal media".into(),
            ));
        }

        self.profiles
            .delete_by_external_user_id(&identity.user_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::{
        InMemoryCatalog, InMemoryCatalogEntry, InMemoryInstanceSettingsStore,
        InMemoryPlaybackHistoryStore, InMemoryProfileStore,
    };
    use canopy_core::{
        IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility, PlaybackHistoryEvent,
        PlaybackHistoryRepository,
    };
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

    #[tokio::test]
    async fn contract_upsert_preserves_existing_history_consent() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let service =
            ProfileService::new(profiles, Arc::new(InMemoryPlaybackHistoryStore::default()));
        let identity = UserIdentity {
            user_id: "user-123".into(),
        };
        service
            .upsert_profile(&identity, "Ada", true)
            .await
            .unwrap();

        let profile = service
            .upsert_from_contract(&identity, Some("Ada Lovelace"))
            .await
            .unwrap();

        assert!(profile.history_enabled);
    }

    #[tokio::test]
    async fn disabling_history_reports_purged_event_count() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = ProfileService::new(profiles, history.clone());
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

        let (_, deleted_count) = service.set_history_enabled(&identity, false).await.unwrap();

        assert_eq!(deleted_count, 1);
    }

    #[tokio::test]
    async fn deleting_instance_owner_fails_precondition() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        let service =
            ProfileService::new(profiles, Arc::new(InMemoryPlaybackHistoryStore::default()))
                .with_deletion_policy(settings.clone(), Arc::new(InMemoryCatalog::default()));
        let identity = UserIdentity {
            user_id: "owner-user".into(),
        };
        let profile = service
            .upsert_from_contract(&identity, Some("Owner"))
            .await
            .unwrap();
        settings.set_owner_profile_id(&profile.id).await.unwrap();

        let error = service.delete_profile(&identity).await.unwrap_err();

        assert!(matches!(error, CanopyError::FailedPrecondition(_)));
    }

    #[tokio::test]
    async fn deleting_profile_with_personal_media_fails_precondition() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let identity = UserIdentity {
            user_id: "media-owner".into(),
        };
        let profile = ProfileService::new(profiles.clone(), history.clone())
            .upsert_from_contract(&identity, Some("Media Owner"))
            .await
            .unwrap();
        let catalog = InMemoryCatalog::from_entries(vec![InMemoryCatalogEntry {
            item: MediaItem {
                id: "personal-track".into(),
                ..MediaItem::default()
            },
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some(profile.id),
        }]);
        let service = ProfileService::new(profiles, history).with_deletion_policy(
            Arc::new(InMemoryInstanceSettingsStore::default()),
            Arc::new(catalog),
        );

        let error = service.delete_profile(&identity).await.unwrap_err();

        assert!(matches!(error, CanopyError::FailedPrecondition(_)));
    }

    #[tokio::test]
    async fn deleting_ordinary_profile_removes_it() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let service =
            ProfileService::new(profiles, Arc::new(InMemoryPlaybackHistoryStore::default()))
                .with_deletion_policy(
                    Arc::new(InMemoryInstanceSettingsStore::default()),
                    Arc::new(InMemoryCatalog::default()),
                );
        let identity = UserIdentity {
            user_id: "ordinary-user".into(),
        };
        service
            .upsert_from_contract(&identity, Some("Listener"))
            .await
            .unwrap();

        service.delete_profile(&identity).await.unwrap();

        assert!(matches!(
            service.get_profile(&identity).await,
            Err(CanopyError::Unauthenticated(_))
        ));
    }
}
