use std::sync::Arc;

use canopy_core::{CanopyError, CanopyResult, PlayableAssetRepository};

use crate::media::storage::{MediaClass, validate_storage_key};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkGrant {
    pub internal_uri: String,
    pub content_type: String,
}

pub struct ArtworkAuthorizer {
    assets: Arc<dyn PlayableAssetRepository>,
}

impl ArtworkAuthorizer {
    pub fn new(assets: Arc<dyn PlayableAssetRepository>) -> Self {
        Self { assets }
    }

    pub async fn authorize(
        &self,
        artwork_id: &str,
        content_hash: &str,
    ) -> CanopyResult<ArtworkGrant> {
        let asset = self
            .assets
            .authorize_artwork(artwork_id, content_hash)
            .await?
            .ok_or_else(invalid_artwork)?;

        validate_storage_key(&asset.storage_key, MediaClass::Artwork)
            .map_err(|_| invalid_artwork())?;

        Ok(ArtworkGrant {
            internal_uri: format!("/_canopy_media/{}", asset.storage_key),
            content_type: asset.content_type,
        })
    }
}

fn invalid_artwork() -> CanopyError {
    CanopyError::unauthenticated("invalid artwork")
}
