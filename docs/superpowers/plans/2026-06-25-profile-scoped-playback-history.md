# Profile-Scoped Playback History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add opt-in durable playback history for real logged-in profiles while keeping anonymous users free of backend history persistence.

**Architecture:** Add a vertical slice behind a new `PlaybackHistoryRepository` port. `HistoryService` verifies auth tokens, resolves them to `profiles`, enforces `history_enabled`, validates playback facts, and records events through in-memory or PostgreSQL adapters. The gRPC layer exposes an explicit `RecordPlaybackHistory` RPC and existing playback/session flows remain anonymous-compatible.

**Tech Stack:** Rust, Tokio, tonic, async-trait, SQLx/PostgreSQL, protobuf, Cargo workspace (`canopy-core`, `canopy-server`, `canopy-proto`).

---

## File Structure

- Modify `crates/canopy-core/src/model.rs`: add `PlaybackHistoryEvent` domain model.
- Modify `crates/canopy-core/src/repository.rs`: add `PlaybackHistoryRepository` port.
- Modify `crates/canopy-core/src/lib.rs`: export new model and port.
- Create `crates/canopy-server/src/history.rs`: authenticated history service and unit tests.
- Modify `crates/canopy-server/src/jade_store/memory.rs`: add `InMemoryPlaybackHistoryStore`.
- Modify `crates/canopy-server/src/jade_store/pg.rs`: add `PgPlaybackHistoryRepository`.
- Modify `crates/canopy-server/src/jade_store/mod.rs`: export history stores.
- Create `migrations/20250625000002_profile_scoped_playback_history.sql`: align history table to `profiles`.
- Modify `crates/canopy-proto/proto/canopy.proto`: add `RecordPlaybackHistory` RPC and messages.
- Modify `crates/canopy-server/src/api/grpc.rs`: wire request/response to `HistoryService`.
- Modify `crates/canopy-server/src/lib.rs`: construct and inject `HistoryService`.
- Modify `crates/canopy-server/tests/pg_integration.rs`: add migration/repository integration coverage.
- Modify `README.md` and `docs/openapi.json`: document logged-in opt-in history.

---

### Task 1: Domain Model And Port

**Files:**
- Modify: `crates/canopy-core/src/model.rs`
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-core/src/lib.rs`

- [ ] **Step 1: Add the domain event model**

Add this struct near `UserProfile` in `crates/canopy-core/src/model.rs`:

```rust
/// A durable playback-history event for a real logged-in profile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackHistoryEvent {
    /// Internal profile identifier that owns the event.
    pub profile_id: String,
    /// Track identifier that was played.
    pub track_id: String,
    /// Duration listened, in milliseconds.
    pub duration_ms: i64,
    /// Completion percentage in the inclusive range `0.0..=1.0`.
    pub completion_pct: f32,
}
```

- [ ] **Step 2: Add the repository port**

Update the import in `crates/canopy-core/src/repository.rs`:

```rust
use crate::model::{
    AudioAsset, MediaItem, MediaPage, Page, PlaybackHistoryEvent, ProviderTrack, Session,
    UserProfile,
};
```

Add this trait after `ProfileRepository`:

```rust
/// Persistence of durable playback history for logged-in profiles.
#[async_trait]
pub trait PlaybackHistoryRepository: Send + Sync {
    /// Records one playback-history event for a real profile.
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<()>;
}
```

- [ ] **Step 3: Export the new types**

Update `crates/canopy-core/src/lib.rs` exports to include:

```rust
pub use model::PlaybackHistoryEvent;
pub use repository::PlaybackHistoryRepository;
```

- [ ] **Step 4: Run formatting and a focused compile check**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' fmt --all
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' check -p canopy-core
```

Expected: both commands exit 0.

---

### Task 2: History Service With Unit Tests

**Files:**
- Create: `crates/canopy-server/src/history.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/mod.rs`

- [ ] **Step 1: Add a minimal in-memory history store for tests**

In `crates/canopy-server/src/jade_store/memory.rs`, extend the imports:

```rust
use canopy_core::{
    AudioAsset, AudioAssetRepository, CanopyResult, CatalogRepository, DiscoveryRepository,
    MediaItem, MediaPage, Page, PlaybackHistoryEvent, PlaybackHistoryRepository,
    ProfileRepository, Session, SessionRepository, UserProfile,
};
```

Add this store after `InMemoryProfileStore`:

```rust
/// In-memory playback-history store for tests and standalone prototype mode.
#[derive(Default)]
pub struct InMemoryPlaybackHistoryStore {
    events: Mutex<Vec<PlaybackHistoryEvent>>,
}

impl InMemoryPlaybackHistoryStore {
    /// Returns a snapshot of stored events for tests.
    pub fn events(&self) -> CanopyResult<Vec<PlaybackHistoryEvent>> {
        Ok(self.events.lock().unwrap().clone())
    }
}

#[async_trait]
impl PlaybackHistoryRepository for InMemoryPlaybackHistoryStore {
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<()> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}
```

In `crates/canopy-server/src/jade_store/mod.rs`, export it:

```rust
pub use memory::{
    InMemoryAudioAssetStore, InMemoryCatalog, InMemoryPlaybackHistoryStore, InMemoryProfileStore,
    InMemorySessionStore,
};
```

- [ ] **Step 2: Write `HistoryService` and tests**

Create `crates/canopy-server/src/history.rs`:

```rust
//! Logged-in playback history service.
//!
//! Anonymous sessions are not durable users. This service records history only
//! after token verification, profile lookup, and `history_enabled` consent.

use std::sync::Arc;

use canopy_core::{
    CanopyError, CanopyResult, PlaybackHistoryEvent, PlaybackHistoryRepository, ProfileRepository,
};

use crate::auth::AuthService;

/// Application service for profile-scoped playback history.
#[derive(Clone)]
pub struct HistoryService {
    profiles: Arc<dyn ProfileRepository>,
    history: Arc<dyn PlaybackHistoryRepository>,
    auth: AuthService,
}

impl HistoryService {
    /// Creates a history service over profile storage, history storage, and auth.
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        history: Arc<dyn PlaybackHistoryRepository>,
        auth: AuthService,
    ) -> Self {
        Self {
            profiles,
            history,
            auth,
        }
    }

    /// Records a playback event when the real logged-in profile has opted in.
    pub async fn record_playback(
        &self,
        auth_token: &str,
        track_id: &str,
        duration_ms: i64,
        completion_pct: f32,
    ) -> CanopyResult<bool> {
        let track_id = track_id.trim();
        if track_id.is_empty() {
            return Err(CanopyError::InvalidArgument("track_id is required".into()));
        }
        if duration_ms < 0 {
            return Err(CanopyError::InvalidArgument(
                "duration_ms must be non-negative".into(),
            ));
        }
        if !(0.0..=1.0).contains(&completion_pct) {
            return Err(CanopyError::InvalidArgument(
                "completion_pct must be between 0.0 and 1.0".into(),
            ));
        }

        let identity = self.auth.verify(auth_token)?;
        let profile = self
            .profiles
            .get_by_external_user_id(&identity.user_id)
            .await?
            .ok_or_else(|| CanopyError::unauthenticated("profile not found"))?;

        if !profile.history_enabled {
            return Ok(false);
        }

        self.history
            .record(PlaybackHistoryEvent {
                profile_id: profile.id,
                track_id: track_id.to_string(),
                duration_ms,
                completion_pct,
            })
            .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::jade_store::{InMemoryPlaybackHistoryStore, InMemoryProfileStore};
    use std::sync::Arc;
    use std::time::Duration;

    fn token(auth: &AuthService) -> String {
        auth.mint("user-123", Duration::from_secs(3600)).unwrap()
    }

    #[tokio::test]
    async fn record_playback_rejects_invalid_token() {
        let auth = AuthService::new("secret");
        let service = HistoryService::new(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaybackHistoryStore::default()),
            auth,
        );

        let err = service
            .record_playback("bad-token", "track-1", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn record_playback_rejects_missing_profile() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let service = HistoryService::new(
            Arc::new(InMemoryProfileStore::default()),
            Arc::new(InMemoryPlaybackHistoryStore::default()),
            auth,
        );

        let err = service
            .record_playback(&auth_token, "track-1", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::Unauthenticated(_)));
    }

    #[tokio::test]
    async fn record_playback_returns_false_when_history_disabled() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), false)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = HistoryService::new(profiles, history.clone(), auth);

        let recorded = service
            .record_playback(&auth_token, "track-1", 1000, 0.5)
            .await
            .unwrap();

        assert!(!recorded);
        assert!(history.events().unwrap().is_empty());
    }

    #[tokio::test]
    async fn record_playback_persists_when_history_enabled() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let profiles = Arc::new(InMemoryProfileStore::default());
        let profile = profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let history = Arc::new(InMemoryPlaybackHistoryStore::default());
        let service = HistoryService::new(profiles, history.clone(), auth);

        let recorded = service
            .record_playback(&auth_token, "track-1", 1000, 0.5)
            .await
            .unwrap();

        assert!(recorded);
        assert_eq!(
            history.events().unwrap(),
            vec![PlaybackHistoryEvent {
                profile_id: profile.id,
                track_id: "track-1".into(),
                duration_ms: 1000,
                completion_pct: 0.5,
            }]
        );
    }

    #[tokio::test]
    async fn record_playback_rejects_invalid_playback_facts() {
        let auth = AuthService::new("secret");
        let auth_token = token(&auth);
        let profiles = Arc::new(InMemoryProfileStore::default());
        profiles
            .upsert_profile("user-123", Some("Ada"), true)
            .await
            .unwrap();
        let service = HistoryService::new(
            profiles,
            Arc::new(InMemoryPlaybackHistoryStore::default()),
            auth,
        );

        let err = service
            .record_playback(&auth_token, " ", 1000, 0.5)
            .await
            .unwrap_err();

        assert!(matches!(err, CanopyError::InvalidArgument(_)));
    }
}
```

- [ ] **Step 3: Export the module**

Add this line to `crates/canopy-server/src/lib.rs`:

```rust
pub mod history;
```

- [ ] **Step 4: Run the focused tests**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test -p canopy-server history::tests
```

Expected: new `history::tests::*` tests pass.

---

### Task 3: PostgreSQL History Adapter And Migration

**Files:**
- Create: `migrations/20250625000002_profile_scoped_playback_history.sql`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Modify: `crates/canopy-server/src/jade_store/mod.rs`
- Modify: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Add the migration**

Create `migrations/20250625000002_profile_scoped_playback_history.sql`:

```sql
-- Align durable playback history with real logged-in profiles.
-- Anonymous sessions are operational only and never own backend history.

ALTER TABLE playback_history DROP CONSTRAINT IF EXISTS playback_history_user_id_fkey;
DROP INDEX IF EXISTS idx_playback_history_user_id;
DROP INDEX IF EXISTS idx_playback_history_user_track;

ALTER TABLE playback_history
    DROP COLUMN IF EXISTS user_id,
    ADD COLUMN IF NOT EXISTS profile_id UUID REFERENCES profiles(id) ON DELETE CASCADE;

DELETE FROM playback_history WHERE profile_id IS NULL;

ALTER TABLE playback_history
    ALTER COLUMN profile_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_playback_history_profile_id
    ON playback_history(profile_id);

CREATE INDEX IF NOT EXISTS idx_playback_history_profile_played_at
    ON playback_history(profile_id, played_at DESC);
```

- [ ] **Step 2: Add the PostgreSQL adapter**

In `crates/canopy-server/src/jade_store/pg.rs`, extend imports to include `PlaybackHistoryEvent` and `PlaybackHistoryRepository`, then add:

```rust
/// PostgreSQL-backed durable playback-history repository.
#[derive(Clone)]
pub struct PgPlaybackHistoryRepository {
    pool: PgPool,
}

impl PgPlaybackHistoryRepository {
    /// Creates a PostgreSQL playback-history repository.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PlaybackHistoryRepository for PgPlaybackHistoryRepository {
    async fn record(&self, event: PlaybackHistoryEvent) -> CanopyResult<()> {
        sqlx::query(
            r#"
            INSERT INTO playback_history (profile_id, track_id, duration_ms, completion_pct)
            VALUES ($1::uuid, $2::uuid, $3, $4)
            "#,
        )
        .bind(&event.profile_id)
        .bind(&event.track_id)
        .bind(event.duration_ms as i32)
        .bind(event.completion_pct)
        .execute(&self.pool)
        .await
        .map_err(|e| CanopyError::Storage(e.to_string()))?;
        Ok(())
    }
}
```

In `crates/canopy-server/src/jade_store/mod.rs`, export it under `pg`:

```rust
pub use pg::{
    PgAudioAssetRepository, PgCatalogRepository, PgPlaybackHistoryRepository,
    PgProfileRepository, PgSessionRepository,
};
```

- [ ] **Step 3: Add PostgreSQL integration test**

In `crates/canopy-server/tests/pg_integration.rs`, import the new repository and event type, then add:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn postgres_migrations_support_profile_scoped_history(pool: sqlx::PgPool) {
    let profiles = PgProfileRepository::new(pool.clone());
    let history = PgPlaybackHistoryRepository::new(pool.clone());
    let profile = profiles
        .upsert_profile("history-user", Some("Ada"), true)
        .await
        .unwrap();

    let artist_id: String = sqlx::query_scalar(
        r#"
        INSERT INTO artists (name, sort_name) VALUES ('History Artist', 'history artist')
        RETURNING id::text
        "#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let album_id: String = sqlx::query_scalar(
        r#"
        INSERT INTO albums (title, artist_id) VALUES ('History Album', $1::uuid)
        RETURNING id::text
        "#,
    )
    .bind(&artist_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    let track_id: String = sqlx::query_scalar(
        r#"
        INSERT INTO tracks (title, artist_id, album_id, duration_ms)
        VALUES ('History Track', $1::uuid, $2::uuid, 1000)
        RETURNING id::text
        "#,
    )
    .bind(&artist_id)
    .bind(&album_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    history
        .record(PlaybackHistoryEvent {
            profile_id: profile.id.clone(),
            track_id: track_id.clone(),
            duration_ms: 1000,
            completion_pct: 0.75,
        })
        .await
        .unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM playback_history WHERE profile_id = $1::uuid AND track_id = $2::uuid",
    )
    .bind(&profile.id)
    .bind(&track_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 1);
}
```

If this test conflicts with existing helper style in `pg_integration.rs`, keep the same assertions but reuse the local helper functions already in that file.

- [ ] **Step 4: Run pg integration test**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test --features canopy-server/pg postgres_migrations_support_profile_scoped_history
```

Expected: the new pg integration test passes.

---

### Task 4: gRPC Contract And Wiring

**Files:**
- Modify: `crates/canopy-proto/proto/canopy.proto`
- Modify: `crates/canopy-server/src/api/grpc.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Add proto RPC and messages**

In `crates/canopy-proto/proto/canopy.proto`, add this RPC near `UpsertProfile`:

```proto
rpc RecordPlaybackHistory(RecordPlaybackHistoryRequest) returns (RecordPlaybackHistoryResponse);
```

Add these messages near profile messages:

```proto
message RecordPlaybackHistoryRequest {
  string auth_token = 1;       // required login token; anonymous sessions are not accepted
  string track_id = 2;         // required catalog track id
  int64 duration_ms = 3;       // listened duration in milliseconds
  float completion_pct = 4;    // inclusive range 0.0..1.0
}
message RecordPlaybackHistoryResponse {
  bool recorded = 1;           // false when profile history is disabled
}
```

- [ ] **Step 2: Wire service construction**

In `crates/canopy-server/src/lib.rs`, add imports and repository variables:

```rust
use history::HistoryService;
```

Add a repository binding next to `profile_repo`:

```rust
let history_repo: Arc<dyn canopy_core::PlaybackHistoryRepository>;
```

Set it in the PostgreSQL branch:

```rust
history_repo = Arc::new(jade_store::PgPlaybackHistoryRepository::new((*pool).clone()));
```

Set it in both in-memory fallback branches:

```rust
history_repo = Arc::new(jade_store::InMemoryPlaybackHistoryStore::default());
```

Create the service after `profile`:

```rust
let history = HistoryService::new(
    profile_repo.clone(),
    history_repo,
    AuthService::new(config.auth_token_secret.clone()),
);
```

Pass it to `GrpcApi::new(...)` after `profile`.

- [ ] **Step 3: Wire gRPC adapter**

In `crates/canopy-server/src/api/grpc.rs`, import:

```rust
RecordPlaybackHistoryRequest, RecordPlaybackHistoryResponse,
```

Add a field to `GrpcApi`:

```rust
history: HistoryService,
```

Update `GrpcApi::new` to accept and assign `history`.

Add the RPC implementation:

```rust
async fn record_playback_history(
    &self,
    request: Request<RecordPlaybackHistoryRequest>,
) -> Result<Response<RecordPlaybackHistoryResponse>, Status> {
    let req = request.into_inner();
    let recorded = self
        .history
        .record_playback(
            &req.auth_token,
            &req.track_id,
            req.duration_ms,
            req.completion_pct,
        )
        .await
        .map_err(to_status)?;
    Ok(Response::new(RecordPlaybackHistoryResponse { recorded }))
}
```

- [ ] **Step 4: Run generated-code compile check**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' check -p canopy-server --all-features
```

Expected: compile succeeds and tonic generated trait methods match the adapter.

---

### Task 5: Documentation And OpenAPI

**Files:**
- Modify: `README.md`
- Modify: `docs/openapi.json`
- Modify: `.env.example` only if no auth secret entry exists; current repo already has `CANOPY_AUTH_TOKEN_SECRET`.

- [ ] **Step 1: Update README status and auth/history docs**

In the status table, update Auth / Profiles notes to mention `RecordPlaybackHistory`:

```markdown
| Auth / Profiles           | ðŸŸ¡ Partial     | Browse/search/playback remain anonymous-compatible. Durable user state starts at `UpsertProfile`; opt-in backend history is recorded through `RecordPlaybackHistory`; anonymous users get no backend history, library, likes, or preferences. |
```

In the Auth section, add this sentence after the `history_enabled` paragraph:

```markdown
`RecordPlaybackHistory` is the first profile-scoped durable state endpoint. It requires a login token, resolves the token to a profile, and records history only when `history_enabled=true`; disabled history returns a successful response with `recorded=false`.
```

- [ ] **Step 2: Update OpenAPI shim**

In `docs/openapi.json`, add path `/canopy.Canopy/RecordPlaybackHistory` with request schema `RecordPlaybackHistoryRequest` and response schema `RecordPlaybackHistoryResponse`. Add schemas matching the proto fields. Update `x-canopy-docs.authModel` and add:

```json
"historyStatus": "RecordPlaybackHistory requires a verified login token and an existing profile. It records profile-scoped history only when history_enabled=true; anonymous sessions never create backend history."
```

- [ ] **Step 3: Validate JSON**

Run:

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy python3 -m json.tool docs/openapi.json > /tmp/canopy-openapi.json
```

Expected: command exits 0.

---

### Task 6: Full Verification And Commit

**Files:**
- All files modified in Tasks 1-5.

- [ ] **Step 1: Run full verification**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' fmt --all
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' clippy --workspace --all-features --tests -- -D warnings
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test --features canopy-server/pg
```

Expected: every command exits 0.

- [ ] **Step 2: Inspect diff hygiene**

Run:

```powershell
git diff --check
& 'C:\Program Files\Git\cmd\git.exe' diff --stat
& 'C:\Program Files\Git\cmd\git.exe' status --short --branch
```

Expected: `rg` exits 1 with no matches, diff contains only the planned files, and status shows only intended changes.

- [ ] **Step 3: Commit the implementation**

Run:

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add README.md docs/openapi.json crates/canopy-core/src/lib.rs crates/canopy-core/src/model.rs crates/canopy-core/src/repository.rs crates/canopy-proto/proto/canopy.proto crates/canopy-server/src/api/grpc.rs crates/canopy-server/src/history.rs crates/canopy-server/src/jade_store/memory.rs crates/canopy-server/src/jade_store/mod.rs crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/src/lib.rs crates/canopy-server/tests/pg_integration.rs migrations/20250625000002_profile_scoped_playback_history.sql
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Add profile-scoped playback history"
```

Expected: commit succeeds with only the implementation files staged.



