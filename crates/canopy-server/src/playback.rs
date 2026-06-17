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
    /// Creates a new service over the given session repository.
    pub fn new(sessions: Arc<dyn SessionRepository>) -> Self {
        Self { sessions }
    }

    /// Fetches a session, returning a default (empty) session when unknown so
    /// the wire contract can always answer a `GetSession` call.
    pub async fn get_session(&self, id: &str) -> CanopyResult<Session> {
        Ok(self.sessions.get(id).await?.unwrap_or_default())
    }

    /// Applies a partial update to a session, creating it if necessary.
    pub async fn update_session(
        &self,
        id: &str,
        media_id: Option<String>,
        position_ms: Option<i64>,
    ) -> CanopyResult<()> {
        let mut session = self.sessions.get(id).await?.unwrap_or_else(|| Session {
            id: id.to_string(),
            ..Session::default()
        });
        session.id = id.to_string();
        if let Some(media_id) = media_id {
            session.current_media_id = Some(media_id);
        }
        if let Some(position_ms) = position_ms {
            session.position_ms = position_ms;
        }
        self.sessions.update(session).await
    }

    /// Ends (removes) a session.
    pub async fn end_session(&self, id: &str) -> CanopyResult<()> {
        self.sessions.delete(id).await
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

/// Playback resolver: turns a track identifier into a presigned, time-limited
/// [`PlaybackSource`].
///
/// The resolver chooses the preferred codec among a track's assets, embeds an
/// expiry into the stream URL, and signs it via the [`UrlSigner`] port. It does
/// not serve bytes itself — the player streams directly from object storage.
#[derive(Clone)]
pub struct ResolverService {
    assets: Arc<dyn AudioAssetRepository>,
    signer: Arc<dyn UrlSigner>,
    config: ResolverConfig,
}

impl ResolverService {
    /// Creates a resolver over the given asset repository and URL signer.
    pub fn new(
        assets: Arc<dyn AudioAssetRepository>,
        signer: Arc<dyn UrlSigner>,
        config: ResolverConfig,
    ) -> Self {
        Self {
            assets,
            signer,
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
        let stream_url = self.presign(&asset.object_key, expires_at_epoch_ms);

        Ok(PlaybackSource {
            track_id: track_id.to_string(),
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

    /// Builds the full presigned URL for an object key.
    fn presign(&self, object_key: &str, expires_at_epoch_ms: u64) -> String {
        let signature = self.signer.sign(object_key, expires_at_epoch_ms);
        // Object storage validates against an epoch-seconds `expires`; the
        // millisecond value is carried separately in `PlaybackSource`.
        let expires_epoch_s = expires_at_epoch_ms / 1_000;
        format!(
            "{base}/{bucket}/{key}?signature={signature}&expires={expires_epoch_s}",
            base = self.config.base_url,
            bucket = self.config.bucket,
            key = object_key,
        )
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
    use crate::jade_store::InMemoryAudioAssetStore;
    use crate::signing::HmacUrlSigner;

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
}
