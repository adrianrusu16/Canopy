//! MP3 metadata, checksum, and artwork inspection.

use std::borrow::Cow;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

use async_trait::async_trait;
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::Accessor;
use sha2::{Digest, Sha256};

use canopy_core::{CanopyError, CanopyResult};

use super::{ArtworkFormat, InspectedArtwork};

/// Validated metadata and payload facts extracted from one MP3 source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectedMp3 {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub artwork: Option<InspectedArtwork>,
}

/// Inspection boundary used by the import orchestration service.
#[async_trait]
pub trait MediaInspector: Send + Sync {
    async fn inspect(&self, source: &Path) -> CanopyResult<InspectedMp3>;
}

/// Lofty-backed MP3 inspector with explicit payload limits.
#[derive(Clone, Debug)]
pub struct Mp3Inspector {
    max_audio_bytes: u64,
    max_artwork_bytes: u64,
}

impl Mp3Inspector {
    pub fn new(max_audio_bytes: u64, max_artwork_bytes: u64) -> Self {
        Self {
            max_audio_bytes,
            max_artwork_bytes,
        }
    }
}

#[async_trait]
impl MediaInspector for Mp3Inspector {
    async fn inspect(&self, source: &Path) -> CanopyResult<InspectedMp3> {
        let source = source.to_path_buf();
        let max_audio_bytes = self.max_audio_bytes;
        let max_artwork_bytes = self.max_artwork_bytes;
        tokio::task::spawn_blocking(move || {
            inspect_mp3(&source, max_audio_bytes, max_artwork_bytes)
        })
        .await
        .map_err(|err| CanopyError::Internal(format!("MP3 inspection task failed: {err}")))?
    }
}

fn inspect_mp3(
    source: &Path,
    max_audio_bytes: u64,
    max_artwork_bytes: u64,
) -> CanopyResult<InspectedMp3> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|err| invalid_media(format!("unable to inspect source: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_media("source must be a regular non-symlink file"));
    }
    if metadata.len() > max_audio_bytes {
        return Err(invalid_media("audio exceeds configured byte limit"));
    }
    if source
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("mp3"))
    {
        return Err(invalid_media("source extension must be mp3"));
    }

    let probe = Probe::open(source).map_err(|_| invalid_media("source is not valid MP3 audio"))?;
    let probe = probe
        .guess_file_type()
        .map_err(|_| invalid_media("source is not valid MP3 audio"))?;
    let tagged = probe
        .read()
        .map_err(|_| invalid_media("source is not valid MP3 audio"))?;
    if tagged.file_type() != FileType::Mpeg {
        return Err(invalid_media("source is not MP3 audio"));
    }

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let title_fallback = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or("Unknown Track");
    let title = normalized_tag(tag.and_then(Accessor::title), title_fallback);
    let artist = normalized_tag(tag.and_then(Accessor::artist), "Unknown Artist");
    let album = normalized_tag(tag.and_then(Accessor::album), "Unknown Album");
    let duration_ms = u64::try_from(tagged.properties().duration().as_millis())
        .map_err(|_| invalid_media("audio duration exceeds supported range"))?;

    let embedded = tag.and_then(|tag| {
        tag.get_picture_type(PictureType::CoverFront)
            .or_else(|| tag.pictures().first())
    });
    let artwork = if let Some(picture) = embedded {
        Some(validate_artwork_bytes(
            picture.data().to_vec(),
            max_artwork_bytes,
        )?)
    } else {
        inspect_sidecar(source, max_artwork_bytes)?
    };

    Ok(InspectedMp3 {
        title,
        artist,
        album,
        duration_ms,
        size_bytes: metadata.len(),
        checksum_sha256: hash_file(source)?,
        artwork,
    })
}

fn normalized_tag(value: Option<Cow<'_, str>>, fallback: &str) -> String {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn inspect_sidecar(
    source: &Path,
    max_artwork_bytes: u64,
) -> CanopyResult<Option<InspectedArtwork>> {
    let Some(parent) = source.parent() else {
        return Ok(None);
    };
    for name in ["cover.jpg", "cover.png"] {
        let candidate = parent.join(name);
        if candidate.exists() {
            let metadata = std::fs::symlink_metadata(&candidate)
                .map_err(|err| invalid_artwork(format!("unable to inspect artwork: {err}")))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(invalid_artwork(
                    "artwork must be a regular non-symlink file",
                ));
            }
            if metadata.len() > max_artwork_bytes {
                return Err(invalid_artwork("artwork exceeds configured byte limit"));
            }
            let bytes = std::fs::read(&candidate)
                .map_err(|err| invalid_artwork(format!("unable to read artwork: {err}")))?;
            return validate_artwork_bytes(bytes, max_artwork_bytes).map(Some);
        }
    }
    Ok(None)
}

fn validate_artwork_bytes(
    bytes: Vec<u8>,
    max_artwork_bytes: u64,
) -> CanopyResult<InspectedArtwork> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_artwork_bytes {
        return Err(invalid_artwork("artwork exceeds configured byte limit"));
    }
    let mut cursor = Cursor::new(bytes.as_slice());
    let picture = Picture::from_reader(&mut cursor)
        .map_err(|_| invalid_artwork("artwork is not a valid JPEG or PNG"))?;
    let format = match picture.mime_type() {
        Some(MimeType::Jpeg) => ArtworkFormat::Jpeg,
        Some(MimeType::Png) => ArtworkFormat::Png,
        _ => return Err(invalid_artwork("artwork must be JPEG or PNG")),
    };
    let checksum_sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(InspectedArtwork {
        bytes,
        checksum_sha256,
        format,
    })
}

fn hash_file(path: &Path) -> CanopyResult<String> {
    let file = File::open(path)
        .map_err(|err| invalid_media(format!("unable to open audio for hashing: {err}")))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| invalid_media(format!("unable to hash audio: {err}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn invalid_media(message: impl Into<String>) -> CanopyError {
    CanopyError::InvalidArgument(format!("invalid media: {}", message.into()))
}

fn invalid_artwork(message: impl Into<String>) -> CanopyError {
    CanopyError::InvalidArgument(format!("invalid artwork: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;
    use crate::media::ArtworkFormat;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/media")
            .join(name)
    }

    async fn copy_fixture(temp: &TempDir, name: &str, target: &str) -> PathBuf {
        let path = temp.path().join(target);
        tokio::fs::copy(fixture(name), &path).await.unwrap();
        path
    }

    #[tokio::test]
    async fn inspector_reads_mp3_tags_and_duration() {
        let temp = TempDir::new().unwrap();
        let source = copy_fixture(&temp, "test-tone.mp3", "song.mp3").await;
        let inspector = Mp3Inspector::new(2_000_000, 1_000_000);

        let inspected = inspector.inspect(&source).await.unwrap();

        assert_eq!(inspected.title, "Test Tone");
        assert_eq!(inspected.artist, "Canopy Tests");
        assert_eq!(inspected.album, "Importer Fixtures");
        assert!((900..=1_100).contains(&inspected.duration_ms));
        assert_eq!(inspected.checksum_sha256.len(), 64);
    }

    #[tokio::test]
    async fn embedded_cover_wins_over_sidecar() {
        let temp = TempDir::new().unwrap();
        let source = copy_fixture(&temp, "test-tone-embedded.mp3", "song.mp3").await;
        copy_fixture(&temp, "cover.png", "cover.png").await;
        let inspector = Mp3Inspector::new(2_000_000, 1_000_000);

        let artwork = inspector
            .inspect(&source)
            .await
            .unwrap()
            .artwork
            .expect("embedded artwork should exist");

        assert_eq!(artwork.format, ArtworkFormat::Jpeg);
        assert_eq!(
            artwork.checksum_sha256,
            "2c92506da715009ab80b900244aba42b935bf5028a424adbb5f1c4a559cac1b3"
        );
    }

    #[tokio::test]
    async fn jpg_sidecar_precedes_png_sidecar() {
        let temp = TempDir::new().unwrap();
        let source = copy_fixture(&temp, "test-tone.mp3", "song.mp3").await;
        copy_fixture(&temp, "cover.jpg", "cover.jpg").await;
        copy_fixture(&temp, "cover.png", "cover.png").await;
        let inspector = Mp3Inspector::new(2_000_000, 1_000_000);

        let artwork = inspector
            .inspect(&source)
            .await
            .unwrap()
            .artwork
            .expect("sidecar artwork should exist");

        assert_eq!(artwork.format, ArtworkFormat::Jpeg);
        assert_eq!(
            artwork.checksum_sha256,
            "2c92506da715009ab80b900244aba42b935bf5028a424adbb5f1c4a559cac1b3"
        );
    }

    #[tokio::test]
    async fn missing_artwork_is_allowed() {
        let temp = TempDir::new().unwrap();
        let source = copy_fixture(&temp, "test-tone.mp3", "song.mp3").await;
        let inspector = Mp3Inspector::new(2_000_000, 1_000_000);

        let inspected = inspector.inspect(&source).await.unwrap();

        assert!(inspected.artwork.is_none());
    }

    #[tokio::test]
    async fn malformed_mp3_is_rejected() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("broken.mp3");
        tokio::fs::write(&source, b"not an mp3").await.unwrap();
        let inspector = Mp3Inspector::new(2_000_000, 1_000_000);

        let error = inspector.inspect(&source).await.unwrap_err();

        assert!(matches!(
            error,
            canopy_core::CanopyError::InvalidArgument(_)
        ));
    }

    #[tokio::test]
    async fn oversized_audio_is_rejected_before_parsing() {
        let temp = TempDir::new().unwrap();
        let source = copy_fixture(&temp, "test-tone.mp3", "song.mp3").await;
        let inspector = Mp3Inspector::new(1, 1_000_000);

        let error = inspector.inspect(&source).await.unwrap_err();

        assert!(matches!(
            error,
            canopy_core::CanopyError::InvalidArgument(_)
        ));
    }
}
