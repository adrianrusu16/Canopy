use std::{
    future::Future,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use canopy_core::CanopyError;
use tokio::net::TcpListener;

use super::StreamAuthorizer;

const TOKEN_HEADER: &str = "x-canopy-stream-token";
const INTERNAL_REDIRECT_HEADER: &str = "x-accel-redirect";
const CONTENT_TYPE_HINT_HEADER: &str = "x-canopy-content-type";

pub fn stream_auth_router(authorizer: Arc<StreamAuthorizer>) -> Router {
    Router::new()
        .route("/internal/stream/authorize", get(authorize_stream))
        .with_state(authorizer)
}

pub async fn serve_stream_auth(
    listener: TcpListener,
    router: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
}

async fn authorize_stream(
    State(authorizer): State<Arc<StreamAuthorizer>>,
    headers: HeaderMap,
) -> Response {
    let Some(token) = headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return forbidden();
    };

    match authorizer.authorize(token, now_epoch_ms()).await {
        Ok(grant) => {
            let Ok(internal_uri) = HeaderValue::from_str(&grant.internal_uri) else {
                return unavailable();
            };
            let Ok(content_type) = HeaderValue::from_str(&grant.content_type) else {
                return unavailable();
            };

            let mut response = StatusCode::NO_CONTENT.into_response();
            response
                .headers_mut()
                .insert(INTERNAL_REDIRECT_HEADER, internal_uri);
            response
                .headers_mut()
                .insert(CONTENT_TYPE_HINT_HEADER, content_type);
            response.headers_mut().remove(header::CONTENT_TYPE);
            response
        }
        Err(
            CanopyError::Unauthenticated(_)
            | CanopyError::InvalidArgument(_)
            | CanopyError::FailedPrecondition(_)
            | CanopyError::Aborted(_)
            | CanopyError::RateLimited(_)
            | CanopyError::NotFound { .. },
        ) => forbidden(),
        Err(CanopyError::Storage(_) | CanopyError::Internal(_)) => unavailable(),
    }
}

fn forbidden() -> Response {
    (StatusCode::FORBIDDEN, "forbidden").into_response()
}

fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "unavailable").into_response()
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use canopy_core::{
        AuthorizedStreamAsset, CanopyError, CanopyResult, PlayableAsset, PlayableAssetRepository,
        StreamAudience,
    };
    use tower::ServiceExt;

    use crate::stream::StreamTokenCodec;

    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
    const ASSET_ID: &str = "018f0000-0000-7000-8000-000000000001";

    struct FakeRepository {
        result: Mutex<Option<CanopyResult<Option<AuthorizedStreamAsset>>>>,
    }

    impl FakeRepository {
        fn new(result: CanopyResult<Option<AuthorizedStreamAsset>>) -> Self {
            Self {
                result: Mutex::new(Some(result)),
            }
        }
    }

    #[async_trait]
    impl PlayableAssetRepository for FakeRepository {
        async fn assets_for_personal_playback(
            &self,
            _owner_profile_id: &str,
            _track_id: &str,
        ) -> CanopyResult<Vec<PlayableAsset>> {
            Ok(Vec::new())
        }
        async fn assets_for_public_playback(
            &self,
            _track_id: &str,
        ) -> CanopyResult<Vec<PlayableAsset>> {
            Ok(Vec::new())
        }

        async fn authorize_stream_asset(
            &self,
            _asset_id: &str,
            _audience: StreamAudience,
        ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
            self.result.lock().unwrap().take().unwrap()
        }
    }

    fn router(result: CanopyResult<Option<AuthorizedStreamAsset>>) -> (Router, StreamTokenCodec) {
        let codec = StreamTokenCodec::new(SECRET).unwrap();
        let authorizer = StreamAuthorizer::new(
            Arc::new(codec.clone()),
            Arc::new(FakeRepository::new(result)),
        );
        (stream_auth_router(Arc::new(authorizer)), codec)
    }

    fn request(token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .uri("/internal/stream/authorize")
            .method("GET");
        if let Some(token) = token {
            builder = builder.header("X-Canopy-Stream-Token", token);
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn response_body(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        (status, body.to_vec())
    }

    #[tokio::test]
    async fn valid_authorization_returns_internal_redirect_metadata_only() {
        let (router, codec) = router(Ok(Some(AuthorizedStreamAsset {
            asset_id: ASSET_ID.into(),
            storage_key: "audio/aa/bb/hash.mp3".into(),
            content_type: "audio/mpeg".into(),
        })));
        let token = codec
            .mint(ASSET_ID, StreamAudience::Public, u64::MAX)
            .unwrap();

        let response = router.oneshot(request(Some(&token))).await.unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()["X-Accel-Redirect"],
            "/_canopy_media/audio/aa/bb/hash.mp3"
        );
        assert_eq!(response.headers()["X-Canopy-Content-Type"], "audio/mpeg");
        assert!(
            to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn invalid_expired_unknown_and_missing_tokens_are_indistinguishable() {
        let (invalid_router, _) = router(Ok(None));
        let invalid = response_body(
            invalid_router
                .oneshot(request(Some("invalid")))
                .await
                .unwrap(),
        )
        .await;

        let (expired_router, codec) = router(Ok(None));
        let expired_token = codec.mint(ASSET_ID, StreamAudience::Public, 0).unwrap();
        let expired = response_body(
            expired_router
                .oneshot(request(Some(&expired_token)))
                .await
                .unwrap(),
        )
        .await;

        let (unknown_router, codec) = router(Ok(None));
        let unknown_token = codec
            .mint(ASSET_ID, StreamAudience::Public, u64::MAX)
            .unwrap();
        let unknown = response_body(
            unknown_router
                .oneshot(request(Some(&unknown_token)))
                .await
                .unwrap(),
        )
        .await;

        let (missing_router, _) = router(Ok(None));
        let missing = response_body(missing_router.oneshot(request(None)).await.unwrap()).await;

        assert_eq!(invalid, (StatusCode::FORBIDDEN, b"forbidden".to_vec()));
        assert_eq!(expired, invalid);
        assert_eq!(unknown, invalid);
        assert_eq!(missing, invalid);
    }

    #[tokio::test]
    async fn repository_failure_returns_service_unavailable_without_details() {
        let (router, codec) = router(Err(CanopyError::Storage("database secret".into())));
        let token = codec
            .mint(ASSET_ID, StreamAudience::Public, u64::MAX)
            .unwrap();

        let response = response_body(router.oneshot(request(Some(&token))).await.unwrap()).await;

        assert_eq!(
            response,
            (StatusCode::SERVICE_UNAVAILABLE, b"unavailable".to_vec())
        );
    }
}
