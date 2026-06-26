# Logged-In Library And Preferences Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build profile-owned saved library items, track likes, and preferences for logged-in users only, with transparent proto and OpenAPI documentation.

**Architecture:** Durable state attaches to `profiles.id`, never anonymous sessions or the legacy `users` table. Domain services receive a verified `UserIdentity`, resolve it to a `UserProfile`, and call narrow repository ports. gRPC handlers reuse metadata auth and expose new logged-in-only RPCs without request-body `auth_token` fields.

**Tech Stack:** Rust workspace, `tonic`/`prost` gRPC, `sqlx` PostgreSQL migrations and repositories, in-memory stores for standalone/tests, `serde_json` for profile preferences, `docs/openapi.json` for Swagger/OpenAPI transparency.

---

## File Structure

- Create `migrations/20250626000001_profile_library_preferences.sql`: profile-owned tables, indexes, and update triggers.
- Modify `crates/canopy-server/tests/pg_integration.rs`: migration and PostgreSQL repository tests.
- Modify `crates/canopy-core/src/model.rs`, `repository.rs`, `lib.rs`: domain types and ports.
- Modify `crates/canopy-server/src/jade_store/memory.rs`, `pg.rs`, `mod.rs`: repository implementations and exports.
- Create `crates/canopy-server/src/library.rs`, `likes.rs`, `preferences.rs`: application services.
- Modify `crates/canopy-server/src/lib.rs`: module declarations and runtime wiring.
- Modify `crates/canopy-proto/proto/canopy.proto`: new authenticated RPCs and messages.
- Modify `crates/canopy-server/src/api/grpc.rs`: new RPC handlers and conversions.
- Modify `docs/openapi.json`: Swagger/OpenAPI paths, schemas, auth notes, and status metadata.
- Modify `README.md`: user-state status and API/Auth docs.

---

### Task 1: Add Profile-Owned Database Schema

**Files:**
- Create: `migrations/20250626000001_profile_library_preferences.sql`
- Modify: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Write the failing migration test**

Append `postgres_migrations_support_profile_library_likes_preferences_schema` to `crates/canopy-server/tests/pg_integration.rs`. It must run `sqlx::migrate!("../../migrations")`, then assert these public tables exist: `profile_library_items`, `profile_track_likes`, `profile_preferences`. It must also assert these indexes exist in `pg_indexes`: `idx_profile_library_items_profile_added_at`, `idx_profile_library_items_track_id`, `idx_profile_track_likes_profile_liked_at`, `idx_profile_track_likes_track_id`.

- [ ] **Step 2: Run the focused migration test**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test --features canopy-server/pg postgres_migrations_support_profile_library_likes_preferences_schema
```

Expected before implementation: fails because the tables do not exist, or skips with the existing database-not-configured message.

- [ ] **Step 3: Add the migration**

Create `migrations/20250626000001_profile_library_preferences.sql`:

```sql
-- Profile-owned durable library, likes, and preferences.
-- Anonymous sessions are operational only and never own these rows.

CREATE TABLE IF NOT EXISTS profile_library_items (
    profile_id  UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    track_id    UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    source      VARCHAR(64) NOT NULL DEFAULT 'manual',
    PRIMARY KEY (profile_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_profile_library_items_profile_added_at
    ON profile_library_items(profile_id, added_at DESC);

CREATE INDEX IF NOT EXISTS idx_profile_library_items_track_id
    ON profile_library_items(track_id);

DROP TRIGGER IF EXISTS trg_profile_library_items_updated_at ON profile_library_items;
CREATE TRIGGER trg_profile_library_items_updated_at
    BEFORE UPDATE ON profile_library_items
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE TABLE IF NOT EXISTS profile_track_likes (
    profile_id  UUID NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    track_id    UUID NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    liked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, track_id)
);

CREATE INDEX IF NOT EXISTS idx_profile_track_likes_profile_liked_at
    ON profile_track_likes(profile_id, liked_at DESC);

CREATE INDEX IF NOT EXISTS idx_profile_track_likes_track_id
    ON profile_track_likes(track_id);

DROP TRIGGER IF EXISTS trg_profile_track_likes_updated_at ON profile_track_likes;
CREATE TRIGGER trg_profile_track_likes_updated_at
    BEFORE UPDATE ON profile_track_likes
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();

CREATE TABLE IF NOT EXISTS profile_preferences (
    profile_id   UUID PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
    preferences  JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

DROP TRIGGER IF EXISTS trg_profile_preferences_updated_at ON profile_preferences;
CREATE TRIGGER trg_profile_preferences_updated_at
    BEFORE UPDATE ON profile_preferences
    FOR EACH ROW
    EXECUTE FUNCTION canopy_update_modified_column();
```

- [ ] **Step 4: Re-run the focused migration test**

Run the command from Step 2.

Expected after implementation: passes, or skips with the existing database-not-configured message.

- [ ] **Step 5: Commit schema slice**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add migrations/20250626000001_profile_library_preferences.sql crates/canopy-server/tests/pg_integration.rs
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Add profile library preferences schema"
```

---

### Task 2: Add Core Models And Repository Ports

**Files:**
- Modify: `crates/canopy-core/src/model.rs`
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-core/src/lib.rs`

- [ ] **Step 1: Add models**

Add these structs to `crates/canopy-core/src/model.rs` after `PlaybackHistoryEvent`:

```rust
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryItem {
    pub profile_id: String,
    pub track_id: String,
    pub added_at_epoch_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackLike {
    pub profile_id: String,
    pub track_id: String,
    pub liked_at_epoch_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfilePreferences {
    pub profile_id: String,
    pub values_json: String,
}
```

- [ ] **Step 2: Add repository ports**

In `crates/canopy-core/src/repository.rs`, import the new types and add:

```rust
#[async_trait]
pub trait LibraryRepository: Send + Sync {
    async fn save_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<LibraryItem>;
    async fn remove_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()>;
    async fn list_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<MediaPage>;
    async fn is_saved(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool>;
}

#[async_trait]
pub trait LikeRepository: Send + Sync {
    async fn like_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<TrackLike>;
    async fn unlike_track(&self, profile_id: &str, track_id: &str) -> CanopyResult<()>;
    async fn list_liked_tracks(&self, profile_id: &str, page: Page) -> CanopyResult<MediaPage>;
    async fn is_liked(&self, profile_id: &str, track_id: &str) -> CanopyResult<bool>;
}

#[async_trait]
pub trait PreferencesRepository: Send + Sync {
    async fn get_preferences(&self, profile_id: &str) -> CanopyResult<ProfilePreferences>;
    async fn upsert_preferences(
        &self,
        profile_id: &str,
        values_json: &str,
    ) -> CanopyResult<ProfilePreferences>;
}
```

- [ ] **Step 3: Export new types**

Update `crates/canopy-core/src/lib.rs` exports to include `LibraryItem`, `TrackLike`, `ProfilePreferences`, `LibraryRepository`, `LikeRepository`, and `PreferencesRepository`.

- [ ] **Step 4: Run core check**

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' check -p canopy-core
```

Expected: exits 0.

- [ ] **Step 5: Commit core slice**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add crates/canopy-core/src/model.rs crates/canopy-core/src/repository.rs crates/canopy-core/src/lib.rs
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Add profile library preference ports"
```

---

### Task 3: Implement Repositories

**Files:**
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Modify: `crates/canopy-server/src/jade_store/mod.rs`
- Modify: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Add PostgreSQL repository behavior test**

Add `postgres_repositories_support_profile_library_likes_preferences` to `crates/canopy-server/tests/pg_integration.rs`. Use the same profile and track setup style as `postgres_migrations_support_profile_scoped_history`. The test must call `save_track`, `save_track` again, `is_saved`, `list_tracks`, `remove_track`, `remove_track` again, `like_track`, `like_track` again, `is_liked`, `list_liked_tracks`, `unlike_track`, `unlike_track` again, `upsert_preferences`, and `get_preferences`.

- [ ] **Step 2: Run repository test to confirm missing implementation**

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test --features canopy-server/pg postgres_repositories_support_profile_library_likes_preferences
```

Expected before implementation: compile fails because the repositories are missing.

- [ ] **Step 3: Add in-memory repositories**

In `memory.rs`, add `InMemoryLibraryStore`, `InMemoryLikeStore`, and `InMemoryPreferencesStore`. Use `Mutex<HashMap<String, LibraryItem>>`, `Mutex<HashMap<String, TrackLike>>`, and `Mutex<HashMap<String, ProfilePreferences>>`. Key saved/liked rows as `format!("{profile_id}:{track_id}")`. Return empty `MediaPage` items containing `MediaItem { id: track_id, ..MediaItem::default() }` for list methods.

- [ ] **Step 4: Add PostgreSQL repositories**

In `pg.rs`, add `PgLibraryRepository`, `PgLikeRepository`, and `PgPreferencesRepository` using the existing `Arc<PgPool>` pattern. Saves/likes use `INSERT ... ON CONFLICT (profile_id, track_id) DO UPDATE SET updated_at = NOW() RETURNING profile_id::text, track_id::text, (EXTRACT(EPOCH FROM added_at) * 1000)::bigint AS added_at_epoch_ms`. Lists join `tracks`, `artists`, `albums`, and `REPRESENTATIVE_ASSET_JOIN`, then map rows with `media_item_from_row`.

- [ ] **Step 5: Normalize errors**

Malformed UUIDs return `CanopyError::InvalidArgument`. Foreign key failures caused by unknown tracks return `CanopyError::not_found("track", track_id)`. Other SQL errors use existing `db_err`.

- [ ] **Step 6: Export repository structs**

Update `jade_store/mod.rs` exports for the new in-memory stores and `#[cfg(feature = "pg")]` PostgreSQL repositories.

- [ ] **Step 7: Run repository tests**

Run the command from Step 2.

Expected after implementation: passes, or skips with the existing database-not-configured message.

- [ ] **Step 8: Commit repository slice**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add crates/canopy-server/src/jade_store/memory.rs crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/src/jade_store/mod.rs crates/canopy-server/tests/pg_integration.rs
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Add profile library preference repositories"
```

---

### Task 4: Add Application Services

**Files:**
- Create: `crates/canopy-server/src/library.rs`
- Create: `crates/canopy-server/src/likes.rs`
- Create: `crates/canopy-server/src/preferences.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Add service tests**

Each service module must include tests for missing profile, validation, idempotency, and success. Use `UserIdentity { user_id: "user-123".into() }`, `InMemoryProfileStore`, and the new in-memory repositories. Preferences tests must assert malformed JSON fails and valid JSON is stored in canonical compact form.

- [ ] **Step 2: Implement services**

Create services with these constructors and methods:

```rust
pub fn new(profiles: Arc<dyn ProfileRepository>, library: Arc<dyn LibraryRepository>) -> Self
pub async fn save_track(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<LibraryItem>
pub async fn remove_track(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<()>
pub async fn list_tracks(&self, identity: &UserIdentity, page: Page) -> CanopyResult<MediaPage>
pub async fn is_saved(&self, identity: &UserIdentity, track_id: &str) -> CanopyResult<bool>
```

Use equivalent method shapes for `LikeService`, and use `get_preferences` / `update_preferences` for `PreferencesService`. All services resolve `identity.user_id` through `ProfileRepository::get_by_external_user_id` and return `CanopyError::unauthenticated("profile not found")` if absent.

- [ ] **Step 3: Add module declarations**

In `crates/canopy-server/src/lib.rs`, add `pub mod library;`, `pub mod likes;`, and `pub mod preferences;`.

- [ ] **Step 4: Run service tests**

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test -p canopy-server library::tests
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test -p canopy-server likes::tests
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test -p canopy-server preferences::tests
```

Expected: all pass.

- [ ] **Step 5: Commit service slice**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add crates/canopy-server/src/library.rs crates/canopy-server/src/likes.rs crates/canopy-server/src/preferences.rs crates/canopy-server/src/lib.rs
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Add profile library preference services"
```

---

### Task 5: Extend Proto And gRPC

**Files:**
- Modify: `crates/canopy-proto/proto/canopy.proto`
- Modify: `crates/canopy-server/src/api/grpc.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Add proto RPCs and messages**

Add RPCs after `RecordPlaybackHistory`: `SaveLibraryItem`, `RemoveLibraryItem`, `ListLibraryItems`, `LikeTrack`, `UnlikeTrack`, `ListLikedTracks`, `GetPreferences`, and `UpdatePreferences`. Add request/response messages with no `auth_token`: track mutation requests use `track_id`; list requests use `limit` and `offset`; preference requests/responses use `preferences_json`.

- [ ] **Step 2: Add gRPC service fields and handlers**

Add `LibraryService`, `LikeService`, and `PreferencesService` to `GrpcServices` and `GrpcApi`. Add `extract_metadata_identity(metadata, auth)` that calls `extract_identity(metadata, "", auth)`. New RPCs must use metadata auth only, map errors through `to_status`, and return `to_proto_items(page)` for list responses.

- [ ] **Step 3: Wire runtime composition**

In `run`, create repository Arcs in both `pg` and in-memory branches, construct the three services after `profile` and `history`, and pass them into `GrpcServices`.

- [ ] **Step 4: Run server check**

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' check -p canopy-server --all-features
```

Expected: exits 0.

- [ ] **Step 5: Commit API slice**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add crates/canopy-proto/proto/canopy.proto crates/canopy-server/src/api/grpc.rs crates/canopy-server/src/lib.rs
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Expose profile library preference RPCs"
```

---

### Task 6: Update Swagger/OpenAPI And README

**Files:**
- Modify: `docs/openapi.json`
- Modify: `README.md`

- [ ] **Step 1: Add OpenAPI paths**

Add paths for all eight new RPCs. Each description must say the RPC requires gRPC metadata auth via `authorization: Bearer <token>` or `x-canopy-auth-token`, and anonymous sessions are not accepted.

- [ ] **Step 2: Add OpenAPI schemas**

Add schemas matching the proto messages exactly. `preferences_json` must say it is a JSON object string. List responses include `items`, `total_count`, and `has_more`.

- [ ] **Step 3: Add OpenAPI status notes**

Add `libraryStatus`, `likesStatus`, and `preferencesStatus` to `x-canopy-docs`, each stating logged-in-only profile-owned persistence and no anonymous backend persistence.

- [ ] **Step 4: Update README**

Update the Auth / Profiles row, the gRPC API snippet, and the Auth section. Add this paragraph:

```markdown
Saved library items, track likes, and preferences follow the same boundary: they require metadata auth, resolve the verified identity to a profile, and persist only under `profiles.id`. Anonymous clients may cache these locally, but Canopy does not store them until the user logs in and calls `UpsertProfile`.
```

- [ ] **Step 5: Validate docs**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy python3 -m json.tool docs/openapi.json
rg -n "â|\?\?" README.md docs/openapi.json crates/canopy-proto/proto/canopy.proto
```

Expected: JSON validates and ripgrep returns no matches.

- [ ] **Step 6: Commit docs slice**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add docs/openapi.json README.md
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Document profile library preference API"
```

---

### Task 7: Full Verification

**Files:**
- Inspect all modified files.

- [ ] **Step 1: Run format, clippy, and tests**

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' fmt --all
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' clippy --workspace --all-features --tests -- -D warnings
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\Cătălina\.cargo\bin\cargo.exe' test --features canopy-server/pg
```

Expected: every command exits 0.

- [ ] **Step 2: Validate docs and whitespace**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy python3 -m json.tool docs/openapi.json
rg -n "â|\?\?" README.md docs/openapi.json crates/canopy-proto/proto/canopy.proto crates/canopy-core/src crates/canopy-server/src
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git diff --check
```

Expected: JSON validates, ripgrep returns no matches, and `git diff --check` exits 0.

- [ ] **Step 3: Inspect final status**

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git status --short --branch
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git log --oneline -8
```

Expected: clean working tree on `master`, with this feature's commits visible.
