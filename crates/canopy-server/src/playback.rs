//! Playback / session service.
//!
//! Manages lightweight playback sessions and resolves playable sources. The
//! [`ResolverService`] implements the architecture's playback resolver: it
//! selects an audio asset for a track and returns a presigned, time-limited
//! URL the player streams directly from object storage, keeping Canopy out of
//! the byte-serving path.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyError, CanopyResult, PlaybackSource, Session,
    SessionRepository, UrlSigner,
};

/// Application service for session state.
#[derive(Clone)]
pub struct PlaybackService {
    sessions: Arc<dyn SessionRepository>,
}

impl PlaybackService {
    /// Session used by older clients that do not send an explicit session ID.
    pub const DEFAULT_SESSION_ID: &'static str = "default";

    /// Creates a new service over the given session repository.
    pub fn new(sessions: Arc<dyn SessionRepository>) -> Self {
        Self { sessions }
    }

    /// Resolves an optional wire session ID into a concrete session key.
    pub fn resolve_session_id(id: &str) -> &str {
        let id = id.trim();
        if id.is_empty() {
            Self::DEFAULT_SESSION_ID
        } else {
            id
        }
    }

    /// Fetches a session, returning a default (empty) session when unknown so
    /// the wire contract can always answer a `GetSession` call.
    pub async fn get_session(&self, id: &str) -> CanopyResult<Session> {
        let id = Self::resolve_session_id(id);
        Ok(self.sessions.get(id).await?.unwrap_or_else(|| Session {
            id: id.to_string(),
            ..Session::default()
        }))
    }

    /// Starts playback in a session, creating it if necessary.
    pub async fn play(
        &self,
        id: &str,
        media_id: String,
        start_pos_ms: i64,
    ) -> CanopyResult<String> {
        if media_id.trim().is_empty() {
            return Err(CanopyError::InvalidArgument(
                "media_id is required".to_string(),
            ));
        }
        if start_pos_ms < 0 {
            return Err(CanopyError::InvalidArgument(
                "start_pos_ms must be non-negative".to_string(),
            ));
        }

        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        session.current_media_id = Some(media_id);
        session.position_ms = start_pos_ms;
        session.is_playing = true;
        self.sessions.update(session).await?;
        Ok(session_id)
    }

    /// Pauses playback without clearing the loaded media.
    pub async fn pause(&self, id: &str) -> CanopyResult<()> {
        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        session.is_playing = false;
        self.sessions.update(session).await
    }

    /// Updates the playback position.
    pub async fn seek(&self, id: &str, position_ms: i64) -> CanopyResult<()> {
        if position_ms < 0 {
            return Err(CanopyError::InvalidArgument(
                "position_ms must be non-negative".to_string(),
            ));
        }

        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        session.position_ms = position_ms;
        self.sessions.update(session).await
    }

    /// Updates playback speed.
    pub async fn set_playback_speed(&self, id: &str, speed: f64) -> CanopyResult<()> {
        if !(0.25..=4.0).contains(&speed) {
            return Err(CanopyError::InvalidArgument(
                "playback speed must be between 0.25 and 4.0".to_string(),
            ));
        }

        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        session.playback_speed = speed;
        self.sessions.update(session).await
    }

    /// Stops playback and resets the current position, keeping the loaded media.
    pub async fn stop(&self, id: &str) -> CanopyResult<()> {
        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        session.is_playing = false;
        session.position_ms = 0;
        self.sessions.update(session).await
    }

    /// Loads media into a session without starting playback.
    pub async fn load_media(&self, id: &str, media_id: String) -> CanopyResult<String> {
        if media_id.trim().is_empty() {
            return Err(CanopyError::InvalidArgument(
                "media_id is required".to_string(),
            ));
        }

        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        if session.current_media_id.as_deref() != Some(media_id.as_str()) {
            session.position_ms = 0;
        }
        session.current_media_id = Some(media_id);
        self.sessions.update(session).await?;
        Ok(session_id)
    }

    /// Applies a partial update to a session, creating it if necessary.
    pub async fn update_session(
        &self,
        id: &str,
        media_id: Option<String>,
        position_ms: Option<i64>,
    ) -> CanopyResult<()> {
        let session_id = Self::resolve_session_id(id).to_string();
        let mut session = self.load_or_default(&session_id).await?;
        if let Some(media_id) = media_id {
            session.current_media_id = Some(media_id);
        }
        if let Some(position_ms) = position_ms {
            if position_ms < 0 {
                return Err(CanopyError::InvalidArgument(
                    "position_ms must be non-negative".to_string(),
                ));
            }
            session.position_ms = position_ms;
        }
        self.sessions.update(session).await
    }

    /// Ends (removes) a session.
    pub async fn end_session(&self, id: &str) -> CanopyResult<()> {
        self.sessions.delete(Self::resolve_session_id(id)).await
    }

    async fn load_or_default(&self, id: &str) -> CanopyResult<Session> {
        Ok(self.sessions.get(id).await?.unwrap_or_else(|| Session {
            id: id.to_string(),
            ..Session::default()
        }))
    }
}

/// Configuration for the playback resolver.
#[derive(Clone, Debug)]
pub struct ResolverConfig {
    /// Base URL of the object store, without a trailing slash
    /// (e.g. `https://rustfs.pandawave.internal`).
    pub base_url: String,
    /// Media bucket name (e.g. `pandawave-media`).
    pub bucket: String,
    /// How long a minted URL remains valid.
    pub url_ttl: Duration,
    /// Codec short names in preference order; the first available wins.
    pub codec_preference: Vec<String>,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            base_url: "https://rustfs.pandawave.internal".to_string(),
            bucket: "pandawave-media".to_string(),
            url_ttl: Duration::from_secs(15 * 60),
            // Lossy-but-small first today; only `mp3` is ingested so far, the
            // rest anticipate future codecs without changing this logic.
            codec_preference: vec!["opus".into(), "mp3".into(), "flac".into()],
        }
    }
}

/// Mints a playable URL for an object-storage asset.
#[async_trait::async_trait]
pub trait PlaybackUrlProvider: Send + Sync {
    /// Returns a stream URL valid until `expires_at_epoch_ms`.
    async fn signed_url(&self, object_key: &str, expires_at_epoch_ms: u64) -> CanopyResult<String>;
}

struct RustfsUrlProvider {
    signer: Arc<dyn UrlSigner>,
    base_url: String,
    bucket: String,
}

#[async_trait::async_trait]
impl PlaybackUrlProvider for RustfsUrlProvider {
    async fn signed_url(&self, object_key: &str, expires_at_epoch_ms: u64) -> CanopyResult<String> {
        let signature = self.signer.sign(object_key, expires_at_epoch_ms);
        let expires_epoch_s = expires_at_epoch_ms / 1_000;
        Ok(format!(
            "{base}/{bucket}/{key}?signature={signature}&expires={expires_epoch_s}",
            base = self.base_url,
            bucket = self.bucket,
            key = object_key,
        ))
    }
}

/// Playback resolver: turns a track identifier into a presigned, time-limited
/// [`PlaybackSource`].
///
/// The resolver chooses the preferred codec among a track's assets, embeds an
/// expiry into the stream URL, and signs it via the [`UrlSigner`] port. It does
/// not serve bytes itself — the player streams directly from object storage.
#[derive(Clone)]
pub struct ResolverService {
    assets: Arc<dyn AudioAssetRepository>,
    url_provider: Arc<dyn PlaybackUrlProvider>,
    config: ResolverConfig,
}

impl ResolverService {
    /// Creates a resolver over the given asset repository and URL signer.
    pub fn new(
        assets: Arc<dyn AudioAssetRepository>,
        signer: Arc<dyn UrlSigner>,
        config: ResolverConfig,
    ) -> Self {
        let url_provider = Arc::new(RustfsUrlProvider {
            signer,
            base_url: config.base_url.clone(),
            bucket: config.bucket.clone(),
        });
        Self::with_url_provider(assets, url_provider, config)
    }

    /// Creates a resolver with a custom URL provider such as Supabase Storage.
    pub fn with_url_provider(
        assets: Arc<dyn AudioAssetRepository>,
        url_provider: Arc<dyn PlaybackUrlProvider>,
        config: ResolverConfig,
    ) -> Self {
        Self {
            assets,
            url_provider,
            config,
        }
    }

    /// Resolves `track_id` to a playable source valid from now.
    pub async fn resolve(&self, track_id: &str) -> CanopyResult<PlaybackSource> {
        self.resolve_at(track_id, now_epoch_ms()).await
    }

    /// Resolves `track_id` using an explicit `now` (epoch ms); the expiry is
    /// computed relative to it. Split out so the time-dependent behavior is
    /// deterministically testable.
    pub async fn resolve_at(
        &self,
        track_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<PlaybackSource> {
        let assets = self.assets.assets_for_track(track_id).await?;
        let asset = self
            .select_asset(assets)
            .ok_or_else(|| CanopyError::not_found("audio_asset", track_id))?;

        let expires_at_epoch_ms = now_epoch_ms + self.config.url_ttl.as_millis() as u64;
        let stream_url = self
            .url_provider
            .signed_url(&asset.object_key, expires_at_epoch_ms)
            .await?;

        Ok(PlaybackSource {
            track_id: track_id.to_string(),
            stream_url,
            content_type: asset.content_type,
            codec: asset.codec,
            duration_ms: asset.duration_ms,
            expires_at_epoch_ms,
        })
    }

    /// Resolves a track and synchronizes the lightweight anonymous session.
    pub async fn resolve_for_session(
        &self,
        playback: &PlaybackService,
        session_id: &str,
        track_id: &str,
        now_epoch_ms: u64,
    ) -> CanopyResult<PlaybackSource> {
        let source = self.resolve_at(track_id, now_epoch_ms).await?;
        playback
            .load_media(session_id, source.track_id.clone())
            .await?;
        Ok(source)
    }

    /// Picks the most preferred available asset, falling back to the first one
    /// when none match the preference list (so an ingested-but-unranked codec
    /// is still playable).
    fn select_asset(&self, assets: Vec<AudioAsset>) -> Option<AudioAsset> {
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

    use crate::jade_store::{InMemoryAudioAssetStore, InMemorySessionStore};
    use crate::signing::HmacUrlSigner;

    struct StaticUrlProvider;

    #[async_trait::async_trait]
    impl PlaybackUrlProvider for StaticUrlProvider {
        async fn signed_url(
            &self,
            object_key: &str,
            _expires_at_epoch_ms: u64,
        ) -> CanopyResult<String> {
            Ok(format!("https://media.test/{object_key}"))
        }
    }

    fn asset(track: &str, codec: &str, key: &str) -> AudioAsset {
        AudioAsset {
            track_id: track.into(),
            codec: codec.into(),
            content_type: format!("audio/{codec}"),
            object_key: key.into(),
            size_bytes: 1_024,
            checksum_sha256: "deadbeef".into(),
            duration_ms: 245_000,
        }
    }

    fn resolver(assets: Vec<AudioAsset>) -> ResolverService {
        ResolverService::new(
            Arc::new(InMemoryAudioAssetStore::with_assets(assets)),
            Arc::new(HmacUrlSigner::new("test-secret")),
            ResolverConfig {
                url_ttl: Duration::from_secs(600),
                ..ResolverConfig::default()
            },
        )
    }

    #[tokio::test]
    async fn resolve_prefers_higher_ranked_codec() {
        let resolver = resolver(vec![
            asset("trk_1", "mp3", "audio/tracks/trk_1.mp3"),
            asset("trk_1", "opus", "audio/tracks/trk_1.opus"),
        ]);

        let source = resolver.resolve_at("trk_1", 1_000_000).await.unwrap();
        // `opus` outranks `mp3` in the default preference list.
        assert_eq!(source.codec, "opus");
        assert_eq!(source.content_type, "audio/opus");
        assert_eq!(source.track_id, "trk_1");
    }

    #[tokio::test]
    async fn resolve_embeds_signed_url_and_expiry() {
        let resolver = resolver(vec![asset("trk_1", "mp3", "audio/tracks/trk_1.mp3")]);

        let now = 1_000_000;
        let source = resolver.resolve_at("trk_1", now).await.unwrap();

        // TTL is 600s => +600_000 ms.
        assert_eq!(source.expires_at_epoch_ms, now + 600_000);
        assert!(
            source
                .stream_url
                .contains("pandawave-media/audio/tracks/trk_1.mp3")
        );
        assert!(source.stream_url.contains("signature="));
        // URL carries the expiry in epoch seconds.
        assert!(
            source
                .stream_url
                .contains(&format!("expires={}", (now + 600_000) / 1_000))
        );
    }

    #[tokio::test]
    async fn resolve_falls_back_when_no_preferred_codec() {
        let resolver = resolver(vec![asset("trk_1", "wav", "audio/tracks/trk_1.wav")]);
        let source = resolver.resolve_at("trk_1", 0).await.unwrap();
        assert_eq!(source.codec, "wav");
    }

    #[tokio::test]
    async fn resolve_missing_track_is_not_found() {
        let resolver = resolver(vec![]);
        let err = resolver.resolve_at("missing", 0).await.unwrap_err();
        assert!(matches!(err, CanopyError::NotFound { entity, .. } if entity == "audio_asset"));
    }

    #[tokio::test]
    async fn resolve_for_session_updates_anonymous_session() {
        let playback = PlaybackService::new(Arc::new(InMemorySessionStore::default()));
        let resolver = ResolverService::with_url_provider(
            Arc::new(InMemoryAudioAssetStore::with_assets(vec![asset(
                "trk_1",
                "mp3",
                "audio/tracks/trk_1.mp3",
            )])),
            Arc::new(StaticUrlProvider),
            ResolverConfig {
                url_ttl: Duration::from_secs(600),
                ..ResolverConfig::default()
            },
        );

        let source = resolver
            .resolve_for_session(&playback, "", "trk_1", 1_000_000)
            .await
            .unwrap();

        assert_eq!(
            source.stream_url,
            "https://media.test/audio/tracks/trk_1.mp3"
        );
        let session = playback
            .get_session(PlaybackService::DEFAULT_SESSION_ID)
            .await
            .unwrap();
        assert_eq!(session.current_media_id.as_deref(), Some("trk_1"));
        assert_eq!(session.position_ms, 0);
    }
}
