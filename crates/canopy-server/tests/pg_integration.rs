#![cfg(feature = "pg")]

use canopy_core::{
    AudioAsset, AudioAssetRepository, CatalogIngest, CatalogRepository, InstanceSettingsRepository,
    LibraryRepository, LikeRepository, MediaImportRepository, Page, PendingImportOutcome,
    PendingMediaImport, PlayableAssetRepository, PlaybackHistoryEvent, PlaybackHistoryRepository,
    PlaylistRepository, PreferencesRepository, ProfileRepository, ProviderAudioAsset,
    ProviderLicense, ProviderTrack, StreamAudience,
};
use canopy_server::jade_store::{
    PgAudioAssetRepository, PgCatalogRepository, PgInstanceSettingsRepository, PgLibraryRepository,
    PgLikeRepository, PgMediaImportRepository, PgPlayableAssetRepository,
    PgPlaybackHistoryRepository, PgPlaylistRepository, PgPreferencesRepository,
    PgProfileRepository,
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
            .get_public_media(&first_id)
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
        .browse_public(
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
            title: format!("Public Policy Track {unique}"),
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
            title: format!("Owner A Policy Track {unique}"),
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
            title: format!("Owner B Policy Track {unique}"),
            visibility: "personal",
            ingest_status: "ready",
            owner_profile_id: Some(&owner_b.id),
            composition_license_id: None,
            recording_license_id: None,
        },
    )
    .await;

    let catalog = PgCatalogRepository::new(pool.clone());
    let assets = PgAudioAssetRepository::new(pool.clone());
    let public = catalog
        .browse_public(
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
            .get_public_media(&personal_a_id)
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
            INSERT INTO tracks (title, artist_id, album_id, duration_ms)
            VALUES ($1, $2::uuid, $3::uuid, 1000)
            RETURNING id::text
        "#,
    )
    .bind(format!("History Track {external_user_id}"))
    .bind(&artist_id)
    .bind(&album_id)
    .fetch_one(&pool)
    .await
    .expect("track should be inserted");

    assert!(
        history
            .record(PlaybackHistoryEvent {
                profile_id: profile.id.clone(),
                track_id: track_id.clone(),
                duration_ms: 1000,
                completion_pct: 0.75,
            })
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
            .record(PlaybackHistoryEvent {
                profile_id: profile.id.clone(),
                track_id: track_id.clone(),
                duration_ms: 500,
                completion_pct: 0.5,
            })
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
            .record(PlaybackHistoryEvent {
                profile_id: other_profile.id.clone(),
                track_id: track_id.clone(),
                duration_ms: 250,
                completion_pct: 0.25,
            })
            .await
            .unwrap()
    );

    let page = history
        .list(
            &profile.id,
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
            .record(PlaybackHistoryEvent {
                profile_id: profile.id.clone(),
                track_id: track_id.clone(),
                duration_ms: 1000,
                completion_pct: 1.0,
            })
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
            INSERT INTO tracks (title, artist_id, album_id, duration_ms)
            VALUES ($1, $2::uuid, $3::uuid, 1000)
            RETURNING id::text
        "#,
    )
    .bind(format!("Library Track {external_user_id}"))
    .bind(&artist_id)
    .bind(&album_id)
    .fetch_one(&pool)
    .await
    .expect("track should be inserted");

    let saved = library
        .save_track(&profile.id, &track_id)
        .await
        .expect("save should work");
    assert_eq!(saved.profile_id, profile.id);
    assert_eq!(saved.track_id, track_id);
    let saved_again = library
        .save_track(&profile.id, &track_id)
        .await
        .expect("resave should work");
    assert_eq!(saved_again.track_id, track_id);
    assert!(
        library
            .is_saved(&profile.id, &track_id)
            .await
            .expect("saved flag should work")
    );
    assert_eq!(
        library
            .list_tracks(
                &profile.id,
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
            .is_saved(&profile.id, &track_id)
            .await
            .expect("saved flag should work after remove")
    );

    let liked = likes
        .like_track(&profile.id, &track_id)
        .await
        .expect("like should work");
    assert_eq!(liked.profile_id, profile.id);
    assert_eq!(liked.track_id, track_id);
    likes
        .like_track(&profile.id, &track_id)
        .await
        .expect("relike should be idempotent");
    assert!(
        likes
            .is_liked(&profile.id, &track_id)
            .await
            .expect("liked flag should work")
    );
    assert_eq!(
        likes
            .list_liked_tracks(
                &profile.id,
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
            .is_liked(&profile.id, &track_id)
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
        .add_track(&profile.id, &created.id, &track_id, None)
        .await
        .expect("track should add");
    playlists
        .add_track(&profile.id, &created.id, &track_id, None)
        .await
        .expect("duplicate track add should be idempotent");
    playlists
        .add_track(&profile.id, &created.id, &track_id_2, Some(0))
        .await
        .expect("second track should add");

    let page = playlists
        .list_tracks(
            &profile.id,
            &created.id,
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
            .get_public_media(&pending.track_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        catalog
            .get_personal_media(&owner.id, &pending.track_id)
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
            .get_public_media(&pending.track_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        catalog
            .get_personal_media(&owner.id, &pending.track_id)
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
