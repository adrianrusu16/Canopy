//! In-memory implementations of the repository ports.
//!
//! These are placeholders for the future PostgreSQL-backed (catalog) and
//! persistent (session) stores. They keep the prototype fully functional
//! without external dependencies.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyError, CanopyResult, CatalogRepository,
    DiscoveryRepository, LibraryItem, LibraryRepository, LikeRepository, MediaItem, MediaPage,
    Page, PlaybackHistoryEntry, PlaybackHistoryEvent, PlaybackHistoryPage,
    PlaybackHistoryRepository, Playlist, PlaylistPage, PlaylistRepository, PreferencesRepository,
    ProfilePreferences, ProfileRepository, Session, SessionRepository, TrackLike, UserProfile,
};

/// In-memory catalog backing store.
#[derive(Clone, Default)]
pub struct InMemoryCatalog {
    items: Vec<MediaItem>,
}

impl InMemoryCatalog {
    /// Creates a catalog seeded with the given items.
    pub fn with_items(items: Vec<MediaItem>) -> Self {
        Self { items }
    }
}

#[async_trait]
impl CatalogRepository for InMemoryCatalog {
    async fn browse(
        &self,
        _parent_id: Option<&str>,
        _genres: &[String],
        page: Page,
    ) -> CanopyResult<MediaPage> {
        let start = (page.offset as usize).min(self.items.len());
        let end = (start + page.limit as usize).min(self.items.len());
        Ok(MediaPage {
            items: self.items[start..end].to_vec(),
            total_count: self.items.len() as i32,
            has_more: end < self.items.len(),
        })
    }

    async fn search(&self, query: &str, _page: Page) -> CanopyResult<MediaPage> {
        // Trivial in-memory match; the production path is PostgreSQL `pg_trgm`.
        let items: Vec<MediaItem> = if query.is_empty() {
            Vec::new()
        } else {
            self.items
                .iter()
                .filter(|i| {
                    i.title.to_lowercase().contains(&query.to_lowercase())
                        || i.artist.to_lowercase().contains(&query.to_lowercase())
                })
                .cloned()
                .collect()
        };
        let total_count = items.len() as i32;
        Ok(MediaPage {
            items,
            total_count,
            has_more: false,
        })
    }

    async fn get_media(&self, media_id: &str) -> CanopyResult<Option<MediaItem>> {
        Ok(self.items.iter().find(|i| i.id == media_id).cloned())
    }
}

#[async_trait]
impl DiscoveryRepository for InMemoryCatalog {
    async fn shuffle_pool(&self) -> CanopyResult<Vec<MediaItem>> {
        // The production view is pre-shuffled offline; the prototype simply
        // exposes the catalog in insertion order and leaves diversification to
        // the discovery service.
        Ok(self.items.clone())
    }
}

/// In-memory audio-asset backing store.
///
/// Stands in for the `audio_assets` table; the production backend will read
/// from PostgreSQL. Assets are grouped by track identifier.
#[derive(Clone, Default)]
pub struct InMemoryAudioAssetStore {
    assets: Vec<AudioAsset>,
}

impl InMemoryAudioAssetStore {
    /// Creates a store seeded with the given assets.
    pub fn with_assets(assets: Vec<AudioAsset>) -> Self {
        Self { assets }
    }
}

#[async_trait]
impl AudioAssetRepository for InMemoryAudioAssetStore {
    async fn assets_for_track(&self, track_id: &str) -> CanopyResult<Vec<AudioAsset>> {
        Ok(self
            .assets
            .iter()
            .filter(|a| a.track_id == track_id)
            .cloned()
            .collect())
    }
}

/// In-memory session store.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: Mutex<HashMap<String, Session>>,
}

#[async_trait]
impl SessionRepository for InMemorySessionStore {
    async fn create(&self) -> CanopyResult<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let session = Session {
            id: id.clone(),
            ..Session::default()
        };
        self.sessions.lock().unwrap().insert(id.clone(), session);
        Ok(id)
    }

    async fn get(&self, id: &str) -> CanopyResult<Option<Session>> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn update(&self, session: Session) -> CanopyResult<()> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session);
        Ok(())
    }

    async fn delete(&self, id: &str) -> CanopyResult<()> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
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

    async fn list_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<MediaPage> {
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
        let media_items = items[start..end]
            .iter()
            .map(|item| MediaItem {
                id: item.track_id.clone(),
                ..MediaItem::default()
            })
            .collect();
        Ok(MediaPage {
            items: media_items,
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

    async fn list_liked_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<MediaPage> {
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
        let media_items = likes[start..end]
            .iter()
            .map(|like| MediaItem {
                id: like.track_id.clone(),
                ..MediaItem::default()
            })
            .collect();
        Ok(MediaPage {
            items: media_items,
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

fn playlist_tracks_match(existing: &[String], requested: &[String]) -> bool {
    let mut existing_sorted = existing.to_vec();
    existing_sorted.sort();
    let mut requested_sorted = requested.to_vec();
    requested_sorted.sort();
    existing_sorted == requested_sorted
}

/// In-memory playlist store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryPlaylistStore {
    playlists: Mutex<HashMap<String, Playlist>>,
    tracks: Mutex<HashMap<String, Vec<String>>>,
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
    ) -> CanopyResult<()> {
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
        if let Some(existing) = entry.iter().position(|id| id == track_id) {
            entry.remove(existing);
        }
        let index = position
            .map(|value| value as usize)
            .unwrap_or(entry.len())
            .min(entry.len());
        entry.insert(index, track_id.to_string());
        Ok(())
    }

    async fn remove_track(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_id: &str,
    ) -> CanopyResult<()> {
        self.get_owned_playlist(profile_id, playlist_id)?;
        if let Some(tracks) = self.tracks.lock().unwrap().get_mut(playlist_id) {
            tracks.retain(|id| id != track_id);
        }
        Ok(())
    }

    async fn reorder_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        track_ids: &[String],
    ) -> CanopyResult<()> {
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
        *entry = track_ids.to_vec();
        Ok(())
    }

    async fn list_tracks(
        &self,
        profile_id: &str,
        playlist_id: &str,
        page: Page,
    ) -> CanopyResult<MediaPage> {
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
            .map(|track_id| MediaItem {
                id: track_id.clone(),
                ..MediaItem::default()
            })
            .collect();
        Ok(MediaPage {
            items,
            total_count,
            has_more: end < tracks.len(),
        })
    }
}
