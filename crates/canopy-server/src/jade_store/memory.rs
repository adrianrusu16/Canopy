//! In-memory implementations of the repository ports.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use canopy_core::{
    AudioAsset, AudioAssetRepository, AuthorizedStreamAsset, CanopyError, CanopyResult,
    CatalogRepository, DiscoveryRepository, IngestStatus, InstanceSettingsRepository, LibraryItem,
    LibraryRepository, LikeRepository, LikedTrackItem, LikedTrackPage, MediaItem, MediaPage,
    MediaVisibility, Page, PlayableAsset, PlayableAssetRepository, PlaybackHistoryEntry,
    PlaybackHistoryEvent, PlaybackHistoryPage, PlaybackHistoryRepository, Playlist, PlaylistPage,
    PlaylistRepository, PlaylistTrack, PlaylistTrackItem, PlaylistTrackPage, PreferencesRepository,
    ProfilePreferences, ProfileRepository, SavedTrackItem, SavedTrackPage, StreamAudience,
    TrackAccessScope, TrackLike, UserProfile,
};

/// Catalog item together with its mandatory access policy.
#[derive(Clone)]
pub struct InMemoryCatalogEntry {
    pub item: MediaItem,
    pub visibility: MediaVisibility,
    pub ingest_status: IngestStatus,
    pub owner_profile_id: Option<String>,
}

/// In-memory catalog backing store.
#[derive(Clone, Default)]
pub struct InMemoryCatalog {
    entries: Vec<InMemoryCatalogEntry>,
}

impl InMemoryCatalog {
    /// Creates a public-ready catalog for demo compatibility.
    pub fn with_items(items: Vec<MediaItem>) -> Self {
        Self::from_entries(
            items
                .into_iter()
                .map(|item| InMemoryCatalogEntry {
                    item,
                    visibility: MediaVisibility::ReleaseSafe,
                    ingest_status: IngestStatus::Ready,
                    owner_profile_id: None,
                })
                .collect(),
        )
    }

    /// Creates a catalog with explicit access policy per item.
    pub fn from_entries(entries: Vec<InMemoryCatalogEntry>) -> Self {
        Self { entries }
    }

    fn accessible_items(&self, scope: &TrackAccessScope) -> Vec<MediaItem> {
        let mut items: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.ingest_status == IngestStatus::Ready
                    && (entry.visibility == MediaVisibility::ReleaseSafe
                        || matches!(
                            scope,
                            TrackAccessScope::Owner { profile_id }
                                if entry.visibility == MediaVisibility::Personal
                                    && entry.owner_profile_id.as_deref()
                                        == Some(profile_id.as_str())
                        ))
            })
            .map(|entry| entry.item.clone())
            .collect();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        items
    }

    fn personal_items(&self, owner_profile_id: &str) -> Vec<MediaItem> {
        self.entries
            .iter()
            .filter(|entry| {
                entry.visibility == MediaVisibility::Personal
                    && entry.ingest_status == IngestStatus::Ready
                    && entry.owner_profile_id.as_deref() == Some(owner_profile_id)
            })
            .map(|entry| entry.item.clone())
            .collect()
    }
}

fn page_items(items: Vec<MediaItem>, page: Page) -> MediaPage {
    let total = items.len();
    let start = (page.offset as usize).min(total);
    let end = (start + page.limit as usize).min(total);
    MediaPage {
        items: items[start..end].to_vec(),
        total_count: total as i32,
        has_more: end < total,
    }
}

fn search_items(items: Vec<MediaItem>, query: &str, page: Page) -> MediaPage {
    if query.is_empty() {
        return MediaPage::default();
    }
    let query = query.to_lowercase();
    let matches = items
        .into_iter()
        .filter(|item| {
            item.title.to_lowercase().contains(&query)
                || item.artist.to_lowercase().contains(&query)
        })
        .collect();
    page_items(matches, page)
}

#[async_trait]
impl CatalogRepository for InMemoryCatalog {
    async fn browse(
        &self,
        scope: &TrackAccessScope,
        _parent_id: Option<&str>,
        _genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage> {
        Ok(page_items(self.accessible_items(scope), page))
    }

    async fn search(
        &self,
        scope: &TrackAccessScope,
        query: &str,
        page: Page,
    ) -> CanopyResult<MediaPage> {
        Ok(search_items(self.accessible_items(scope), query, page))
    }

    async fn get_media(
        &self,
        scope: &TrackAccessScope,
        media_id: &str,
    ) -> CanopyResult<Option<MediaItem>> {
        Ok(self
            .accessible_items(scope)
            .into_iter()
            .find(|item| item.id == media_id))
    }

    async fn list_personal(&self, owner_profile_id: &str, page: Page) -> CanopyResult<MediaPage> {
        Ok(page_items(self.personal_items(owner_profile_id), page))
    }
}

#[async_trait]
impl DiscoveryRepository for InMemoryCatalog {
    async fn shuffle_pool(&self, scope: &TrackAccessScope) -> CanopyResult<Vec<MediaItem>> {
        Ok(self.accessible_items(scope))
    }
}

/// Audio asset together with its track access policy.
#[derive(Clone)]
pub struct InMemoryAudioAssetEntry {
    pub asset: AudioAsset,
    pub visibility: MediaVisibility,
    pub ingest_status: IngestStatus,
    pub owner_profile_id: Option<String>,
}

/// In-memory audio-asset backing store.
#[derive(Clone, Default)]
pub struct InMemoryAudioAssetStore {
    entries: Vec<InMemoryAudioAssetEntry>,
    asset_ids: Vec<String>,
    instance_settings: Option<Arc<dyn InstanceSettingsRepository>>,
}

impl InMemoryAudioAssetStore {
    /// Creates a public-ready store for demo compatibility.
    pub fn with_assets(assets: Vec<AudioAsset>) -> Self {
        Self::from_entries(
            assets
                .into_iter()
                .map(|asset| InMemoryAudioAssetEntry {
                    asset,
                    visibility: MediaVisibility::ReleaseSafe,
                    ingest_status: IngestStatus::Ready,
                    owner_profile_id: None,
                })
                .collect(),
        )
    }

    /// Creates a store with explicit access policy per asset.
    pub fn from_entries(entries: Vec<InMemoryAudioAssetEntry>) -> Self {
        let asset_ids = entries
            .iter()
            .map(|_| uuid::Uuid::new_v4().to_string())
            .collect();
        Self {
            entries,
            asset_ids,
            instance_settings: None,
        }
    }

    /// Shares current instance-owner settings for personal stream authorization.
    pub fn with_instance_settings(mut self, settings: Arc<dyn InstanceSettingsRepository>) -> Self {
        self.instance_settings = Some(settings);
        self
    }
}

#[async_trait]
impl AudioAssetRepository for InMemoryAudioAssetStore {
    async fn assets_for_public_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                entry.asset.track_id == track_id
                    && entry.visibility == MediaVisibility::ReleaseSafe
                    && entry.ingest_status == IngestStatus::Ready
            })
            .map(|entry| entry.asset.clone())
            .collect())
    }

    async fn assets_for_personal_track(
        &self,
        owner_profile_id: &str,
        track_id: &str,
    ) -> CanopyResult<Vec<AudioAsset>> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                entry.asset.track_id == track_id
                    && entry.visibility == MediaVisibility::Personal
                    && entry.ingest_status == IngestStatus::Ready
                    && entry.owner_profile_id.as_deref() == Some(owner_profile_id)
            })
            .map(|entry| entry.asset.clone())
            .collect())
    }
}

#[async_trait]
impl PlayableAssetRepository for InMemoryAudioAssetStore {
    async fn assets_for_personal_playback(
        &self,
        owner_profile_id: &str,
        track_id: &str,
    ) -> CanopyResult<Vec<PlayableAsset>> {
        Ok(self
            .entries
            .iter()
            .zip(&self.asset_ids)
            .filter(|(entry, _)| {
                entry.asset.track_id == track_id
                    && entry.visibility == MediaVisibility::Personal
                    && entry.ingest_status == IngestStatus::Ready
                    && entry.owner_profile_id.as_deref() == Some(owner_profile_id)
            })
            .map(|(entry, asset_id)| PlayableAsset {
                asset_id: asset_id.clone(),
                track_id: entry.asset.track_id.clone(),
                codec: entry.asset.codec.clone(),
                content_type: entry.asset.content_type.clone(),
                duration_ms: entry.asset.duration_ms,
            })
            .collect())
    }
    async fn assets_for_public_playback(&self, track_id: &str) -> CanopyResult<Vec<PlayableAsset>> {
        Ok(self
            .entries
            .iter()
            .zip(&self.asset_ids)
            .filter(|(entry, _)| {
                entry.asset.track_id == track_id
                    && entry.visibility == MediaVisibility::ReleaseSafe
                    && entry.ingest_status == IngestStatus::Ready
            })
            .map(|(entry, asset_id)| PlayableAsset {
                asset_id: asset_id.clone(),
                track_id: entry.asset.track_id.clone(),
                codec: entry.asset.codec.clone(),
                content_type: entry.asset.content_type.clone(),
                duration_ms: entry.asset.duration_ms,
            })
            .collect())
    }

    async fn authorize_stream_asset(
        &self,
        asset_id: &str,
        audience: StreamAudience,
    ) -> CanopyResult<Option<AuthorizedStreamAsset>> {
        let configured_owner = match audience {
            StreamAudience::Public => None,
            StreamAudience::Personal => {
                let Some(settings) = &self.instance_settings else {
                    return Ok(None);
                };
                settings.owner_profile_id().await?
            }
        };

        Ok(self
            .entries
            .iter()
            .zip(&self.asset_ids)
            .find(|(entry, id)| {
                id.as_str() == asset_id
                    && entry.ingest_status == IngestStatus::Ready
                    && match audience {
                        StreamAudience::Public => entry.visibility == MediaVisibility::ReleaseSafe,
                        StreamAudience::Personal => {
                            entry.visibility == MediaVisibility::Personal
                                && entry.owner_profile_id.as_deref() == configured_owner.as_deref()
                        }
                    }
            })
            .map(|(entry, id)| AuthorizedStreamAsset {
                asset_id: id.clone(),
                storage_key: entry.asset.storage_key.clone(),
                content_type: entry.asset.content_type.clone(),
            }))
    }
}

/// In-memory logged-in profile store.
#[derive(Default)]
pub struct InMemoryProfileStore {
    profiles: Mutex<HashMap<String, UserProfile>>,
}

#[async_trait]
impl ProfileRepository for InMemoryProfileStore {
    async fn upsert_profile(
        &self,
        external_user_id: &str,
        display_name: Option<&str>,
        history_enabled: bool,
    ) -> CanopyResult<UserProfile> {
        let mut profiles = self.profiles.lock().unwrap();
        let profile = profiles
            .entry(external_user_id.to_string())
            .or_insert_with(|| UserProfile {
                id: uuid::Uuid::new_v4().to_string(),
                external_user_id: external_user_id.to_string(),
                ..UserProfile::default()
            });
        profile.display_name = display_name.map(ToString::to_string);
        profile.history_enabled = history_enabled;
        Ok(profile.clone())
    }

    async fn get_by_external_user_id(
        &self,
        external_user_id: &str,
    ) -> CanopyResult<Option<UserProfile>> {
        Ok(self.profiles.lock().unwrap().get(external_user_id).cloned())
    }

    async fn delete_by_external_user_id(&self, external_user_id: &str) -> CanopyResult<()> {
        self.profiles.lock().unwrap().remove(external_user_id);
        Ok(())
    }
}

/// In-memory singleton settings for tests and standalone mode.
#[derive(Default)]
pub struct InMemoryInstanceSettingsStore {
    owner_profile_id: Mutex<Option<String>>,
}

#[async_trait]
impl InstanceSettingsRepository for InMemoryInstanceSettingsStore {
    async fn set_owner_profile_id(&self, profile_id: &str) -> CanopyResult<()> {
        *self.owner_profile_id.lock().unwrap() = Some(profile_id.to_string());
        Ok(())
    }

    async fn owner_profile_id(&self) -> CanopyResult<Option<String>> {
        Ok(self.owner_profile_id.lock().unwrap().clone())
    }
}

#[derive(Clone)]
struct InMemoryHistoryRow {
    id: String,
    event: PlaybackHistoryEvent,
    played_at_epoch_ms: u64,
}

/// In-memory playback-history store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryPlaybackHistoryStore {
    events: Mutex<Vec<InMemoryHistoryRow>>,
}

impl InMemoryPlaybackHistoryStore {
    /// Returns a snapshot of stored events for tests.
    pub fn events(&self) -> CanopyResult<Vec<PlaybackHistoryEvent>> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|row| row.event.clone())
            .collect())
    }
}

#[async_trait]
impl PlaybackHistoryRepository for InMemoryPlaybackHistoryStore {
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<bool> {
        self.events.lock().unwrap().push(InMemoryHistoryRow {
            id: uuid::Uuid::new_v4().to_string(),
            event,
            played_at_epoch_ms: current_epoch_ms(),
        });
        Ok(true)
    }

    async fn list(&self, profile_id: &str, page: Page) -> CanopyResult<PlaybackHistoryPage> {
        let mut rows: Vec<InMemoryHistoryRow> = self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.event.profile_id == profile_id)
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            right
                .played_at_epoch_ms
                .cmp(&left.played_at_epoch_ms)
                .then_with(|| right.id.cmp(&left.id))
        });

        let total_count = rows.len() as i32;
        let start = (page.offset as usize).min(rows.len());
        let end = start.saturating_add(page.limit as usize).min(rows.len());
        let entries = rows[start..end]
            .iter()
            .map(|row| PlaybackHistoryEntry {
                id: row.id.clone(),
                played_at_epoch_ms: row.played_at_epoch_ms,
                duration_ms: row.event.duration_ms,
                completion_pct: row.event.completion_pct,
                item: MediaItem {
                    id: row.event.track_id.clone(),
                    ..MediaItem::default()
                },
            })
            .collect();

        Ok(PlaybackHistoryPage {
            entries,
            total_count,
            has_more: end < rows.len(),
        })
    }

    async fn delete_entry(&self, profile_id: &str, history_id: &str) -> CanopyResult<bool> {
        let mut rows = self.events.lock().unwrap();
        let before = rows.len();
        rows.retain(|row| row.event.profile_id != profile_id || row.id != history_id);
        Ok(rows.len() != before)
    }

    async fn clear(&self, profile_id: &str) -> CanopyResult<u64> {
        let mut rows = self.events.lock().unwrap();
        let before = rows.len();
        rows.retain(|row| row.event.profile_id != profile_id);
        Ok((before - rows.len()) as u64)
    }
}

fn current_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn profile_track_key(profile_id: &str, track_id: &str) -> String {
    format!("{profile_id}:{track_id}")
}

/// In-memory saved-library store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryLibraryStore {
    items: Mutex<HashMap<String, LibraryItem>>,
}

#[async_trait]
impl LibraryRepository for InMemoryLibraryStore {
    async fn save_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<LibraryItem> {
        let mut items = self.items.lock().unwrap();
        let item = items
            .entry(profile_track_key(profile_id, track_id))
            .or_insert_with(|| LibraryItem {
                profile_id: profile_id.to_string(),
                track_id: track_id.to_string(),
                added_at_epoch_ms: current_epoch_ms(),
            });
        Ok(item.clone())
    }

    async fn remove_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()> {
        self.items
            .lock()
            .unwrap()
            .remove(&profile_track_key(profile_id, track_id));
        Ok(())
    }

    async fn list_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<SavedTrackPage> {
        let mut items: Vec<LibraryItem> = self
            .items
            .lock()
            .unwrap()
            .values()
            .filter(|item| item.profile_id == profile_id)
            .cloned()
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.added_at_epoch_ms));
        let total_count = items.len() as i32;
        let start = (page.offset as usize).min(items.len());
        let end = (start + page.limit as usize).min(items.len());
        let saved_items = items[start..end]
            .iter()
            .map(|item| SavedTrackItem {
                item: MediaItem {
                    id: item.track_id.clone(),
                    ..MediaItem::default()
                },
                saved_at_epoch_ms: item.added_at_epoch_ms,
            })
            .collect();
        Ok(SavedTrackPage {
            items: saved_items,
            total_count,
            has_more: end < items.len(),
        })
    }

    async fn is_saved(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .contains_key(&profile_track_key(profile_id, track_id)))
    }
}

/// In-memory track-like store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryLikeStore {
    likes: Mutex<HashMap<String, TrackLike>>,
}

#[async_trait]
impl LikeRepository for InMemoryLikeStore {
    async fn like_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<TrackLike> {
        let mut likes = self.likes.lock().unwrap();
        let like = likes
            .entry(profile_track_key(profile_id, track_id))
            .or_insert_with(|| TrackLike {
                profile_id: profile_id.to_string(),
                track_id: track_id.to_string(),
                liked_at_epoch_ms: current_epoch_ms(),
            });
        Ok(like.clone())
    }

    async fn unlike_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()> {
        self.likes
            .lock()
            .unwrap()
            .remove(&profile_track_key(profile_id, track_id));
        Ok(())
    }

    async fn list_liked_tracks(
        &self,
        profile_id: &str,
        page: Page,
    ) -> CanopyResult<LikedTrackPage> {
        let mut likes: Vec<TrackLike> = self
            .likes
            .lock()
            .unwrap()
            .values()
            .filter(|like| like.profile_id == profile_id)
            .cloned()
            .collect();
        likes.sort_by_key(|like| std::cmp::Reverse(like.liked_at_epoch_ms));
        let total_count = likes.len() as i32;
        let start = (page.offset as usize).min(likes.len());
        let end = (start + page.limit as usize).min(likes.len());
        let liked_items = likes[start..end]
            .iter()
            .map(|like| LikedTrackItem {
                item: MediaItem {
                    id: like.track_id.clone(),
                    ..MediaItem::default()
                },
                liked_at_epoch_ms: like.liked_at_epoch_ms,
            })
            .collect();
        Ok(LikedTrackPage {
            items: liked_items,
            total_count,
            has_more: end < likes.len(),
        })
    }

    async fn is_liked(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool> {
        Ok(self
            .likes
            .lock()
            .unwrap()
            .contains_key(&profile_track_key(profile_id, track_id)))
    }
}

/// In-memory profile-preferences store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryPreferencesStore {
    preferences: Mutex<HashMap<String, ProfilePreferences>>,
}

#[async_trait]
impl PreferencesRepository for InMemoryPreferencesStore {
    async fn get_preferences(&self, profile_id: &str) -> CanopyResult<ProfilePreferences> {
        Ok(self
            .preferences
            .lock()
            .unwrap()
            .get(profile_id)
            .cloned()
            .unwrap_or_else(|| ProfilePreferences {
                profile_id: profile_id.to_string(),
                values_json: "{}".to_string(),
            }))
    }

    async fn upsert_preferences(
        &self,
        profile_id: &str,
        values_json: &str,
    ) -> CanopyResult<ProfilePreferences> {
        let preferences = ProfilePreferences {
            profile_id: profile_id.to_string(),
            values_json: values_json.to_string(),
        };
        self.preferences
            .lock()
            .unwrap()
            .insert(profile_id.to_string(), preferences.clone());
        Ok(preferences)
    }
}

fn playlist_not_found(playlist_id: &str) -> CanopyError {
    CanopyError::not_found("playlist", playlist_id)
}

fn playlist_tracks_match(existing: &[PlaylistTrack], requested: &[String]) -> bool {
    let mut existing_sorted: Vec<String> = existing
        .iter()
        .map(|track| track.track_id.clone())
        .collect();
    existing_sorted.sort();
    let mut requested_sorted = requested.to_vec();
    requested_sorted.sort();
    existing_sorted == requested_sorted
}

/// In-memory playlist store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryPlaylistStore {
    playlists: Mutex<HashMap<String, Playlist>>,
    tracks: Mutex<HashMap<String, Vec<PlaylistTrack>>>,
}

impl InMemoryPlaylistStore {
    fn get_owned_playlist(&self, profile_id: &str, playlist_id: &str) -> CanopyResult<Playlist> {
        let playlist = self
            .playlists
            .lock()
            .unwrap()
            .get(playlist_id)
            .cloned()
            .ok_or_else(|| playlist_not_found(playlist_id))?;
        if playlist.profile_id != profile_id {
            return Err(playlist_not_found(playlist_id));
        }
        Ok(playlist)
    }
}

#[async_trait]
impl PlaylistRepository for InMemoryPlaylistStore {
    async fn create_playlist(
        &self,
        profile_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist> {
        let now = current_epoch_ms();
        let playlist = Playlist {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            created_at_epoch_ms: now,
            updated_at_epoch_ms: now,
        };
        self.playlists
            .lock()
            .unwrap()
            .insert(playlist.id.clone(), playlist.clone());
        self.tracks
            .lock()
            .unwrap()
            .insert(playlist.id.clone(), Vec::new());
        Ok(playlist)
    }

    async fn get_playlist(
        &self,
        profile_id: &str,
        playlist_id: &str,
    ) -> CanopyResult<Option<Playlist>> {
        match self.get_owned_playlist(profile_id, playlist_id) {
            Ok(playlist) => Ok(Some(playlist)),
            Err(CanopyError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn update_playlist(
        &self,
        profile_id: &str,
        playlist_id: &str,
        name: &str,
        description: &str,
    ) -> CanopyResult<Playlist> {
        let mut playlists = self.playlists.lock().unwrap();
        let playlist = playlists
            .get_mut(playlist_id)
            .ok_or_else(|| playlist_not_found(playlist_id))?;
        if playlist.profile_id != profile_id {
            return Err(playlist_not_found(playlist_id));
        }
        playlist.name = name.to_string();
        playlist.description = description.to_string();
        playlist.updated_at_epoch_ms = current_epoch_ms();
        Ok(playlist.clone())
    }

    async fn delete_playlist(&self, profile_id: &str, playlist_id: &str) -> CanopyResult<()> {
        self.get_owned_playlist(profile_id, playlist_id)?;
        self.playlists.lock().unwrap().remove(playlist_id);
        self.tracks.lock().unwrap().remove(playlist_id);
        Ok(())
    }

    async fn list_playlists(&self, profile_id: &str, page: Page) -> CanopyResult<PlaylistPage> {
        let mut playlists: Vec<Playlist> = self
            .playlists
            .lock()
            .unwrap()
            .values()
            .filter(|playlist| playlist.profile_id == profile_id)
            .cloned()
            .collect();
        playlists.sort_by_key(|playlist| std::cmp::Reverse(playlist.updated_at_epoch_ms));
        let total_count = playlists.len() as i32;
        let start = (page.offset as usize).min(playlists.len());
        let end = (start + page.limit as usize).min(playlists.len());
        Ok(PlaylistPage {
            items: playlists[start..end].to_vec(),
            total_count,
            has_more: end < playlists.len(),
        })
    }

    async fn add_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
        position: Option<i32>,
    ) -> CanopyResult<PlaylistTrackItem> {
        self.get_owned_playlist(profile_id, playlist_id)?;
        if let Some(position) = position
            && position < 0
        {
            return Err(CanopyError::InvalidArgument(
                "position must be non-negative".into(),
            ));
        }
        let mut tracks = self.tracks.lock().unwrap();
        let entry = tracks.entry(playlist_id.to_string()).or_default();
        let added_at_epoch_ms = entry
            .iter()
            .find(|track| track.track_id == track_id)
            .map(|track| track.added_at_epoch_ms)
            .unwrap_or_else(current_epoch_ms);
        if let Some(existing) = entry.iter().position(|track| track.track_id == track_id) {
            entry.remove(existing);
        }
        let index = position
            .map(|value| value as usize)
            .unwrap_or(entry.len())
            .min(entry.len());
        entry.insert(
            index,
            PlaylistTrack {
                playlist_id: playlist_id.to_string(),
                track_id: track_id.to_string(),
                position: 0,
                added_at_epoch_ms,
            },
        );
        for (position, track) in entry.iter_mut().enumerate() {
            track.position = i32::try_from(position)
                .map_err(|_| CanopyError::InvalidArgument("playlist is too large".into()))?;
        }
        let track = entry[index].clone();
        drop(tracks);
        if let Some(playlist) = self.playlists.lock().unwrap().get_mut(playlist_id) {
            playlist.updated_at_epoch_ms = current_epoch_ms();
        }
        Ok(PlaylistTrackItem {
            playlist_id: track.playlist_id,
            item: MediaItem {
                id: track.track_id,
                ..MediaItem::default()
            },
            position: track.position,
            added_at_epoch_ms: track.added_at_epoch_ms,
        })
    }

    async fn remove_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
    ) -> CanopyResult<()> {
        self.get_owned_playlist(profile_id, playlist_id)?;
        if let Some(tracks) = self.tracks.lock().unwrap().get_mut(playlist_id) {
            tracks.retain(|track| track.track_id != track_id);
        }
        if let Some(playlist) = self.playlists.lock().unwrap().get_mut(playlist_id) {
            playlist.updated_at_epoch_ms = current_epoch_ms();
        }
        Ok(())
    }

    async fn reorder_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_ids: &[String],
    ) -> CanopyResult<Playlist> {
        self.get_owned_playlist(profile_id, playlist_id)?;
        let mut seen = std::collections::HashSet::new();
        if !track_ids
            .iter()
            .all(|track_id| seen.insert(track_id.clone()))
        {
            return Err(CanopyError::InvalidArgument(
                "reorder track_ids must be unique".into(),
            ));
        }
        let mut tracks = self.tracks.lock().unwrap();
        let entry = tracks.entry(playlist_id.to_string()).or_default();
        if !playlist_tracks_match(entry, track_ids) {
            return Err(CanopyError::InvalidArgument(
                "reorder must include exactly the playlist track_ids".into(),
            ));
        }
        let mut reordered = Vec::with_capacity(entry.len());
        for (position, track_id) in track_ids.iter().enumerate() {
            let mut track = entry
                .iter()
                .find(|track| track.track_id == *track_id)
                .cloned()
                .ok_or_else(|| {
                    CanopyError::InvalidArgument("unknown track_id in reorder".into())
                })?;
            track.position = i32::try_from(position)
                .map_err(|_| CanopyError::InvalidArgument("playlist is too large".into()))?;
            reordered.push(track);
        }
        *entry = reordered;
        drop(tracks);

        let mut playlists = self.playlists.lock().unwrap();
        let playlist = playlists
            .get_mut(playlist_id)
            .ok_or_else(|| playlist_not_found(playlist_id))?;
        playlist.updated_at_epoch_ms = current_epoch_ms();
        Ok(playlist.clone())
    }

    async fn list_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        page: Page,
    ) -> CanopyResult<PlaylistTrackPage> {
        self.get_owned_playlist(profile_id, playlist_id)?;
        let tracks = self
            .tracks
            .lock()
            .unwrap()
            .get(playlist_id)
            .cloned()
            .unwrap_or_default();
        let total_count = tracks.len() as i32;
        let start = (page.offset as usize).min(tracks.len());
        let end = (start + page.limit as usize).min(tracks.len());
        let items = tracks[start..end]
            .iter()
            .map(|track| PlaylistTrackItem {
                playlist_id: track.playlist_id.clone(),
                item: MediaItem {
                    id: track.track_id.clone(),
                    ..MediaItem::default()
                },
                position: track.position,
                added_at_epoch_ms: track.added_at_epoch_ms,
            })
            .collect();
        Ok(PlaylistTrackPage {
            items,
            total_count,
            has_more: end < tracks.len(),
        })
    }
}
