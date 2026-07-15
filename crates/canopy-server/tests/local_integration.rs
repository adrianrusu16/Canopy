#![cfg(feature = "pg")]

use std::time::{Duration, Instant};

use canopy_proto::{
    GetStatusRequest, ListSessionsRequest, LoginPasswordRequest, LogoutRequest,
    RefreshSessionRequest, RegisterPasswordRequest, ResolvePlaybackRequest, SearchRequest,
    VerifyEmailRequest, auth_service_client::AuthServiceClient,
    catalog_service_client::CatalogServiceClient, playback_service_client::PlaybackServiceClient,
    system_service_client::SystemServiceClient,
};
use reqwest::{StatusCode, Url, header::RANGE};
use tokio::time::sleep;
use tonic::{
    Code, Request,
    metadata::MetadataValue,
    transport::{Channel, Endpoint},
};

const PASSWORD: &str = "Canopy-Local-Test-Password-42!";

fn endpoint(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

fn authorized<T>(message: T, access_token: &str) -> Request<T> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(format!("Bearer {access_token}"))
        .expect("access token must fit ASCII metadata");
    request.metadata_mut().insert("authorization", value);
    request
}

fn extract_verification_token(message: &str) -> Result<String, &'static str> {
    let action = message
        .lines()
        .filter_map(|line| Url::parse(line.trim()).ok())
        .find(|url| url.path().ends_with("/verify-email"))
        .ok_or("verification action URL is missing")?;
    action
        .query_pairs()
        .find_map(|(key, value)| (key == "token").then(|| value.into_owned()))
        .filter(|token| !token.is_empty())
        .ok_or("verification token is missing")
}

async fn channel() -> Result<Channel, &'static str> {
    let endpoint = endpoint("CANOPY_LOCAL_GRPC_ENDPOINT", "http://127.0.0.1:50051");
    Endpoint::from_shared(endpoint)
        .map_err(|_| "invalid local gRPC endpoint")?
        .connect()
        .await
        .map_err(|_| "local gRPC endpoint is unavailable")
}

async fn wait_for_ready() -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(channel) = channel().await {
            let mut system = SystemServiceClient::new(channel);
            if let Ok(response) = system.get_status(GetStatusRequest {}).await {
                let status = response.into_inner();
                let dependencies_ready = ["postgres", "media_library", "auth_email_delivery"]
                    .iter()
                    .all(|name| {
                        status.dependencies.iter().any(|dependency| {
                            dependency.name == *name && dependency.status == "healthy"
                        })
                    });
                if status.healthy && dependencies_ready {
                    let http = reqwest::Client::new();
                    let nginx = endpoint("CANOPY_LOCAL_STREAM_ENDPOINT", "http://127.0.0.1:8080");
                    let openapi = endpoint(
                        "CANOPY_LOCAL_OPENAPI_ENDPOINT",
                        "http://127.0.0.1:8080/openapi.json",
                    );
                    let mailpit =
                        endpoint("CANOPY_LOCAL_MAILPIT_ENDPOINT", "http://127.0.0.1:8025");
                    let nginx_ready = http
                        .get(format!("{nginx}/nginx-health"))
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success());
                    let openapi_ready = http
                        .get(openapi)
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success());
                    let mailpit_ready = http
                        .get(format!("{mailpit}/readyz"))
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success());
                    if nginx_ready && openapi_ready && mailpit_ready {
                        return Ok(());
                    }
                }
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err("local integration environment did not become ready")
}

async fn wait_for_verification_token(email: &str) -> Result<String, &'static str> {
    let base = endpoint("CANOPY_LOCAL_MAILPIT_ENDPOINT", "http://127.0.0.1:8025");
    let mut url = Url::parse(&base)
        .map_err(|_| "invalid Mailpit endpoint")?
        .join("/view/latest.txt")
        .map_err(|_| "invalid Mailpit message endpoint")?;
    url.query_pairs_mut()
        .append_pair("query", &format!("to:{email}"));

    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(url.clone()).send().await
            && response.status().is_success()
            && let Ok(body) = response.text().await
            && let Ok(token) = extract_verification_token(&body)
        {
            return Ok(token);
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err("verification message did not arrive before timeout")
}

#[test]
fn verification_token_parser_uses_the_action_url() {
    let body = "Verify your account\n\nhttp://127.0.0.1:3000/auth/verify-email?token=opaque%2Fvalue&expires_at=42\n";
    let token = extract_verification_token(body).unwrap();
    assert_eq!(token, "opaque/value");
}

#[tokio::test]
#[ignore = "requires scripts/local-integration.sh"]
async fn local_environment_is_ready() {
    wait_for_ready()
        .await
        .expect("local integration readiness should succeed");
}

#[tokio::test]
#[ignore = "requires scripts/local-integration.sh"]
async fn local_environment_auth_and_playback_smoke() {
    wait_for_ready().await.expect("environment should be ready");
    let channel = channel().await.expect("gRPC should be reachable");
    let mut auth = AuthServiceClient::new(channel.clone());

    let email = format!("local-integration-{}@example.test", uuid::Uuid::new_v4());
    auth.register_password(RegisterPasswordRequest {
        email: email.clone(),
        password: PASSWORD.into(),
    })
    .await
    .expect("registration should be accepted");

    let verification_token = wait_for_verification_token(&email)
        .await
        .expect("verification delivery should succeed");
    let verified = auth
        .verify_email(VerifyEmailRequest {
            verification_token,
            device_label: "local-integration-verification".into(),
        })
        .await
        .expect("verification should succeed")
        .into_inner();
    assert!(
        !verified.access_token.is_empty() && !verified.refresh_token.is_empty(),
        "verification must return a complete session envelope"
    );

    auth.list_sessions(authorized(
        ListSessionsRequest { page: None },
        &verified.access_token,
    ))
    .await
    .expect("verified session should authorize protected calls");
    auth.logout(authorized(LogoutRequest {}, &verified.access_token))
        .await
        .expect("verification session logout should succeed");

    let logged_in = auth
        .login_password(LoginPasswordRequest {
            email,
            password: PASSWORD.into(),
            device_label: "local-integration-login".into(),
        })
        .await
        .expect("password login should succeed")
        .into_inner();
    let refreshed = auth
        .refresh_session(RefreshSessionRequest {
            refresh_token: logged_in.refresh_token.clone(),
        })
        .await
        .expect("single refresh should succeed")
        .into_inner();
    assert!(
        logged_in.refresh_token != refreshed.refresh_token,
        "refresh must rotate the opaque refresh token"
    );

    auth.logout(authorized(LogoutRequest {}, &refreshed.access_token))
        .await
        .expect("refreshed session logout should succeed");
    let rejected = auth
        .list_sessions(authorized(
            ListSessionsRequest { page: None },
            &refreshed.access_token,
        ))
        .await
        .expect_err("logged-out session must be rejected");
    assert_eq!(rejected.code(), Code::Unauthenticated);

    let mut catalog = CatalogServiceClient::new(channel.clone());
    let search = catalog
        .search(SearchRequest {
            query: "Moonlight Sonata".into(),
            page: None,
        })
        .await
        .expect("seeded catalog search should succeed")
        .into_inner();
    let track = search
        .tracks
        .into_iter()
        .find(|track| track.title == "Moonlight Sonata")
        .expect("seeded track should be discoverable");

    let mut playback = PlaybackServiceClient::new(channel);
    let source = playback
        .resolve_playback(ResolvePlaybackRequest { track_id: track.id })
        .await
        .expect("playback resolution should succeed")
        .into_inner();
    assert!(
        source
            .stream_url
            .starts_with("http://127.0.0.1:8080/stream/"),
        "playback must use the local Nginx stream origin"
    );

    let stream_url = Url::parse(&source.stream_url).expect("stream URL should be valid");
    let capability = stream_url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|token| !token.is_empty())
        .expect("stream URL should contain an opaque capability");
    let client = reqwest::Client::new();
    let direct = client
        .get("http://127.0.0.1:18081/internal/stream/authorize")
        .header("X-Canopy-Stream-Token", capability)
        .send()
        .await
        .map_err(|_| "direct stream authorization request failed")
        .expect("direct stream authorization should be reachable");
    assert_eq!(direct.status(), StatusCode::NO_CONTENT);

    let response = client
        .get(stream_url)
        .header(RANGE, "bytes=0-15")
        .send()
        .await
        .map_err(|_| "stream range request failed")
        .expect("stream range request should succeed");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "stream body read failed")
        .expect("stream body should be readable");
    assert!(
        !bytes.is_empty(),
        "stream range response must contain bytes"
    );
}
