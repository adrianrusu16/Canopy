//! Managed local-media inspection, storage, and import orchestration.

/// Validated artwork encoding accepted by managed storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtworkFormat {
    /// JPEG artwork.
    Jpeg,
    /// PNG artwork.
    Png,
}

impl ArtworkFormat {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }
}

/// Bounded and validated artwork extracted during media inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectedArtwork {
    /// Original accepted JPEG or PNG bytes.
    pub bytes: Vec<u8>,
    /// Lowercase SHA-256 checksum of the artwork bytes.
    pub checksum_sha256: String,
    /// Validated artwork encoding.
    pub format: ArtworkFormat,
}

pub mod inspector;
pub mod service;
pub mod storage;
