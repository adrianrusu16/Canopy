#![cfg(feature = "pg")]

#[test]
fn nginx_config_is_private_and_redacted() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("deploy/nginx/canopy-stream.conf");
    let config = std::fs::read_to_string(path).expect("Nginx stream config should exist");

    for required in [
        "auth_request",
        "proxy_set_header X-Canopy-Original-URI $request_uri;",
        "auth_request_set",
        "/_canopy_media/",
        "internal",
        "alias /srv/canopy/media/library/",
        "autoindex off",
        "access_log off",
    ] {
        assert!(
            config.contains(required),
            "Nginx stream config is missing {required}"
        );
    }

    assert!(!config.contains("ssl_certificate"));
    assert!(!config.contains("access_log on"));
    assert!(!config.contains("X-Canopy-Stream-Token"));
}

const STREAM_SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";
const STREAM_ASSET_KEY: &str = "audio/aa/bb/stream-test.mp3";

async fn seed_public_stream_asset(pool: &sqlx::PgPool) -> (String, String) {
    let unique = uuid::Uuid::new_v4();
    let artist_id: uuid::Uuid =
        sqlx::query_scalar("INSERT INTO artists (name, sort_name) VALUES ($1, $1) RETURNING id")
            .bind(format!("Stream Test Artist {unique}"))
            .fetch_one(pool)
            .await
            .unwrap();
    let album_id: uuid::Uuid =
        sqlx::query_scalar("INSERT INTO albums (title, artist_id) VALUES ($1, $2) RETURNING id")
            .bind(format!("Stream Test Album {unique}"))
            .bind(artist_id)
            .fetch_one(pool)
            .await
            .unwrap();
    let license_id: uuid::Uuid = sqlx::query_scalar(
        r#"
            INSERT INTO licenses (license_type, source_url, review_status, reviewed_at)
            VALUES ('CC0', $1, 'approved', NOW())
            RETURNING id
        "#,
    )
    .bind(format!("https://license.test/stream/{unique}"))
    .fetch_one(pool)
    .await
    .unwrap();
    let track_id: uuid::Uuid = sqlx::query_scalar(
        r#"
            INSERT INTO tracks (
                title, artist_id, album_id, duration_ms, visibility, ingest_status,
                composition_license_id, recording_license_id
            )
            VALUES ($1, $2, $3, 1000, 'release_safe', 'ready', $4, $4)
            RETURNING id
        "#,
    )
    .bind(format!("Stream Test Track {unique}"))
    .bind(artist_id)
    .bind(album_id)
    .bind(license_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let checksum = unique.simple().to_string().repeat(2);
    let asset_id: uuid::Uuid = sqlx::query_scalar(
        r#"
            INSERT INTO audio_assets (
                track_id, codec, content_type, storage_key,
                size_bytes, checksum_sha256, duration_ms
            )
            VALUES ($1, 'mp3', 'audio/mpeg', $2, 1024, $3, 1000)
            RETURNING id
        "#,
    )
    .bind(track_id)
    .bind(STREAM_ASSET_KEY)
    .bind(checksum)
    .fetch_one(pool)
    .await
    .unwrap();

    (track_id.to_string(), asset_id.to_string())
}

#[tokio::test]
#[ignore = "requires scripts/test-streaming.sh"]
async fn nginx_serves_ranges_and_rechecks_revoked_policy() {
    use std::sync::Arc;

    use canopy_core::StreamAudience;
    use canopy_server::{
        jade_store::PgPlayableAssetRepository,
        stream::{ArtworkAuthorizer, StreamAuthorizer, StreamTokenCodec, serve_stream_auth, stream_auth_router},
    };

    let database_url =
        std::env::var("CANOPY_STREAM_TEST_DATABASE_URL").expect("stream test database URL");
    let media_root =
        std::path::PathBuf::from(std::env::var("CANOPY_STREAM_TEST_MEDIA_ROOT").unwrap());
    let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();

    let (track_id, asset_id) = seed_public_stream_asset(&pool).await;
    let repository = Arc::new(PgPlayableAssetRepository::new(pool.clone()));
    let codec = Arc::new(StreamTokenCodec::new(STREAM_SECRET).unwrap());
    let authorizer = Arc::new(StreamAuthorizer::new(codec.clone(), repository.clone()));
    let artwork_authorizer = Arc::new(ArtworkAuthorizer::new(repository));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:18081")
        .await
        .unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_stream_auth(
        listener,
        stream_auth_router(authorizer, artwork_authorizer),
        async {
            let _ = shutdown_rx.await;
        },
    ));

    let token = codec
        .mint(&asset_id, StreamAudience::Public, u64::MAX)
        .unwrap();
    let stream_url = format!("http://127.0.0.1:18080/stream/{token}");
    let client = reqwest::Client::new();
    let response = client
        .get(&stream_url)
        .header(reqwest::header::RANGE, "bytes=0-15")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert!(
        response.headers()[reqwest::header::CONTENT_RANGE]
            .to_str()
            .unwrap()
            .starts_with("bytes 0-15/")
    );
    let actual = response.bytes().await.unwrap();
    let expected = tokio::fs::read(media_root.join("library").join(STREAM_ASSET_KEY))
        .await
        .unwrap();
    assert_eq!(actual.as_ref(), &expected[..16]);

    sqlx::query(
        "UPDATE tracks SET visibility = 'quarantined', ingest_status = 'quarantined' WHERE id = $1::uuid",
    )
    .bind(&track_id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        client.get(&stream_url).send().await.unwrap().status(),
        reqwest::StatusCode::FORBIDDEN
    );

    let tampered = format!("{stream_url}x");
    assert_eq!(
        client.get(tampered).send().await.unwrap().status(),
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .get("http://127.0.0.1:18080/_canopy_media/audio/aa/bb/stream-test.mp3")
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let _ = shutdown_tx.send(());
    server.await.unwrap().unwrap();
}
