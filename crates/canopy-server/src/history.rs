//! Logged-in playback history service.
//!
//! Anonymous sessions are not durable users. This service records history only
//! after authenticated profile lookup and `history_enabled` consent.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, PlaybackHistoryEvent, PlaybackHistoryRepository, ProfileRepository,
    UserIdentity,
};

/// Application service for profile-scoped playback history.
#[derive(Clone)]
pub struct HistoryService {
    profiles: Arc<dyn ProfileRepository>,
    history: Arc<dyn PlaybackHistoryRepository>,
}

impl HistoryService {
    /// Creates a history service over profile storage and history storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        history: Arc<dyn PlaybackHistoryRepository>,
    ) -> Self {
        Self { profiles, history }
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

        let profile = self
            .profiles
            .get_by_external_user_id(&identity.user_id)
            .await?
            .ok_or_else(|| CanopyError::unauthenticated("profile not found"))?;

        if !profile.history_enabled {
            return Ok(false);
        }

        self.history
            .record(PlaybackHistoryEvent {
                profile_id: profile.id,
                track_id: track_id.to_string(),
                duration_ms,
                completion_pct,
            })
            .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::{InMemoryPlaybackHistoryStore, InMemoryProfileStore};
    use std::sync::Arc;

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    #[tokio::test]
    async fn record_playback_rejects_missing_profile() {
        let service = HistoryService::new(
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
        let service = HistoryService::new(profiles, history.clone());

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
        let service = HistoryService::new(profiles, history.clone());

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
    async fn record_playback_rejects_invalid_playback_facts() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service =
            HistoryService::new(profiles, Arc::new(InMemoryPlaybackHistoryStore::default()));

        let err = service
            .record_playback(&identity(), " ", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }
}
