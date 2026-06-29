//! Search service.
//!
//! The target implementation runs PostgreSQL full-text search with `pg_trgm`
//! trigram similarity and streams paginated results. Until that backend is
//! wired up, search is served by the [`CatalogRepository`] port's in-memory
//! match. This service owns the transport-agnostic concerns that are
//! independent of the backend — query normalization and page-size clamping —
//! so they survive the move to PostgreSQL unchanged.

use std::sync::Arc;

use canopy_core::{CanopyResult, CatalogRepository, MediaPage, Page};

/// Application service for catalog search.
#[derive(Clone)]
pub struct SearchService {
    repo: Arc<dyn CatalogRepository>,
}

impl SearchService {
    /// Page size used when a request asks for `0` items.
    const DEFAULT_LIMIT: u32 = 20;
    /// Upper bound on the page size a single request may ask for.
    const MAX_LIMIT: u32 = 100;

    /// Creates a new service over the given catalog repository.
    pub fn new(repo: Arc<dyn CatalogRepository>) -> Self {
        Self { repo }
    }

    /// Searches the catalog for `raw_query`.
    ///
    /// The query is normalized (trimmed, internal whitespace collapsed, and
    /// lower-cased) and the page size is clamped to a sane range before the
    /// request reaches the backend. An empty query short-circuits to an empty
    /// page without touching the repository.
    pub async fn search(&self, raw_query: &str, page: Page) -> CanopyResult<MediaPage> {
        let query = normalize_query(raw_query);
        if query.is_empty() {
            return Ok(MediaPage::default());
        }
        self.repo.search_public(&query, self.clamp_page(page)).await
    }

    /// Clamps a requested page to the service's supported limits.
    fn clamp_page(&self, page: Page) -> Page {
        let limit = match page.limit {
            0 => Self::DEFAULT_LIMIT,
            n => n.min(Self::MAX_LIMIT),
        };
        Page {
            limit,
            offset: page.offset,
        }
    }
}

/// Normalizes a raw search query: trims, collapses internal whitespace, and
/// lower-cases it so matching is case- and spacing-insensitive.
fn normalize_query(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::normalize_query;

    #[test]
    fn normalize_collapses_and_lowercases() {
        assert_eq!(normalize_query("  Moonlight   Sonata "), "moonlight sonata");
        assert_eq!(normalize_query("\tDEBUSSY\n"), "debussy");
        assert_eq!(normalize_query("   "), "");
    }
}
