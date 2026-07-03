//! Discovery service.
//!
//! Serves the shuffle channel: randomized playback with diversity filtering
//! and exclusion of recently played tracks. The randomization itself lives in
//! the [`DiscoveryRepository`] (a pre-shuffled materialized view in the
//! production design, so the hot path never pays for `ORDER BY random()`).
//!
//! This service owns the transport- and backend-agnostic concerns layered on
//! top of that pool: dropping recently played tracks, spreading items so the
//! same artist does not play back-to-back, and clamping the page size. Keeping
//! that logic here means it survives the move from the in-memory prototype to
//! the real materialized view unchanged.

use std::collections::HashSet;
use std::sync::Arc;

use canopy_core::{CanopyResult, DiscoveryRepository, MediaItem, MediaPage, Page};

/// Application service for the discovery shuffle channel.
#[derive(Clone)]
pub struct DiscoveryService {
    repo: Arc<dyn DiscoveryRepository>,
}

impl DiscoveryService {
    /// Page size used when a request asks for `0` items.
    const DEFAULT_LIMIT: u32 = 20;
    /// Upper bound on the page size a single request may ask for.
    const MAX_LIMIT: u32 = 100;

    /// Creates a new service over the given discovery repository.
    pub fn new(repo: Arc<dyn DiscoveryRepository>) -> Self {
        Self { repo }
    }

    /// Returns the next batch of shuffle-channel items.
    ///
    /// `recently_played` lists track identifiers to exclude (most recent
    /// listening history). The result excludes those tracks, is reordered so
    /// no two consecutive items share an artist where avoidable, and is capped
    /// at a clamped page size.
    pub async fn next(&self, recently_played: &[String], limit: u32) -> CanopyResult<MediaPage> {
        self.feed(recently_played, Page { limit, offset: 0 }).await
    }

    /// Returns an offset page from the diversified discovery pool.
    pub async fn feed(&self, recently_played: &[String], page: Page) -> CanopyResult<MediaPage> {
        let limit = Self::clamp_limit(page.limit) as usize;
        let offset = page.offset as usize;
        let excluded: HashSet<&str> = recently_played.iter().map(String::as_str).collect();

        let candidates: Vec<MediaItem> = self
            .repo
            .shuffle_pool()
            .await?
            .into_iter()
            .filter(|item| !excluded.contains(item.id.as_str()))
            .collect();

        let total = candidates.len();
        let items: Vec<_> = diversify(candidates, total)
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        let has_more = total > offset.saturating_add(items.len());

        Ok(MediaPage {
            total_count: items.len() as i32,
            has_more,
            items,
        })
    }

    /// Clamps a requested limit to the service's supported range.
    fn clamp_limit(limit: u32) -> u32 {
        match limit {
            0 => Self::DEFAULT_LIMIT,
            n => n.min(Self::MAX_LIMIT),
        }
    }
}

/// Greedily reorders `candidates` (preserving the pool's relative order) so
/// that consecutive items avoid repeating an artist when an alternative is
/// available, then truncates to `limit` items.
fn diversify(mut candidates: Vec<MediaItem>, limit: usize) -> Vec<MediaItem> {
    let mut selected: Vec<MediaItem> = Vec::with_capacity(limit.min(candidates.len()));

    while selected.len() < limit && !candidates.is_empty() {
        let last_artist = selected.last().map(|item| item.artist.clone());
        // Prefer the first candidate from a different artist than the previous
        // pick; fall back to the first remaining candidate when none differ.
        let pick = match &last_artist {
            Some(artist) => candidates
                .iter()
                .position(|item| &item.artist != artist)
                .unwrap_or(0),
            None => 0,
        };
        selected.push(candidates.remove(pick));
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::diversify;
    use canopy_core::MediaItem;

    fn item(id: &str, artist: &str) -> MediaItem {
        MediaItem {
            id: id.into(),
            artist: artist.into(),
            ..MediaItem::default()
        }
    }

    #[test]
    fn diversify_avoids_consecutive_same_artist() {
        let pool = vec![
            item("1", "A"),
            item("2", "A"),
            item("3", "B"),
            item("4", "B"),
        ];
        let out = diversify(pool, 4);
        for pair in out.windows(2) {
            assert_ne!(pair[0].artist, pair[1].artist);
        }
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn diversify_falls_back_when_no_alternative() {
        // All from the same artist: diversity is impossible, but no item is
        // dropped on that account.
        let pool = vec![item("1", "A"), item("2", "A"), item("3", "A")];
        let out = diversify(pool, 3);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn diversify_truncates_to_limit() {
        let pool = vec![item("1", "A"), item("2", "B"), item("3", "C")];
        let out = diversify(pool, 2);
        assert_eq!(out.len(), 2);
    }
}
