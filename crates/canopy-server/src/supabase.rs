//! Supabase Storage integration.
//!
//! Canopy asks Supabase Storage for short-lived signed object URLs and returns
//! those URLs to the player. Canopy remains a control-plane proxy and does not
//! serve audio bytes.

use canopy_core::{CanopyError, CanopyResult};
use serde::{Deserialize, Serialize};

use crate::playback::PlaybackUrlProvider;

/// Configuration for Supabase Storage signed URL generation.
#[derive(Clone, Debug)]
pub struct SupabaseConfig {
    /// Supabase project URL, e.g. `https://project.supabase.co`.
    pub project_url: String,
    /// Supabase anon or service role key.
    pub api_key: String,
    /// Storage bucket containing music objects.
    pub bucket: String,
    /// Signed URL lifetime in seconds.
    pub signed_url_ttl_secs: u64,
}

/// Client that requests signed Storage URLs from Supabase.
#[derive(Clone)]
pub struct SupabaseStorageUrlProvider {
    client: reqwest::Client,
    config: SupabaseConfig,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignObjectRequest {
    expires_in: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignObjectResponse {
    #[serde(alias = "signedURL")]
    signed_url: String,
}

impl SupabaseStorageUrlProvider {
    /// Creates a Supabase Storage URL provider.
    pub fn new(config: SupabaseConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    fn sign_endpoint(&self, storage_key: &str) -> CanopyResult<String> {
        if self.config.project_url.trim().is_empty() {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_SUPABASE_URL is required when CANOPY_MUSIC_SOURCE=supabase".to_string(),
            ));
        }
        if self.config.api_key.trim().is_empty() {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_SUPABASE_KEY is required when CANOPY_MUSIC_SOURCE=supabase".to_string(),
            ));
        }
        if self.config.bucket.trim().is_empty() {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_SUPABASE_STORAGE_BUCKET is required when CANOPY_MUSIC_SOURCE=supabase"
                    .to_string(),
            ));
        }

        let base = self.config.project_url.trim_end_matches('/');
        let key = storage_key.trim_start_matches('/');
        Ok(format!(
            "{base}/storage/v1/object/sign/{bucket}/{key}",
            bucket = self.config.bucket
        ))
    }
}

#[async_trait::async_trait]
impl PlaybackUrlProvider for SupabaseStorageUrlProvider {
    async fn signed_url(
        &self,
        storage_key: &str,
        _expires_at_epoch_ms: u64,
    ) -> CanopyResult<String> {
        let endpoint = self.sign_endpoint(storage_key)?;
        let response = self
            .client
            .post(endpoint)
            .header("apikey", &self.config.api_key)
            .bearer_auth(&self.config.api_key)
            .json(&SignObjectRequest {
                expires_in: self.config.signed_url_ttl_secs,
            })
            .send()
            .await
            .map_err(|err| CanopyError::Storage(err.to_string()))?;

        if !response.status().is_success() {
            return Err(CanopyError::Storage(format!(
                "Supabase signed URL request failed: {}",
                response.status()
            )));
        }

        let body = response
            .json::<SignObjectResponse>()
            .await
            .map_err(|err| CanopyError::Storage(err.to_string()))?;
        Ok(expand_signed_url(
            &self.config.project_url,
            &body.signed_url,
        ))
    }
}

fn expand_signed_url(project_url: &str, signed_url: &str) -> String {
    if signed_url.starts_with("http://") || signed_url.starts_with("https://") {
        signed_url.to_string()
    } else {
        format!(
            "{}/{}",
            project_url.trim_end_matches('/'),
            signed_url.trim_start_matches('/')
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_url_response_accepts_storage_api_casing() {
        let body: SignObjectResponse =
            serde_json::from_str(r#"{"signedURL":"/storage/v1/object/sign/music/demo.mp3"}"#)
                .unwrap();
        assert_eq!(body.signed_url, "/storage/v1/object/sign/music/demo.mp3");
    }

    #[test]
    fn signed_url_response_expands_relative_path() {
        let url = expand_signed_url(
            "https://demo.supabase.co",
            "/storage/v1/object/sign/music/audio/demo.mp3?token=abc",
        );
        assert_eq!(
            url,
            "https://demo.supabase.co/storage/v1/object/sign/music/audio/demo.mp3?token=abc"
        );
    }

    #[test]
    fn signed_url_response_preserves_absolute_url() {
        let url = expand_signed_url(
            "https://demo.supabase.co",
            "https://cdn.example.test/audio/demo.mp3?token=abc",
        );
        assert_eq!(url, "https://cdn.example.test/audio/demo.mp3?token=abc");
    }
}
