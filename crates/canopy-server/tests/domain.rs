//! Domain-level tests exercising the services over the in-memory repository
//! implementations, with no transport involved.

use std::sync::Arc;

use canopy_core::{
    AudioAsset, AudioAssetRepository, AuthorizedStreamAsset, CanopyError, CatalogRepository,
    IngestStatus, InstanceSettingsRepository, MediaItem, MediaVisibility, Page, PageTokenCodec,
    PendingImportOutcome, PendingMediaImport, PlayableAsset, PlayableAssetRepository,
    LibraryRepository, PlaylistRepository, ProfileRepository, StreamAudience, TrackAccessScope,
    UserIdentity,
};
use canopy_server::catalog::CatalogService;
use canopy_server::discovery::DiscoveryService;
use canopy_server::jade_store::{
    InMemoryAudioAssetEntry, InMemoryAudioAssetStore, InMemoryCatalog, InMemoryCatalogEntry,
    InMemoryInstanceSettingsStore, InMemoryLibraryStore, InMemoryPlaylistStore,
    InMemoryProfileStore,
};
use canopy_server::library::LibraryService;
use canopy_server::playback::{ResolverConfig, ResolverService};
use canopy_server::playlists::PlaylistService;
use canopy_server::principal::PrincipalService;
use canopy_server::providers::{ProviderAdapter, TestFixtureProvider};
use canopy_server::search::SearchService;
use canopy_server::stream::StreamTokenCodec;

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

fn access_entry(
    id: &str,
    artist: &str,
    visibility: MediaVisibility,
    ingest_status: IngestStatus,
    owner_profile_id: Option<&str>,
) -> InMemoryCatalogEntry {
    InMemoryCatalogEntry {
        item: MediaItem {
            id: id.into(),
            title: id.into(),
            artist: artist.into(),
            ..MediaItem::default()
        },
        visibility,
        ingest_status,
        owner_profile_id: owner_profile_id.map(str::to_owned),
    }
}

fn access_matrix_entries() -> Vec<InMemoryCatalogEntry> {
    vec![
        access_entry(
            "public-ready",
            "Artist Public",
            MediaVisibility::ReleaseSafe,
            IngestStatus::Ready,
            None,
        ),
        access_entry(
            "owner-ready",
            "Artist Owner",
            MediaVisibility::Personal,
            IngestStatus::Ready,
            Some("owner-a"),
        ),
        access_entry(
            "other-ready",
            "Artist Other",
            MediaVisibility::Personal,
            IngestStatus::Ready,
            Some("owner-b"),
        ),
        access_entry(
            "owner-pending",
            "Artist Pending",
            MediaVisibility::Personal,
            IngestStatus::Pending,
            Some("owner-a"),
        ),
        access_entry(
            "quarantined",
            "Artist Quarantined",
            MediaVisibility::Quarantined,
            IngestStatus::Quarantined,
            None,
        ),
    ]
}

fn access_asset(
    track_id: &str,
    visibility: MediaVisibility,
    ingest_status: IngestStatus,
    owner_profile_id: Option<&str>,
) -> InMemoryAudioAssetEntry {
    InMemoryAudioAssetEntry {
        asset: AudioAsset {
            track_id: track_id.into(),
            codec: "mp3".into(),
            content_type: "audio/mpeg".into(),
            storage_key: format!("audio/{track_id}.mp3"),
            size_bytes: 1_024,
            checksum_sha256: "a".repeat(64),
            duration_ms: 1_000,
        },
        visibility,
        ingest_status,
        owner_profile_id: owner_profile_id.map(str::to_owned),
    }
}

async fn save_visible_tracks_and_create_playlist(
    library: &LibraryService,
    playlists: &PlaylistService,
    identity: &UserIdentity,
    visible_track_ids: &[&str],
) -> String {
    let playlist = playlists
        .create_playlist(identity, "Access invariant", "")
        .await
        .unwrap();

    for track_id in visible_track_ids {
        library.save_track(identity, track_id).await.unwrap();
        playlists
            .add_track(identity, &playlist.id, track_id, None)
            .await
            .unwrap();
    }

    playlist.id
}

#[tokio::test]
async fn track_visibility_is_consistent_across_every_read_and_playback_surface() {
    const OWNER: &str = "owner-a";
    const LISTENER: &str = "listener";
    const PAGE_SIZE: u32 = 100;

    let profiles = Arc::new(InMemoryProfileStore::default());
    let owner_profile = profiles
        .upsert_profile(OWNER, Some("Owner"), true)
        .await
        .unwrap();
    let listener_profile = profiles
        .upsert_profile(LISTENER, Some("Listener"), true)
        .await
        .unwrap();
    let settings = Arc::new(InMemoryInstanceSettingsStore::default());
    settings
        .set_owner_profile_id(&owner_profile.id)
        .await
        .unwrap();
    let principal = PrincipalService::new(profiles.clone(), settings);

    let catalog = Arc::new(InMemoryCatalog::from_entries(vec![
        access_entry(
            "public-ready",
            "Artist Public",
            MediaVisibility::ReleaseSafe,
            IngestStatus::Ready,
            None,
        ),
        access_entry(
            "owner-ready",
            "Artist Owner",
            MediaVisibility::Personal,
            IngestStatus::Ready,
            Some(&owner_profile.id),
        ),
        access_entry(
            "other-ready",
            "Artist Other",
            MediaVisibility::Personal,
            IngestStatus::Ready,
            Some("owner-b"),
        ),
        access_entry(
            "owner-pending",
            "Artist Pending",
            MediaVisibility::Personal,
            IngestStatus::Pending,
            Some(&owner_profile.id),
        ),
        access_entry(
            "quarantined",
            "Artist Quarantined",
            MediaVisibility::Quarantined,
            IngestStatus::Quarantined,
            None,
        ),
    ]));
    let catalog_service = CatalogService::new(catalog.clone());
    let search = SearchService::new(catalog.clone());
    let discovery = DiscoveryService::new(catalog.clone());
    let library_store = Arc::new(InMemoryLibraryStore::new(catalog.clone()));
    let library = LibraryService::new(
        profiles.clone(),
        library_store.clone(),
        principal.clone(),
    );
    let playlist_store = Arc::new(InMemoryPlaylistStore::new(catalog.clone()));
    let playlists = PlaylistService::new(
        profiles,
        playlist_store.clone(),
        principal,
    );
    let owner = UserIdentity {
        user_id: OWNER.into(),
    };
    let listener = UserIdentity {
        user_id: LISTENER.into(),
    };
    let owner_playlist_id = save_visible_tracks_and_create_playlist(
        &library,
        &playlists,
        &owner,
        &["public-ready", "owner-ready"],
    )
    .await;
    let listener_playlist_id =
        save_visible_tracks_and_create_playlist(&library, &playlists, &listener, &["public-ready"])
            .await;
    let personal_scopes = [
        TrackAccessScope::Owner {
            profile_id: owner_profile.id.clone(),
        },
        TrackAccessScope::Owner {
            profile_id: "owner-b".into(),
        },
    ];
    let saved_personal_tracks = [
        ("owner-ready", &personal_scopes[0]),
        ("other-ready", &personal_scopes[1]),
    ];
    for (profile_id, playlist_id) in [
        (owner_profile.id.as_str(), owner_playlist_id.as_str()),
        (listener_profile.id.as_str(), listener_playlist_id.as_str()),
    ] {
        for (track_id, scope) in saved_personal_tracks {
            library_store
                .save_track(profile_id, track_id, scope)
                .await
                .unwrap();
            playlist_store
                .add_track(profile_id, playlist_id, track_id, None, scope)
                .await
                .unwrap();
        }
    }

    let assets = Arc::new(InMemoryAudioAssetStore::from_entries(vec![
        access_asset(
            "public-ready",
            MediaVisibility::ReleaseSafe,
            IngestStatus::Ready,
            None,
        ),
        access_asset(
            "owner-ready",
            MediaVisibility::Personal,
            IngestStatus::Ready,
            Some(&owner_profile.id),
        ),
        access_asset(
            "other-ready",
            MediaVisibility::Personal,
            IngestStatus::Ready,
            Some("owner-b"),
        ),
        access_asset(
            "owner-pending",
            MediaVisibility::Personal,
            IngestStatus::Pending,
            Some(&owner_profile.id),
        ),
        access_asset(
            "quarantined",
            MediaVisibility::Quarantined,
            IngestStatus::Quarantined,
            None,
        ),
    ]));
    let playback = ResolverService::new(
        assets,
        Arc::new(StreamTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap()),
        ResolverConfig::default(),
    );

    let cases = [
        (
            "listener",
            &listener,
            TrackAccessScope::Public,
            &listener_playlist_id,
            [true, false, false, false, false],
        ),
        (
            "owner",
            &owner,
            TrackAccessScope::Owner {
                profile_id: owner_profile.id.clone(),
            },
            &owner_playlist_id,
            [true, true, false, false, false],
        ),
    ];
    let tracks = [
        ("public-ready", "Artist Public"),
        ("owner-ready", "Artist Owner"),
        ("other-ready", "Artist Other"),
        ("owner-pending", "Artist Pending"),
        ("quarantined", "Artist Quarantined"),
    ];

    for (principal_name, identity, scope, playlist_id, expectations) in cases {
        let browse = catalog_service
            .browse(&scope, None, &[], page(PAGE_SIZE, 0))
            .await
            .unwrap();
        let discovery_page = discovery.next(&scope, &[], PAGE_SIZE).await.unwrap();
        let library_page = library
            .list_tracks(identity, page(PAGE_SIZE, 0))
            .await
            .unwrap();
        let playlist_page = playlists
            .list_tracks(identity, playlist_id, page(PAGE_SIZE, 0))
            .await
            .unwrap();

        for ((track_id, query), expected_visible) in tracks.iter().zip(expectations) {
            let search_page = search
                .search(&scope, query, page(PAGE_SIZE, 0))
                .await
                .unwrap();
            let outcomes = [
                (
                    "browse",
                    browse.items.iter().any(|item| item.id == *track_id),
                ),
                (
                    "search",
                    search_page.items.iter().any(|item| item.id == *track_id),
                ),
                (
                    "discovery",
                    discovery_page.items.iter().any(|item| item.id == *track_id),
                ),
                (
                    "library",
                    library_page
                        .items
                        .iter()
                        .any(|item| item.item.id == *track_id),
                ),
                (
                    "playlist",
                    playlist_page
                        .items
                        .iter()
                        .any(|item| item.item.id == *track_id),
                ),
                (
                    "playback",
                    playback.resolve_at(&scope, track_id, 1_000).await.is_ok(),
                ),
            ];

            for (surface, visible) in outcomes {
                assert_eq!(
                    visible, expected_visible,
                    "{surface} disagreed for track {track_id} and {principal_name} scope"
                );
            }
        }
    }
}

#[tokio::test]
async fn owner_catalog_pages_over_public_and_owned_personal_tracks_only() {
    let catalog = InMemoryCatalog::from_entries(access_matrix_entries());
    let scope = TrackAccessScope::Owner {
        profile_id: "owner-a".into(),
    };

    let first = catalog.browse(&scope, None, &[], page(1, 0)).await.unwrap();
    let second = catalog.browse(&scope, None, &[], page(1, 1)).await.unwrap();

    assert_eq!(first.total_count, 2);
    assert_eq!(second.total_count, 2);
    assert_eq!(
        vec![first.items[0].id.as_str(), second.items[0].id.as_str()],
        vec!["owner-ready", "public-ready"]
    );
    assert!(!second.has_more);
}

#[tokio::test]
async fn public_scope_conceals_every_non_public_partition() {
    let catalog = InMemoryCatalog::from_entries(access_matrix_entries());

    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, "owner-ready")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn owner_discovery_includes_public_and_owned_personal_tracks() {
    let discovery = DiscoveryService::new(Arc::new(InMemoryCatalog::from_entries(
        access_matrix_entries(),
    )));
    let feed = discovery
        .feed(
            &TrackAccessScope::Owner {
                profile_id: "owner-a".into(),
            },
            &[],
            Page {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        feed.items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["owner-ready", "public-ready"]
    );
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

    let page = catalog
        .browse(&TrackAccessScope::Public, None, &[], page(10, 0))
        .await
        .unwrap();
    assert_eq!(page.items, vec![scoped_item("public", "Public Track")]);
    assert_eq!(page.total_count, 1);
    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, "personal")
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
    let scope = TrackAccessScope::Owner {
        profile_id: "owner-a".to_string(),
    };
    assert!(
        catalog
            .get_media(&scope, "owner-b-track")
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

    let first = catalog
        .browse(&TrackAccessScope::Public, None, &[], page(1, 0))
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.total_count, 2);
    assert!(first.has_more);

    let second = catalog
        .browse(&TrackAccessScope::Public, None, &[], page(1, 1))
        .await
        .unwrap();
    assert_eq!(second.items[0].id, "trk_2");
    assert!(!second.has_more);
}

#[tokio::test]
async fn search_matches_title_and_artist() {
    let catalog = CatalogService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    let by_title = catalog
        .search(&TrackAccessScope::Public, "moonlight", page(10, 0))
        .await
        .unwrap();
    assert_eq!(by_title.items.len(), 1);
    assert_eq!(by_title.items[0].id, "trk_1");

    let by_artist = catalog
        .search(&TrackAccessScope::Public, "debussy", page(10, 0))
        .await
        .unwrap();
    assert_eq!(by_artist.items[0].id, "trk_2");

    let empty = catalog
        .search(&TrackAccessScope::Public, "", page(10, 0))
        .await
        .unwrap();
    assert!(empty.items.is_empty());
}

#[tokio::test]
async fn search_service_normalizes_query() {
    let search = SearchService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    // Mixed case and noisy whitespace still match.
    let hit = search
        .search(&TrackAccessScope::Public, "  MoonLight  ", page(10, 0))
        .await
        .unwrap();
    assert_eq!(hit.items.len(), 1);
    assert_eq!(hit.items[0].id, "trk_1");

    // Whitespace-only queries short-circuit to an empty page.
    let empty = search
        .search(&TrackAccessScope::Public, "   \t ", page(10, 0))
        .await
        .unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.total_count, 0);
}

#[tokio::test]
async fn search_service_defaults_zero_limit() {
    let search = SearchService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    // A limit of 0 must not silently drop all results; it falls back to the
    // service default page size.
    let result = search
        .search(&TrackAccessScope::Public, "beethoven", page(0, 0))
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, "trk_1");
}

#[tokio::test]
async fn get_media_returns_none_for_unknown() {
    let catalog = CatalogService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, "trk_1")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        catalog
            .get_media(&TrackAccessScope::Public, "missing")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn discovery_excludes_recently_played() {
    let discovery = DiscoveryService::new(Arc::new(InMemoryCatalog::with_items(sample_items())));

    let recent = vec!["trk_1".to_string()];
    let result = discovery
        .next(&TrackAccessScope::Public, &recent, 10)
        .await
        .unwrap();
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

    let result = discovery
        .next(&TrackAccessScope::Public, &[], 3)
        .await
        .unwrap();
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

    let result = discovery
        .feed(&TrackAccessScope::Public, &[], page(1, 1))
        .await
        .unwrap();

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
