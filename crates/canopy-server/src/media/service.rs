//! Recoverable orchestration for owner-scoped local media imports.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use canopy_core::{
    AudioAsset, CanopyError, CanopyResult, InstanceSettingsRepository, MediaImportRepository,
    PendingImportOutcome, PendingMediaImport,
};

use super::inspector::MediaInspector;
use super::storage::MediaStorage;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportItemStatus {
    Imported,
    Duplicate,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportItemResult {
    pub file_name: String,
    pub status: ImportItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportSummary {
    pub items: Vec<ImportItemResult>,
    pub imported: u32,
    pub duplicate: u32,
    pub failed: u32,
}

pub struct MediaImportService {
    settings: Arc<dyn InstanceSettingsRepository>,
    repository: Arc<dyn MediaImportRepository>,
    inspector: Arc<dyn MediaInspector>,
    storage: Arc<dyn MediaStorage>,
}

impl MediaImportService {
    pub fn new(
        settings: Arc<dyn InstanceSettingsRepository>,
        repository: Arc<dyn MediaImportRepository>,
        inspector: Arc<dyn MediaInspector>,
        storage: Arc<dyn MediaStorage>,
    ) -> Self {
        Self {
            settings,
            repository,
            inspector,
            storage,
        }
    }

    pub async fn import_path(&self, source: &Path) -> CanopyResult<ImportSummary> {
        let owner_profile_id = self
            .settings
            .owner_profile_id()
            .await?
            .filter(|owner| !owner.trim().is_empty())
            .ok_or_else(|| {
                CanopyError::InvalidArgument(
                    "instance owner must be configured before importing media".into(),
                )
            })?;
        let sources = select_sources(source).await?;
        if sources.is_empty() {
            return Err(CanopyError::InvalidArgument(
                "directory contains no top-level MP3 files".into(),
            ));
        }

        let mut summary = ImportSummary {
            items: Vec::with_capacity(sources.len()),
            imported: 0,
            duplicate: 0,
            failed: 0,
        };
        for source in sources {
            let item = self.import_one(&owner_profile_id, &source).await;
            match item.status {
                ImportItemStatus::Imported => summary.imported += 1,
                ImportItemStatus::Duplicate => summary.duplicate += 1,
                ImportItemStatus::Failed => summary.failed += 1,
            }
            summary.items.push(item);
        }
        Ok(summary)
    }

    async fn import_one(&self, owner_profile_id: &str, source: &Path) -> ImportItemResult {
        let file_name = display_file_name(source);
        let inspected = match self.inspector.inspect(source).await {
            Ok(inspected) => inspected,
            Err(error) => return failed(file_name, inspection_error_code(&error), None),
        };
        match self
            .repository
            .find_track_by_audio_checksum(&inspected.checksum_sha256)
            .await
        {
            Ok(Some(track_id)) => return duplicate(file_name, track_id),
            Ok(None) => {}
            Err(_) => return failed(file_name, "database_unavailable", None),
        }

        let track_id = Uuid::new_v4().to_string();
        let staged = match self
            .storage
            .stage(
                &track_id,
                source,
                &inspected.checksum_sha256,
                inspected.artwork.as_ref(),
            )
            .await
        {
            Ok(staged) => staged,
            Err(error) => return failed(file_name, staging_error_code(&error), None),
        };
        let pending = PendingMediaImport {
            track_id: track_id.clone(),
            owner_profile_id: owner_profile_id.to_string(),
            title: inspected.title,
            artist: inspected.artist,
            album: inspected.album,
            duration_ms: inspected.duration_ms,
            artwork_storage_key: staged.artwork_storage_key.clone(),
            audio: AudioAsset {
                track_id: track_id.clone(),
                codec: "mp3".into(),
                content_type: "audio/mpeg".into(),
                storage_key: staged.audio_storage_key.clone(),
                size_bytes: inspected.size_bytes,
                checksum_sha256: inspected.checksum_sha256,
                duration_ms: inspected.duration_ms,
            },
        };

        match self.repository.insert_pending(&pending).await {
            Ok(PendingImportOutcome::Inserted) => {}
            Ok(PendingImportOutcome::Duplicate { track_id: winner }) => {
                return match self.storage.discard_before_persistence(&track_id).await {
                    Ok(()) => duplicate(file_name, winner),
                    Err(_) => failed(file_name, "storage_unavailable", Some(winner)),
                };
            }
            Err(_) => {
                let code = if self
                    .storage
                    .discard_before_persistence(&track_id)
                    .await
                    .is_ok()
                {
                    "database_unavailable"
                } else {
                    "storage_unavailable"
                };
                return failed(file_name, code, None);
            }
        }

        if self.storage.finalize(&staged).await.is_err() {
            return failed(file_name, "storage_unavailable", Some(track_id));
        }
        if self.repository.mark_ready(&track_id).await.is_err() {
            return failed(file_name, "database_unavailable", Some(track_id));
        }
        ImportItemResult {
            file_name,
            status: ImportItemStatus::Imported,
            track_id: Some(track_id),
            error_code: None,
        }
    }
}

async fn select_sources(source: &Path) -> CanopyResult<Vec<PathBuf>> {
    let metadata = tokio::fs::symlink_metadata(source)
        .await
        .map_err(|_| CanopyError::InvalidArgument("media source does not exist".into()))?;
    if metadata.is_file() && !metadata.file_type().is_symlink() {
        return Ok(vec![source.to_path_buf()]);
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CanopyError::InvalidArgument(
            "media source must be a regular file or directory".into(),
        ));
    }

    let mut entries = tokio::fs::read_dir(source)
        .await
        .map_err(|_| CanopyError::Storage("unable to read media directory".into()))?;
    let mut sources = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| CanopyError::Storage("unable to read media directory entry".into()))?
    {
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| CanopyError::Storage("unable to inspect media directory entry".into()))?;
        let is_mp3 = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
        if metadata.is_file() && !metadata.file_type().is_symlink() && is_mp3 {
            sources.push(path);
        }
    }
    sources.sort_by(|left, right| {
        let left_name = display_file_name(left);
        let right_name = display_file_name(right);
        left_name
            .to_lowercase()
            .cmp(&right_name.to_lowercase())
            .then_with(|| left_name.cmp(&right_name))
    });
    Ok(sources)
}

fn display_file_name(source: &Path) -> String {
    source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown.mp3".into())
}

fn inspection_error_code(error: &CanopyError) -> &'static str {
    match error {
        CanopyError::InvalidArgument(message) if message.starts_with("invalid artwork:") => {
            "invalid_artwork"
        }
        _ => "invalid_media",
    }
}

fn staging_error_code(error: &CanopyError) -> &'static str {
    match error {
        CanopyError::InvalidArgument(_) => "invalid_media",
        _ => "storage_unavailable",
    }
}

fn duplicate(file_name: String, track_id: String) -> ImportItemResult {
    ImportItemResult {
        file_name,
        status: ImportItemStatus::Duplicate,
        track_id: Some(track_id),
        error_code: None,
    }
}

fn failed(
    file_name: String,
    error_code: &'static str,
    track_id: Option<String>,
) -> ImportItemResult {
    ImportItemResult {
        file_name,
        status: ImportItemStatus::Failed,
        track_id,
        error_code: Some(error_code.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use canopy_core::{
        CanopyError, CanopyResult, InstanceSettingsRepository, MediaImportRepository,
        PendingImportOutcome, PendingMediaImport,
    };

    use super::MediaImportService;
    use crate::media::InspectedArtwork;
    use crate::media::inspector::{InspectedMp3, MediaInspector};
    use crate::media::storage::{MediaStorage, StagedMedia};

    struct MissingOwner;

    #[async_trait]
    impl InstanceSettingsRepository for MissingOwner {
        async fn set_owner_profile_id(&self, _profile_id: &str) -> CanopyResult<()> {
            unreachable!()
        }

        async fn owner_profile_id(&self) -> CanopyResult<Option<String>> {
            Ok(None)
        }
    }

    struct MustNotRun;

    #[async_trait]
    impl MediaImportRepository for MustNotRun {
        async fn find_track_by_audio_checksum(
            &self,
            _checksum: &str,
        ) -> CanopyResult<Option<String>> {
            panic!("repository must not run without an owner")
        }

        async fn insert_pending(
            &self,
            _pending: &PendingMediaImport,
        ) -> CanopyResult<PendingImportOutcome> {
            panic!("repository must not run without an owner")
        }

        async fn mark_ready(&self, _track_id: &str) -> CanopyResult<()> {
            panic!("repository must not run without an owner")
        }
    }

    #[async_trait]
    impl MediaInspector for MustNotRun {
        async fn inspect(&self, _source: &Path) -> CanopyResult<InspectedMp3> {
            panic!("inspection must not run without an owner")
        }
    }

    #[async_trait]
    impl MediaStorage for MustNotRun {
        async fn stage(
            &self,
            _track_id: &str,
            _source: &Path,
            _checksum: &str,
            _artwork: Option<&InspectedArtwork>,
        ) -> CanopyResult<StagedMedia> {
            panic!("staging must not run without an owner")
        }

        async fn finalize(&self, _staged: &StagedMedia) -> CanopyResult<()> {
            panic!("finalization must not run without an owner")
        }

        async fn discard_before_persistence(&self, _track_id: &str) -> CanopyResult<()> {
            panic!("discard must not run without an owner")
        }
    }

    #[tokio::test]
    async fn missing_owner_fails_before_inspection_or_staging() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("track.mp3");
        std::fs::write(&source, b"fixture").unwrap();
        let must_not_run = Arc::new(MustNotRun);
        let service = MediaImportService::new(
            Arc::new(MissingOwner),
            must_not_run.clone(),
            must_not_run.clone(),
            must_not_run,
        );

        let error = service.import_path(&source).await.unwrap_err();

        assert!(matches!(error, CanopyError::InvalidArgument(_)));
    }
    #[derive(Clone, Copy)]
    enum InsertBehavior {
        Inserted,
        Duplicate,
        Fail,
    }

    struct World {
        existing: Mutex<Option<String>>,
        insert_behavior: Mutex<InsertBehavior>,
        rejected: Mutex<HashSet<String>>,
        finalize_fails: AtomicBool,
        ready_fails: AtomicBool,
        events: Mutex<Vec<String>>,
    }

    impl World {
        fn new() -> Self {
            Self {
                existing: Mutex::new(None),
                insert_behavior: Mutex::new(InsertBehavior::Inserted),
                rejected: Mutex::new(HashSet::new()),
                finalize_fails: AtomicBool::new(false),
                ready_fails: AtomicBool::new(false),
                events: Mutex::new(Vec::new()),
            }
        }
    }

    struct FixedOwner;

    #[async_trait]
    impl InstanceSettingsRepository for FixedOwner {
        async fn set_owner_profile_id(&self, _profile_id: &str) -> CanopyResult<()> {
            unreachable!()
        }

        async fn owner_profile_id(&self) -> CanopyResult<Option<String>> {
            Ok(Some("owner-a".into()))
        }
    }

    #[async_trait]
    impl MediaImportRepository for World {
        async fn find_track_by_audio_checksum(
            &self,
            _checksum: &str,
        ) -> CanopyResult<Option<String>> {
            self.events.lock().unwrap().push("find".into());
            Ok(self.existing.lock().unwrap().clone())
        }

        async fn insert_pending(
            &self,
            _pending: &PendingMediaImport,
        ) -> CanopyResult<PendingImportOutcome> {
            self.events.lock().unwrap().push("insert".into());
            match *self.insert_behavior.lock().unwrap() {
                InsertBehavior::Inserted => Ok(PendingImportOutcome::Inserted),
                InsertBehavior::Duplicate => Ok(PendingImportOutcome::Duplicate {
                    track_id: "winning-track".into(),
                }),
                InsertBehavior::Fail => Err(CanopyError::Storage("database unavailable".into())),
            }
        }

        async fn mark_ready(&self, _track_id: &str) -> CanopyResult<()> {
            self.events.lock().unwrap().push("ready".into());
            if self.ready_fails.load(Ordering::Relaxed) {
                Err(CanopyError::Storage("database unavailable".into()))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl MediaInspector for World {
        async fn inspect(&self, source: &Path) -> CanopyResult<InspectedMp3> {
            let name = source.file_name().unwrap().to_string_lossy().into_owned();
            self.events.lock().unwrap().push(format!("inspect:{name}"));
            if self.rejected.lock().unwrap().contains(&name) {
                return Err(CanopyError::InvalidArgument(
                    "invalid media: fixture".into(),
                ));
            }
            Ok(InspectedMp3 {
                title: name.clone(),
                artist: "Test Artist".into(),
                album: "Test Album".into(),
                duration_ms: 1_000,
                size_bytes: 1_024,
                checksum_sha256: Sha256::digest(name.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                artwork: None,
            })
        }
    }

    #[async_trait]
    impl MediaStorage for World {
        async fn stage(
            &self,
            track_id: &str,
            _source: &Path,
            checksum: &str,
            _artwork: Option<&InspectedArtwork>,
        ) -> CanopyResult<StagedMedia> {
            self.events.lock().unwrap().push("stage".into());
            Ok(StagedMedia::new(
                track_id.to_string(),
                format!(
                    "audio/{}/{}/{}.mp3",
                    &checksum[..2],
                    &checksum[2..4],
                    checksum
                ),
                None,
                checksum.to_string(),
                None,
            ))
        }

        async fn finalize(&self, _staged: &StagedMedia) -> CanopyResult<()> {
            self.events.lock().unwrap().push("finalize".into());
            if self.finalize_fails.load(Ordering::Relaxed) {
                Err(CanopyError::Storage("filesystem unavailable".into()))
            } else {
                Ok(())
            }
        }

        async fn discard_before_persistence(&self, _track_id: &str) -> CanopyResult<()> {
            self.events.lock().unwrap().push("discard".into());
            Ok(())
        }
    }

    fn harness() -> (MediaImportService, Arc<World>) {
        let world = Arc::new(World::new());
        let service = MediaImportService::new(
            Arc::new(FixedOwner),
            world.clone(),
            world.clone(),
            world.clone(),
        );
        (service, world)
    }

    fn source_file(temp: &TempDir, name: &str) -> PathBuf {
        let path = temp.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        path
    }

    #[tokio::test]
    async fn existing_checksum_returns_duplicate_without_staging() {
        let temp = TempDir::new().unwrap();
        let source = source_file(&temp, "track.mp3");
        let (service, world) = harness();
        *world.existing.lock().unwrap() = Some("existing-track".into());
        let summary = service.import_path(&source).await.unwrap();
        assert_eq!(summary.duplicate, 1);
        assert_eq!(summary.items[0].status, super::ImportItemStatus::Duplicate);
        assert_eq!(summary.items[0].track_id.as_deref(), Some("existing-track"));
        assert_eq!(*world.events.lock().unwrap(), ["inspect:track.mp3", "find"]);
    }

    #[tokio::test]
    async fn new_import_follows_pending_finalize_ready_order() {
        let temp = TempDir::new().unwrap();
        let source = source_file(&temp, "track.mp3");
        let (service, world) = harness();
        let summary = service.import_path(&source).await.unwrap();
        assert_eq!(summary.imported, 1);
        assert_eq!(summary.items[0].status, super::ImportItemStatus::Imported);
        assert!(summary.items[0].track_id.is_some());
        assert_eq!(
            *world.events.lock().unwrap(),
            [
                "inspect:track.mp3",
                "find",
                "stage",
                "insert",
                "finalize",
                "ready"
            ]
        );
    }

    #[tokio::test]
    async fn duplicate_race_discards_staging_and_returns_winner() {
        let temp = TempDir::new().unwrap();
        let source = source_file(&temp, "track.mp3");
        let (service, world) = harness();
        *world.insert_behavior.lock().unwrap() = InsertBehavior::Duplicate;
        let summary = service.import_path(&source).await.unwrap();
        assert_eq!(summary.items[0].track_id.as_deref(), Some("winning-track"));
        assert_eq!(
            *world.events.lock().unwrap(),
            ["inspect:track.mp3", "find", "stage", "insert", "discard"]
        );
    }

    #[tokio::test]
    async fn insert_failure_discards_staging() {
        let temp = TempDir::new().unwrap();
        let source = source_file(&temp, "track.mp3");
        let (service, world) = harness();
        *world.insert_behavior.lock().unwrap() = InsertBehavior::Fail;
        let summary = service.import_path(&source).await.unwrap();
        assert_eq!(
            summary.items[0].error_code.as_deref(),
            Some("database_unavailable")
        );
        assert_eq!(
            *world.events.lock().unwrap(),
            ["inspect:track.mp3", "find", "stage", "insert", "discard"]
        );
    }

    #[tokio::test]
    async fn finalize_failure_leaves_pending_staging_recoverable() {
        let temp = TempDir::new().unwrap();
        let source = source_file(&temp, "track.mp3");
        let (service, world) = harness();
        world.finalize_fails.store(true, Ordering::Relaxed);
        let summary = service.import_path(&source).await.unwrap();
        assert_eq!(
            summary.items[0].error_code.as_deref(),
            Some("storage_unavailable")
        );
        assert_eq!(
            *world.events.lock().unwrap(),
            ["inspect:track.mp3", "find", "stage", "insert", "finalize"]
        );
    }

    #[tokio::test]
    async fn ready_failure_leaves_final_files_and_pending_row() {
        let temp = TempDir::new().unwrap();
        let source = source_file(&temp, "track.mp3");
        let (service, world) = harness();
        world.ready_fails.store(true, Ordering::Relaxed);
        let summary = service.import_path(&source).await.unwrap();
        assert_eq!(
            summary.items[0].error_code.as_deref(),
            Some("database_unavailable")
        );
        assert_eq!(
            *world.events.lock().unwrap(),
            [
                "inspect:track.mp3",
                "find",
                "stage",
                "insert",
                "finalize",
                "ready"
            ]
        );
    }

    #[tokio::test]
    async fn directory_selection_is_shallow_case_insensitive_and_sorted() {
        let temp = TempDir::new().unwrap();
        source_file(&temp, "b.mp3");
        source_file(&temp, "a.MP3");
        source_file(&temp, "ignore.txt");
        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("hidden.mp3"), b"fixture").unwrap();
        let (service, _) = harness();
        let summary = service.import_path(temp.path()).await.unwrap();
        assert_eq!(summary.imported, 2);
        assert_eq!(
            summary
                .items
                .iter()
                .map(|item| item.file_name.as_str())
                .collect::<Vec<_>>(),
            ["a.MP3", "b.mp3"]
        );
    }

    #[tokio::test]
    async fn mixed_directory_results_preserve_successes_and_failures() {
        let temp = TempDir::new().unwrap();
        source_file(&temp, "a.mp3");
        source_file(&temp, "b.mp3");
        source_file(&temp, "c.mp3");
        let (service, world) = harness();
        world.rejected.lock().unwrap().insert("b.mp3".into());
        let summary = service.import_path(temp.path()).await.unwrap();
        assert_eq!(
            (summary.imported, summary.duplicate, summary.failed),
            (2, 0, 1)
        );
        assert_eq!(summary.items[1].status, super::ImportItemStatus::Failed);
        assert_eq!(
            summary.items[1].error_code.as_deref(),
            Some("invalid_media")
        );
    }
}
