# Audited Canopy V1 Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the prototype monolithic protobuf API with a clean, locally generated `canopy.v1` bounded-service contract and a fully working Canopy server before publishing anything to the BSR.

**Architecture:** One protobuf package defines bounded gRPC services that Canopy registers on the existing listener. PandaEngine retains local player control; Canopy owns catalog, playback-source authorization, discovery, profile state, history, durable collections, and status. The existing `canopy-proto` crate remains the local generation facade until the later BSR cutover.

**Tech Stack:** Rust, Tonic 0.14, Prost, protobuf v3, SQLx, PostgreSQL, Tokio, Buf CLI.

**Repository rule:** Keep every change unstaged and uncommitted. Do not modify Git history.

---

## File Map

- Modify `crates/canopy-proto/proto/canopy.proto`: audited `canopy.v1` package, bounded services, shared resources, and canonical status conventions.
- Modify `crates/canopy-proto/build.rs`: compile the audited package without changing local generation ownership yet.
- Modify `crates/canopy-proto/src/lib.rs`: include and re-export `canopy.v1`.
- Create `buf.yaml`: local Buf module, lint, and future compatibility policy.
- Split `crates/canopy-server/src/api/grpc.rs` into service adapters under `crates/canopy-server/src/api/grpc/`.
- Modify `crates/canopy-server/src/lib.rs`: register all generated service servers on one Tonic listener.
- Modify `crates/canopy-core/src/model.rs` and `repository.rs`: typed page tokens, profile lifecycle, playlist revisions, and platform-neutral artwork references.
- Modify PostgreSQL and in-memory adapters under `crates/canopy-server/src/jade_store/`.
- Add a new migration under `migrations/`: profile lifecycle support, playlist revisioning, and removal of obsolete server playback sessions.
- Modify `docs/openapi.json` and `README.md`: describe the bounded gRPC contract and remove prototype claims.
- Create `crates/canopy-server/tests/proto_contract.rs`: generated-contract and service-registration assertions.

### Task 1: Establish The Audited Protobuf Package

**Files:**
- Create: `buf.yaml`
- Modify: `crates/canopy-proto/proto/canopy.proto`
- Modify: `crates/canopy-proto/src/lib.rs`
- Test: `crates/canopy-server/tests/proto_contract.rs`

- [ ] **Step 1: Add a compile-failing contract test**

Create `proto_contract.rs` importing the desired generated modules:

```rust
use canopy_proto::{
    catalog_service_server::CatalogService,
    discovery_service_server::DiscoveryService,
    history_service_server::HistoryService,
    library_service_server::LibraryService,
    playback_service_server::PlaybackService,
    playlist_service_server::PlaylistService,
    profile_service_server::ProfileService,
    system_service_server::SystemService,
};

#[test]
fn audited_v1_services_are_generated() {
    fn assert_service<T: ?Sized>() {}

    assert_service::<dyn CatalogService>();
    assert_service::<dyn PlaybackService>();
    assert_service::<dyn DiscoveryService>();
    assert_service::<dyn ProfileService>();
    assert_service::<dyn HistoryService>();
    assert_service::<dyn LibraryService>();
    assert_service::<dyn PlaylistService>();
    assert_service::<dyn SystemService>();
}
```

- [ ] **Step 2: Verify the contract test is red**

Run: `cargo test -p canopy-server --test proto_contract --all-features`

Expected: compilation fails because the bounded generated service modules do not exist.

- [ ] **Step 3: Replace the monolithic service declaration**

Create the local Buf configuration first:

```yaml
version: v2
modules:
  - path: crates/canopy-proto/proto
lint:
  use:
    - STANDARD
breaking:
  use:
    - FILE
```

Set `package canopy.v1;` and declare exactly:

```proto
service CatalogService {
  rpc Browse(BrowseRequest) returns (BrowseResponse);
  rpc Search(SearchRequest) returns (SearchResponse);
  rpc GetMedia(GetMediaRequest) returns (Track);
}

service PlaybackService {
  rpc ResolvePlayback(ResolvePlaybackRequest) returns (PlaybackSource);
}

service DiscoveryService {
  rpc GetDiscoveryFeed(GetDiscoveryFeedRequest)
      returns (GetDiscoveryFeedResponse);
}

service ProfileService {
  rpc UpsertProfile(UpsertProfileRequest) returns (Profile);
  rpc GetProfile(GetProfileRequest) returns (Profile);
  rpc UpdateProfile(UpdateProfileRequest) returns (Profile);
  rpc DeleteProfile(DeleteProfileRequest) returns (google.protobuf.Empty);
  rpc GetPreferences(GetPreferencesRequest) returns (Preferences);
  rpc UpdatePreferences(UpdatePreferencesRequest) returns (Preferences);
}

service HistoryService {
  rpc GetHistorySettings(GetHistorySettingsRequest) returns (HistorySettings);
  rpc UpdateHistorySettings(UpdateHistorySettingsRequest)
      returns (UpdateHistorySettingsResponse);
  rpc RecordPlayback(RecordPlaybackRequest) returns (RecordPlaybackResponse);
  rpc ListHistory(ListHistoryRequest) returns (ListHistoryResponse);
  rpc DeleteHistoryEntry(DeleteHistoryEntryRequest)
      returns (google.protobuf.Empty);
  rpc ClearHistory(ClearHistoryRequest) returns (ClearHistoryResponse);
}

service LibraryService {
  rpc SaveTrack(SaveTrackRequest) returns (SavedTrack);
  rpc RemoveSavedTrack(RemoveSavedTrackRequest)
      returns (google.protobuf.Empty);
  rpc ListSavedTracks(ListSavedTracksRequest)
      returns (ListSavedTracksResponse);
  rpc LikeTrack(LikeTrackRequest) returns (LikedTrack);
  rpc UnlikeTrack(UnlikeTrackRequest) returns (google.protobuf.Empty);
  rpc ListLikedTracks(ListLikedTracksRequest)
      returns (ListLikedTracksResponse);
}

service PlaylistService {
  rpc CreatePlaylist(CreatePlaylistRequest) returns (Playlist);
  rpc GetPlaylist(GetPlaylistRequest) returns (Playlist);
  rpc UpdatePlaylist(UpdatePlaylistRequest) returns (Playlist);
  rpc DeletePlaylist(DeletePlaylistRequest) returns (google.protobuf.Empty);
  rpc ListPlaylists(ListPlaylistsRequest) returns (ListPlaylistsResponse);
  rpc AddPlaylistTrack(AddPlaylistTrackRequest) returns (PlaylistTrack);
  rpc RemovePlaylistTrack(RemovePlaylistTrackRequest)
      returns (google.protobuf.Empty);
  rpc ReorderPlaylistTracks(ReorderPlaylistTracksRequest) returns (Playlist);
  rpc ListPlaylistTracks(ListPlaylistTracksRequest)
      returns (ListPlaylistTracksResponse);
}

service SystemService {
  rpc GetStatus(GetStatusRequest) returns (GetStatusResponse);
}
```

Import `google/protobuf/empty.proto`, `field_mask.proto`,
`struct.proto`, and `timestamp.proto`.

- [ ] **Step 4: Define common resources and pagination**

Use these exact shared shapes:

```proto
message PageRequest {
  uint32 page_size = 1;
  string page_token = 2;
}

message PageInfo {
  string next_page_token = 1;
}

message ArtworkRef {
  string id = 1;
}

message ArtistSummary {
  string id = 1;
  string name = 2;
}

message AlbumSummary {
  string id = 1;
  string title = 2;
}

message TrackSummary {
  string id = 1;
  string title = 2;
  ArtistSummary artist = 3;
  optional AlbumSummary album = 4;
  uint64 duration_ms = 5;
  bool explicit = 6;
  optional ArtworkRef artwork = 7;
}

message Track {
  TrackSummary summary = 1;
  repeated string genres = 2;
}
```

Remove `MediaItem.artwork_uri`, bitrate, MIME type, and the negative-duration
sentinel. Playback asset codec/content type remain in `PlaybackSource`.

- [ ] **Step 5: Remove local player/session wire operations**

Delete the prototype control and session RPC/messages. Define playback as:

```proto
message ResolvePlaybackRequest {
  string track_id = 1;
}

message PlaybackSource {
  string track_id = 1;
  string stream_url = 2;
  string content_type = 3;
  string codec = 4;
  uint64 duration_ms = 5;
  google.protobuf.Timestamp expires_at = 6;
}
```

- [ ] **Step 6: Include the versioned generated package**

Change the facade to:

```rust
tonic::include_proto!("canopy.v1");
```

- [ ] **Step 7: Verify generation reaches the intended red state**

Run: `cargo test -p canopy-server --test proto_contract --all-features`

Expected: bounded modules now generate; compilation fails only because Canopy
has not yet implemented their generated traits.

### Task 2: Introduce Shared Contract Conventions

**Files:**
- Modify: `crates/canopy-core/src/model.rs`
- Modify: `crates/canopy-core/src/error.rs`
- Modify: `crates/canopy-server/src/api/grpc/mod.rs`
- Test: `crates/canopy-server/tests/domain.rs`

- [ ] **Step 1: Add failing opaque-pagination tests**

```rust
#[test]
fn page_token_round_trips_offset_without_exposing_it() {
    let codec = PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap();
    let token = codec.encode(42).unwrap();

    assert!(!token.contains("42"));
    assert_eq!(codec.decode(&token).unwrap(), 42);
}

#[test]
fn page_token_rejects_tampering() {
    let codec = PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap();
    assert!(matches!(
        codec.decode("tampered"),
        Err(CanopyError::InvalidArgument(_))
    ));
}
```

- [ ] **Step 2: Run and verify red**

Run: `cargo test -p canopy-server --test domain page_token --all-features`

Expected: FAIL because `PageTokenCodec` does not exist.

- [ ] **Step 3: Implement signed opaque page tokens**

Create a focused codec using URL-safe Base64 plus HMAC-SHA256, versioned claims,
constant-time signature verification, and a domain-separated key derived from
the existing 32-byte stream capability secret. Invalid tokens map to `InvalidArgument`. Tokens carry no expiry; continuation validity is bounded by the underlying query semantics.

- [ ] **Step 4: Centralize transport error mapping**

Add these domain variants before writing the mapper:

```rust
FailedPrecondition(String),
Aborted(String),
```

Map domain errors exactly:

```rust
match error {
    CanopyError::InvalidArgument(message) => Status::invalid_argument(message),
    CanopyError::Unauthenticated(message) => Status::unauthenticated(message),
    CanopyError::NotFound { entity, id } => {
        Status::not_found(format!("{entity} not found: {id}"))
    }
    CanopyError::FailedPrecondition(message) => Status::failed_precondition(message),
    CanopyError::Aborted(message) => Status::aborted(message),
    CanopyError::Storage(message) => Status::unavailable(message),
    CanopyError::Internal(message) => Status::internal(message),
}
```

Do not add success/error fields to response messages.

- [ ] **Step 5: Run convention tests**

Run: `cargo test -p canopy-server --test domain --all-features`

Expected: PASS.

### Task 3: Split The gRPC Adapter By Service

**Files:**
- Create: `crates/canopy-server/src/api/grpc/mod.rs`
- Create: `crates/canopy-server/src/api/grpc/catalog.rs`
- Create: `crates/canopy-server/src/api/grpc/playback.rs`
- Create: `crates/canopy-server/src/api/grpc/discovery.rs`
- Create: `crates/canopy-server/src/api/grpc/profile.rs`
- Create: `crates/canopy-server/src/api/grpc/history.rs`
- Create: `crates/canopy-server/src/api/grpc/library.rs`
- Create: `crates/canopy-server/src/api/grpc/playlist.rs`
- Create: `crates/canopy-server/src/api/grpc/system.rs`
- Delete after parity: `crates/canopy-server/src/api/grpc.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Add service-construction compile tests**

Extend `proto_contract.rs` to construct each adapter from a shared
`Arc<GrpcServices>` and wrap it with its generated server type. Assert every
server implements `tower::Service<http::Request<tonic::body::Body>>`.

- [ ] **Step 2: Verify red**

Run: `cargo test -p canopy-server --test proto_contract --all-features`

Expected: FAIL because bounded adapter structs do not exist.

- [ ] **Step 3: Create shared adapter dependencies**

```rust
#[derive(Clone)]
pub struct GrpcServices {
    pub catalog: CatalogService,
    pub search: SearchService,
    pub resolver: ResolverService,
    pub discovery: DiscoveryService,
    pub profile: ProfileService,
    pub history: HistoryService,
    pub library: LibraryService,
    pub likes: LikeService,
    pub preferences: PreferencesService,
    pub playlists: PlaylistService,
    pub health: HealthService,
    pub auth: AuthService,
    pub principal: PrincipalService,
    pub page_tokens: Arc<PageTokenCodec>,
}
```

Each adapter owns only `Arc<GrpcServices>` and implements one generated trait.
Shared metadata-auth, conversion, pagination, and error helpers remain in
`grpc/mod.rs`.

- [ ] **Step 4: Implement catalog, playback, discovery, and system adapters**

Preserve current behavior while translating to audited messages:

- catalog methods return `TrackSummary`/`Track`;
- playback uses optional strict metadata auth and owner-first resolution;
- discovery returns a bounded page rather than one item;
- system status maps dependency details and timestamps.

- [ ] **Step 5: Implement profile, history, library, and playlist adapters**

All durable methods use metadata-only authentication. Prost maps
`google.protobuf.Empty` to `()`, so delete operations return
`tonic::Response<()>`. Preferences map
`prost_types::Struct` to canonical JSON only at the existing repository
boundary.

- [ ] **Step 6: Register every service on one listener**

In `run`, replace `CanopyServer::new(api)` with one `Server::builder()`
chain containing every generated server plus Tonic's standard health reporter.

- [ ] **Step 7: Verify adapter compilation**

Run: `cargo test -p canopy-server --test proto_contract --all-features`

Expected: PASS.

### Task 4: Complete Profile Lifecycle And History Consent

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/profile.rs`
- Modify: `crates/canopy-server/src/history.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Create: `migrations/20260702000001_audited_v1_contract.sql`
- Test: `crates/canopy-server/tests/domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Add failing lifecycle tests**

Cover:

```rust
#[tokio::test]
async fn profile_update_uses_explicit_field_mask();

#[tokio::test]
async fn disabling_history_returns_deleted_count_and_purges_atomically();

#[tokio::test]
async fn deleting_instance_owner_fails_precondition();

#[tokio::test]
async fn deleting_profile_that_owns_personal_media_fails_precondition();

#[tokio::test]
async fn deleting_ordinary_profile_cascades_durable_state();
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p canopy-server profile::tests history::tests --all-features`

Expected: FAIL because get/update/delete lifecycle methods and explicit history
settings responses do not exist.

- [ ] **Step 3: Extend profile and settings repository ports**

Add get/update/delete methods with explicit domain requests. Profile deletion
must execute transactionally and return `FailedPrecondition` when the profile
is the configured owner or owns tracks.

Use the `CanopyError::FailedPrecondition(String)` and
`CanopyError::Aborted(String)` variants introduced in Task 2.

- [ ] **Step 4: Add playlist revisioning and profile deletion constraints**

The migration adds:

```sql
ALTER TABLE playlists
    ADD COLUMN IF NOT EXISTS revision BIGINT NOT NULL DEFAULT 1;

DROP TABLE IF EXISTS playback_sessions;
```

Repository reorder uses:

```sql
UPDATE playlists
SET revision = revision + 1, updated_at = NOW()
WHERE id = $1
  AND profile_id = $2
  AND revision = $3
RETURNING revision
```

Zero rows maps to `CanopyError::Aborted("stale playlist revision".into())`.

- [ ] **Step 5: Implement lifecycle behavior in both adapters**

PostgreSQL profile deletion uses one transaction. In-memory behavior must match
the same observable preconditions and cascades.

- [ ] **Step 6: Run domain and PostgreSQL tests**

Run: `cargo test -p canopy-server --test domain --all-features`

Expected: PASS.

Run: `bash scripts/test-pg.sh`

Expected: PASS including profile preconditions, consent purge, revision
conflicts, and obsolete-session removal.

### Task 5: Remove Backend Player Session Ownership

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/playback.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Test: `crates/canopy-server/tests/domain.rs`

- [ ] **Step 1: Add a playback-resolution regression test**

```rust
#[tokio::test]
async fn resolving_playback_has_no_session_side_effect() {
    let source = resolver
        .resolve_at(&PlaybackPrincipal::Anonymous, "trk_1", 1_000)
        .await
        .unwrap();

    assert_eq!(source.track_id, "trk_1");
}
```

The test fixture must not construct any `SessionRepository`.

- [ ] **Step 2: Verify the old wiring is still required**

Run: `cargo test -p canopy-server playback::tests --all-features`

Expected: the new fixture fails to compile while resolver/session wiring is
still coupled.

- [ ] **Step 3: Remove obsolete session code**

Remove `SessionRepository`, `PlaybackService`, in-memory/PostgreSQL session
adapters, startup wiring, and session tests. Keep `ResolverService` as the
playback application service and remove `resolve_for_session`.

- [ ] **Step 4: Run playback and full domain tests**

Run: `cargo test -p canopy-server playback::tests --all-features`

Expected: PASS with no session repository fixture.

Run: `cargo test -p canopy-server --test domain --all-features`

Expected: PASS.

### Task 6: Make Durable Collections Resource-Oriented

**Files:**
- Modify: `crates/canopy-server/src/library.rs`
- Modify: `crates/canopy-server/src/likes.rs`
- Modify: `crates/canopy-server/src/playlists.rs`
- Modify: `crates/canopy-core/src/model.rs`
- Modify: `crates/canopy-core/src/repository.rs`
- Test: unit tests in each service
- Test: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Add failing idempotency/resource tests**

Assert repeated save/like returns the same relationship resource, absent
remove succeeds, playlist get returns revision, and stale reorder returns
`Aborted`.

- [ ] **Step 2: Verify red**

Run: `cargo test -p canopy-server library::tests likes::tests playlists::tests --all-features`

Expected: FAIL because mutation methods return booleans and playlists do not
carry revisions.

- [ ] **Step 3: Implement relationship resources**

Add `SavedTrack`, `LikedTrack`, and `PlaylistTrack` domain values with
protobuf timestamps at the adapter boundary. Preserve idempotent SQL upserts
and return existing rows rather than booleans.

- [ ] **Step 4: Implement revision-checked reordering**

Require the complete ordered track ID set and expected revision. Validate
duplicates before opening the transaction. Lock playlist membership, verify
the set, update positions, increment revision, and commit atomically.

- [ ] **Step 5: Run collection tests**

Run: `cargo test -p canopy-server library::tests likes::tests playlists::tests --all-features`

Expected: PASS.

Run: `bash scripts/test-pg.sh`

Expected: PASS.

### Task 7: Update Documentation And Run The Complete Gate

**Files:**
- Modify: `README.md`
- Modify: `docs/openapi.json`
- Modify: `docs/canopy-api-bsr-design.md`

- [ ] **Step 1: Update documentation**

Document bounded services, local player ownership, metadata-only auth, opaque
pagination, platform-neutral artwork, profile lifecycle, history consent,
playlist revisions, and that protobuf remains canonical.

OpenAPI keeps the real private HTTP authorization operation and marks every
gRPC projection as noncanonical.

- [ ] **Step 2: Validate Buf locally**

Run:

```bash
buf format --diff --exit-code
buf lint
buf build
```

Expected: all commands pass. Do not run `buf breaking` before the bootstrap
`v0.1.0` release exists.

- [ ] **Step 3: Run Rust quality gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

Expected: PASS with no warnings or failures.

- [ ] **Step 4: Run infrastructure integration gates**

Run:

```bash
bash scripts/test-pg.sh
bash scripts/test-streaming.sh
```

Expected: PostgreSQL and real Nginx streaming tests pass.

- [ ] **Step 5: Inspect the final working tree**

Run:

```bash
git diff --check
git diff --cached --name-only
git status --short
```

Expected: no whitespace errors, no staged files, and only intended audited-v1,
existing owner-playback, documentation, and test changes.

## Deferred Plans

After this phase passes:

1. Create and publish the private `buf.build/pandawave/canopy-api` module,
   verify generated SDK compatibility, and cut Canopy's facade over.
2. Update PandaEngine to the exact same generated SDK versions.
3. Implement recommendation ranking as an additive `canopy.v1` capability
   once its backend behavior is designed and tested.
