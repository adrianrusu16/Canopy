//! Domain-level tests exercising the services over the in-memory repository
//! implementations, with no transport involved.

use std::sync::Arc;

use canopy_core::{
    AudioAsset, AudioAssetRepository, AuthorizedStreamAsset, CanopyError, CatalogRepository,
    IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility, Page, PageTokenCodec,
    PendingImportOutcome, PendingMediaImport, PlayableAsset, PlayableAssetRepository,
    StreamAudience,
};
use canopy_server::catalog::CatalogService;
use canopy_server::discovery::DiscoveryService;
use canopy_server::jade_store::{
    InMemoryAudioAssetEntry, InMemoryAudioAssetStore, InMemoryCatalog, InMemoryCatalogEntry,
    InMemoryInstanceSettingsStore,
};
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

#[test]
fn page_token_round_trips_offset_without_exposing_it() {
    let codec = PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap();
    let token = codec.encode(42).unwrap();

    assert!(!token.contains("42"));
    assert_eq!(codec.decode(&token).unwrap(), 42);
}

#[test]
fn page_token_rejects_tampering() {
    let codec = PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap();

    assert!(matches!(
        codec.decode("tampered"),
        Err(CanopyError::InvalidArgument(_))
    ));
}

#[test]
fn pending_media_import_carries_only_managed_metadata() {
    let pending = PendingMediaImport {
        track_id: "018f0000-0000-7000-8000-000000000001".into(),
        owner_profile_id: "018f0000-0000-7000-8000-000000000002".into(),
        title: "Test Tone".into(),
        artist: "Canopy Tests".into(),
        album: "Importer Fixtures".into(),
        duration_ms: 1_000,
        artwork_storage_key: Some("artwork/aa/bb/hash.jpg".into()),
        audio: AudioAsset {
            track_id: "018f0000-0000-7000-8000-000000000001".into(),
            codec: "mp3".into(),
            content_type: "audio/mpeg".into(),
            storage_key: "audio/aa/bb/hash.mp3".into(),
            size_bytes: 1_024,
            checksum_sha256: "a".repeat(64),
            duration_ms: 1_000,
        },
    };

    assert_eq!(pending.audio.track_id, pending.track_id);
    assert_eq!(
        PendingImportOutcome::Duplicate {
            track_id: "existing".into(),
        },
        PendingImportOutcome::Duplicate {
            track_id: "existing".into(),
        }
    );
}

#[test]
fn stream_assets_carry_identity_and_relative_keys_without_urls() {
    let playable = PlayableAsset {
        asset_id: "018f0000-0000-7000-8000-000000000001".into(),
        track_id: "018f0000-0000-7000-8000-000000000002".into(),
        codec: "mp3".into(),
        content_type: "audio/mpeg".into(),
        duration_ms: 1_000,
    };
    let authorized = AuthorizedStreamAsset {
        asset_id: playable.asset_id.clone(),
        storage_key: "audio/aa/bb/hash.mp3".into(),
        content_type: playable.content_type.clone(),
    };

    assert_eq!(StreamAudience::Public.as_str(), "public");
    assert_eq!(StreamAudience::Personal.as_str(), "personal");
    assert_eq!(authorized.asset_id, playable.asset_id);
    assert!(!authorized.storage_key.starts_with('/'));
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
    let settings = Arc::new(InMemoryInstanceSettingsStore::default());
    settings.set_owner_profile_id("owner-a").await.unwrap();
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
    ])
    .with_instance_settings(settings.clone());

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

    let personal_playable = assets
        .assets_for_personal_playback("owner-a", "personal")
        .await
        .unwrap();
    assert_eq!(personal_playable.len(), 1);
    assert!(
        assets
            .assets_for_personal_playback("owner-b", "personal")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        assets
            .authorize_stream_asset(&personal_playable[0].asset_id, StreamAudience::Personal,)
            .await
            .unwrap()
            .is_some()
    );
    settings.set_owner_profile_id("owner-b").await.unwrap();
    assert!(
        assets
            .authorize_stream_asset(&personal_playable[0].asset_id, StreamAudience::Personal,)
            .await
            .unwrap()
            .is_none()
    );

    let playable = assets.assets_for_public_playback("public").await.unwrap();
    assert_eq!(playable.len(), 1);
    assert!(uuid::Uuid::parse_str(&playable[0].asset_id).is_ok());
    assert!(
        assets
            .authorize_stream_asset(&playable[0].asset_id, StreamAudience::Public)
            .await
            .unwrap()
            .is_some()
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
async fn discovery_feed_applies_offset_after_diversification() {
    let discovery = DiscoveryService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    let result = discovery.feed(&[], page(1, 1)).await.unwrap();

    assert_eq!(result.items[0].id, "trk_2");
    assert!(!result.has_more);
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
