//! Catalog service.
//!
//! Owns the hierarchical browse experience and metadata retrieval. It depends
//! only on the [`CatalogRepository`] port, so the in-memory and future
//! PostgreSQL backends are interchangeable behind it.

use std::sync::Arc;

use canopy_core::{CanopyResult, CatalogRepository, MediaItem, MediaPage, Page};

/// Application service for catalog browsing and lookup.
#[derive(Clone)]
pub struct CatalogService {
    repo: Arc<dyn CatalogRepository>,
}

impl CatalogService {
    /// Creates a new service over the given repository.
    pub fn new(repo: Arc<dyn CatalogRepository>) -> Self {
        Self { repo }
    }

    /// Returns a hierarchical browse page.
    pub async fn browse(
        &self,
        parent_id: Option<&str>,
        genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage> {
        self.repo.browse_public(parent_id, genres, page).await
    }

    /// Searches the catalog.
    pub async fn search(&self, query: &str, page: Page) -> CanopyResult<MediaPage> {
        self.repo.search_public(query, page).await
    }

    /// Fetches a single media item by identifier.
    pub async fn get_media(&self, media_id: &str) -> CanopyResult<Option<MediaItem>> {
        self.repo.get_public_media(media_id).await
    }
}
