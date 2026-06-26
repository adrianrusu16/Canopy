//! Logged-in profile preferences service.
//!
//! Preferences are profile-owned durable state. Anonymous preferences remain a
//! client-side responsibility.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, PreferencesRepository, ProfilePreferences, ProfileRepository,
    UserIdentity,
};

/// Application service for profile-owned preferences.
#[derive(Clone)]
pub struct PreferencesService {
    profiles: Arc<dyn ProfileRepository>,
    preferences: Arc<dyn PreferencesRepository>,
}

impl PreferencesService {
    /// Creates a preferences service over profile storage and preferences storage.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        preferences: Arc<dyn PreferencesRepository>,
    ) -> Self {
        Self {
            profiles,
            preferences,
        }
    }

    /// Returns preferences for the authenticated profile.
    pub async fn get_preferences(
        &self,
        identity: &UserIdentity,
    ) -> CanopyResult<ProfilePreferences> {
        let profile_id = self.profile_id(identity).await?;
        self.preferences.get_preferences(&profile_id).await
    }

    /// Replaces preferences for the authenticated profile with a JSON object.
    pub async fn update_preferences(
        &self,
        identity: &UserIdentity,
        values_json: &str,
    ) -> CanopyResult<ProfilePreferences> {
        let profile_id = self.profile_id(identity).await?;
        let values_json = canonical_json_object(values_json)?;
        self.preferences
            .upsert_preferences(&profile_id, &values_json)
            .await
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

fn canonical_json_object(values_json: &str) -> CanopyResult<String> {
    let value: serde_json::Value = serde_json::from_str(values_json)
        .map_err(|e| CanopyError::InvalidArgument(format!("invalid preferences_json: {e}")))?;
    if !value.is_object() {
        return Err(CanopyError::InvalidArgument(
            "preferences_json must be a JSON object".into(),
        ));
    }
    serde_json::to_string(&value)
        .map_err(|e| CanopyError::InvalidArgument(format!("invalid preferences_json: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jade_store::{InMemoryPreferencesStore, InMemoryProfileStore};

    fn identity() -> UserIdentity {
        UserIdentity {
            user_id: "user-123".into(),
        }
    }

    fn service(
        profiles: Arc<InMemoryProfileStore>,
        preferences: Arc<InMemoryPreferencesStore>,
    ) -> PreferencesService {
        PreferencesService::new(profiles, preferences)
    }

    #[tokio::test]
    async fn get_preferences_rejects_missing_profile() {
        let service = service(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPreferencesStore::default()),
        );

        let err = service.get_preferences(&identity()).await.unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn update_preferences_rejects_malformed_json() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryPreferencesStore::default()));

        let err = service
            .update_preferences(&identity(), "not-json")
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn update_preferences_rejects_non_object_json() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryPreferencesStore::default()));

        let err = service
            .update_preferences(&identity(), "[]")
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }

    #[tokio::test]
    async fn update_and_get_preferences_round_trip_canonical_json() {
        let profiles = Arc::new(InMemoryProfileStore::default());
        let profile = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = service(profiles, Arc::new(InMemoryPreferencesStore::default()));

        let saved = service
            .update_preferences(
                &identity(),
                r#"{"explicit_content": false, "preferred_codecs": ["opus", "mp4"]}"#,
            )
            .await
            .unwrap();
        let fetched = service.get_preferences(&identity()).await.unwrap();

        assert_eq!(saved.profile_id, profile.id);
        assert_eq!(saved.values_json, fetched.values_json);
        assert_eq!(
            fetched.values_json,
            r#"{"explicit_content":false,"preferred_codecs":["opus","mp4"]}"#
        );
    }
}
