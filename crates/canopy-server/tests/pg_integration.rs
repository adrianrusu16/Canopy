#![cfg(feature = "pg")]

use canopy_core::{
    AudioAssetRepository, CatalogIngest, CatalogRepository, LibraryRepository, LikeRepository,
    Page, PlaybackHistoryEvent, PlaybackHistoryRepository, PreferencesRepository,
    ProfileRepository, ProviderAudioAsset, ProviderLicense, ProviderTrack,
};
use canopy_server::jade_store::{
    PgAudioAssetRepository, PgCatalogRepository, PgLibraryRepository, PgLikeRepository,
    PgPlaybackHistoryRepository, PgPreferencesRepository, PgProfileRepository,
};
use sqlx::{Row, postgres::PgPoolOptions};

async fn connect_test_pool() -> Option<sqlx::PgPool> {
    let database_url = std::env::var("CANOPY_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;

    match PgPoolOptions::new()
        .max_connections(3)
        .connect(&database_url)
        .await
    {
        Ok(pool) => Some(pool),
        Err(err) => {
            eprintln!("skipping postgres integration test: {err}");
            None
        }
    }
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
                object_key: "audio/tracks/canopy-test/integration-nocturne.mp3".to_string(),
                size_bytes: 7_200_000,
                checksum_sha256: "1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
                duration_ms: 181_000,
            },
            ProviderAudioAsset {
                codec: "opus".to_string(),
                content_type: "audio/ogg".to_string(),
                object_key: "audio/tracks/canopy-test/integration-nocturne.opus".to_string(),
                size_bytes: 4_100_000,
                checksum_sha256: "2222222222222222222222222222222222222222222222222222222222222222"
                    .to_string(),
                duration_ms: 181_000,
            },
        ],
        artwork_key: Some("artwork/tracks/canopy-test/integration-nocturne.png".to_string()),
        album_artwork_key: Some("artwork/albums/canopy-test/album.png".to_string()),
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
    let Some(pool) = connect_test_pool().await else {
        eprintln!(
            "skipping postgres integration test: no CANOPY_TEST_DATABASE_URL or DATABASE_URL"
        );
        return;
    };

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

    let mut updated = track;
    updated.title = "Integration Nocturne Revised".to_string();
    updated.assets[0].size_bytes = 7_500_000;

    let second_id = catalog
        .ingest(updated)
        .await
        .expect("second ingest should update in place");

    assert_eq!(second_id, first_id);

    let stored_assets = assets
        .assets_for_track(&first_id)
        .await
        .expect("assets should be queryable after ingest");
    assert_eq!(stored_assets.len(), 2);

    let item = catalog
        .get_media(&first_id)
        .await
        .expect("media lookup should succeed")
        .expect("ingested media should exist");
    assert_eq!(item.title, "Integration Nocturne Revised");

    let page = catalog
        .browse(
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
    assert_eq!(occurrences, 1);

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
    let Some(pool) = connect_test_pool().await else {
        eprintln!(
            "skipping postgres integration test: no CANOPY_TEST_DATABASE_URL or DATABASE_URL"
        );
        return;
    };

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

#[tokio::test]
async fn postgres_migrations_support_profile_scoped_history() {
    let Some(pool) = connect_test_pool().await else {
        eprintln!(
            "skipping postgres integration test: no CANOPY_TEST_DATABASE_URL or DATABASE_URL"
        );
        return;
    };

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

    history
        .record(PlaybackHistoryEvent {
            profile_id: profile.id.clone(),
            track_id: track_id.clone(),
            duration_ms: 1000,
            completion_pct: 0.75,
        })
        .await
        .expect("history should be recorded");

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
    let Some(pool) = connect_test_pool().await else {
        eprintln!(
            "skipping postgres integration test: no CANOPY_TEST_DATABASE_URL or DATABASE_URL"
        );
        return;
    };

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
async fn postgres_repositories_support_profile_library_likes_preferences() {
    let Some(pool) = connect_test_pool().await else {
        eprintln!(
            "skipping postgres integration test: no CANOPY_TEST_DATABASE_URL or DATABASE_URL"
        );
        return;
    };

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
