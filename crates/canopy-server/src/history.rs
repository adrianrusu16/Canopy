//! Logged-in playback history service.
//!
//! Anonymous sessions are not durable users. This service records history only
//! after authenticated profile lookup and `history_enabled` consent.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, Page, PlaybackHistoryEvent, PlaybackHistoryPage,
    PlaybackHistoryRepository, ProfileRepository, UserIdentity, UserProfile,
};

use crate::principal::PrincipalService;

/// Application service for profile-scoped playback history.
#[derive(Clone)]
pub struct HistoryService {
    profiles: Arc<dyn ProfileRepository>,
    history: Arc<dyn PlaybackHistoryRepository>,
    principal: PrincipalService,
}

impl HistoryService {
    /// Creates a history service over profile storage and history storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        history: Arc<dyn PlaybackHistoryRepository>,
        principal: PrincipalService,
    ) -> Self {
        Self {
            profiles,
            history,
            principal,
        }
    }

    /// Records a playback event when the real logged-in profile has opted in.
    pub async fn record_playback(
        &self,
        identity: &UserIdentity,
        track_id: &str,
        duration_ms: i64,
        completion_pct: f32,
    ) -> CanopyResult<bool> {
        let track_id = track_id.trim();
        if track_id.is_empty() {
            return Err(CanopyError::InvalidArgument("track_id is required".into()));
        }
        if duration_ms < 0 {
            return Err(CanopyError::InvalidArgument(
                "duration_ms must be non-negative".into(),
            ));
        }
        if !(0.0..=1.0).contains(&completion_pct) {
            return Err(CanopyError::InvalidArgument(
                "completion_pct must be between 0.0 and 1.0".into(),
            ));
        }

        let profile = self.profile(identity).await?;
        if !profile.history_enabled {
            return Ok(false);
        }
        let scope = self.principal.track_scope_for_profile(&profile.id).await?;

        self.history
            .record(
                PlaybackHistoryEvent {
                    profile_id: profile.id,
                    track_id: track_id.to_string(),
                    duration_ms,
                    completion_pct,
                },
                &scope,
            )
            .await
    }

    /// Lists playback events for the authenticated profile.
    pub async fn list_history(
        &self,
        identity: &UserIdentity,
        page: Page,
    ) -> CanopyResult<PlaybackHistoryPage> {
        let profile = self.profile(identity).await?;
        let scope = self.principal.track_scope_for_profile(&profile.id).await?;
        self.history
            .list(&profile.id, &scope, normalize_history_page(page))
            .await
    }

    /// Deletes one playback event owned by the authenticated profile.
    pub async fn delete_entry(
        &self,
        identity: &UserIdentity,
        history_id: &str,
    ) -> CanopyResult<bool> {
        let history_id = validate_history_id(history_id)?;
        let profile = self.profile(identity).await?;
        self.history.delete_entry(&profile.id, history_id).await
    }

    /// Deletes every playback event owned by the authenticated profile.
    pub async fn clear_history(&self, identity: &UserIdentity) -> CanopyResult<u64> {
        let profile = self.profile(identity).await?;
        self.history.clear(&profile.id).await
    }

    async fn profile(&self, identity: &UserIdentity) -> CanopyResult<UserProfile> {
        self.profiles
            .get_by_external_user_id(&identity.user_id)
            .await?
            .ok_or_else(|| CanopyError::unauthenticated("profile not found"))
    }
}

fn validate_history_id(history_id: &str) -> CanopyResult<&str> {
    let history_id = history_id.trim();
    if history_id.is_empty() {
        return Err(CanopyError::InvalidArgument(
            "history_id is required".into(),
        ));
    }
    uuid::Uuid::parse_str(history_id)
        .map_err(|_| CanopyError::InvalidArgument("history_id must be a UUID".into()))?;
    Ok(history_id)
}

fn normalize_history_page(page: Page) -> Page {
    Page {
        limit: if page.limit == 0 {
            50
        } else {
            page.limit.min(100)
        },
        offset: page.offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::{
        InMemoryCatalog, InMemoryCatalogEntry, InMemoryInstanceSettingsStore,
        InMemoryPlaybackHistoryStore, InMemoryProfileStore,
    };
    use canopy_core::{IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility};
    use std::sync::Arc;

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(
        profiles: Arc<InMemoryProfileStore>,
        history: Arc<InMemoryPlaybackHistoryStore>,
    ) -> HistoryService {
        let principal = PrincipalService::new(
            profiles.clone(),
            Arc::new(InMemoryInstanceSettingsStore::default()),
        );
        HistoryService::new(profiles, history, principal)
    }

    #[tokio::test]
    async fn record_playback_rejects_missing_profile() {
        let service = service(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaybackHistoryStore::default()),
        );

        let err = service
            .record_playback(&identity(), "track-1", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn record_playback_returns_false_when_history_disabled() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), false)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = service(profiles, history.clone());

        let recorded = service
            .record_playback(&identity(), "track-1", 1000, 0.5)
            .await
            .unwrap();

        assert!(!recorded);
        assert!(history.events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn record_playback_persists_when_history_enabled() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let profile = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = service(profiles, history.clone());

        let recorded = service
            .record_playback(&identity(), "track-1", 1000, 0.5)
            .await
            .unwrap();

        assert!(recorded);
        assert_eq!(
            history.events().unwrap(),
            vec![PlaybackHistoryEvent {
                profile_id: profile.id,
                track_id: "track-1".into(),
                duration_ms: 1000,
                completion_pct: 0.5,
            }]
        );
    }

    #[tokio::test]
    async fn list_delete_and_clear_history_are_profile_scoped() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        profiles
            .upsert_profile("other-user", Some("Grace"), true)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = service(profiles, history);

        service
            .record_playback(&identity(), "track-1", 1000, 0.5)
            .await
            .unwrap();
        service
            .record_playback(&identity(), "track-1", 500, 0.25)
            .await
            .unwrap();

        let page = service
            .list_history(
                &identity(),
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.total_count, 2);
        assert_eq!(page.entries.len(), 2);
        assert!(page.entries[0].played_at_epoch_ms >= page.entries[1].played_at_epoch_ms);

        let other_identity = UserIdentity {
            user_id: "other-user".into(),
        };
        assert!(
            !service
                .delete_entry(&other_identity, &page.entries[0].id)
                .await
                .unwrap()
        );
        assert!(
            service
                .delete_entry(&identity(), &page.entries[0].id)
                .await
                .unwrap()
        );
        assert_eq!(service.clear_history(&identity()).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn delete_history_rejects_empty_id() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryPlaybackHistoryStore::default()));

        let err = service.delete_entry(&identity(), " ").await.unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[test]
    fn history_page_defaults_and_clamps_limit() {
        let defaulted = normalize_history_page(Page {
            limit: 0,
            offset: 7,
        });
        assert_eq!(defaulted.limit, 50);
        assert_eq!(defaulted.offset, 7);

        let clamped = normalize_history_page(Page {
            limit: 500,
            offset: 7,
        });
        assert_eq!(clamped.limit, 100);
        assert_eq!(clamped.offset, 7);
    }

    #[tokio::test]
    async fn record_playback_rejects_invalid_playback_facts() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryPlaybackHistoryStore::default()));

        let err = service
            .record_playback(&identity(), " ", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn ownership_transfer_hides_personal_history_but_allows_deletion() {
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
        let history = Arc::new(InMemoryPlaybackHistoryStore::new(catalog));
        let principal = PrincipalService::new(profiles.clone(), settings.clone());
        let service = HistoryService::new(profiles, history, principal);

        service
            .record_playback(&identity(), "personal-track", 1000, 1.0)
            .await
            .unwrap();
        let visible = service
            .list_history(
                &identity(),
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        let history_id = visible.entries[0].id.clone();
        settings
            .set_owner_profile_id(&replacement.id)
            .await
            .unwrap();

        assert_eq!(
            service
                .list_history(
                    &identity(),
                    Page {
                        limit: 10,
                        offset: 0
                    }
                )
                .await
                .unwrap()
                .total_count,
            0
        );
        assert!(matches!(
            service
                .record_playback(&identity(), "personal-track", 1000, 1.0)
                .await,
            Err(CanopyError::NotFound { .. })
        ));

        assert!(
            service
                .delete_entry(&identity(), &history_id)
                .await
                .unwrap()
        );
        settings.set_owner_profile_id(&owner.id).await.unwrap();
        assert_eq!(
            service
                .list_history(
                    &identity(),
                    Page {
                        limit: 10,
                        offset: 0
                    }
                )
                .await
                .unwrap()
                .total_count,
            0
        );
    }
}
