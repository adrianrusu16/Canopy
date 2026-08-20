//! Playback / session service.
//!
//! Manages lightweight playback sessions and resolves playable sources. The
//! [`ResolverService`] selects a public-ready asset and returns a short-lived
//! Canopy capability. Nginx delegates authorization back to Canopy and remains
//! responsible for the byte-serving path.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canopy_core::{
    CanopyError, CanopyResult, PlayableAsset, PlayableAssetRepository, PlaybackSource,
    StreamAudience, TrackAccessScope,
};

use crate::stream::StreamTokenCodec;

/// Configuration for the playback resolver.
#[derive(Clone, Debug)]
pub struct ResolverConfig {
    /// Public Nginx base URL, without a trailing slash.
    pub public_base_url: String,
    /// How long a minted capability remains valid.
    pub token_ttl: Duration,
    /// Codec short names in preference order; the first available wins.
    pub codec_preference: Vec<String>,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            public_base_url: "https://media.pandawave.internal".to_string(),
            token_ttl: Duration::from_secs(10 * 60),
            codec_preference: vec!["opus".into(), "mp3".into(), "flac".into()],
        }
    }
}

/// Playback resolver: turns a track identifier into a short-lived
/// [`PlaybackSource`] capability.
///
/// The resolver chooses the preferred codec among public-ready assets and signs
/// only the asset identity, audience, expiry, version, and nonce. Storage keys
/// remain behind the private authorization boundary.
#[derive(Clone)]
pub struct ResolverService {
    assets: Arc<dyn PlayableAssetRepository>,
    tokens: Arc<StreamTokenCodec>,
    config: ResolverConfig,
}

impl ResolverService {
    /// Creates a resolver over current playback policy and capability signing.
    pub fn new(
        assets: Arc<dyn PlayableAssetRepository>,
        tokens: Arc<StreamTokenCodec>,
        config: ResolverConfig,
    ) -> Self {
        Self {
            assets,
            tokens,
            config,
        }
    }

    /// Resolves a track for an anonymous caller using the current time.
    pub async fn resolve(&self, track_id: &str) -> CanopyResult<PlaybackSource> {
        self.resolve_at(&TrackAccessScope::Public, track_id, now_epoch_ms())
            .await
    }

    /// Resolves a track for the access scope using an explicit epoch time.
    pub async fn resolve_at(
        &self,
        scope: &TrackAccessScope,
        track_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<PlaybackSource> {
        let (assets, audience) = match scope {
            TrackAccessScope::Owner { profile_id } => {
                let personal = self
                    .assets
                    .assets_for_personal_playback(profile_id, track_id)
                    .await?;
                if personal.is_empty() {
                    (
                        self.assets.assets_for_public_playback(track_id).await?,
                        StreamAudience::Public,
                    )
                } else {
                    (personal, StreamAudience::Personal)
                }
            }
            TrackAccessScope::Public => (
                self.assets.assets_for_public_playback(track_id).await?,
                StreamAudience::Public,
            ),
        };
        let asset = self
            .select_asset(assets)
            .ok_or_else(|| CanopyError::not_found("audio_asset", track_id))?;

        self.playback_source(asset, audience, now_epoch_ms)
    }

    fn playback_source(
        &self,
        asset: PlayableAsset,
        audience: StreamAudience,
        now_epoch_ms: u64,
    ) -> CanopyResult<PlaybackSource> {
        let ttl_ms = u64::try_from(self.config.token_ttl.as_millis())
            .map_err(|_| CanopyError::Internal("stream token TTL is too large".into()))?;
        let expires_at_epoch_ms = now_epoch_ms
            .checked_add(ttl_ms)
            .ok_or_else(|| CanopyError::Internal("stream token expiry overflow".into()))?;
        let token = self
            .tokens
            .mint(&asset.asset_id, audience, expires_at_epoch_ms)?;
        let stream_url = format!(
            "{}/stream/{token}",
            self.config.public_base_url.trim_end_matches('/')
        );

        Ok(PlaybackSource {
            track_id: asset.track_id,
            stream_url,
            content_type: asset.content_type,
            codec: asset.codec,
            duration_ms: asset.duration_ms,
            expires_at_epoch_ms,
        })
    }
    /// Picks the most preferred available asset, falling back to the first one
    /// when none match the preference list (so an ingested-but-unranked codec
    /// is still playable).
    fn select_asset(&self, assets: Vec<PlayableAsset>) -> Option<PlayableAsset> {
        if assets.is_empty() {
            return None;
        }
        for codec in &self.config.codec_preference {
            if let Some(found) = assets.iter().find(|a| &a.codec == codec) {
                return Some(found.clone());
            }
        }
        assets.into_iter().next()
    }
}

/// Current wall-clock time in epoch milliseconds.
fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::stream::StreamTokenCodec;
    use async_trait::async_trait;
    use canopy_core::{
        AuthorizedStreamAsset, PlayableAsset, PlayableAssetRepository, StreamAudience,
    };

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    const ASSET_ID: &str = "018f0000-0000-7000-8000-000000000001";

    struct FakePlayableAssets {
        public_assets: Vec<PlayableAsset>,
        personal_assets: Vec<PlayableAsset>,
        personal_error: bool,
    }

    #[async_trait]
    impl PlayableAssetRepository for FakePlayableAssets {
        async fn assets_for_personal_playback(
            &self,
            _owner_profile_id: &str,
            track_id: &str,
        ) -> CanopyResult<Vec<PlayableAsset>> {
            if self.personal_error {
                return Err(CanopyError::Storage("personal lookup failed".into()));
            }
            Ok(self
                .personal_assets
                .iter()
                .filter(|asset| asset.track_id == track_id)
                .cloned()
                .collect())
        }

        async fn assets_for_public_playback(
            &self,
            track_id: &str,
        ) -> CanopyResult<Vec<PlayableAsset>> {
            Ok(self
                .public_assets
                .iter()
                .filter(|asset| asset.track_id == track_id)
                .cloned()
                .collect())
        }

        async fn authorize_stream_asset(
            &self,
            _asset_id: &str,
            _audience: StreamAudience,
        ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
            Ok(None)
        }
    }

    fn asset(track: &str, codec: &str, _legacy_key: &str) -> PlayableAsset {
        PlayableAsset {
            asset_id: ASSET_ID.into(),
            track_id: track.into(),
            codec: codec.into(),
            content_type: format!("audio/{codec}"),
            duration_ms: 245_000,
        }
    }

    fn resolver(assets: Vec<PlayableAsset>) -> ResolverService {
        resolver_with_assets(assets, Vec::new(), false)
    }

    fn resolver_with_assets(
        public_assets: Vec<PlayableAsset>,
        personal_assets: Vec<PlayableAsset>,
        personal_error: bool,
    ) -> ResolverService {
        ResolverService::new(
            Arc::new(FakePlayableAssets {
                public_assets,
                personal_assets,
                personal_error,
            }),
            Arc::new(StreamTokenCodec::new(SECRET).unwrap()),
            ResolverConfig {
                public_base_url: "https://media.test".into(),
                token_ttl: Duration::from_secs(600),
                codec_preference: vec!["opus".into(), "mp3".into(), "flac".into()],
            },
        )
    }

    #[tokio::test]
    async fn owner_prefers_personal_asset_and_mints_personal_capability() {
        let resolver = resolver_with_assets(
            vec![asset("trk_1", "mp3", "public.mp3")],
            vec![asset("trk_1", "opus", "personal.opus")],
            false,
        );

        let source = resolver
            .resolve_at(
                &TrackAccessScope::Owner {
                    profile_id: "owner-a".into(),
                },
                "trk_1",
                1_000,
            )
            .await
            .unwrap();
        let token = source
            .stream_url
            .strip_prefix("https://media.test/stream/")
            .unwrap();
        let claims = StreamTokenCodec::new(SECRET)
            .unwrap()
            .verify(token, 1_000)
            .unwrap();

        assert_eq!(source.codec, "opus");
        assert_eq!(claims.audience, StreamAudience::Personal);
    }

    #[tokio::test]
    async fn owner_falls_back_to_public_capability() {
        let resolver =
            resolver_with_assets(vec![asset("trk_1", "mp3", "public.mp3")], Vec::new(), false);

        let source = resolver
            .resolve_at(
                &TrackAccessScope::Owner {
                    profile_id: "owner-a".into(),
                },
                "trk_1",
                1_000,
            )
            .await
            .unwrap();
        let token = source
            .stream_url
            .strip_prefix("https://media.test/stream/")
            .unwrap();
        let claims = StreamTokenCodec::new(SECRET)
            .unwrap()
            .verify(token, 1_000)
            .unwrap();

        assert_eq!(claims.audience, StreamAudience::Public);
    }

    #[tokio::test]
    async fn anonymous_cannot_resolve_personal_only_media() {
        let resolver = resolver_with_assets(
            Vec::new(),
            vec![asset("trk_1", "mp3", "personal.mp3")],
            false,
        );

        let error = resolver
            .resolve_at(&TrackAccessScope::Public, "trk_1", 1_000)
            .await
            .unwrap_err();

        assert!(matches!(error, CanopyError::NotFound { entity, .. } if entity == "audio_asset"));
    }

    #[tokio::test]
    async fn personal_lookup_failure_does_not_fall_back_to_public() {
        let resolver =
            resolver_with_assets(vec![asset("trk_1", "mp3", "public.mp3")], Vec::new(), true);

        let error = resolver
            .resolve_at(
                &TrackAccessScope::Owner {
                    profile_id: "owner-a".into(),
                },
                "trk_1",
                1_000,
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, CanopyError::Storage(message) if message == "personal lookup failed")
        );
    }

    #[tokio::test]
    async fn resolve_prefers_higher_ranked_codec() {
        let resolver = resolver(vec![
            asset("trk_1", "mp3", "audio/tracks/trk_1.mp3"),
            asset("trk_1", "opus", "audio/tracks/trk_1.opus"),
        ]);

        let source = resolver
            .resolve_at(&TrackAccessScope::Public, "trk_1", 1_000_000)
            .await
            .unwrap();
        // `opus` outranks `mp3` in the default preference list.
        assert_eq!(source.codec, "opus");
        assert_eq!(source.content_type, "audio/opus");
        assert_eq!(source.track_id, "trk_1");
    }

    #[tokio::test]
    async fn resolve_returns_opaque_public_capability_and_expiry() {
        let resolver = resolver(vec![asset("trk_1", "mp3", "audio/tracks/trk_1.mp3")]);

        let now = 1_000_000;
        let source = resolver
            .resolve_at(&TrackAccessScope::Public, "trk_1", now)
            .await
            .unwrap();

        assert_eq!(source.expires_at_epoch_ms, now + 600_000);
        let token = source
            .stream_url
            .strip_prefix("https://media.test/stream/")
            .unwrap();
        let claims = StreamTokenCodec::new(SECRET)
            .unwrap()
            .verify(token, now)
            .unwrap();
        assert_eq!(claims.asset_id, ASSET_ID);
        assert_eq!(claims.audience, StreamAudience::Public);
        assert!(!source.stream_url.contains("audio/tracks"));
    }

    #[tokio::test]
    async fn resolve_falls_back_when_no_preferred_codec() {
        let resolver = resolver(vec![asset("trk_1", "wav", "audio/tracks/trk_1.wav")]);
        let source = resolver
            .resolve_at(&TrackAccessScope::Public, "trk_1", 0)
            .await
            .unwrap();
        assert_eq!(source.codec, "wav");
    }

    #[tokio::test]
    async fn resolve_missing_track_is_not_found() {
        let resolver = resolver(vec![]);
        let err = resolver
            .resolve_at(&TrackAccessScope::Public, "missing", 0)
            .await
            .unwrap_err();
        assert!(matches!(err, CanopyError::NotFound { entity, .. } if entity == "audio_asset"));
    }
}
