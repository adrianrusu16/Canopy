//! Domain-level tests exercising the services over the in-memory repository
//! implementations, with no transport involved.

use std::sync::Arc;

use canopy_core::{
    AudioAsset, AudioAssetRepository, CatalogRepository, IngestStatus, MediaItem, MediaVisibility,
    Page,
};
use canopy_server::catalog::CatalogService;
use canopy_server::discovery::DiscoveryService;
use canopy_server::jade_store::{
    InMemoryAudioAssetEntry, InMemoryAudioAssetStore, InMemoryCatalog, InMemoryCatalogEntry,
    InMemorySessionStore,
};
use canopy_server::playback::PlaybackService;
use canopy_server::providers::{ProviderAdapter, TestFixtureProvider};
use canopy_server::search::SearchService;

fn sample_items() -> Vec<MediaItem> {
    vec![
        MediaItem {
            id: "trk_1".into(),
            title: "Moonlight Sonata".into(),
            artist: "Beethoven".into(),
            ..MediaItem::default()
        },
        MediaItem {
            id: "trk_2".into(),
            title: "Clair de Lune".into(),
            artist: "Debussy".into(),
            ..MediaItem::default()
        },
    ]
}

fn page(limit: u32, offset: u32) -> Page {
    Page { limit, offset }
}

fn scoped_item(id: &str, title: &str) -> MediaItem {
    MediaItem {
        id: id.to_string(),
        title: title.to_string(),
        artist: "Scope Artist".to_string(),
        ..MediaItem::default()
    }
}

#[tokio::test]
async fn catalog_scope_public_hides_non_public_items() {
    let catalog = InMemoryCatalog::from_entries(vec![
        InMemoryCatalogEntry {
            item: scoped_item("public", "Public Track"),
            visibility: MediaVisibility::ReleaseSafe,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: None,
        },
        InMemoryCatalogEntry {
            item: scoped_item("personal", "Personal Track"),
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some("owner-a".to_string()),
        },
        InMemoryCatalogEntry {
            item: scoped_item("pending", "Pending Track"),
            visibility: MediaVisibility::ReleaseSafe,
            ingest_status: IngestStatus::Pending,
            owner_profile_id: None,
        },
        InMemoryCatalogEntry {
            item: scoped_item("quarantined", "Quarantined Track"),
            visibility: MediaVisibility::Quarantined,
            ingest_status: IngestStatus::Quarantined,
            owner_profile_id: None,
        },
    ]);

    let page = catalog.browse_public(None, &[], page(10, 0)).await.unwrap();
    assert_eq!(page.items, vec![scoped_item("public", "Public Track")]);
    assert_eq!(page.total_count, 1);
    assert!(
        catalog
            .get_public_media("personal")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn catalog_scope_personal_is_owner_scoped() {
    let catalog = InMemoryCatalog::from_entries(vec![
        InMemoryCatalogEntry {
            item: scoped_item("owner-a-track", "Owner A Track"),
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some("owner-a".to_string()),
        },
        InMemoryCatalogEntry {
            item: scoped_item("owner-b-track", "Owner B Track"),
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some("owner-b".to_string()),
        },
    ]);

    let page = catalog.list_personal("owner-a", page(10, 0)).await.unwrap();
    assert_eq!(
        page.items,
        vec![scoped_item("owner-a-track", "Owner A Track")]
    );
    assert!(
        catalog
            .get_personal_media("owner-a", "owner-b-track")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn catalog_scope_public_assets_hide_personal_tracks() {
    let public_asset = AudioAsset {
        track_id: "public".to_string(),
        storage_key: "audio/public.mp3".to_string(),
        ..AudioAsset::default()
    };
    let personal_asset = AudioAsset {
        track_id: "personal".to_string(),
        storage_key: "audio/personal.mp3".to_string(),
        ..AudioAsset::default()
    };
    let assets = InMemoryAudioAssetStore::from_entries(vec![
        InMemoryAudioAssetEntry {
            asset: public_asset.clone(),
            visibility: MediaVisibility::ReleaseSafe,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: None,
        },
        InMemoryAudioAssetEntry {
            asset: personal_asset.clone(),
            visibility: MediaVisibility::Personal,
            ingest_status: IngestStatus::Ready,
            owner_profile_id: Some("owner-a".to_string()),
        },
    ]);

    assert_eq!(
        assets.assets_for_public_track("public").await.unwrap(),
        vec![public_asset]
    );
    assert!(
        assets
            .assets_for_public_track("personal")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        assets
            .assets_for_personal_track("owner-a", "personal")
            .await
            .unwrap(),
        vec![personal_asset]
    );
    assert!(
        assets
            .assets_for_personal_track("owner-b", "personal")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn browse_paginates() {
    let catalog = CatalogService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    let first = catalog.browse(None, &[], page(1, 0)).await.unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.total_count, 2);
    assert!(first.has_more);

    let second = catalog.browse(None, &[], page(1, 1)).await.unwrap();
    assert_eq!(second.items[0].id, "trk_2");
    assert!(!second.has_more);
}

#[tokio::test]
async fn search_matches_title_and_artist() {
    let catalog = CatalogService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    let by_title = catalog.search("moonlight", page(10, 0)).await.unwrap();
    assert_eq!(by_title.items.len(), 1);
    assert_eq!(by_title.items[0].id, "trk_1");

    let by_artist = catalog.search("debussy", page(10, 0)).await.unwrap();
    assert_eq!(by_artist.items[0].id, "trk_2");

    let empty = catalog.search("", page(10, 0)).await.unwrap();
    assert!(empty.items.is_empty());
}

#[tokio::test]
async fn search_service_normalizes_query() {
    let search = SearchService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    // Mixed case and noisy whitespace still match.
    let hit = search.search("  MoonLight  ", page(10, 0)).await.unwrap();
    assert_eq!(hit.items.len(), 1);
    assert_eq!(hit.items[0].id, "trk_1");

    // Whitespace-only queries short-circuit to an empty page.
    let empty = search.search("   \t ", page(10, 0)).await.unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.total_count, 0);
}

#[tokio::test]
async fn search_service_defaults_zero_limit() {
    let search = SearchService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    // A limit of 0 must not silently drop all results; it falls back to the
    // service default page size.
    let result = search.search("beethoven", page(0, 0)).await.unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, "trk_1");
}

#[tokio::test]
async fn get_media_returns_none_for_unknown() {
    let catalog = CatalogService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    assert!(catalog.get_media("trk_1").await.unwrap().is_some());
    assert!(catalog.get_media("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn discovery_excludes_recently_played() {
    let discovery = DiscoveryService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    let recent = vec!["trk_1".to_string()];
    let result = discovery.next(&recent, 10).await.unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, "trk_2");
}

#[tokio::test]
async fn discovery_spreads_artists() {
    // Two tracks per artist, interleaved so a naive pass would repeat an
    // artist; the service must alternate them.
    let items = vec![
        MediaItem {
            id: "a1".into(),
            artist: "A".into(),
            ..MediaItem::default()
        },
        MediaItem {
            id: "a2".into(),
            artist: "A".into(),
            ..MediaItem::default()
        },
        MediaItem {
            id: "b1".into(),
            artist: "B".into(),
            ..MediaItem::default()
        },
    ];
    let discovery = DiscoveryService::new(Arc::new(InMemoryCatalog::with_items(items)));

    let result = discovery.next(&[], 3).await.unwrap();
    assert_eq!(result.items.len(), 3);
    for pair in result.items.windows(2) {
        // Where an alternative exists, neighbours differ in artist.
        if pair[0].artist == "A" {
            assert_ne!(pair[0].artist, pair[1].artist);
        }
    }
}

#[tokio::test]
async fn session_lifecycle() {
    let playback = PlaybackService::new(Arc::new(InMemorySessionStore::default()));

    let empty = playback.get_session("sess_1").await.unwrap();
    assert_eq!(empty.current_media_id, None);

    playback
        .update_session("sess_1", Some("trk_1".into()), Some(5_000))
        .await
        .unwrap();
    let updated = playback.get_session("sess_1").await.unwrap();
    assert_eq!(updated.current_media_id.as_deref(), Some("trk_1"));
    assert_eq!(updated.position_ms, 5_000);

    playback.end_session("sess_1").await.unwrap();
    let after = playback.get_session("sess_1").await.unwrap();
    assert_eq!(after.current_media_id, None);
}

#[tokio::test]
async fn playback_controls_mutate_session_state() {
    let playback = PlaybackService::new(Arc::new(InMemorySessionStore::default()));

    let session_id = playback.play("", "trk_1".into(), 1_250).await.unwrap();
    assert_eq!(session_id, PlaybackService::DEFAULT_SESSION_ID);

    let playing = playback.get_session(&session_id).await.unwrap();
    assert_eq!(playing.current_media_id.as_deref(), Some("trk_1"));
    assert_eq!(playing.position_ms, 1_250);
    assert!(playing.is_playing);

    playback.seek(&session_id, 9_000).await.unwrap();
    playback.set_playback_speed(&session_id, 1.5).await.unwrap();
    playback.pause(&session_id).await.unwrap();

    let paused = playback.get_session(&session_id).await.unwrap();
    assert_eq!(paused.position_ms, 9_000);
    assert_eq!(paused.playback_speed, 1.5);
    assert!(!paused.is_playing);

    playback.stop(&session_id).await.unwrap();
    let stopped = playback.get_session(&session_id).await.unwrap();
    assert_eq!(stopped.position_ms, 0);
    assert!(!stopped.is_playing);
}

#[tokio::test]
async fn playback_controls_reject_invalid_inputs() {
    let playback = PlaybackService::new(Arc::new(InMemorySessionStore::default()));

    assert!(playback.play("sess_1", "".into(), 0).await.is_err());
    assert!(playback.play("sess_1", "trk_1".into(), -1).await.is_err());
    assert!(playback.seek("sess_1", -1).await.is_err());
    assert!(playback.set_playback_speed("sess_1", 0.0).await.is_err());
}

#[tokio::test]
async fn fixture_provider_reads_wrapped_catalog_file() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
        .join("catalog.json");
    let provider = TestFixtureProvider::new(path).unwrap();

    let tracks = provider.fetch_catalog().await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].provider, "fixture");
}
