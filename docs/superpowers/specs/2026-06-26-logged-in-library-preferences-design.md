# Logged-In Library And Preferences Design

## Goal

Add the database and service foundation for logged-in-only durable library state: saved library items, track likes, and profile preferences. Anonymous users can still browse, search, resolve playback, and play without backend persistence; any anonymous cache remains a client responsibility.

## Context

Canopy now has a real authenticated profile boundary. UpsertProfile creates durable profiles from verified end-user identity, and RecordPlaybackHistory records profile-scoped history only when history_enabled is true. The README names library items, likes, and preferences as future durable user features, but the codebase does not yet model them.

The older schema contains users, playlists, playlist_tracks, and user_favorites from the early target architecture. The current auth boundary deliberately moved durable user identity to profiles. New logged-in state should therefore attach to profiles.id, not to users.id or anonymous session_id.

## Chosen Approach

Use profile-owned canonical tables and repository ports:

- profile_library_items stores saved catalog items for a profile.
- profile_track_likes stores a profile's positive like state for tracks.
- profile_preferences stores profile-scoped settings as a typed JSON document plus selected indexed columns when they become query-relevant.

Library saves and likes should be idempotent. Saving the same track twice refreshes updated_at/added_at metadata without creating duplicates. Unsave/unlike operations should be successful when the item is already absent, because clients may retry after uncertain network failures.

Preferences should be profile-scoped and explicitly updated by authenticated clients. They are not inferred from anonymous usage, playback history, or session state. The first preference document should include history_enabled only if we intentionally migrate it out of profiles later; for now history_enabled stays on profiles because it gates writes in a hot path and is already part of UserProfile.

## Alternatives Considered

### Reuse users and user_favorites

This is tempting because user_favorites already exists, but it conflicts with the new boundary. The users table has no active service contract, while profiles is the durable authenticated identity root. Reusing user_favorites would force profile-to-user mapping or duplicate identity semantics.

### Store all user state in profile_preferences JSON

This is flexible, but it makes library membership and likes hard to query, dedupe, paginate, and constrain. It also makes common operations such as save, unsave, like, and unlike rewrite a larger document than needed.

### Build playlists first

Playlists are important, but saved library items and likes are a smaller primitive. They establish profile-owned durable state and can later feed playlists, recommendations, and sync without committing to playlist ordering and collaboration semantics now.

## Data Model

profile_library_items:

- profile_id UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE
- track_id UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE
- added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
- updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
- source VARCHAR(64) NOT NULL DEFAULT 'manual'
- PRIMARY KEY (profile_id, track_id)

Indexes:

- (profile_id, added_at DESC) for library listing.
- (track_id) for catalog-side aggregate or cleanup queries.

profile_track_likes:

- profile_id UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE
- track_id UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE
- liked_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
- updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
- PRIMARY KEY (profile_id, track_id)

Indexes:

- (profile_id, liked_at DESC) for liked-track listing.
- (track_id) for aggregate counts later.

profile_preferences:

- profile_id UUID PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE
- preferences JSONB NOT NULL DEFAULT '{}'::jsonb
- created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
- updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()

Use JSONB for settings that are not queried server-side yet, such as explicit-content visibility, default discovery mode, preferred codecs, and client feature toggles. Add typed columns later only when the server needs constraints or indexes.

## Domain Model And Ports

Add small domain types in canopy-core:

- LibraryItem { profile_id, track_id, added_at_epoch_ms }
- TrackLike { profile_id, track_id, liked_at_epoch_ms }
- ProfilePreferences { profile_id, values }

The repository ports should stay narrow:

- LibraryRepository: save_track, remove_track, list_tracks, is_saved
- LikeRepository: like_track, unlike_track, list_liked_tracks, is_liked
- PreferencesRepository: get_preferences, upsert_preferences

Ports accept profile_id, not auth tokens or external_user_id. Application services are responsible for resolving UserIdentity to UserProfile through ProfileRepository first, matching the playback history service pattern.

## Services

Add three application services:

- LibraryService resolves the authenticated identity to a profile, then saves/removes/lists profile library items.
- LikeService resolves the authenticated identity to a profile, then likes/unlikes/lists liked tracks.
- PreferencesService resolves the authenticated identity to a profile, then reads or updates profile preferences.

All services reject requests when the verified identity has no profile. That keeps UpsertProfile as the explicit moment where durable backend state starts.

The first implementation should validate track_id as non-empty and rely on repository/database foreign keys to reject unknown tracks. If we want friendlier errors later, services can consult CatalogRepository before writing.

## API Contract

Add authenticated gRPC RPCs after the profile/history RPCs:

- SaveLibraryItem(SaveLibraryItemRequest) returns SaveLibraryItemResponse
- RemoveLibraryItem(RemoveLibraryItemRequest) returns RemoveLibraryItemResponse
- ListLibraryItems(ListLibraryItemsRequest) returns ListLibraryItemsResponse
- LikeTrack(LikeTrackRequest) returns LikeTrackResponse
- UnlikeTrack(UnlikeTrackRequest) returns UnlikeTrackResponse
- ListLikedTracks(ListLikedTracksRequest) returns ListLikedTracksResponse
- GetPreferences(GetPreferencesRequest) returns GetPreferencesResponse
- UpdatePreferences(UpdatePreferencesRequest) returns UpdatePreferencesResponse

These RPCs use the same gRPC metadata auth extractor as UpsertProfile and RecordPlaybackHistory. New request messages should not add auth_token fields. Compatibility fallback fields only exist on the older RPCs.

List responses should return MediaItem summaries so the client can render saved/liked lists without separate get-media fanout. Pagination uses limit and offset, with the same clamping policy as catalog/search.

Preferences should start as a JSON string in proto if we want to avoid committing generated proto schemas to every setting. A later typed proto can replace or wrap it when settings stabilize.

## Error Handling

- Missing or invalid auth metadata returns Unauthenticated.
- Verified identity without a profile returns Unauthenticated until UpsertProfile creates the profile.
- Empty track_id returns InvalidArgument.
- Unknown track_id returns NotFound. The implementation may precheck CatalogRepository or normalize PostgreSQL foreign key failures, but callers should see the same domain error either way.
- Malformed preference JSON returns InvalidArgument.

Idempotent remove/unlike operations return success even when no row existed.

## Testing

Migration tests should assert all new tables, primary keys, foreign keys, and indexes exist. Repository tests should cover idempotent save/like, remove/unlike, pagination order, profile isolation, and preferences upsert/readback.

Service tests should cover profile-required behavior, auth identity resolution, validation, and idempotency. gRPC tests can focus on compile wiring and metadata auth behavior if the extractor is already covered.

Full verification remains cargo fmt, all-feature clippy with warnings denied, default tests, PostgreSQL-feature tests, OpenAPI JSON validation, and git diff --check.

## Documentation

Update README status and Auth sections to state that library, likes, and preferences are logged-in-only durable state. Update docs/openapi.json with the new RPCs, request/response schemas, and x-canopy-docs notes. Proto comments should explicitly say these RPCs require metadata auth and do not support anonymous persistence.

## Non-Goals

This slice does not implement playlists, collaborative playlists, recommendations, social follows, offline conflict resolution, cross-device merge policies, preference schema migrations, removing legacy users/user_favorites, or deleting the auth_token compatibility fields from existing profile/history messages.
