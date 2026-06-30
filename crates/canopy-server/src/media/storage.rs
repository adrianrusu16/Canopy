//! Managed filesystem placement for local media.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use canopy_core::{CanopyError, CanopyResult};

use super::InspectedArtwork;

/// Managed media namespace used to validate relative storage keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaClass {
    /// Encoded audio under the audio namespace.
    Audio,
    /// Cover artwork under the artwork namespace.
    Artwork,
}

impl MediaClass {
    fn directory(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Artwork => "artwork",
        }
    }

    fn supports_extension(self, extension: &str) -> bool {
        match self {
            Self::Audio => extension.eq_ignore_ascii_case("mp3"),
            Self::Artwork => {
                extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("png")
            }
        }
    }
}

/// Builds a normalized content-addressed key for a managed media payload.
pub fn content_addressed_key(
    class: MediaClass,
    checksum_sha256: &str,
    extension: &str,
) -> CanopyResult<String> {
    if checksum_sha256.len() != 64 || !checksum_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(CanopyError::InvalidArgument(
            "checksum_sha256 must be 64 hexadecimal characters".into(),
        ));
    }
    if !class.supports_extension(extension) {
        return Err(CanopyError::InvalidArgument(format!(
            "unsupported {} extension: {extension}",
            class.directory()
        )));
    }

    let checksum = checksum_sha256.to_ascii_lowercase();
    Ok(format!(
        "{}/{}/{}/{}.{}",
        class.directory(),
        &checksum[0..2],
        &checksum[2..4],
        checksum,
        extension.to_ascii_lowercase()
    ))
}

/// Validates a relative key without resolving it against the filesystem.
pub fn validate_storage_key(key: &str, expected: MediaClass) -> CanopyResult<()> {
    if key.is_empty() || key.starts_with('/') || key.contains('\\') {
        return Err(CanopyError::InvalidArgument(
            "storage key must be a forward-slash relative path".into(),
        ));
    }

    let components: Vec<_> = key.split('/').collect();
    if components.len() < 2
        || components[0] != expected.directory()
        || components
            .iter()
            .any(|component| component.is_empty() || matches!(*component, "." | ".."))
    {
        return Err(CanopyError::InvalidArgument(
            "storage key escapes its managed media class".into(),
        ));
    }

    let extension = components
        .last()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension)
        .ok_or_else(|| CanopyError::InvalidArgument("storage key has no extension".into()))?;
    if !expected.supports_extension(extension) {
        return Err(CanopyError::InvalidArgument(format!(
            "unsupported {} extension: {extension}",
            expected.directory()
        )));
    }

    Ok(())
}

/// Files staged for one recoverable local-media import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedMedia {
    pub track_id: String,
    pub audio_storage_key: String,
    pub artwork_storage_key: Option<String>,
    audio_checksum_sha256: String,
    artwork_checksum_sha256: Option<String>,
}

impl StagedMedia {
    pub(crate) fn new(
        track_id: String,
        audio_storage_key: String,
        artwork_storage_key: Option<String>,
        audio_checksum_sha256: String,
        artwork_checksum_sha256: Option<String>,
    ) -> Self {
        Self {
            track_id,
            audio_storage_key,
            artwork_storage_key,
            audio_checksum_sha256,
            artwork_checksum_sha256,
        }
    }
}

/// Filesystem operations required by the media import state machine.
#[async_trait]
pub trait MediaStorage: Send + Sync {
    async fn stage(
        &self,
        track_id: &str,
        source: &Path,
        expected_audio_checksum: &str,
        artwork: Option<&InspectedArtwork>,
    ) -> CanopyResult<StagedMedia>;

    async fn finalize(&self, staged: &StagedMedia) -> CanopyResult<()>;

    async fn discard_before_persistence(&self, track_id: &str) -> CanopyResult<()>;
}

/// Managed local filesystem rooted at the configured Canopy media directory.
#[derive(Clone, Debug)]
pub struct ManagedMediaStore {
    root: PathBuf,
}

impl ManagedMediaStore {
    /// Creates a store handle. Call prepare before importing.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates private staging and managed-library directories.
    pub async fn prepare(&self) -> CanopyResult<()> {
        for relative in [
            "staging",
            "library/audio",
            "library/artwork",
            "quarantine",
            "originals",
        ] {
            tokio::fs::create_dir_all(self.root.join(relative))
                .await
                .map_err(|err| storage_err("create managed media directory", err))?;
        }
        Ok(())
    }

    fn staging_dir(&self, track_id: &str) -> CanopyResult<PathBuf> {
        validate_track_id(track_id)?;
        Ok(self.root.join("staging").join(track_id))
    }

    fn library_path(&self, key: &str, class: MediaClass) -> CanopyResult<PathBuf> {
        validate_storage_key(key, class)?;
        let mut path = self.root.join("library");
        for component in key.split('/') {
            path.push(component);
        }
        Ok(path)
    }

    async fn move_verified(
        &self,
        source: &Path,
        destination: &Path,
        expected_checksum: &str,
    ) -> CanopyResult<()> {
        let parent = destination.parent().ok_or_else(|| {
            CanopyError::InvalidArgument("managed destination has no parent".into())
        })?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| storage_err("create content-addressed directory", err))?;

        if tokio::fs::try_exists(destination)
            .await
            .map_err(|err| storage_err("inspect existing managed payload", err))?
        {
            let actual = hash_path(destination).await?;
            if !actual.eq_ignore_ascii_case(expected_checksum) {
                return Err(CanopyError::Storage(
                    "managed destination exists with different content".into(),
                ));
            }
            tokio::fs::remove_file(source)
                .await
                .map_err(|err| storage_err("remove duplicate staged payload", err))?;
            return Ok(());
        }

        tokio::fs::rename(source, destination)
            .await
            .map_err(|err| storage_err("atomically place managed payload", err))
    }
}

#[async_trait]
impl MediaStorage for ManagedMediaStore {
    async fn stage(
        &self,
        track_id: &str,
        source: &Path,
        expected_audio_checksum: &str,
        artwork: Option<&InspectedArtwork>,
    ) -> CanopyResult<StagedMedia> {
        validate_checksum(expected_audio_checksum)?;
        let metadata = tokio::fs::symlink_metadata(source)
            .await
            .map_err(|err| storage_err("inspect import source", err))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CanopyError::InvalidArgument(
                "import source must be a regular non-symlink file".into(),
            ));
        }

        let staging_dir = self.staging_dir(track_id)?;
        tokio::fs::create_dir_all(&staging_dir)
            .await
            .map_err(|err| storage_err("create track staging directory", err))?;

        let staged_audio = staging_dir.join("audio.mp3");
        tokio::fs::copy(source, &staged_audio)
            .await
            .map_err(|err| storage_err("copy audio into staging", err))?;
        let actual_audio_checksum = hash_path(&staged_audio).await?;
        if !actual_audio_checksum.eq_ignore_ascii_case(expected_audio_checksum) {
            let _ = tokio::fs::remove_dir_all(&staging_dir).await;
            return Err(CanopyError::InvalidArgument(
                "source changed after media inspection".into(),
            ));
        }

        let audio_storage_key =
            content_addressed_key(MediaClass::Audio, expected_audio_checksum, "mp3")?;
        let mut artwork_storage_key = None;
        let mut artwork_checksum_sha256 = None;

        if let Some(artwork) = artwork {
            validate_checksum(&artwork.checksum_sha256)?;
            let actual_artwork_checksum = hex::encode(Sha256::digest(&artwork.bytes));
            if !actual_artwork_checksum.eq_ignore_ascii_case(&artwork.checksum_sha256) {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                return Err(CanopyError::InvalidArgument(
                    "artwork checksum does not match inspected bytes".into(),
                ));
            }

            let extension = artwork.format.extension();
            let staged_artwork = staging_dir.join(format!("artwork.{extension}"));
            tokio::fs::write(&staged_artwork, &artwork.bytes)
                .await
                .map_err(|err| storage_err("write artwork into staging", err))?;
            artwork_storage_key = Some(content_addressed_key(
                MediaClass::Artwork,
                &artwork.checksum_sha256,
                extension,
            )?);
            artwork_checksum_sha256 = Some(artwork.checksum_sha256.to_ascii_lowercase());
        }

        Ok(StagedMedia::new(
            track_id.to_string(),
            audio_storage_key,
            artwork_storage_key,
            expected_audio_checksum.to_ascii_lowercase(),
            artwork_checksum_sha256,
        ))
    }

    async fn finalize(&self, staged: &StagedMedia) -> CanopyResult<()> {
        let staging_dir = self.staging_dir(&staged.track_id)?;
        let staged_audio = staging_dir.join("audio.mp3");
        let audio_destination = self.library_path(&staged.audio_storage_key, MediaClass::Audio)?;
        self.move_verified(
            &staged_audio,
            &audio_destination,
            &staged.audio_checksum_sha256,
        )
        .await?;

        if let (Some(key), Some(checksum)) = (
            staged.artwork_storage_key.as_deref(),
            staged.artwork_checksum_sha256.as_deref(),
        ) {
            let extension = key.rsplit_once('.').map(|(_, ext)| ext).ok_or_else(|| {
                CanopyError::InvalidArgument("artwork key has no extension".into())
            })?;
            let staged_artwork = staging_dir.join(format!("artwork.{extension}"));
            let artwork_destination = self.library_path(key, MediaClass::Artwork)?;
            self.move_verified(&staged_artwork, &artwork_destination, checksum)
                .await?;
        }

        tokio::fs::remove_dir(&staging_dir)
            .await
            .map_err(|err| storage_err("remove completed staging directory", err))?;
        Ok(())
    }

    async fn discard_before_persistence(&self, track_id: &str) -> CanopyResult<()> {
        let staging_dir = self.staging_dir(track_id)?;
        if tokio::fs::try_exists(&staging_dir)
            .await
            .map_err(|err| storage_err("inspect track staging directory", err))?
        {
            tokio::fs::remove_dir_all(staging_dir)
                .await
                .map_err(|err| storage_err("discard unpersisted staging", err))?;
        }
        Ok(())
    }
}

fn validate_track_id(track_id: &str) -> CanopyResult<()> {
    if track_id.is_empty()
        || !track_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(CanopyError::InvalidArgument(
            "track_id is not safe for managed staging".into(),
        ));
    }
    Ok(())
}

fn validate_checksum(checksum: &str) -> CanopyResult<()> {
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CanopyError::InvalidArgument(
            "checksum_sha256 must be 64 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

async fn hash_path(path: &Path) -> CanopyResult<String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|err| storage_err("open managed payload for hashing", err))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|err| storage_err("read managed payload for hashing", err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn storage_err(operation: &str, err: std::io::Error) -> CanopyError {
    CanopyError::Storage(format!("{operation}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ArtworkFormat;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    fn checksum(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn storage_key_requires_expected_media_class() {
        assert!(validate_storage_key("audio/aa/bb/hash.mp3", MediaClass::Audio).is_ok());
        assert!(validate_storage_key("artwork/aa/bb/hash.jpg", MediaClass::Artwork).is_ok());
        assert!(validate_storage_key("../audio/hash.mp3", MediaClass::Audio).is_err());
        assert!(validate_storage_key("/srv/canopy/hash.mp3", MediaClass::Audio).is_err());
        assert!(validate_storage_key("artwork/aa/bb/hash.jpg", MediaClass::Audio).is_err());
    }

    #[test]
    fn content_addressed_key_uses_checksum_prefixes() {
        let checksum = "aabb".to_string() + &"0".repeat(60);

        assert_eq!(
            content_addressed_key(MediaClass::Audio, &checksum, "mp3").unwrap(),
            format!("audio/aa/bb/{checksum}.mp3")
        );
    }

    #[tokio::test]
    async fn prepare_creates_managed_directories() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let store = ManagedMediaStore::new(&root);

        store.prepare().await.unwrap();

        for relative in [
            "staging",
            "library/audio",
            "library/artwork",
            "quarantine",
            "originals",
        ] {
            assert!(root.join(relative).is_dir(), "missing {relative}");
        }
    }

    #[tokio::test]
    async fn stage_and_finalize_preserve_source_and_place_content() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let source = temp.path().join("source.mp3");
        let audio = b"stable mp3 fixture bytes";
        tokio::fs::write(&source, audio).await.unwrap();
        let store = ManagedMediaStore::new(&root);
        store.prepare().await.unwrap();

        let staged = store
            .stage("track-1", &source, &checksum(audio), None)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&source).await.unwrap(), audio);
        assert!(root.join("staging/track-1/audio.mp3").is_file());

        store.finalize(&staged).await.unwrap();

        assert_eq!(
            tokio::fs::read(root.join("library").join(&staged.audio_storage_key))
                .await
                .unwrap(),
            audio
        );
        assert!(!root.join("staging/track-1").exists());
    }

    #[tokio::test]
    async fn stage_rejects_content_changed_after_inspection() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let source = temp.path().join("source.mp3");
        tokio::fs::write(&source, b"changed").await.unwrap();
        let store = ManagedMediaStore::new(&root);
        store.prepare().await.unwrap();

        let err = store
            .stage("track-2", &source, &checksum(b"original"), None)
            .await
            .unwrap_err();

        assert!(matches!(err, canopy_core::CanopyError::InvalidArgument(_)));
        assert!(tokio::fs::read(&source).await.is_ok());
    }

    #[tokio::test]
    async fn stage_and_finalize_place_artwork_by_validated_format() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let source = temp.path().join("source.mp3");
        let audio = b"audio";
        let artwork_bytes = b"jpeg fixture";
        tokio::fs::write(&source, audio).await.unwrap();
        let store = ManagedMediaStore::new(&root);
        store.prepare().await.unwrap();
        let artwork = InspectedArtwork {
            bytes: artwork_bytes.to_vec(),
            checksum_sha256: checksum(artwork_bytes),
            format: ArtworkFormat::Jpeg,
        };

        let staged = store
            .stage("track-3", &source, &checksum(audio), Some(&artwork))
            .await
            .unwrap();
        store.finalize(&staged).await.unwrap();

        let artwork_key = staged.artwork_storage_key.unwrap();
        assert_eq!(
            tokio::fs::read(root.join("library").join(artwork_key))
                .await
                .unwrap(),
            artwork_bytes
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn stage_rejects_symlink_source() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let target = temp.path().join("target.mp3");
        let source = temp.path().join("source.mp3");
        let audio = b"audio";
        tokio::fs::write(&target, audio).await.unwrap();
        symlink(&target, &source).unwrap();
        let store = ManagedMediaStore::new(&root);
        store.prepare().await.unwrap();

        let error = store
            .stage("track-symlink", &source, &checksum(audio), None)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            canopy_core::CanopyError::InvalidArgument(_)
        ));
        assert!(!root.join("staging/track-symlink").exists());
    }

    #[tokio::test]
    async fn discard_before_persistence_removes_only_staging() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let source = temp.path().join("source.mp3");
        let audio = b"audio";
        tokio::fs::write(&source, audio).await.unwrap();
        let store = ManagedMediaStore::new(&root);
        store.prepare().await.unwrap();
        store
            .stage("track-discard", &source, &checksum(audio), None)
            .await
            .unwrap();

        store
            .discard_before_persistence("track-discard")
            .await
            .unwrap();

        assert!(!root.join("staging/track-discard").exists());
        assert_eq!(tokio::fs::read(&source).await.unwrap(), audio);
    }

    #[tokio::test]
    async fn finalize_rejects_different_existing_content() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("media");
        let source = temp.path().join("source.mp3");
        let audio = b"audio";
        tokio::fs::write(&source, audio).await.unwrap();
        let store = ManagedMediaStore::new(&root);
        store.prepare().await.unwrap();
        let staged = store
            .stage("track-collision", &source, &checksum(audio), None)
            .await
            .unwrap();
        let destination = root.join("library").join(&staged.audio_storage_key);
        tokio::fs::create_dir_all(destination.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&destination, b"different").await.unwrap();

        let error = store.finalize(&staged).await.unwrap_err();

        assert!(matches!(error, canopy_core::CanopyError::Storage(_)));
        assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"different");
        assert!(root.join("staging/track-collision/audio.mp3").is_file());
    }
}
