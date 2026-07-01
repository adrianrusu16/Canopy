use std::sync::Arc;

use canopy_core::{CanopyError, CanopyResult, PlayableAssetRepository};

use super::StreamTokenCodec;
use crate::media::storage::{MediaClass, validate_storage_key};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamGrant {
    pub internal_uri: String,
    pub content_type: String,
}

pub struct StreamAuthorizer {
    tokens: Arc<StreamTokenCodec>,
    assets: Arc<dyn PlayableAssetRepository>,
}

impl StreamAuthorizer {
    pub fn new(tokens: Arc<StreamTokenCodec>, assets: Arc<dyn PlayableAssetRepository>) -> Self {
        Self { tokens, assets }
    }

    pub async fn authorize(&self, token: &str, now_epoch_ms: u64) -> CanopyResult<StreamGrant> {
        let claims = self.tokens.verify(token, now_epoch_ms)?;
        let asset = self
            .assets
            .authorize_stream_asset(&claims.asset_id, claims.audience)
            .await?
            .ok_or_else(invalid_capability)?;

        validate_storage_key(&asset.storage_key, MediaClass::Audio)
            .map_err(|_| invalid_capability())?;

        Ok(StreamGrant {
            internal_uri: format!("/_canopy_media/{}", asset.storage_key),
            content_type: asset.content_type,
        })
    }
}

fn invalid_capability() -> CanopyError {
    CanopyError::unauthenticated("invalid stream capability")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use canopy_core::{AuthorizedStreamAsset, CanopyError, PlayableAsset, StreamAudience};

    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    const ASSET_ID: &str = "018f0000-0000-7000-8000-000000000001";

    struct FakeRepository {
        authorization: Mutex<Option<CanopyResult<Option<AuthorizedStreamAsset>>>>,
        calls: Mutex<Vec<StreamAudience>>,
    }

    impl FakeRepository {
        fn allowing(storage_key: &str) -> Self {
            Self {
                authorization: Mutex::new(Some(Ok(Some(AuthorizedStreamAsset {
                    asset_id: ASSET_ID.into(),
                    storage_key: storage_key.into(),
                    content_type: "audio/mpeg".into(),
                })))),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn returning(result: CanopyResult<Option<AuthorizedStreamAsset>>) -> Self {
            Self {
                authorization: Mutex::new(Some(result)),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PlayableAssetRepository for FakeRepository {
        async fn assets_for_public_playback(
            &self,
            _track_id: &str,
        ) -> CanopyResult<Vec<PlayableAsset>> {
            Ok(Vec::new())
        }

        async fn authorize_stream_asset(
            &self,
            _asset_id: &str,
            audience: StreamAudience,
        ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
            self.calls.lock().unwrap().push(audience);
            self.authorization.lock().unwrap().take().unwrap()
        }
    }

    fn codec() -> Arc<StreamTokenCodec> {
        Arc::new(StreamTokenCodec::new(SECRET).unwrap())
    }

    fn token(audience: StreamAudience) -> String {
        codec().mint(ASSET_ID, audience, 5_000).unwrap()
    }

    fn assert_generic_denial(error: CanopyError) {
        assert_eq!(
            error.to_string(),
            "unauthenticated: invalid stream capability"
        );
    }

    #[tokio::test]
    async fn valid_public_claims_return_internal_grant() {
        let repository = Arc::new(FakeRepository::allowing("audio/aa/bb/hash.mp3"));
        let authorizer = StreamAuthorizer::new(codec(), repository);

        let grant = authorizer
            .authorize(&token(StreamAudience::Public), 4_000)
            .await
            .unwrap();

        assert_eq!(grant.internal_uri, "/_canopy_media/audio/aa/bb/hash.mp3");
        assert_eq!(grant.content_type, "audio/mpeg");
    }

    #[tokio::test]
    async fn valid_personal_claims_use_personal_policy_lookup() {
        let repository = Arc::new(FakeRepository::allowing("audio/aa/bb/hash.mp3"));
        let authorizer = StreamAuthorizer::new(codec(), repository.clone());

        authorizer
            .authorize(&token(StreamAudience::Personal), 4_000)
            .await
            .unwrap();

        assert_eq!(
            repository.calls.lock().unwrap().as_slice(),
            &[StreamAudience::Personal]
        );
    }

    #[tokio::test]
    async fn unknown_assets_return_generic_denial() {
        let repository = Arc::new(FakeRepository::returning(Ok(None)));
        let authorizer = StreamAuthorizer::new(codec(), repository);

        assert_generic_denial(
            authorizer
                .authorize(&token(StreamAudience::Public), 4_000)
                .await
                .unwrap_err(),
        );
    }

    #[tokio::test]
    async fn repository_failures_remain_storage_errors() {
        let repository = Arc::new(FakeRepository::returning(Err(CanopyError::Storage(
            "database unavailable".into(),
        ))));
        let authorizer = StreamAuthorizer::new(codec(), repository);

        assert!(matches!(
            authorizer
                .authorize(&token(StreamAudience::Public), 4_000)
                .await,
            Err(CanopyError::Storage(_))
        ));
    }

    #[tokio::test]
    async fn invalid_storage_keys_fail_closed() {
        let repository = Arc::new(FakeRepository::allowing("../audio/hash.mp3"));
        let authorizer = StreamAuthorizer::new(codec(), repository);

        assert_generic_denial(
            authorizer
                .authorize(&token(StreamAudience::Public), 4_000)
                .await
                .unwrap_err(),
        );
    }

    #[tokio::test]
    async fn invalid_tokens_never_call_repository() {
        let repository = Arc::new(FakeRepository::allowing("audio/aa/bb/hash.mp3"));
        let authorizer = StreamAuthorizer::new(codec(), repository.clone());

        assert_generic_denial(authorizer.authorize("invalid", 4_000).await.unwrap_err());
        assert!(repository.calls.lock().unwrap().is_empty());
    }
}
