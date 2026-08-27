//! Test fixture provider adapter.
//!
//! Reads a JSON file containing one or more [`ProviderTrack`] records and
//! returns them as a catalog batch. This is used for deterministic integration
//! testing and local development without relying on external API availability.

use std::path::Path;

use async_trait::async_trait;
use canopy_core::{CanopyError, CanopyResult, ProviderTrack};
use serde::Deserialize;

use crate::providers::adapter::ProviderAdapter;

/// Provider that reads [`ProviderTrack`] records from a JSON file.
///
/// The JSON file may be either a bare array of [`ProviderTrack`] objects or a
/// wrapped object with a `tracks` field. Example:
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
///         "storage_key": "audio/tracks/demo-1.m4a",
///         "size_bytes": 9600000,
///         "checksum_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
///         "duration_ms": 240000
///       }
///     ],
///     "artwork_storage_key": "artwork/tracks/demo-1.png",
///     "album_artwork_storage_key": "artwork/albums/demo-album.png"
///   }
/// ]
/// ```
#[derive(Deserialize)]
#[serde(untagged)]
enum FixtureCatalog {
    Tracks(Vec<ProviderTrack>),
    Wrapped { tracks: Vec<ProviderTrack> },
}

impl FixtureCatalog {
    fn into_tracks(self) -> Vec<ProviderTrack> {
        match self {
            Self::Tracks(tracks) | Self::Wrapped { tracks } => tracks,
        }
    }
}

pub struct TestFixtureProvider {
    #[allow(dead_code)]
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
        let catalog: FixtureCatalog =
            serde_json::from_str(&content).map_err(|e| CanopyError::Internal(e.to_string()))?;
        Ok(Self {
            path,
            tracks: catalog.into_tracks(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderAdapter;

    const TRACK: &str = r#"{
        "provider_id": "demo-1",
        "provider": "fixture",
        "title": "Demo Track",
        "artist": "Demo Artist",
        "album": "Demo Album",
        "release_year": 2024,
        "duration_ms": 240000,
        "is_explicit": false,
        "license": {
            "license_type": "CC0",
            "source_url": "https://musopen.org",
            "attribution_text": "Public Domain"
        },
        "assets": []
    }"#;

    fn write_catalog(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.json");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn reads_wrapped_catalog_object() {
        let (_dir, path) = write_catalog(&format!(
            r#"{{"schema_version":"1","tracks":[{TRACK},{TRACK}]}}"#
        ));
        let provider = TestFixtureProvider::new(path).unwrap();
        let tracks = provider.fetch_catalog().await.unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].provider, "fixture");
        assert_eq!(tracks[0].provider_id, "demo-1");
    }

    #[tokio::test]
    async fn reads_bare_track_array() {
        let (_dir, path) = write_catalog(&format!("[{TRACK}]"));
        let provider = TestFixtureProvider::new(path).unwrap();
        let tracks = provider.fetch_catalog().await.unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].provider_id, "demo-1");
    }
}
