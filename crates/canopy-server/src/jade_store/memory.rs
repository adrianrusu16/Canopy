//! In-memory implementations of the repository ports.
//!
//! These are placeholders for the future PostgreSQL-backed (catalog) and
//! persistent (session) stores. They keep the prototype fully functional
//! without external dependencies.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyResult, CatalogRepository, DiscoveryRepository,
    MediaItem, MediaPage, Page, Session, SessionRepository,
};

/// In-memory catalog backing store.
#[derive(Clone, Default)]
pub struct InMemoryCatalog {
    items: Vec<MediaItem>,
}

impl InMemoryCatalog {
    /// Creates a catalog seeded with the given items.
    pub fn with_items(items: Vec<MediaItem>) -> Self {
        Self { items }
    }
}

#[async_trait]
impl CatalogRepository for InMemoryCatalog {
    async fn browse(
        &self,
        _parent_id: Option<&str>,
        _genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let start = (page.offset as usize).min(self.items.len());
        let end = (start + page.limit as usize).min(self.items.len());
        Ok(MediaPage {
            items: self.items[start..end].to_vec(),
            total_count: self.items.len() as i32,
            has_more: end < self.items.len(),
        })
    }

    async fn search(&self, query: &str, _page: Page) -> CanopyResult<MediaPage> {
        // Trivial in-memory match; the production path is PostgreSQL `pg_trgm`.
        let items: Vec<MediaItem> = if query.is_empty() {
            Vec::new()
        } else {
            self.items
                .iter()
                .filter(|i| {
                    i.title.to_lowercase().contains(&query.to_lowercase())
                        || i.artist.to_lowercase().contains(&query.to_lowercase())
                })
                .cloned()
                .collect()
        };
        let total_count = items.len() as i32;
        Ok(MediaPage {
            items,
            total_count,
            has_more: false,
        })
    }

    async fn get_media(&self, media_id: &str) -> CanopyResult<Option<MediaItem>> {
        Ok(self.items.iter().find(|i| i.id == media_id).cloned())
    }
}

#[async_trait]
impl DiscoveryRepository for InMemoryCatalog {
    async fn shuffle_pool(&self) -> CanopyResult<Vec<MediaItem>> {
        // The production view is pre-shuffled offline; the prototype simply
        // exposes the catalog in insertion order and leaves diversification to
        // the discovery service.
        Ok(self.items.clone())
    }
}

/// In-memory audio-asset backing store.
///
/// Stands in for the `audio_assets` table; the production backend will read
/// from PostgreSQL. Assets are grouped by track identifier.
#[derive(Clone, Default)]
pub struct InMemoryAudioAssetStore {
    assets: Vec<AudioAsset>,
}

impl InMemoryAudioAssetStore {
    /// Creates a store seeded with the given assets.
    pub fn with_assets(assets: Vec<AudioAsset>) -> Self {
        Self { assets }
    }
}

#[async_trait]
impl AudioAssetRepository for InMemoryAudioAssetStore {
    async fn assets_for_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>> {
        Ok(self
            .assets
            .iter()
            .filter(|a| a.track_id == track_id)
            .cloned()
            .collect())
    }
}

/// In-memory session store.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: Mutex<HashMap<String, Session>>,
}

#[async_trait]
impl SessionRepository for InMemorySessionStore {
    async fn create(&self) -> CanopyResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let session = Session {
            id: id.clone(),
            ..Session::default()
        };
        self.sessions.lock().unwrap().insert(id.clone(), session);
        Ok(id)
    }

    async fn get(&self, id: &str) -> CanopyResult<Option<Session>> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn update(&self, session: Session) -> CanopyResult<()> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session);
        Ok(())
    }

    async fn delete(&self, id: &str) -> CanopyResult<()> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }
}
