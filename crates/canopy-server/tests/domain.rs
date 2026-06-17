//! Domain-level tests exercising the services over the in-memory repository
//! implementations, with no transport involved.

use std::sync::Arc;

use canopy_core::{MediaItem, Page};
use canopy_server::catalog::CatalogService;
use canopy_server::discovery::DiscoveryService;
use canopy_server::jade_store::{InMemoryCatalog, InMemorySessionStore};
use canopy_server::playback::PlaybackService;
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

    // Unknown session reads back as default.
    let empty = playback.get_session("sess_1").await.unwrap();
    assert_eq!(empty.current_media_id, None);

    // Update creates and mutates the session.
    playback
        .update_session("sess_1", Some("trk_1".into()), Some(5_000))
        .await
        .unwrap();
    let updated = playback.get_session("sess_1").await.unwrap();
    assert_eq!(updated.current_media_id.as_deref(), Some("trk_1"));
    assert_eq!(updated.position_ms, 5_000);

    // End removes it.
    playback.end_session("sess_1").await.unwrap();
    let after = playback.get_session("sess_1").await.unwrap();
    assert_eq!(after.current_media_id, None);
}
