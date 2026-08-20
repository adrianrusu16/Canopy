#![cfg(feature = "pg")]

use canopy_core::{
    AudioAsset, AudioAssetRepository, AuthOutboxFailureKind, AuthOutboxRepository, CatalogIngest,
    CatalogRepository, ClaimAuthOutboxBatch, ConsumeChallenge, CreateGoogleLinkChallenge,
    CreateGoogleLoginChallenge, CreateSessionRecord, DiscoveryRepository, ExternalIdentityRecord,
    IdentityRepository, InstanceSettingsRepository, LibraryRepository, LikeRepository,
    MarkAuthOutboxFailed, MediaImportRepository, Page, PendingImportOutcome, PendingMediaImport,
    PlayableAssetRepository, PlaybackHistoryEvent, PlaybackHistoryRepository, PlaylistRepository,
    PreferencesRepository, ProfileRepository, ProviderAudioAsset, ProviderLicense, ProviderTrack,
    RegisterPasswordRecord, RotateRefreshTokenRecord, StreamAudience, TrackAccessScope,
};
use canopy_server::jade_store::{
    PgAudioAssetRepository, PgAuthOutboxRepository, PgCatalogRepository, PgIdentityRepository,
    PgInstanceSettingsRepository, PgLibraryRepository, PgLikeRepository, PgMediaImportRepository,
    PgPlayableAssetRepository, PgPlaybackHistoryRepository, PgPlaylistRepository,
    PgPreferencesRepository, PgProfileRepository,
};
use sqlx::{Row, postgres::PgPoolOptions};

async fn connect_test_pool() -> sqlx::PgPool {
    let database_url = std::env::var("CANOPY_TEST_DATABASE_URL").expect(
        "CANOPY_TEST_DATABASE_URL is required for PostgreSQL integration tests; \
         run scripts/test-pg.sh or provide an isolated test database",
    );

    PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await
        .unwrap_or_else(|err| panic!("failed to connect to CANOPY_TEST_DATABASE_URL: {err}"))
}

fn identity_epoch_ms(offset_ms: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_millis() as u64
        + offset_ms
}

fn identity_digest(seed: u8) -> [u8; 32] {
    [seed; 32]
}

fn identity_session(seed: u8, expires_at_epoch_ms: u64) -> CreateSessionRecord {
    CreateSessionRecord {
        account_id: "ignored-during-email-activation".into(),
        device_label: format!("PandaWave test device {seed}"),
        refresh_token_hash: identity_digest(seed),
        expires_at_epoch_ms,
    }
}

fn google_link_payload(identity: &ExternalIdentityRecord) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "provider": identity.provider.clone(),
        "provider_subject": identity.provider_subject.clone(),
        "provider_email_at_link_time": identity.provider_email_at_link_time.clone(),
    }))
    .expect("google link payload should encode")
}

#[tokio::test]
async fn postgres_identity_activation_consumes_challenge_once_and_creates_profile_session() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let identity = PgIdentityRepository::new(pool.clone());
    let email = format!("identity-{}@example.test", uuid::Uuid::new_v4());
    let verification_hash = identity_digest(7);
    let now = identity_epoch_ms(0);
    let expires_at = identity_epoch_ms(3_600_000);

    identity
        .register_password(RegisterPasswordRecord {
            normalized_email: email.clone(),
            password_hash_phc: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$ZmFrZS1oYXNo".into(),
            policy_version: 1,
            verification_token_hash: verification_hash,
            verification_expires_at_epoch_ms: expires_at,
            encrypted_outbox_payload: vec![42; 32],
            outbox_key_id: "test-key".into(),
        })
        .await
        .expect("pending password account should be registered");

    let first = identity.activate_email_and_create_session(
        ConsumeChallenge {
            token_hash: verification_hash,
            challenge_type: "email_verification",
            now_epoch_ms: now,
        },
        identity_session(11, expires_at),
    );
    let second = identity.activate_email_and_create_session(
        ConsumeChallenge {
            token_hash: verification_hash,
            challenge_type: "email_verification",
            now_epoch_ms: now,
        },
        identity_session(12, expires_at),
    );

    let (first, second) = tokio::join!(first, second);
    let activated = match (first, second) {
        (Ok(session), Err(_)) | (Err(_), Ok(session)) => session,
        (Ok(_), Ok(_)) => panic!("challenge was consumed more than once"),
        (Err(first), Err(second)) => {
            panic!("both activation attempts failed: {first:?}; {second:?}")
        }
    };

    assert_eq!(
        activated.account.primary_email.as_deref(),
        Some(email.as_str())
    );
    assert!(activated.session.is_active_at(now));
    assert_eq!(activated.session.account_id, activated.account.id);

    let profile_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM profiles WHERE account_id = $1::uuid AND external_user_id = $1",
    )
    .bind(&activated.account.id)
    .fetch_one(&pool)
    .await
    .expect("profile count should be queryable");
    assert_eq!(profile_count, 1);

    let consumed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_challenges WHERE token_hash = $1 AND consumed_at IS NOT NULL",
    )
    .bind(verification_hash.as_slice())
    .fetch_one(&pool)
    .await
    .expect("challenge count should be queryable");
    assert_eq!(consumed_count, 1);

    let refresh_token_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_session_tokens WHERE session_id = $1::uuid")
            .bind(&activated.session.id)
            .fetch_one(&pool)
            .await
            .expect("session token count should be queryable");
    assert_eq!(refresh_token_count, 1);
}
fn provider_track(provider_id: String) -> ProviderTrack {
    ProviderTrack {
        provider_id,
        provider: "canopy-test".to_string(),
        title: "Integration Nocturne".to_string(),
        artist: "Canopy Test Artist".to_string(),
        album: "Canopy Test Album".to_string(),
        release_year: Some(2026),
        duration_ms: 181_000,
        is_explicit: false,
        license: ProviderLicense {
            license_type: "CC0".to_string(),
            source_url: "https://example.invalid/licenses/canopy-test".to_string(),
            attribution_text: "Canopy integration fixture".to_string(),
        },
        assets: vec![
            ProviderAudioAsset {
                codec: "mp3".to_string(),
                content_type: "audio/mpeg".to_string(),
                storage_key: "audio/tracks/canopy-test/integration-nocturne.mp3".to_string(),
                size_bytes: 7_200_000,
                checksum_sha256: "1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
                duration_ms: 181_000,
            },
            ProviderAudioAsset {
                codec: "opus".to_string(),
                content_type: "audio/ogg".to_string(),
                storage_key: "audio/tracks/canopy-test/integration-nocturne.opus".to_string(),
                size_bytes: 4_100_000,
                checksum_sha256: "2222222222222222222222222222222222222222222222222222222222222222"
                    .to_string(),
                duration_ms: 181_000,
            },
        ],
        artwork_storage_key: Some(
            "artwork/tracks/canopy-test/integration-nocturne.png".to_string(),
        ),
        album_artwork_storage_key: Some("artwork/albums/canopy-test/album.png".to_string()),
    }
}

async fn cleanup_provider_track(pool: &sqlx::PgPool, provider: &str, provider_id: &str) {
    let rows = sqlx::query(
        "SELECT track_id FROM provider_tracks WHERE provider = $1 AND provider_track_id = $2",
    )
    .bind(provider)
    .bind(provider_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for row in rows {
        if let Ok(track_id) = row.try_get::<uuid::Uuid, _>("track_id") {
            let _ = sqlx::query("DELETE FROM tracks WHERE id = $1")
                .bind(track_id)
                .execute(pool)
                .await;
        }
    }
}

#[tokio::test]
async fn postgres_identity_repository_supports_password_login_and_session_creation() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let identity = PgIdentityRepository::new(pool.clone());
    let email = format!("login-{}@example.test", uuid::Uuid::new_v4());
    let verification_hash = identity_digest(57);
    let now = identity_epoch_ms(0);
    let expires_at = identity_epoch_ms(3_600_000);

    identity
        .register_password(RegisterPasswordRecord {
            normalized_email: email.clone(),
            password_hash_phc: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$ZmFrZS1oYXNo".into(),
            policy_version: 1,
            verification_token_hash: verification_hash,
            verification_expires_at_epoch_ms: expires_at,
            encrypted_outbox_payload: vec![12; 32],
            outbox_key_id: "test-key".into(),
        })
        .await
        .expect("pending password account should be registered");

    identity
        .activate_email_and_create_session(
            ConsumeChallenge {
                token_hash: verification_hash,
                challenge_type: "email_verification",
                now_epoch_ms: now,
            },
            identity_session(61, expires_at),
        )
        .await
        .expect("activation should create an initial session");

    let login = identity
        .password_login_record(&email)
        .await
        .expect("login lookup should query successfully")
        .expect("login record should exist");
    assert_eq!(login.account.primary_email.as_deref(), Some(email.as_str()));
    assert_eq!(login.policy_version, 1);

    identity
        .update_password_hash(
            &login.account.id,
            "$argon2id$v=19$m=19456,t=2,p=1$dXBkYXRlZA$aGFzaA",
            2,
        )
        .await
        .expect("password hash should update");

    let updated = identity
        .password_login_record(&email)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.policy_version, 2);

    let created = identity
        .create_session(CreateSessionRecord {
            account_id: login.account.id.clone(),
            device_label: "PandaWave login device".into(),
            refresh_token_hash: identity_digest(62),
            expires_at_epoch_ms: expires_at,
        })
        .await
        .expect("active account should receive a new session");
    assert_eq!(created.account.id, login.account.id);
    assert_eq!(created.session.device_label, "PandaWave login device");
    assert!(created.session.is_active_at(now));
}
#[tokio::test]
async fn postgres_identity_session_validation_listing_and_revocation_are_account_scoped() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let identity = PgIdentityRepository::new(pool.clone());
    let email = format!("sessions-{}@example.test", uuid::Uuid::new_v4());
    let verification_hash = identity_digest(87);
    let now = identity_epoch_ms(0);
    let expires_at = identity_epoch_ms(3_600_000);

    identity
        .register_password(RegisterPasswordRecord {
            normalized_email: email,
            password_hash_phc: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$ZmFrZS1oYXNo".into(),
            policy_version: 1,
            verification_token_hash: verification_hash,
            verification_expires_at_epoch_ms: expires_at,
            encrypted_outbox_payload: vec![33; 32],
            outbox_key_id: "test-key".into(),
        })
        .await
        .expect("pending password account should be registered");

    let activated = identity
        .activate_email_and_create_session(
            ConsumeChallenge {
                token_hash: verification_hash,
                challenge_type: "email_verification",
                now_epoch_ms: now,
            },
            CreateSessionRecord {
                account_id: "ignored-during-email-activation".into(),
                device_label: "PandaWave primary device".into(),
                refresh_token_hash: identity_digest(88),
                expires_at_epoch_ms: expires_at,
            },
        )
        .await
        .expect("email verification should create an initial session");
    assert!(activated.session.created_at_epoch_ms > 0);
    assert!(activated.session.last_used_at_epoch_ms >= activated.session.created_at_epoch_ms);
    assert_eq!(activated.session.expires_at_epoch_ms, expires_at);

    let second = identity
        .create_session(CreateSessionRecord {
            account_id: activated.account.id.clone(),
            device_label: "PandaWave secondary device".into(),
            refresh_token_hash: identity_digest(89),
            expires_at_epoch_ms: expires_at,
        })
        .await
        .expect("active account should receive a second session");

    identity
        .validate_active_session(&activated.account.id, &activated.session.id, now)
        .await
        .expect("fresh primary session should validate");
    identity
        .validate_active_session(&activated.account.id, &second.session.id, now)
        .await
        .expect("fresh secondary session should validate");

    let listed = identity
        .list_sessions(&activated.account.id)
        .await
        .expect("account sessions should list");
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|session| session.created_at_epoch_ms > 0));
    assert!(
        listed
            .iter()
            .all(|session| session.last_used_at_epoch_ms >= session.created_at_epoch_ms)
    );
    assert!(
        listed
            .iter()
            .all(|session| session.expires_at_epoch_ms == expires_at)
    );
    assert!(
        listed
            .iter()
            .all(|session| session.account_id == activated.account.id)
    );

    identity
        .revoke_session(&activated.account.id, &second.session.id)
        .await
        .expect("single-session revocation should be idempotent");
    identity
        .revoke_session(&activated.account.id, &second.session.id)
        .await
        .expect("repeating single-session revocation should remain idempotent");

    assert!(
        identity
            .validate_active_session(&activated.account.id, &second.session.id, now)
            .await
            .is_err(),
        "revoked session should fail validation immediately"
    );
    identity
        .validate_active_session(&activated.account.id, &activated.session.id, now)
        .await
        .expect("other account-owned session should remain active");

    identity
        .revoke_all_sessions(&activated.account.id, "logout_all")
        .await
        .expect("all account sessions should revoke idempotently");
    identity
        .revoke_all_sessions(&activated.account.id, "logout_all")
        .await
        .expect("repeating all-session revocation should remain idempotent");

    assert!(
        identity
            .validate_active_session(&activated.account.id, &activated.session.id, now)
            .await
            .is_err(),
        "logout-all should revoke the remaining active session"
    );

    let listed = identity
        .list_sessions(&activated.account.id)
        .await
        .expect("revoked sessions should remain visible for device management");
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|session| session.revoked_at_epoch_ms.is_some())
    );
}
#[tokio::test]
async fn postgres_identity_refresh_rotation_reuse_revokes_session_family() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let identity = PgIdentityRepository::new(pool.clone());
    let email = format!("refresh-{}@example.test", uuid::Uuid::new_v4());
    let verification_hash = identity_digest(17);
    let initial_refresh_hash = identity_digest(31);
    let now = identity_epoch_ms(0);
    let expires_at = identity_epoch_ms(3_600_000);

    identity
        .register_password(RegisterPasswordRecord {
            normalized_email: email,
            password_hash_phc: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$ZmFrZS1oYXNo".into(),
            policy_version: 1,
            verification_token_hash: verification_hash,
            verification_expires_at_epoch_ms: expires_at,
            encrypted_outbox_payload: vec![24; 32],
            outbox_key_id: "test-key".into(),
        })
        .await
        .expect("pending password account should be registered");

    let activated = identity
        .activate_email_and_create_session(
            ConsumeChallenge {
                token_hash: verification_hash,
                challenge_type: "email_verification",
                now_epoch_ms: now,
            },
            CreateSessionRecord {
                account_id: "ignored-during-email-activation".into(),
                device_label: "PandaWave refresh test".into(),
                refresh_token_hash: initial_refresh_hash,
                expires_at_epoch_ms: expires_at,
            },
        )
        .await
        .expect("email verification should create a session");

    let first = identity.rotate_refresh_token(RotateRefreshTokenRecord {
        presented_token_hash: initial_refresh_hash,
        replacement_token_hash: identity_digest(41),
        replacement_expires_at_epoch_ms: expires_at,
        now_epoch_ms: now,
    });
    let second = identity.rotate_refresh_token(RotateRefreshTokenRecord {
        presented_token_hash: initial_refresh_hash,
        replacement_token_hash: identity_digest(42),
        replacement_expires_at_epoch_ms: expires_at,
        now_epoch_ms: now,
    });

    let (first, second) = tokio::join!(first, second);
    let refreshed = match (first, second) {
        (Ok(session), Err(_)) | (Err(_), Ok(session)) => session,
        (Ok(_), Ok(_)) => panic!("refresh token was rotated more than once"),
        (Err(first), Err(second)) => {
            panic!("both refresh attempts failed: {first:?}; {second:?}")
        }
    };
    assert_eq!(refreshed.session.last_used_at_epoch_ms, now);
    assert_eq!(refreshed.session.expires_at_epoch_ms, expires_at);

    let revoked_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE id = $1::uuid AND revoked_at IS NOT NULL",
    )
    .bind(&activated.session.id)
    .fetch_one(&pool)
    .await
    .expect("revoked session count should be queryable");
    assert_eq!(revoked_count, 1);

    let live_token_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_session_tokens WHERE session_id = $1::uuid AND consumed_at IS NULL",
    )
    .bind(&activated.session.id)
    .fetch_one(&pool)
    .await
    .expect("live token count should be queryable");
    assert_eq!(live_token_count, 0);

    let reuse_signal_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COALESCE(SUM(request_count), 0)::bigint
            FROM auth_rate_limits
            WHERE operation = 'refresh_token_reuse'
              AND subject_hash = $1
        "#,
    )
    .bind(initial_refresh_hash.as_slice())
    .fetch_one(&pool)
    .await
    .expect("refresh reuse signal should be queryable");
    assert_eq!(reuse_signal_count, 1);
}
#[tokio::test]
async fn postgres_identity_google_login_challenge_is_single_use() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let identity = PgIdentityRepository::new(pool.clone());
    let now = identity_epoch_ms(0);
    let challenge_id = identity
        .create_google_login_challenge(CreateGoogleLoginChallenge {
            nonce_hash: identity_digest(91),
            expires_at_epoch_ms: identity_epoch_ms(600_000),
            encrypted_payload: b"{\"nonce\":\"nonce-for-google\"}".to_vec(),
        })
        .await
        .expect("google login challenge should be created");

    let first = identity
        .consume_google_login_challenge(&challenge_id, now)
        .await
        .expect("first consume should return payload");
    assert_eq!(
        first.encrypted_payload.as_slice(),
        b"{\"nonce\":\"nonce-for-google\"}"
    );

    let second = identity
        .consume_google_login_challenge(&challenge_id, now)
        .await;
    assert!(second.is_err(), "google nonce challenge must be single-use");
}

#[tokio::test]
async fn postgres_identity_google_external_account_and_link_flow_use_provider_subject() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let identity = PgIdentityRepository::new(pool.clone());
    let now = identity_epoch_ms(0);
    let expires_at = identity_epoch_ms(3_600_000);
    let google = ExternalIdentityRecord {
        provider: "google".into(),
        provider_subject: format!("google-sub-{}", uuid::Uuid::new_v4()),
        provider_email_at_link_time: Some(format!("google-{}@example.test", uuid::Uuid::new_v4())),
    };

    let created = identity
        .create_external_identity_session(
            google.clone(),
            CreateSessionRecord {
                account_id: "ignored-for-google-account-create".into(),
                device_label: "Google test device".into(),
                refresh_token_hash: identity_digest(92),
                expires_at_epoch_ms: expires_at,
            },
        )
        .await
        .expect("unknown google sub should create an active external account");
    assert_eq!(created.account.status, canopy_core::AccountStatus::Active);
    assert_eq!(
        created.account.primary_email.as_deref(),
        google.provider_email_at_link_time.as_deref()
    );
    assert_eq!(created.session.account_id, created.account.id);

    let by_subject = identity
        .account_by_external_identity(&google)
        .await
        .expect("external identity lookup should query successfully")
        .expect("external account should be found by provider subject");
    assert_eq!(by_subject.id, created.account.id);

    let native_email = format!("native-{}@example.test", uuid::Uuid::new_v4());
    let verification_hash = identity_digest(93);
    identity
        .register_password(RegisterPasswordRecord {
            normalized_email: native_email.clone(),
            password_hash_phc: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$ZmFrZS1oYXNo".into(),
            policy_version: 1,
            verification_token_hash: verification_hash,
            verification_expires_at_epoch_ms: expires_at,
            encrypted_outbox_payload: vec![55; 32],
            outbox_key_id: "test-key".into(),
        })
        .await
        .expect("pending native account should be registered");
    let native = identity
        .activate_email_and_create_session(
            ConsumeChallenge {
                token_hash: verification_hash,
                challenge_type: "email_verification",
                now_epoch_ms: now,
            },
            identity_session(94, expires_at),
        )
        .await
        .expect("native account should activate");

    let by_primary_email = identity
        .account_by_primary_email(&native_email)
        .await
        .expect("primary email lookup should query successfully")
        .expect("active verified account should be found by email");
    assert_eq!(by_primary_email.id, native.account.id);

    let wrong_link_token = format!("wrong-link-token-{}", uuid::Uuid::new_v4());
    let wrong_link_identity = ExternalIdentityRecord {
        provider: "google".into(),
        provider_subject: format!("google-sub-{}", uuid::Uuid::new_v4()),
        provider_email_at_link_time: Some(native_email.clone()),
    };
    identity
        .create_google_link_challenge(CreateGoogleLinkChallenge {
            account_id: native.account.id.clone(),
            token_hash: identity_digest(95),
            identity: wrong_link_identity.clone(),
            expires_at_epoch_ms: expires_at,
            encrypted_payload: google_link_payload(&wrong_link_identity),
        })
        .await
        .expect("link challenge should be created");
    assert!(
        identity
            .link_external_identity(&native.account.id, &wrong_link_token, now)
            .await
            .is_err(),
        "unmatched presented token must not link"
    );

    let link_token = format!("link-token-{}", uuid::Uuid::new_v4());
    let token_hash = canopy_server::identity::TokenDigest::from_secret(&link_token);
    let link_identity = ExternalIdentityRecord {
        provider: "google".into(),
        provider_subject: format!("google-sub-{}", uuid::Uuid::new_v4()),
        provider_email_at_link_time: Some(native_email),
    };
    identity
        .create_google_link_challenge(CreateGoogleLinkChallenge {
            account_id: native.account.id.clone(),
            token_hash: *token_hash.as_bytes(),
            identity: link_identity.clone(),
            expires_at_epoch_ms: expires_at,
            encrypted_payload: google_link_payload(&link_identity),
        })
        .await
        .expect("second link challenge should be created");
    identity
        .link_external_identity(&native.account.id, &link_token, now)
        .await
        .expect("matching link challenge should attach google identity");

    let linked = identity
        .account_by_external_identity(&link_identity)
        .await
        .expect("linked external identity lookup should query successfully")
        .expect("linked google identity should resolve");
    assert_eq!(linked.id, native.account.id);

    identity
        .unlink_external_identity(&native.account.id, "google")
        .await
        .expect("unlink should be idempotent for google identities");
    assert!(
        identity
            .account_by_external_identity(&link_identity)
            .await
            .expect("post-unlink lookup should query successfully")
            .is_none()
    );
}
#[tokio::test]
async fn postgres_migrations_support_idempotent_provider_ingest() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let provider_id = format!("track-{}", uuid::Uuid::new_v4());
    cleanup_provider_track(&pool, "canopy-test", &provider_id).await;

    let catalog = PgCatalogRepository::new(pool.clone());
    let assets = PgAudioAssetRepository::new(pool.clone());

    let track = provider_track(provider_id.clone());
    let first_id = catalog
        .ingest(track.clone())
        .await
        .expect("first ingest should succeed");

    sqlx::query(
        r#"
            UPDATE licenses
            SET review_status = 'approved', reviewed_at = NOW()
            WHERE id = (SELECT license_id FROM tracks WHERE id = $1::uuid)
        "#,
    )
    .bind(&first_id)
    .execute(&pool)
    .await
    .expect("fixture license should be promotable");
    sqlx::query(
        r#"
            UPDATE tracks
            SET composition_license_id = license_id,
                recording_license_id = license_id,
                visibility = 'release_safe',
                ingest_status = 'ready'
            WHERE id = $1::uuid
        "#,
    )
    .bind(&first_id)
    .execute(&pool)
    .await
    .expect("fixture track should be promotable before re-ingest");

    let mut updated = track;
    updated.title = "Integration Nocturne Revised".to_string();
    updated.assets[0].size_bytes = 7_500_000;

    let second_id = catalog
        .ingest(updated)
        .await
        .expect("second ingest should update in place");

    assert_eq!(second_id, first_id);

    let stored_asset_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM audio_assets WHERE track_id = $1::uuid")
            .bind(&first_id)
            .fetch_one(&pool)
            .await
            .expect("ingested assets should remain stored while quarantined");
    assert_eq!(stored_asset_count, 2);

    let stored_policy: (String, String, String) =
        sqlx::query_as("SELECT title, visibility, ingest_status FROM tracks WHERE id = $1::uuid")
            .bind(&first_id)
            .fetch_one(&pool)
            .await
            .expect("ingested track should remain stored while quarantined");
    assert_eq!(stored_policy.0, "Integration Nocturne Revised");
    assert_eq!(stored_policy.1, "quarantined");
    assert_eq!(stored_policy.2, "quarantined");

    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, &first_id)
            .await
            .expect("public media lookup should succeed")
            .is_none()
    );
    assert!(
        assets
            .assets_for_public_track(&first_id)
            .await
            .expect("public asset lookup should succeed")
            .is_empty()
    );

    let page = catalog
        .browse(
            &TrackAccessScope::Public,
            None,
            &[],
            Page {
                limit: 500,
                offset: 0,
            },
        )
        .await
        .expect("browse should survive multi-asset tracks");
    let occurrences = page.items.iter().filter(|item| item.id == first_id).count();
    assert_eq!(occurrences, 0);

    let duplicate_discovery_rows: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM (
                SELECT track_id
                FROM mv_discovery_pool
                GROUP BY track_id
                HAVING COUNT(*) > 1
            ) duplicates
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("discovery pool should be queryable");
    assert_eq!(duplicate_discovery_rows, 0);

    cleanup_provider_track(&pool, "canopy-test", &provider_id).await;
}
#[tokio::test]
async fn postgres_discovery_fallback_preserves_public_visibility_policy() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let provider_id = format!("discovery-fallback-{}", uuid::Uuid::new_v4());
    cleanup_provider_track(&pool, "canopy-test", &provider_id).await;

    let catalog = PgCatalogRepository::new(pool.clone());
    let quarantined_id = catalog
        .ingest(provider_track(provider_id.clone()))
        .await
        .expect("quarantined discovery fixture should ingest");

    let unique = uuid::Uuid::new_v4();
    let artist_id: String = sqlx::query_scalar(
        "INSERT INTO artists (name, sort_name) VALUES ($1, $1) RETURNING id::text",
    )
    .bind(format!("Discovery Fallback Artist {unique}"))
    .fetch_one(&pool)
    .await
    .expect("fallback artist should insert");
    let album_id: String = sqlx::query_scalar(
        "INSERT INTO albums (title, artist_id) VALUES ($1, $2::uuid) RETURNING id::text",
    )
    .bind(format!("Discovery Fallback Album {unique}"))
    .bind(&artist_id)
    .fetch_one(&pool)
    .await
    .expect("fallback album should insert");
    let approved_license: String = sqlx::query_scalar(
        r#"
            INSERT INTO licenses (license_type, source_url, review_status, reviewed_at)
            VALUES ('CC0', $1, 'approved', NOW())
            RETURNING id::text
        "#,
    )
    .bind(format!("https://license.test/discovery-fallback/{unique}"))
    .fetch_one(&pool)
    .await
    .expect("fallback license should insert");
    let explicit_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Explicit Discovery Fallback Track {unique}"),
            visibility: "release_safe",
            ingest_status: "ready",
            owner_profile_id: None,
            composition_license_id: Some(&approved_license),
            recording_license_id: Some(&approved_license),
        },
    )
    .await;
    let eligible_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Eligible Discovery Fallback Track {unique}"),
            visibility: "release_safe",
            ingest_status: "ready",
            owner_profile_id: None,
            composition_license_id: Some(&approved_license),
            recording_license_id: Some(&approved_license),
        },
    )
    .await;

    let public_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        r#"
            SELECT id
            FROM tracks
            WHERE visibility = 'release_safe' AND ingest_status = 'ready'
            ORDER BY id
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("release-safe fixtures should be queryable");

    sqlx::query(
        r#"
            UPDATE tracks
            SET visibility = 'quarantined', ingest_status = 'quarantined'
            WHERE visibility = 'release_safe'
        "#,
    )
    .execute(&pool)
    .await
    .expect("release-safe rows should be temporarily quarantined");
    sqlx::query("REFRESH MATERIALIZED VIEW mv_discovery_pool")
        .execute(&pool)
        .await
        .expect("discovery view should refresh to an empty snapshot");
    sqlx::query(
        r#"
            UPDATE tracks
            SET visibility = 'release_safe', ingest_status = 'ready'
            WHERE id = ANY($1)
        "#,
    )
    .bind(&public_ids)
    .execute(&pool)
    .await
    .expect("release-safe rows should be restored without refreshing the view");

    sqlx::query("UPDATE tracks SET is_explicit = TRUE WHERE id::text = $1")
        .bind(&explicit_id)
        .execute(&pool)
        .await
        .expect("one restored row should become explicit");

    let fallback_result = catalog.shuffle_pool(&TrackAccessScope::Public).await;

    sqlx::query("DELETE FROM tracks WHERE id::text = ANY($1)")
        .bind(vec![explicit_id.clone(), eligible_id.clone()])
        .execute(&pool)
        .await
        .expect("fallback track fixtures should be removed");
    sqlx::query("DELETE FROM albums WHERE id::text = $1")
        .bind(&album_id)
        .execute(&pool)
        .await
        .expect("fallback album fixture should be removed");
    sqlx::query("DELETE FROM artists WHERE id::text = $1")
        .bind(&artist_id)
        .execute(&pool)
        .await
        .expect("fallback artist fixture should be removed");
    sqlx::query("DELETE FROM licenses WHERE id::text = $1")
        .bind(&approved_license)
        .execute(&pool)
        .await
        .expect("fallback license fixture should be removed");
    cleanup_provider_track(&pool, "canopy-test", &provider_id).await;
    sqlx::query("REFRESH MATERIALIZED VIEW mv_discovery_pool")
        .execute(&pool)
        .await
        .expect("discovery view should be restored after the assertion snapshot");

    let items = fallback_result.expect("fallback discovery should succeed");
    let item_ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();

    assert!(!item_ids.contains(&quarantined_id.as_str()));
    assert!(!item_ids.contains(&explicit_id.as_str()));
    assert!(item_ids.contains(&eligible_id.as_str()));
}

#[tokio::test]
async fn postgres_migrations_support_profile_upsert() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let external_user_id = format!("profile-{}", uuid::Uuid::new_v4());
    let profiles = PgProfileRepository::new(pool.clone());

    let created = profiles
        .upsert_profile(&external_user_id, Some("Ada"), true)
        .await
        .expect("profile should be created");
    assert_eq!(created.external_user_id, external_user_id);
    assert_eq!(created.display_name.as_deref(), Some("Ada"));
    assert!(created.history_enabled);

    let updated = profiles
        .upsert_profile(&created.external_user_id, Some("Ada Lovelace"), false)
        .await
        .expect("profile should update in place");
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.display_name.as_deref(), Some("Ada Lovelace"));
    assert!(!updated.history_enabled);

    let fetched = profiles
        .get_by_external_user_id(&updated.external_user_id)
        .await
        .expect("profile lookup should succeed")
        .expect("profile should exist");
    assert_eq!(fetched.id, created.id);

    let _ = sqlx::query("DELETE FROM profiles WHERE external_user_id = $1")
        .bind(&updated.external_user_id)
        .execute(&pool)
        .await;
}

#[test]
fn history_lifecycle_migration_enforces_consent_purge() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20250628000001_profile_history_lifecycle.sql"
    );
    let migration =
        std::fs::read_to_string(path).expect("history lifecycle migration should exist");

    assert!(migration.contains("AFTER UPDATE OF history_enabled ON profiles"));
    assert!(migration.contains("DELETE FROM playback_history WHERE profile_id = NEW.id"));
}

#[test]
fn local_media_foundation_migration_is_fail_closed() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20250629000001_local_media_foundation.sql"
    );
    let sql = std::fs::read_to_string(path).expect("local media foundation migration should exist");

    for required in [
        "RENAME COLUMN object_key TO storage_key",
        "CREATE TABLE instance_settings",
        "owner_profile_id",
        "visibility",
        "ingest_status",
        "composition_license_id",
        "recording_license_id",
        "review_status",
        "canopy_enforce_release_safe_track",
        "canopy_quarantine_tracks_on_license_revocation",
        "visibility = 'release_safe'",
        "ingest_status = 'ready'",
    ] {
        assert!(sql.contains(required), "migration is missing {required}");
    }
}

#[test]
fn local_media_import_migration_is_deduplicated_and_attributed() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../migrations/20250629000002_local_media_imports.sql"
    );
    let sql = std::fs::read_to_string(path).expect("local media import migration should exist");

    for required in [
        "ingest_source",
        "legacy_provider",
        "local_admin",
        "uq_audio_assets_checksum_sha256",
        "LOWER(checksum_sha256)",
        "^[0-9a-fA-F]{64}$",
        "duplicate audio checksums prevent local import uniqueness",
    ] {
        assert!(sql.contains(required), "migration is missing {required}");
    }
}

struct PolicyTrackFixture<'a> {
    title: String,
    visibility: &'a str,
    ingest_status: &'a str,
    owner_profile_id: Option<&'a str>,
    composition_license_id: Option<&'a str>,
    recording_license_id: Option<&'a str>,
}

async fn insert_policy_track(
    pool: &sqlx::PgPool,
    artist_id: &str,
    album_id: &str,
    track: PolicyTrackFixture<'_>,
) -> String {
    let track_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO tracks (
                title, artist_id, album_id, duration_ms,
                visibility, ingest_status, owner_profile_id,
                composition_license_id, recording_license_id
            )
            VALUES (
                $1, $2::uuid, $3::uuid, 180000,
                $4, $5, $6::uuid, $7::uuid, $8::uuid
            )
            RETURNING id::text
        "#,
    )
    .bind(&track.title)
    .bind(artist_id)
    .bind(album_id)
    .bind(track.visibility)
    .bind(track.ingest_status)
    .bind(track.owner_profile_id)
    .bind(track.composition_license_id)
    .bind(track.recording_license_id)
    .fetch_one(pool)
    .await
    .expect("policy track should insert");

    sqlx::query(
        r#"
            INSERT INTO audio_assets (
                track_id, codec, content_type, storage_key,
                size_bytes, checksum_sha256, duration_ms
            )
            VALUES ($1::uuid, 'mp3', 'audio/mpeg', $2, 1024, $3, 180000)
        "#,
    )
    .bind(&track_id)
    .bind(format!("audio/{track_id}.mp3"))
    .bind(track_id.replace('-', "").repeat(2))
    .execute(pool)
    .await
    .expect("policy asset should insert");

    track_id
}

#[tokio::test]
async fn postgres_catalog_scopes_and_license_revocation_are_enforced() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let profiles = PgProfileRepository::new(pool.clone());
    let settings = PgInstanceSettingsRepository::new(pool.clone());
    let owner_a = profiles
        .upsert_profile(&format!("owner-a-{}", uuid::Uuid::new_v4()), None, false)
        .await
        .unwrap();
    let owner_b = profiles
        .upsert_profile(&format!("owner-b-{}", uuid::Uuid::new_v4()), None, false)
        .await
        .unwrap();
    settings.set_owner_profile_id(&owner_a.id).await.unwrap();
    assert_eq!(
        settings.owner_profile_id().await.unwrap(),
        Some(owner_a.id.clone())
    );

    let unique = uuid::Uuid::new_v4();
    let artist_id: String = sqlx::query_scalar(
        "INSERT INTO artists (name, sort_name) VALUES ($1, $1) RETURNING id::text",
    )
    .bind(format!("Policy Artist {unique}"))
    .fetch_one(&pool)
    .await
    .unwrap();
    let album_id: String = sqlx::query_scalar(
        "INSERT INTO albums (title, artist_id) VALUES ($1, $2::uuid) RETURNING id::text",
    )
    .bind(format!("Policy Album {unique}"))
    .bind(&artist_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let approved_license: String = sqlx::query_scalar(
        r#"
            INSERT INTO licenses (license_type, source_url, review_status, reviewed_at)
            VALUES ('CC0', $1, 'approved', NOW())
            RETURNING id::text
        "#,
    )
    .bind(format!("https://license.test/approved/{unique}"))
    .fetch_one(&pool)
    .await
    .unwrap();
    let pending_license: String = sqlx::query_scalar(
        r#"
            INSERT INTO licenses (license_type, source_url)
            VALUES ('pending', $1)
            RETURNING id::text
        "#,
    )
    .bind(format!("https://license.test/pending/{unique}"))
    .fetch_one(&pool)
    .await
    .unwrap();

    let public_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Access Matrix {unique}"),
            visibility: "release_safe",
            ingest_status: "ready",
            owner_profile_id: None,
            composition_license_id: Some(&approved_license),
            recording_license_id: Some(&approved_license),
        },
    )
    .await;
    let personal_a_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Access Matrix {unique}"),
            visibility: "personal",
            ingest_status: "ready",
            owner_profile_id: Some(&owner_a.id),
            composition_license_id: None,
            recording_license_id: None,
        },
    )
    .await;
    let personal_b_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Access Matrix {unique}"),
            visibility: "personal",
            ingest_status: "ready",
            owner_profile_id: Some(&owner_b.id),
            composition_license_id: None,
            recording_license_id: None,
        },
    )
    .await;
    let owner_pending_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Access Matrix {unique}"),
            visibility: "personal",
            ingest_status: "pending",
            owner_profile_id: Some(&owner_a.id),
            composition_license_id: None,
            recording_license_id: None,
        },
    )
    .await;
    let quarantined_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Access Matrix {unique}"),
            visibility: "quarantined",
            ingest_status: "quarantined",
            owner_profile_id: None,
            composition_license_id: None,
            recording_license_id: None,
        },
    )
    .await;
    let owner_scope = TrackAccessScope::Owner {
        profile_id: owner_a.id.clone(),
    };

    let catalog = PgCatalogRepository::new(pool.clone());
    let assets = PgAudioAssetRepository::new(pool.clone());
    let public = catalog
        .browse(
            &TrackAccessScope::Public,
            None,
            &[],
            Page {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert!(public.items.iter().any(|item| item.id == public_id));
    assert!(!public.items.iter().any(|item| item.id == personal_a_id));
    let public_search = catalog
        .search(
            &TrackAccessScope::Public,
            &format!("Access Matrix {unique}"),
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(public_search.total_count, 1);
    assert_eq!(
        public_search
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![public_id.as_str()]
    );
    let owner_catalog = catalog
        .browse(
            &owner_scope,
            None,
            &[],
            Page {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert!(owner_catalog.items.iter().any(|item| item.id == public_id));
    assert!(
        owner_catalog
            .items
            .iter()
            .any(|item| item.id == personal_a_id)
    );
    assert!(
        !owner_catalog
            .items
            .iter()
            .any(|item| item.id == personal_b_id)
    );
    assert!(
        !owner_catalog
            .items
            .iter()
            .any(|item| item.id == owner_pending_id)
    );
    assert!(
        !owner_catalog
            .items
            .iter()
            .any(|item| item.id == quarantined_id)
    );

    let public_discovery = catalog
        .shuffle_pool(&TrackAccessScope::Public)
        .await
        .unwrap();
    assert!(public_discovery.iter().any(|item| item.id == public_id));
    assert!(!public_discovery.iter().any(|item| item.id == personal_a_id));

    let owner_discovery = catalog.shuffle_pool(&owner_scope).await.unwrap();
    assert!(owner_discovery.iter().any(|item| item.id == public_id));
    assert!(owner_discovery.iter().any(|item| item.id == personal_a_id));
    assert!(!owner_discovery.iter().any(|item| item.id == personal_b_id));
    assert!(
        !owner_discovery
            .iter()
            .any(|item| item.id == owner_pending_id)
    );
    assert!(!owner_discovery.iter().any(|item| item.id == quarantined_id));

    let owner_search = catalog
        .search(
            &owner_scope,
            &format!("Access Matrix {unique}"),
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    let mut expected_search_ids = vec![public_id.clone(), personal_a_id.clone()];
    expected_search_ids.sort();
    assert_eq!(
        owner_search
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        expected_search_ids
    );
    assert_eq!(owner_search.total_count, 2);
    assert!(
        catalog
            .get_media(&owner_scope, &personal_a_id)
            .await
            .unwrap()
            .is_some()
    );
    let owner_a_page = catalog
        .list_personal(
            &owner_a.id,
            Page {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert!(
        owner_a_page
            .items
            .iter()
            .any(|item| item.id == personal_a_id)
    );
    assert!(
        !owner_a_page
            .items
            .iter()
            .any(|item| item.id == personal_b_id)
    );
    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, &personal_a_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        assets
            .assets_for_public_track(&personal_a_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        assets
            .assets_for_personal_track(&owner_a.id, &personal_a_id)
            .await
            .unwrap()
            .len(),
        1
    );
    let stream_assets = PgPlayableAssetRepository::new(pool.clone());
    let public_playable = stream_assets
        .assets_for_public_playback(&public_id)
        .await
        .unwrap();
    assert_eq!(public_playable.len(), 1);
    assert!(
        stream_assets
            .assets_for_public_playback(&personal_a_id)
            .await
            .unwrap()
            .is_empty()
    );

    let personal_playable = stream_assets
        .assets_for_personal_playback(&owner_a.id, &personal_a_id)
        .await
        .unwrap();
    assert_eq!(personal_playable.len(), 1);
    assert!(
        stream_assets
            .assets_for_personal_playback(&owner_b.id, &personal_a_id)
            .await
            .unwrap()
            .is_empty()
    );

    let public_asset_id = public_playable[0].asset_id.clone();
    let personal_asset_id: String =
        sqlx::query_scalar("SELECT id::text FROM audio_assets WHERE track_id = $1::uuid")
            .bind(&personal_a_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(
        stream_assets
            .authorize_stream_asset(&public_asset_id, StreamAudience::Public)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        stream_assets
            .authorize_stream_asset(&public_asset_id, StreamAudience::Personal)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        stream_assets
            .authorize_stream_asset(&personal_asset_id, StreamAudience::Personal)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        stream_assets
            .authorize_stream_asset(&personal_asset_id, StreamAudience::Public)
            .await
            .unwrap()
            .is_none()
    );
    settings.set_owner_profile_id(&owner_b.id).await.unwrap();
    assert!(
        stream_assets
            .authorize_stream_asset(&personal_asset_id, StreamAudience::Personal)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        stream_assets
            .authorize_stream_asset(
                "018f0000-0000-7000-8000-000000000099",
                StreamAudience::Public,
            )
            .await
            .unwrap()
            .is_none()
    );

    sqlx::query(
        "UPDATE tracks SET visibility = 'quarantined', ingest_status = 'quarantined' WHERE id = $1::uuid",
    )
    .bind(&public_id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        stream_assets
            .authorize_stream_asset(&public_asset_id, StreamAudience::Public)
            .await
            .unwrap()
            .is_none()
    );

    let promotable_id = insert_policy_track(
        &pool,
        &artist_id,
        &album_id,
        PolicyTrackFixture {
            title: format!("Promotable Policy Track {unique}"),
            visibility: "quarantined",
            ingest_status: "quarantined",
            owner_profile_id: None,
            composition_license_id: Some(&pending_license),
            recording_license_id: Some(&approved_license),
        },
    )
    .await;
    let rejected = sqlx::query(
        "UPDATE tracks SET visibility = 'release_safe', ingest_status = 'ready' WHERE id = $1::uuid",
    )
    .bind(&promotable_id)
    .execute(&pool)
    .await;
    assert!(rejected.is_err());

    sqlx::query(
        "UPDATE licenses SET review_status = 'approved', reviewed_at = NOW() WHERE id = $1::uuid",
    )
    .bind(&pending_license)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE tracks SET visibility = 'release_safe', ingest_status = 'ready' WHERE id = $1::uuid")
        .bind(&promotable_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE licenses SET review_status = 'rejected' WHERE id = $1::uuid")
        .bind(&pending_license)
        .execute(&pool)
        .await
        .unwrap();
    let state: (String, String) =
        sqlx::query_as("SELECT visibility, ingest_status FROM tracks WHERE id = $1::uuid")
            .bind(&promotable_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        state,
        ("quarantined".to_string(), "quarantined".to_string())
    );
}

#[tokio::test]
async fn postgres_migrations_support_profile_scoped_history() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let profiles = PgProfileRepository::new(pool.clone());
    let history = PgPlaybackHistoryRepository::new(pool.clone());
    let external_user_id = format!("history-user-{}", uuid::Uuid::new_v4());
    let profile = profiles
        .upsert_profile(&external_user_id, Some("Ada"), true)
        .await
        .expect("profile should be created");

    let artist_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO artists (name, sort_name)
            VALUES ($1, $2)
            RETURNING id::text
        "#,
    )
    .bind(format!("History Artist {external_user_id}"))
    .bind(format!("history artist {external_user_id}"))
    .fetch_one(&pool)
    .await
    .expect("artist should be inserted");

    let album_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO albums (title, artist_id)
            VALUES ($1, $2::uuid)
            RETURNING id::text
        "#,
    )
    .bind(format!("History Album {external_user_id}"))
    .bind(&artist_id)
    .fetch_one(&pool)
    .await
    .expect("album should be inserted");

    let track_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO tracks (
                title, artist_id, album_id, duration_ms,
                visibility, ingest_status, owner_profile_id
            )
            VALUES ($1, $2::uuid, $3::uuid, 1000, 'personal', 'ready', $4::uuid)
            RETURNING id::text
        "#,
    )
    .bind(format!("History Track {external_user_id}"))
    .bind(&artist_id)
    .bind(&album_id)
    .bind(&profile.id)
    .fetch_one(&pool)
    .await
    .expect("track should be inserted");
    let owner_scope = TrackAccessScope::Owner {
        profile_id: profile.id.clone(),
    };

    assert!(
        history
            .record(
                PlaybackHistoryEvent {
                    profile_id: profile.id.clone(),
                    track_id: track_id.clone(),
                    duration_ms: 1000,
                    completion_pct: 0.75,
                },
                &owner_scope
            )
            .await
            .expect("history should be recorded")
    );

    let count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM playback_history
            WHERE profile_id = $1::uuid AND track_id = $2::uuid
        "#,
    )
    .bind(&profile.id)
    .bind(&track_id)
    .fetch_one(&pool)
    .await
    .expect("history count should be queryable");

    assert_eq!(count, 1);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE profiles SET history_enabled = FALSE WHERE id = $1::uuid")
        .bind(&profile.id)
        .execute(&mut *tx)
        .await
        .unwrap();

    let purged_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM playback_history WHERE profile_id = $1::uuid")
            .bind(&profile.id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(purged_count, 0);

    tx.rollback().await.unwrap();
    let restored_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM playback_history WHERE profile_id = $1::uuid")
            .bind(&profile.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(restored_count, 1);

    assert!(
        history
            .record(
                PlaybackHistoryEvent {
                    profile_id: profile.id.clone(),
                    track_id: track_id.clone(),
                    duration_ms: 500,
                    completion_pct: 0.5,
                },
                &owner_scope
            )
            .await
            .unwrap()
    );
    let other_profile = profiles
        .upsert_profile(
            &format!("other-history-user-{}", uuid::Uuid::new_v4()),
            Some("Grace"),
            true,
        )
        .await
        .unwrap();
    assert!(
        history
            .record(
                PlaybackHistoryEvent {
                    profile_id: other_profile.id.clone(),
                    track_id: track_id.clone(),
                    duration_ms: 250,
                    completion_pct: 0.25,
                },
                &owner_scope
            )
            .await
            .unwrap()
    );

    let page = history
        .list(
            &profile.id,
            &owner_scope,
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.total_count, 2);
    assert_eq!(page.entries.len(), 2);
    assert!(page.entries[0].played_at_epoch_ms >= page.entries[1].played_at_epoch_ms);
    assert_eq!(page.entries[0].item.id, track_id);

    assert!(
        !history
            .delete_entry(&other_profile.id, &page.entries[0].id)
            .await
            .unwrap()
    );
    assert!(
        history
            .delete_entry(&profile.id, &page.entries[0].id)
            .await
            .unwrap()
    );
    assert!(
        !history
            .delete_entry(&profile.id, &page.entries[0].id)
            .await
            .unwrap()
    );
    assert_eq!(history.clear(&profile.id).await.unwrap(), 1);

    profiles
        .upsert_profile(&profile.external_user_id, Some("Ada"), false)
        .await
        .unwrap();
    assert!(
        !history
            .record(
                PlaybackHistoryEvent {
                    profile_id: profile.id.clone(),
                    track_id: track_id.clone(),
                    duration_ms: 1000,
                    completion_pct: 1.0,
                },
                &owner_scope
            )
            .await
            .unwrap()
    );
    let disabled_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM playback_history WHERE profile_id = $1::uuid")
            .bind(&profile.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(disabled_count, 0);

    let _ = sqlx::query("DELETE FROM profiles WHERE id = $1::uuid")
        .bind(&other_profile.id)
        .execute(&pool)
        .await;

    let _ = sqlx::query("DELETE FROM profiles WHERE id = $1::uuid")
        .bind(&profile.id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM tracks WHERE id = $1::uuid")
        .bind(&track_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM albums WHERE id = $1::uuid")
        .bind(&album_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM artists WHERE id = $1::uuid")
        .bind(&artist_id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn postgres_migrations_support_profile_library_likes_preferences_schema() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let table_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name IN (
                  'profile_library_items',
                  'profile_track_likes',
                  'profile_preferences'
              )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("table count should be queryable");
    assert_eq!(table_count, 3);

    let profile_fk_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM information_schema.constraint_column_usage
            WHERE table_name = 'profiles'
              AND constraint_name IN (
                  'profile_library_items_profile_id_fkey',
                  'profile_track_likes_profile_id_fkey',
                  'profile_preferences_profile_id_fkey'
              )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("foreign key count should be queryable");
    assert_eq!(profile_fk_count, 3);

    let index_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM pg_indexes
            WHERE schemaname = 'public'
              AND indexname IN (
                  'idx_profile_library_items_profile_added_at',
                  'idx_profile_library_items_track_id',
                  'idx_profile_track_likes_profile_liked_at',
                  'idx_profile_track_likes_track_id'
              )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("index count should be queryable");
    assert_eq!(index_count, 4);
}

#[tokio::test]
async fn postgres_migrations_support_profile_owned_playlists_schema() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let table_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM information_schema.tables
            WHERE table_schema = 'public'
              AND table_name IN ('profile_playlists', 'profile_playlist_tracks')
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("table count should be queryable");
    assert_eq!(table_count, 2);

    let fk_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM information_schema.table_constraints
            WHERE constraint_schema = 'public'
              AND constraint_type = 'FOREIGN KEY'
              AND constraint_name IN (
                  'profile_playlists_profile_id_fkey',
                  'profile_playlist_tracks_playlist_id_fkey',
                  'profile_playlist_tracks_track_id_fkey'
              )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("foreign key count should be queryable");
    assert_eq!(fk_count, 3);

    let index_count: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM pg_indexes
            WHERE schemaname = 'public'
              AND indexname IN (
                  'idx_profile_playlists_profile_updated_at',
                  'idx_profile_playlist_tracks_playlist_position',
                  'idx_profile_playlist_tracks_track_id'
              )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("index count should be queryable");
    assert_eq!(index_count, 3);
}
#[tokio::test]
async fn postgres_repositories_support_profile_library_likes_preferences() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let profiles = PgProfileRepository::new(pool.clone());
    let library = PgLibraryRepository::new(pool.clone());
    let likes = PgLikeRepository::new(pool.clone());
    let preferences = PgPreferencesRepository::new(pool.clone());
    let external_user_id = format!("library-user-{}", uuid::Uuid::new_v4());
    let profile = profiles
        .upsert_profile(&external_user_id, Some("Ada"), true)
        .await
        .expect("profile should be created");

    let artist_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO artists (name, sort_name)
            VALUES ($1, $2)
            RETURNING id::text
        "#,
    )
    .bind(format!("Library Artist {external_user_id}"))
    .bind(format!("library artist {external_user_id}"))
    .fetch_one(&pool)
    .await
    .expect("artist should be inserted");

    let album_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO albums (title, artist_id)
            VALUES ($1, $2::uuid)
            RETURNING id::text
        "#,
    )
    .bind(format!("Library Album {external_user_id}"))
    .bind(&artist_id)
    .fetch_one(&pool)
    .await
    .expect("album should be inserted");

    let track_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO tracks (
                title, artist_id, album_id, duration_ms,
                visibility, ingest_status, owner_profile_id
            )
            VALUES ($1, $2::uuid, $3::uuid, 1000, 'personal', 'ready', $4::uuid)
            RETURNING id::text
        "#,
    )
    .bind(format!("Library Track {external_user_id}"))
    .bind(&artist_id)
    .bind(&album_id)
    .bind(&profile.id)
    .fetch_one(&pool)
    .await
    .expect("track should be inserted");
    let owner_scope = TrackAccessScope::Owner {
        profile_id: profile.id.clone(),
    };

    let saved = library
        .save_track(&profile.id, &track_id, &owner_scope)
        .await
        .expect("save should work");
    assert_eq!(saved.profile_id, profile.id);
    assert_eq!(saved.track_id, track_id);
    let saved_again = library
        .save_track(&profile.id, &track_id, &owner_scope)
        .await
        .expect("resave should work");
    assert_eq!(saved_again.track_id, track_id);
    assert!(
        library
            .is_saved(&profile.id, &track_id, &owner_scope)
            .await
            .expect("saved flag should work")
    );
    assert_eq!(
        library
            .list_tracks(
                &profile.id,
                &owner_scope,
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .expect("library list should work")
            .items
            .len(),
        1
    );
    library
        .remove_track(&profile.id, &track_id)
        .await
        .expect("remove should work");
    library
        .remove_track(&profile.id, &track_id)
        .await
        .expect("second remove should be idempotent");
    assert!(
        !library
            .is_saved(&profile.id, &track_id, &owner_scope)
            .await
            .expect("saved flag should work after remove")
    );

    let liked = likes
        .like_track(&profile.id, &track_id, &owner_scope)
        .await
        .expect("like should work");
    assert_eq!(liked.profile_id, profile.id);
    assert_eq!(liked.track_id, track_id);
    likes
        .like_track(&profile.id, &track_id, &owner_scope)
        .await
        .expect("relike should be idempotent");
    assert!(
        likes
            .is_liked(&profile.id, &track_id, &owner_scope)
            .await
            .expect("liked flag should work")
    );
    assert_eq!(
        likes
            .list_liked_tracks(
                &profile.id,
                &owner_scope,
                Page {
                    limit: 10,
                    offset: 0,
                },
            )
            .await
            .expect("liked list should work")
            .items
            .len(),
        1
    );
    likes
        .unlike_track(&profile.id, &track_id)
        .await
        .expect("unlike should work");
    likes
        .unlike_track(&profile.id, &track_id)
        .await
        .expect("second unlike should be idempotent");
    assert!(
        !likes
            .is_liked(&profile.id, &track_id, &owner_scope)
            .await
            .expect("liked flag should work after unlike")
    );

    let prefs = preferences
        .upsert_preferences(
            &profile.id,
            r#"{"explicit_content":false,"preferred_codecs":["opus","mp4"]}"#,
        )
        .await
        .expect("preferences should save");
    assert_eq!(prefs.profile_id, profile.id);
    assert!(prefs.values_json.contains("preferred_codecs"));
    let fetched = preferences
        .get_preferences(&profile.id)
        .await
        .expect("preferences should load");
    assert_eq!(fetched.values_json, prefs.values_json);

    let _ = sqlx::query("DELETE FROM profiles WHERE id = $1::uuid")
        .bind(&profile.id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM tracks WHERE id = $1::uuid")
        .bind(&track_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM albums WHERE id = $1::uuid")
        .bind(&album_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM artists WHERE id = $1::uuid")
        .bind(&artist_id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn postgres_repositories_support_profile_owned_playlists() {
    let pool = connect_test_pool().await;

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let profiles = PgProfileRepository::new(pool.clone());
    let playlists = PgPlaylistRepository::new(pool.clone());
    let unique = uuid::Uuid::new_v4();
    let profile = profiles
        .upsert_profile(&format!("playlist-user-{unique}"), Some("Ada"), true)
        .await
        .expect("profile should be created");
    let other_profile = profiles
        .upsert_profile(&format!("playlist-other-{unique}"), Some("Grace"), true)
        .await
        .expect("other profile should be created");

    let artist_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO artists (name, sort_name)
            VALUES ($1, $2)
            RETURNING id::text
        "#,
    )
    .bind(format!("Playlist Artist {unique}"))
    .bind(format!("playlist artist {unique}"))
    .fetch_one(&pool)
    .await
    .expect("artist should be inserted");

    let album_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO albums (title, artist_id)
            VALUES ($1, $2::uuid)
            RETURNING id::text
        "#,
    )
    .bind(format!("Playlist Album {unique}"))
    .bind(&artist_id)
    .fetch_one(&pool)
    .await
    .expect("album should be inserted");

    let track_id: String = sqlx::query_scalar(
        r#"
            INSERT INTO tracks (title, artist_id, album_id, duration_ms)
            VALUES ($1, $2::uuid, $3::uuid, 1000)
            RETURNING id::text
        "#,
    )
    .bind(format!("Playlist Track A {unique}"))
    .bind(&artist_id)
    .bind(&album_id)
    .fetch_one(&pool)
    .await
    .expect("first track should be inserted");

    let track_id_2: String = sqlx::query_scalar(
        r#"
            INSERT INTO tracks (title, artist_id, album_id, duration_ms)
            VALUES ($1, $2::uuid, $3::uuid, 2000)
            RETURNING id::text
        "#,
    )
    .bind(format!("Playlist Track B {unique}"))
    .bind(&artist_id)
    .bind(&album_id)
    .fetch_one(&pool)
    .await
    .expect("second track should be inserted");

    for track_id in [&track_id, &track_id_2] {
        sqlx::query(
            "UPDATE tracks SET visibility = 'personal', ingest_status = 'ready', owner_profile_id = $2::uuid WHERE id = $1::uuid",
        )
        .bind(track_id)
        .bind(&profile.id)
        .execute(&pool)
        .await
        .expect("playlist track should become owner-visible");
    }
    let owner_scope = TrackAccessScope::Owner {
        profile_id: profile.id.clone(),
    };

    let created = playlists
        .create_playlist(&profile.id, "Road Mix", "For drives")
        .await
        .expect("playlist should be created");
    assert_eq!(created.name, "Road Mix");

    let updated = playlists
        .update_playlist(&profile.id, &created.id, "Night Drive", "Late routes")
        .await
        .expect("playlist should update");
    assert_eq!(updated.description, "Late routes");

    let playlist_page = playlists
        .list_playlists(
            &profile.id,
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("playlists should list");
    assert_eq!(playlist_page.total_count, 1);

    playlists
        .add_track(&profile.id, &created.id, &track_id, None, &owner_scope)
        .await
        .expect("track should add");
    playlists
        .add_track(&profile.id, &created.id, &track_id, None, &owner_scope)
        .await
        .expect("duplicate track add should be idempotent");
    playlists
        .add_track(&profile.id, &created.id, &track_id_2, Some(0), &owner_scope)
        .await
        .expect("second track should add");

    let page = playlists
        .list_tracks(
            &profile.id,
            &created.id,
            &owner_scope,
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("playlist tracks should list");
    assert_eq!(page.total_count, 2);

    playlists
        .reorder_tracks(
            &profile.id,
            &created.id,
            &[track_id.clone(), track_id_2.clone()],
            &owner_scope,
        )
        .await
        .expect("playlist tracks should reorder");

    playlists
        .remove_track(&profile.id, &created.id, &track_id_2)
        .await
        .expect("track should remove");
    playlists
        .remove_track(&profile.id, &created.id, &track_id_2)
        .await
        .expect("second remove should be idempotent");

    let other_profile_result = playlists
        .list_tracks(
            &other_profile.id,
            &created.id,
            &TrackAccessScope::Public,
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await;
    assert!(matches!(
        other_profile_result,
        Err(canopy_core::CanopyError::NotFound { .. })
    ));

    playlists
        .delete_playlist(&profile.id, &created.id)
        .await
        .expect("playlist should delete");
    let deleted = playlists
        .delete_playlist(&profile.id, &created.id)
        .await
        .unwrap_err();
    assert!(matches!(deleted, canopy_core::CanopyError::NotFound { .. }));

    let _ = sqlx::query("DELETE FROM profiles WHERE id = $1::uuid")
        .bind(&profile.id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM profiles WHERE id = $1::uuid")
        .bind(&other_profile.id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM tracks WHERE id IN ($1::uuid, $2::uuid)")
        .bind(&track_id)
        .bind(&track_id_2)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM albums WHERE id = $1::uuid")
        .bind(&album_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM artists WHERE id = $1::uuid")
        .bind(&artist_id)
        .execute(&pool)
        .await;
}
fn pending_local_import(owner_profile_id: &str, checksum: &str) -> PendingMediaImport {
    let track_id = uuid::Uuid::new_v4().to_string();
    let checksum = checksum.to_ascii_lowercase();
    PendingMediaImport {
        track_id: track_id.clone(),
        owner_profile_id: owner_profile_id.to_string(),
        title: "Imported Test Tone".into(),
        artist: "Local Import Artist".into(),
        album: "Local Import Album".into(),
        duration_ms: 1_000,
        artwork_storage_key: Some(format!("artwork/aa/bb/{checksum}.jpg")),
        audio: AudioAsset {
            track_id,
            codec: "mp3".into(),
            content_type: "audio/mpeg".into(),
            storage_key: format!(
                "audio/{}/{}/{}.mp3",
                &checksum[0..2],
                &checksum[2..4],
                checksum
            ),
            size_bytes: 1_024,
            checksum_sha256: checksum,
            duration_ms: 1_000,
        },
    }
}

#[tokio::test]
async fn postgres_local_import_is_deduplicated_and_hidden_until_ready() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");

    let profiles = PgProfileRepository::new(pool.clone());
    let owner = profiles
        .upsert_profile(
            &format!("local-import-owner-{}", uuid::Uuid::new_v4()),
            None,
            false,
        )
        .await
        .expect("owner profile should be created");
    let repository = PgMediaImportRepository::new(pool.clone());
    let catalog = PgCatalogRepository::new(pool.clone());
    let checksum = uuid::Uuid::new_v4().simple().to_string().repeat(2);
    let pending = pending_local_import(&owner.id, &checksum);

    assert_eq!(
        repository
            .find_track_by_audio_checksum(&checksum.to_ascii_uppercase())
            .await
            .expect("checksum lookup should succeed"),
        None
    );
    assert_eq!(
        repository
            .insert_pending(&pending)
            .await
            .expect("pending import should insert"),
        PendingImportOutcome::Inserted
    );

    let stored: (String, String, String, String, Option<String>, String) = sqlx::query_as(
        r#"
            SELECT t.owner_profile_id::text, t.visibility, t.ingest_status,
                   t.ingest_source, t.artwork_storage_key, aa.storage_key
            FROM tracks t
            JOIN audio_assets aa ON aa.track_id = t.id
            WHERE t.id = $1::uuid
        "#,
    )
    .bind(&pending.track_id)
    .fetch_one(&pool)
    .await
    .expect("pending import should be stored");
    assert_eq!(stored.0, owner.id);
    assert_eq!(
        (&stored.1, &stored.2, &stored.3),
        (&"personal".into(), &"pending".into(), &"local_admin".into())
    );
    assert_eq!(stored.4, pending.artwork_storage_key);
    assert_eq!(stored.5, pending.audio.storage_key);

    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, &pending.track_id)
            .await
            .unwrap()
            .is_none()
    );
    let owner_scope = TrackAccessScope::Owner {
        profile_id: owner.id.clone(),
    };
    assert!(
        catalog
            .get_media(&owner_scope, &pending.track_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repository
            .find_track_by_audio_checksum(&checksum.to_ascii_uppercase())
            .await
            .unwrap(),
        Some(pending.track_id.clone())
    );

    repository
        .mark_ready(&pending.track_id)
        .await
        .expect("pending import should become ready");
    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, &pending.track_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        catalog
            .get_media(&owner_scope, &pending.track_id)
            .await
            .unwrap()
            .is_some()
    );

    let duplicate = pending_local_import(&owner.id, &checksum.to_ascii_uppercase());
    assert_eq!(
        repository
            .insert_pending(&duplicate)
            .await
            .expect("duplicate checksum should resolve cleanly"),
        PendingImportOutcome::Duplicate {
            track_id: pending.track_id.clone()
        }
    );
    let checksum_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audio_assets WHERE LOWER(checksum_sha256) = LOWER($1)",
    )
    .bind(&checksum)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(checksum_count, 1);

    sqlx::query("DELETE FROM tracks WHERE id = $1::uuid")
        .bind(&pending.track_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM profiles WHERE id = $1::uuid")
        .bind(&owner.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn postgres_local_import_rejects_unknown_owner() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");
    let repository = PgMediaImportRepository::new(pool.clone());
    let checksum = uuid::Uuid::new_v4().simple().to_string().repeat(2);
    let pending = pending_local_import(&uuid::Uuid::new_v4().to_string(), &checksum);

    let error = repository.insert_pending(&pending).await.unwrap_err();

    assert!(matches!(error, canopy_core::CanopyError::Storage(_)));
    let track_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tracks WHERE id = $1::uuid")
        .bind(&pending.track_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(track_count, 0);
}
async fn reset_auth_outbox(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM auth_outbox")
        .execute(pool)
        .await
        .expect("auth outbox should reset");
}

async fn insert_auth_outbox(pool: &sqlx::PgPool, seed: u8) -> String {
    sqlx::query_scalar::<_, String>(
        r#"
            INSERT INTO auth_outbox (kind, encrypted_payload, key_id)
            VALUES ('email_verification', $1, 'test-key')
            RETURNING id::text
        "#,
    )
    .bind(vec![seed; 32])
    .fetch_one(pool)
    .await
    .expect("auth outbox fixture should insert")
}

fn claim_auth_outbox(
    now_epoch_ms: u64,
    lease_token: &str,
    batch_size: u32,
) -> ClaimAuthOutboxBatch {
    ClaimAuthOutboxBatch {
        now_epoch_ms,
        lease_expires_at_epoch_ms: now_epoch_ms + 1_000,
        batch_size,
        lease_token: lease_token.into(),
    }
}

#[tokio::test]
async fn postgres_auth_outbox_claims_each_row_once_across_workers() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");
    reset_auth_outbox(&pool).await;
    let first_id = insert_auth_outbox(&pool, 61).await;
    let second_id = insert_auth_outbox(&pool, 62).await;
    let first = PgAuthOutboxRepository::new(pool.clone());
    let second = PgAuthOutboxRepository::new(pool.clone());
    let now = identity_epoch_ms(1_000);

    let (first_claim, second_claim) = tokio::join!(
        first.claim_auth_outbox_batch(claim_auth_outbox(
            now,
            "00000000-0000-0000-0000-000000000061",
            1,
        )),
        second.claim_auth_outbox_batch(claim_auth_outbox(
            now,
            "00000000-0000-0000-0000-000000000062",
            1,
        )),
    );
    let first_claim = first_claim.expect("first worker should claim");
    let second_claim = second_claim.expect("second worker should claim");

    assert_eq!(first_claim.len(), 1);
    assert_eq!(second_claim.len(), 1);
    assert_ne!(first_claim[0].id, second_claim[0].id);
    let claimed = [first_claim[0].id.as_str(), second_claim[0].id.as_str()];
    assert!(claimed.contains(&first_id.as_str()));
    assert!(claimed.contains(&second_id.as_str()));
}

#[tokio::test]
async fn postgres_auth_outbox_expired_lease_can_be_reclaimed() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");
    reset_auth_outbox(&pool).await;
    let id = insert_auth_outbox(&pool, 63).await;
    let repository = PgAuthOutboxRepository::new(pool);
    let now = identity_epoch_ms(1_000);

    let first = repository
        .claim_auth_outbox_batch(claim_auth_outbox(
            now,
            "00000000-0000-0000-0000-000000000063",
            1,
        ))
        .await
        .expect("first lease should claim");
    let second = repository
        .claim_auth_outbox_batch(claim_auth_outbox(
            now + 2_000,
            "00000000-0000-0000-0000-000000000064",
            1,
        ))
        .await
        .expect("expired lease should be reclaimed");

    assert_eq!(first[0].id, id);
    assert_eq!(second[0].id, id);
    assert_eq!(second[0].attempts, 2);
    assert_eq!(
        second[0].lease_token,
        "00000000-0000-0000-0000-000000000064"
    );
}

#[tokio::test]
async fn postgres_auth_outbox_stale_lease_cannot_complete_reclaimed_row() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");
    reset_auth_outbox(&pool).await;
    let id = insert_auth_outbox(&pool, 65).await;
    let repository = PgAuthOutboxRepository::new(pool);
    let now = identity_epoch_ms(1_000);
    let stale_token = "00000000-0000-0000-0000-000000000065";
    let active_token = "00000000-0000-0000-0000-000000000066";

    repository
        .claim_auth_outbox_batch(claim_auth_outbox(now, stale_token, 1))
        .await
        .expect("first lease should claim");
    repository
        .claim_auth_outbox_batch(claim_auth_outbox(now + 2_000, active_token, 1))
        .await
        .expect("second lease should reclaim");

    assert!(
        !repository
            .mark_auth_outbox_delivered(&id, stale_token, now + 2_100)
            .await
            .expect("stale completion should be checked")
    );
    assert!(
        repository
            .mark_auth_outbox_delivered(&id, active_token, now + 2_100)
            .await
            .expect("active completion should succeed")
    );
}

#[tokio::test]
async fn postgres_auth_outbox_success_clears_ciphertext() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");
    reset_auth_outbox(&pool).await;
    let id = insert_auth_outbox(&pool, 67).await;
    let repository = PgAuthOutboxRepository::new(pool.clone());
    let now = identity_epoch_ms(1_000);
    let lease_token = "00000000-0000-0000-0000-000000000067";

    repository
        .claim_auth_outbox_batch(claim_auth_outbox(now, lease_token, 1))
        .await
        .expect("row should be claimed");
    assert!(
        repository
            .mark_auth_outbox_delivered(&id, lease_token, now + 100)
            .await
            .expect("delivery should update")
    );

    let row = sqlx::query(
        "SELECT encrypted_payload, delivered_at IS NOT NULL AS delivered FROM auth_outbox WHERE id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .expect("delivered row should exist");
    assert!(
        row.try_get::<Option<Vec<u8>>, _>("encrypted_payload")
            .unwrap()
            .is_none()
    );
    assert!(row.try_get::<bool, _>("delivered").unwrap());
}

#[tokio::test]
async fn postgres_auth_outbox_retry_and_exhaustion_leave_safe_state() {
    let pool = connect_test_pool().await;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("migrations should apply cleanly");
    reset_auth_outbox(&pool).await;
    let id = insert_auth_outbox(&pool, 68).await;
    let repository = PgAuthOutboxRepository::new(pool.clone());
    let now = identity_epoch_ms(1_000);
    let first_token = "00000000-0000-0000-0000-000000000068";
    let final_token = "00000000-0000-0000-0000-000000000069";

    repository
        .claim_auth_outbox_batch(claim_auth_outbox(now, first_token, 1))
        .await
        .expect("row should be claimed");
    assert!(
        repository
            .mark_auth_outbox_failed(MarkAuthOutboxFailed {
                id: id.clone(),
                lease_token: first_token.into(),
                error_kind: AuthOutboxFailureKind::Connection,
                available_at_epoch_ms: now + 2_000,
                failed_at_epoch_ms: None,
            })
            .await
            .expect("retry should update")
    );

    let reclaimed = repository
        .claim_auth_outbox_batch(claim_auth_outbox(now + 2_100, final_token, 1))
        .await
        .expect("retry should become available");
    assert_eq!(reclaimed[0].attempts, 2);
    assert!(
        repository
            .mark_auth_outbox_failed(MarkAuthOutboxFailed {
                id: id.clone(),
                lease_token: final_token.into(),
                error_kind: AuthOutboxFailureKind::Rejected,
                available_at_epoch_ms: now + 2_100,
                failed_at_epoch_ms: Some(now + 2_100),
            })
            .await
            .expect("terminal failure should update")
    );

    let exhausted = repository
        .claim_auth_outbox_batch(claim_auth_outbox(
            now + 10_000,
            "00000000-0000-0000-0000-000000000070",
            1,
        ))
        .await
        .expect("exhausted claim should be checked");
    assert!(exhausted.is_empty());

    let row = sqlx::query(
        "SELECT encrypted_payload IS NOT NULL AS sealed, failed_at IS NOT NULL AS failed, last_error_kind FROM auth_outbox WHERE id = $1::uuid",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .expect("exhausted row should exist");
    assert!(row.try_get::<bool, _>("sealed").unwrap());
    assert!(row.try_get::<bool, _>("failed").unwrap());
    assert_eq!(
        row.try_get::<String, _>("last_error_kind").unwrap(),
        "rejected"
    );
}
