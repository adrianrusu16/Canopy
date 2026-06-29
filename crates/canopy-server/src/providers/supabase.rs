//! Supabase catalog provider adapter.
//!
//! Fetches music metadata from Supabase's auto-generated REST API and maps rows
//! into the provider ingestion model used by Canopy's catalog pipeline.

use async_trait::async_trait;
use canopy_core::{CanopyError, CanopyResult, ProviderAudioAsset, ProviderLicense, ProviderTrack};
use serde::Deserialize;

use crate::providers::adapter::ProviderAdapter;

/// Runtime configuration for Supabase catalog fetches.
#[derive(Clone, Debug)]
pub struct SupabaseCatalogConfig {
    /// Supabase project URL, e.g. `https://project.supabase.co`.
    pub project_url: String,
    /// Supabase anon or service role key used for REST reads.
    pub api_key: String,
    /// Supabase REST table or view containing normalized music rows.
    pub table: String,
}

/// Provider adapter that reads music rows from Supabase REST.
#[derive(Clone)]
pub struct SupabaseCatalogProvider {
    client: reqwest::Client,
    config: SupabaseCatalogConfig,
}

/// Flexible row shape expected from Supabase REST.
///
/// This is intentionally close to `ProviderTrack`, with flat license fields and
/// an `assets` JSON array matching `ProviderAudioAsset`. It works well as an
/// initial Supabase view/table contract and can later be backed by normalized
/// Supabase tables without changing Canopy's ingestion boundary.
#[derive(Clone, Debug, Deserialize)]
struct SupabaseCatalogRow {
    #[serde(alias = "provider_id")]
    id: String,
    #[serde(default)]
    provider: Option<String>,
    title: String,
    artist: String,
    #[serde(default)]
    album: String,
    #[serde(default)]
    release_year: Option<i32>,
    #[serde(default)]
    duration_ms: i64,
    #[serde(default)]
    is_explicit: bool,
    #[serde(default)]
    license_type: String,
    #[serde(default, alias = "source_url")]
    license_url: String,
    #[serde(default, alias = "attribution_text")]
    attribution: String,
    #[serde(default)]
    assets: Vec<ProviderAudioAsset>,
    #[serde(default)]
    artwork_key: Option<String>,
    #[serde(default)]
    album_artwork_key: Option<String>,
}

impl SupabaseCatalogProvider {
    /// Creates a provider over the given Supabase REST configuration.
    pub fn new(config: SupabaseCatalogConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    fn catalog_endpoint(&self) -> CanopyResult<String> {
        validate_supabase_catalog_config(&self.config)?;
        Ok(format!(
            "{}/rest/v1/{}?select=*",
            self.config.project_url.trim_end_matches('/'),
            self.config.table
        ))
    }
}

#[async_trait]
impl ProviderAdapter for SupabaseCatalogProvider {
    fn name(&self) -> &str {
        "supabase"
    }

    async fn fetch_catalog(&self) -> CanopyResult<Vec<ProviderTrack>> {
        let endpoint = self.catalog_endpoint()?;
        let response = self
            .client
            .get(endpoint)
            .header("apikey", &self.config.api_key)
            .bearer_auth(&self.config.api_key)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|err| CanopyError::Storage(err.to_string()))?;

        if !response.status().is_success() {
            return Err(CanopyError::Storage(format!(
                "Supabase catalog request failed: {}",
                response.status()
            )));
        }

        let rows = response
            .json::<Vec<SupabaseCatalogRow>>()
            .await
            .map_err(|err| CanopyError::Storage(err.to_string()))?;
        Ok(rows
            .into_iter()
            .map(SupabaseCatalogRow::into_provider_track)
            .collect())
    }
}

impl SupabaseCatalogRow {
    fn into_provider_track(self) -> ProviderTrack {
        ProviderTrack {
            provider_id: self.id,
            provider: self.provider.unwrap_or_else(|| "supabase".to_string()),
            title: self.title,
            artist: self.artist,
            album: self.album,
            release_year: self.release_year,
            duration_ms: self.duration_ms,
            is_explicit: self.is_explicit,
            license: ProviderLicense {
                license_type: self.license_type,
                source_url: self.license_url,
                attribution_text: self.attribution,
            },
            assets: self.assets,
            artwork_storage_key: self.artwork_key,
            album_artwork_storage_key: self.album_artwork_key,
        }
    }
}

fn validate_supabase_catalog_config(config: &SupabaseCatalogConfig) -> CanopyResult<()> {
    if config.project_url.trim().is_empty() {
        return Err(CanopyError::InvalidArgument(
            "CANOPY_SUPABASE_URL is required when syncing Supabase catalog".to_string(),
        ));
    }
    if config.api_key.trim().is_empty() {
        return Err(CanopyError::InvalidArgument(
            "CANOPY_SUPABASE_KEY is required when syncing Supabase catalog".to_string(),
        ));
    }
    if config.table.trim().is_empty()
        || config
            .table
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
    {
        return Err(CanopyError::InvalidArgument(
            "CANOPY_SUPABASE_CATALOG_TABLE must be a non-empty table or view name".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_endpoint_targets_rest_table_with_select_all() {
        let config = SupabaseCatalogConfig {
            project_url: "https://demo.supabase.co/".to_string(),
            api_key: "key".to_string(),
            table: "tracks".to_string(),
        };
        let provider = SupabaseCatalogProvider::new(config);

        assert_eq!(
            provider.catalog_endpoint().unwrap(),
            "https://demo.supabase.co/rest/v1/tracks?select=*"
        );
    }

    #[test]
    fn row_maps_into_provider_track() {
        let row: SupabaseCatalogRow = serde_json::from_str(
            r#"{
              "id": "song-1",
              "title": "Soft Signal",
              "artist": "Canopy Test",
              "album": "Backend Sessions",
              "release_year": 2026,
              "duration_ms": 181000,
              "is_explicit": false,
              "license_type": "Private",
              "license_url": "https://example.test/license",
              "attribution": "Canopy Test",
              "assets": [
                {
                  "codec": "mp3",
                  "content_type": "audio/mpeg",
                  "object_key": "audio/song-1.mp3",
                  "size_bytes": 1234,
                  "checksum_sha256": "abc",
                  "duration_ms": 181000
                }
              ],
              "artwork_key": "artwork/song-1.png"
            }"#,
        )
        .unwrap();

        let track = row.into_provider_track();

        assert_eq!(track.provider, "supabase");
        assert_eq!(track.provider_id, "song-1");
        assert_eq!(track.title, "Soft Signal");
        assert_eq!(track.artist, "Canopy Test");
        assert_eq!(track.album, "Backend Sessions");
        assert_eq!(track.release_year, Some(2026));
        assert_eq!(track.license.license_type, "Private");
        assert_eq!(track.license.source_url, "https://example.test/license");
        assert_eq!(track.assets[0].storage_key, "audio/song-1.mp3");
        assert_eq!(
            track.artwork_storage_key.as_deref(),
            Some("artwork/song-1.png")
        );
    }
}
