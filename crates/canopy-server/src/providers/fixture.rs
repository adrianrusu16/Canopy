//! Test fixture provider adapter.
//!
//! Reads a JSON file containing one or more [`ProviderTrack`] records and
//! returns them as a catalog batch. This is used for deterministic integration
//! testing and local development without relying on external API availability.

use std::path::Path;

use async_trait::async_trait;
use canopy_core::{CanopyError, CanopyResult, ProviderTrack};
use serde_json;

use crate::providers::adapter::ProviderAdapter;

/// Provider that reads [`ProviderTrack`] records from a JSON file.
///
/// The JSON file is expected to be an array of objects matching the
/// [`ProviderTrack`] serialization. Example:
///
/// ```json
/// [
///   {
///     "provider_id": "demo-1",
///     "provider": "fixture",
///     "title": "Demo Track",
///     "artist": "Demo Artist",
///     "album": "Demo Album",
///     "release_year": 2024,
///     "duration_ms": 240000,
///     "is_explicit": false,
///     "license": {
///       "license_type": "CC0",
///       "source_url": "https://musopen.org",
///       "attribution_text": "Public Domain"
///     },
///     "assets": [
///       {
///         "codec": "mp4",
///         "content_type": "audio/mp4",
///         "object_key": "audio/tracks/demo-1.m4a",
///         "size_bytes": 9600000,
///         "checksum_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
///         "duration_ms": 240000
///       }
///     ],
///     "artwork_key": "artwork/tracks/demo-1.png",
///     "album_artwork_key": "artwork/albums/demo-album.png"
///   }
/// ]
/// ```
pub struct TestFixtureProvider {
    path: String,
    tracks: Vec<ProviderTrack>,
}

impl TestFixtureProvider {
    /// Creates a new fixture provider from a JSON file path.
    ///
    /// The file is read eagerly so that `fetch_catalog` is cheap and
    /// deterministic. If the file does not exist or is malformed, this
    /// constructor returns an error.
    pub fn new(path: impl AsRef<Path>) -> CanopyResult<Self> {
        let path = path.as_ref().to_string_lossy().to_string();
        let content =
            std::fs::read_to_string(&path).map_err(|e| CanopyError::Internal(e.to_string()))?;
        let tracks: Vec<ProviderTrack> =
            serde_json::from_str(&content).map_err(|e| CanopyError::Internal(e.to_string()))?;
        Ok(Self { path, tracks })
    }
}

#[async_trait]
impl ProviderAdapter for TestFixtureProvider {
    fn name(&self) -> &str {
        "fixture"
    }

    async fn fetch_catalog(&self) -> CanopyResult<Vec<ProviderTrack>> {
        Ok(self.tracks.clone())
    }
}
