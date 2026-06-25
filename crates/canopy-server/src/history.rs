//! Logged-in playback history service.
//!
//! Anonymous sessions are not durable users. This service records history only
//! after token verification, profile lookup, and `history_enabled` consent.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, PlaybackHistoryEvent, PlaybackHistoryRepository, ProfileRepository,
};

use crate::auth::AuthService;

/// Application service for profile-scoped playback history.
#[derive(Clone)]
pub struct HistoryService {
    profiles: Arc<dyn ProfileRepository>,
    history: Arc<dyn PlaybackHistoryRepository>,
    auth: AuthService,
}

impl HistoryService {
    /// Creates a history service over profile storage, history storage, and auth.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        history: Arc<dyn PlaybackHistoryRepository>,
        auth: AuthService,
    ) -> Self {
        Self {
            profiles,
            history,
            auth,
        }
    }

    /// Records a playback event when the real logged-in profile has opted in.
    pub async fn record_playback(
        &self,
        auth_token: &str,
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

        let identity = self.auth.verify(auth_token)?;
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
    use crate::auth::AuthService;
    use crate::jade_store::{InMemoryPlaybackHistoryStore, InMemoryProfileStore};
    use std::sync::Arc;
    use std::time::Duration;

    fn token(auth: &AuthService) -> String {
        auth.mint("user-123", Duration::from_secs(3600)).unwrap()
    }

    #[tokio::test]
    async fn record_playback_rejects_invalid_token() {
        let auth = AuthService::new("secret");
        let service = HistoryService::new(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaybackHistoryStore::default()),
            auth,
        );

        let err = service
            .record_playback("bad-token", "track-1", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn record_playback_rejects_missing_profile() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let service = HistoryService::new(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaybackHistoryStore::default()),
            auth,
        );

        let err = service
            .record_playback(&auth_token, "track-1", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn record_playback_returns_false_when_history_disabled() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), false)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = HistoryService::new(profiles, history.clone(), auth);

        let recorded = service
            .record_playback(&auth_token, "track-1", 1000, 0.5)
            .await
            .unwrap();

        assert!(!recorded);
        assert!(history.events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn record_playback_persists_when_history_enabled() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let profiles = Arc::new(InMemoryProfileStore::default());
        let profile = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = HistoryService::new(profiles, history.clone(), auth);

        let recorded = service
            .record_playback(&auth_token, "track-1", 1000, 0.5)
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
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = HistoryService::new(
            profiles,
            Arc::new(InMemoryPlaybackHistoryStore::default()),
            auth,
        );

        let err = service
            .record_playback(&auth_token, " ", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }
}
