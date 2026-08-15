# Canopy Backend Architecture

> This document describes the Canopy Rust backend, its implementation boundaries, runtime configuration, deployment, and verification. Adjacent components appear only where they define a backend integration boundary.

## Status

This document describes the current Canopy architecture and identifies the remaining roadmap explicitly. The codebase is a Cargo workspace (`canopy-proto` / `canopy-core` / `canopy-server`) with domain-oriented services, repository ports, PostgreSQL adapters, managed local media, and a gRPC-first control plane.

| Area                      | Status         | Notes                                                                                       |
| ------------------------- | -------------- | ------------------------------------------------------------------------------------------- |
| Workspace / modularization | ✅ Implemented | Cargo workspace: `canopy-proto` (BSR SDK facade), `canopy-core` (domain model, `CanopyError`, repository ports), `canopy-server` (bounded gRPC adapters + `jade_store`). |
| gRPC server (`tonic`)     | ✅ Implemented | Nine bounded `canopy.v1` services share one listener and use the audited generated BSR contract. |
| Configuration             | ✅ Implemented | Env-driven configuration validates streaming, identity keys, TLS-only SMTP delivery, PostgreSQL, and managed media before listeners start. |
| API contract / BSR        | ✅ `v0.2.0` | Implements BSR commit `af019e2d7fa245a2a7d9fc21a4dd9afa`; Prost `=0.5.0-00000000000000-af019e2d7fa2.2`; Tonic `=0.5.0-00000000000000-af019e2d7fa2.4`. |
| Catalog service           | Partial | Public repository paths are explicit; PostgreSQL and in-memory adapters isolate `release_safe` media from owner-scoped personal media. |
| Player/session handling   | ✅ Engine-owned | Canopy owns playback-source authorization only. Play/pause/seek/speed/queue/session state belongs to PandaEngine and no backend session repository or control RPC remains. |
| Search (`pg_trgm`)        | 🟡 Prototype   | Dedicated `SearchService` over the `CatalogRepository` port: query normalization + page-size clamping. PostgreSQL mode uses trigram similarity over tracks, artists, and albums; in-memory mode keeps the lightweight demo matcher. |
| Discovery service         | ✅ Implemented | `GetDiscoveryFeed`, `GetForYouFeed`, and `GetRecommendations` serve the same release-safe discovery feed for now; personalized ranking remains future work. |
| Playback Resolver         | ✅ Implemented | `ResolvePlayback` is anonymous-compatible and auth-aware: the configured owner receives owner-scoped personal media first with public fallback; other callers receive release-safe public media. Capabilities remain opaque and storage keys never enter client responses. |
| Auth / Profiles           | ✅ Implemented | Native identity, SMTP challenge delivery, per-device sessions, Google linking, account lifecycle, and authorization for durable profile state are implemented. |
| Provider Adapters         | Implemented | Deterministic fixture ingestion remains idempotent and quarantined by default; external provider runtime code has been removed. |
| Persistence (PostgreSQL)  | Partial | Typed adapters cover ownership, media policy, local-import transactions, checksum deduplication, and fail-closed promotion. |
| Music storage             | ✅ Implemented | MP3 and artwork import into content-addressed local storage; bundled Nginx authorizes through Canopy and serves byte ranges from an internal read-only location. |
| Observability             | 🟡 Partial     | `tracing` initialized; no correlation-ID propagation or Prometheus metrics.                 |
| Health checks             | ✅ Implemented | Readiness checks PostgreSQL, the managed library directory, and authentication email delivery. SMTP outages affect readiness but not liveness or unrelated RPCs. |
| CI / Verification         | ✅ Implemented | Lockfile-enforced Rust, PostgreSQL, and streaming gates run in Canopy; `canopy-api` owns contract compatibility and publication gates. |

Legend: ✅ Implemented · 🟡 Partial / prototype · 🔴 Planned

### Local-media transition

Canopy uses a fully owned local-media architecture. PostgreSQL remains the metadata and policy authority; audio, artwork, retained originals, and quarantined files live under `/srv/canopy/media`; Nginx serves authorized files with HTTPS and byte-range support. Supabase is not part of the architecture. RustFS is reserved as an inactive, future-compatible storage option and is not part of active playback.

Phases 1 and 2 are implemented. Storage fields use `storage_key`; one existing profile can be assigned as the instance owner; catalog and asset repositories separate public and owner-scoped paths; and `canopy-admin` imports MP3 files and optional artwork into content-addressed managed storage. PostgreSQL records each import as personal and pending before file finalization, then exposes it to the matching owner only after the row becomes ready. Checksum uniqueness makes retries idempotent.

Public Nginx/X-Accel streaming and the public playback cutover are implemented. Supabase runtime/configuration and the obsolete direct-object-URL abstractions have been removed. Owner-first personal playback issuance is implemented. Reconciliation tooling and universal artwork fallback remain subsequent phases. RustFS stays outside the runtime as optional future-reserved Compose infrastructure.
#### Owner and local media administration

Apply the migration chain and create the profile before assigning it as the instance owner. The admin process is intentionally stricter than the server: both the database URL and managed media root are required, and it never falls back to in-memory storage.

```bash
export CANOPY_DATABASE_URL='postgres://canopy:canopy@localhost:5432/canopy'
export CANOPY_MEDIA_ROOT='/srv/canopy/media'

cargo run -p canopy-server --features pg --bin canopy-admin -- \
  owner set <external-user-id>

cargo run -p canopy-server --features pg --bin canopy-admin -- \
  media import /path/to/track-or-directory
```

`CANOPY_MAX_AUDIO_BYTES` and `CANOPY_MAX_ARTWORK_BYTES` optionally override the 2 GiB audio and 20 MiB artwork limits. Values must be positive integers.

The importer accepts one MP3 file or the top-level MP3 files in one directory; directory traversal is deliberately non-recursive and deterministic. Metadata comes from the MP3 tags with conservative fallbacks. Artwork selection is embedded cover first, then `cover.jpg`, then `cover.png`; missing artwork is valid. Source files are never modified or deleted, and absolute source paths are neither persisted nor returned in JSON output.

Each new import is staged under `staging/<track-id>`, inserted into PostgreSQL as `personal` + `pending` + `local_admin`, finalized into `library/audio/<sha-prefix>/<sha>.mp3` and optional `library/artwork/<sha-prefix>/<sha>.<ext>`, then marked `ready`. Pending imports remain invisible. A repeated audio checksum returns the existing track ID instead of creating a second track. Failures before persistence discard staging; failures after persistence retain recoverable pending state.

## Ecosystem Overview

```mermaid
flowchart TD
    PW[PandaWave AAOS Media App]
    PE[PandaEngine Rust Middleware]
    CAN[Canopy Rust Backend]
    DB[(PostgreSQL)]
    NGINX[Nginx protected streaming]
    MEDIA[(Canopy media volume)]

    PW -->|AIDL| PE
    PE -->|gRPC / tonic| CAN
    CAN --> DB
    CAN --> NGINX
    NGINX --> MEDIA
```

---

## Naming Hierarchy

| Domain            | Name        |
| ----------------- | ----------- |
| Product           | PandaWave   |
| Engine            | PandaEngine |
| Backend           | Canopy      |
| Design System     | BambooUI    |
| Future OS         | PandaOS     |
| Persistence Layer | JadeStore   |
| Cache Layer       | JadeCache   |
| Future Sync Layer | JadeSync    |

---

## Runtime Architecture

```mermaid
flowchart TB

    subgraph PandaWave
        UI[BambooUI]
        MP[BambooMiniPlayer]
        MS[MediaSession]
        EXO[ExoPlayer]
    end

    subgraph PandaEngine
        CMD[Command Dispatcher]
        STATE[State Machine]
        PLAY[Playback Coordinator]
        SEARCH[Search Coordinator]
        QUEUE[Queue Management]
        REPO[Repository Layer]
    end

    subgraph Canopy
        API[gRPC API]

        AUTH[Auth]
        CAT[Catalog]
        DISC[Discovery]
        SRCH[Search]
        RES[Playback Resolver]
        PROV[Provider Adapters]

        STORE[JadeStore]
        OBS[Observability]
        HEALTH[Health]
    end

    DB[(PostgreSQL)]
    NGINX[Nginx protected streaming]
    MEDIA[(Canopy media volume)]

    UI --> CMD

    CMD --> STATE
    CMD --> SEARCH
    CMD --> PLAY

    REPO --> API

    API --> AUTH
    API --> CAT
    API --> DISC
    API --> SRCH
    API --> RES

    CAT --> STORE
    DISC --> STORE
    SRCH --> STORE
    RES --> STORE

    PROV --> STORE

    STORE --> DB
    RES --> NGINX
```

---

## Canopy Backend Structure

Canopy is a Cargo **workspace**. The wire contract lives in its own crate so
PandaEngine can depend on it without pulling in the backend, the domain is kept
transport- and storage-agnostic in `canopy-core`, and the server crate holds the
adapters and wiring. Dependencies point inwards only: `canopy-server →
canopy-core` and `canopy-server → canopy-proto`.

```text
canopy/
+-- crates/
|   +-- canopy-proto/
|   |   +-- proto/canopy/v1/canopy.proto # temporary audited rollback copy
|   |   `-- src/lib.rs                    # immutable BSR SDK re-exports
|   +-- canopy-core/
|   |   +-- model.rs              # domain values and playback contracts
|   |   +-- error.rs              # CanopyError / CanopyResult
|   |   `-- repository.rs         # storage-independent domain ports
|   `-- canopy-server/
|       +-- api/grpc/             # bounded Tonic adapters and auth boundary
|       +-- admin.rs              # owner assignment and media import application
|       +-- auth.rs               # login-token verification
|       +-- catalog.rs
|       +-- search.rs
|       +-- discovery.rs
|       +-- playback.rs           # owner-aware playback-source resolution
|       +-- profile.rs
|       +-- history.rs
|       +-- library.rs
|       +-- likes.rs
|       +-- preferences.rs
|       +-- playlists.rs
|       +-- owner.rs
|       +-- media/                # MP3 inspection, staging, and managed storage
|       +-- stream/               # capability codec, authorization, private HTTP adapter
|       +-- providers/            # provider boundary and deterministic fixture adapter
|       +-- jade_store/           # in-memory and PostgreSQL repositories
|       +-- observability.rs
|       +-- health.rs
|       +-- config.rs
|       `-- bin/canopy-admin.rs
+-- migrations/
+-- deploy/nginx/
+-- scripts/
`-- docs/
```

> **Naming note.** The persistence layer is `jade_store` in code, matching the
> **JadeStore** entry in the Naming Hierarchy above, rather than a generic
> `storage`. Domain services depend on the repository **ports** in `canopy-core`,
> so the in-memory and PostgreSQL stores remain interchangeable behind the same
> repository interfaces. RustFS is reserved outside the active persistence path.

---

## Canopy Responsibilities

### gRPC API Layer

The gRPC API is Canopy's single control-plane contract with PandaEngine. It owns search, browse, discovery, session playback controls, playback resolution, metadata retrieval, and authenticated profile state. Audio bytes never travel over this channel — gRPC resolves *what* to play and *where* to get it; HTTP handles the actual streaming.

```proto
rpc Search(SearchRequest)
    returns (SearchResponse);

rpc Browse(BrowseRequest)
    returns (BrowseResponse);

rpc ResolvePlayback(PlaybackRequest)
    returns (PlaybackSource);

rpc GetDiscoveryFeed(GetDiscoveryFeedRequest)
    returns (GetDiscoveryFeedResponse);
rpc GetForYouFeed(GetForYouFeedRequest)
    returns (GetForYouFeedResponse);
rpc GetRecommendations(GetRecommendationsRequest)
    returns (GetRecommendationsResponse);

rpc UpsertProfile(UpsertProfileRequest)
    returns (UpsertProfileResponse);

rpc RecordPlaybackHistory(RecordPlaybackHistoryRequest)
    returns (RecordPlaybackHistoryResponse);

rpc ListPlaybackHistory(ListPlaybackHistoryRequest)
    returns (ListPlaybackHistoryResponse);

rpc DeletePlaybackHistoryEntry(DeletePlaybackHistoryEntryRequest)
    returns (DeletePlaybackHistoryEntryResponse);

rpc ClearPlaybackHistory(ClearPlaybackHistoryRequest)
    returns (ClearPlaybackHistoryResponse);

rpc SaveLibraryItem(SaveLibraryItemRequest)
    returns (SaveLibraryItemResponse);

rpc RemoveLibraryItem(RemoveLibraryItemRequest)
    returns (RemoveLibraryItemResponse);

rpc ListLibraryItems(ListLibraryItemsRequest)
    returns (ListLibraryItemsResponse);

rpc LikeTrack(LikeTrackRequest)
    returns (LikeTrackResponse);

rpc UnlikeTrack(UnlikeTrackRequest)
    returns (UnlikeTrackResponse);

rpc ListLikedTracks(ListLikedTracksRequest)
    returns (ListLikedTracksResponse);

rpc GetPreferences(GetPreferencesRequest)
    returns (GetPreferencesResponse);

rpc UpdatePreferences(UpdatePreferencesRequest)
    returns (UpdatePreferencesResponse);

rpc CreatePlaylist(CreatePlaylistRequest)
    returns (CreatePlaylistResponse);

rpc UpdatePlaylist(UpdatePlaylistRequest)
    returns (UpdatePlaylistResponse);

rpc DeletePlaylist(DeletePlaylistRequest)
    returns (DeletePlaylistResponse);

rpc ListPlaylists(ListPlaylistsRequest)
    returns (ListPlaylistsResponse);

rpc AddPlaylistTrack(AddPlaylistTrackRequest)
    returns (AddPlaylistTrackResponse);

rpc RemovePlaylistTrack(RemovePlaylistTrackRequest)
    returns (RemovePlaylistTrackResponse);

rpc ReorderPlaylistTracks(ReorderPlaylistTracksRequest)
    returns (ReorderPlaylistTracksResponse);

rpc ListPlaylistTracks(ListPlaylistTracksRequest)
    returns (ListPlaylistTracksResponse);
```

This proto is the single source of truth for the wire contract between PandaEngine and Canopy. It is defined once, in a shared `canopy_proto` crate, and consumed by both PandaEngine (as a client) and Canopy (as a server). Neither side maintains its own copy of these message shapes.

---

### Auth

Canopy is designed to let anonymous users browse, search, and play music without logging in. Anonymous `session_id` values are operational playback state only; they are not users and must not own durable backend history, libraries, likes, preferences, or playlists. The client is responsible for any anonymous local cache.

Native identity is now separate from profile/app state. `AuthService.RegisterPassword` creates a pending account and stores only hashed credentials and hashed challenge tokens. `VerifyEmail` atomically consumes the single-use email-verification challenge, activates the account, creates the profile row, and issues the first access/refresh token pair. `LoginPassword` issues later device sessions for active accounts, and `RefreshSession` transactionally rotates refresh tokens; reuse of a consumed refresh token revokes the session family.

Registration, verification resend, and password-reset requests transactionally enqueue authenticated-encrypted email payloads. A supervised PostgreSQL worker leases committed rows and delivers them through TLS-only SMTP with at-least-once semantics. Deterministic message IDs reduce duplicate presentation, challenges remain single-use, and successful delivery clears ciphertext. SMTP outages make email readiness unhealthy without affecting liveness or unrelated RPCs; the development-only undelivered-email escape hatch reports degraded readiness.

Identity access tokens are sent to implemented AuthService protected calls as `authorization: Bearer <access-token>`. `Logout`, `LogoutAll`, `ListSessions`, `RevokeSession`, `ChangePassword`, `LinkGoogle`, `UnlinkGoogle`, `GetAccount`, and `DeleteAccount` verify the Ed25519 access token and recheck the referenced session in PostgreSQL before reading or mutating account/session state. Revocation is idempotent, account-scoped, consumes outstanding refresh tokens, and is checked on refresh and protected AuthService calls. Google login verifies ID tokens when `CANOPY_GOOGLE_OIDC_CLIENT_IDS` is configured; otherwise it fails closed.

Durable profile/history/library/likes/preferences/playlist RPCs use native identity metadata as `authorization: Bearer <identity-access-token>` and recheck the session before resolving the account/profile scope. Anonymous browse/search/playback remain available, but durable user state requires a native account session. The profile includes `history_enabled`, so even logged-in playback history remains an explicit opt-in.

`RecordPlaybackHistory` records one append-only event only while `history_enabled=true`; disabled history returns `recorded=false`. Authenticated clients can list repeated events newest first, delete one event idempotently, or clear all history. Each listed event includes its ID, timestamp, listening facts, and renderable media metadata. Disabling history is destructive: PostgreSQL purges the profile's rows inside the profile-update transaction, and consent-safe recording prevents a concurrent request from repopulating them.

Saved library items, track likes, preferences, and playlists follow the same boundary: they require metadata auth, resolve the verified identity to a profile, and persist only under `profiles.id`. Anonymous clients may cache these locally, but Canopy does not store them until the user logs in and calls `UpsertProfile`.

Private playlists support metadata updates, deletion, pagination, idempotent track membership, explicit ordering, and ordered track listing. Reordering requires the complete current set of track IDs, and cross-profile access is reported as not found.

---

### Search Service

Search runs through the `CatalogRepository` port. Queries are normalized, clamped, and returned as a unary `SearchResponse`. PostgreSQL mode uses `pg_trgm` similarity over tracks, artists, and albums; in-memory mode keeps the lightweight demo matcher. There is no separate search engine or external index to operate.

Personalized suggestions are intentionally separate from text search. They can evolve once discovery and observed catalog/query patterns justify a dedicated ranking layer.

### Catalog Service

The catalog service owns artists, albums, and tracks and serves the hierarchical browsing experience used for discovery navigation. User-created playlists are a separate profile-owned domain and never attach to anonymous sessions.

---

### Discovery Service

Discovery serves batched feed endpoints for the shuffle channel: randomized playback with diversity filtering and exclusion of recently played tracks. Track selection is sourced from a pre-shuffled materialized view rather than an `ORDER BY random()` query against the live catalog table, so selection cost stays flat as the catalog grows. If the materialized view is empty, the PostgreSQL fallback applies the same public policy filters before reading the live catalog.

`GetDiscoveryFeed`, `GetForYouFeed`, and `GetRecommendations` currently delegate to the same discovery selector. A recommendation engine remains a future layer on top of these RPCs; the initial implementation is uniform random selection with the diversity and exclusion rules above.

---

### Playback Resolver

The playback resolver returns `{CANOPY_STREAM_PUBLIC_BASE_URL}/stream/{opaque-capability}` in `PlaybackSource`. `ResolvePlayback` accepts optional authentication metadata. With no credentials, or with a valid non-owner identity, it selects only `release_safe` + `ready` media. For the configured instance owner it first selects `personal` + `ready` media owned by that profile, then falls back to release-safe public media. Invalid supplied credentials return `Unauthenticated`; inaccessible personal media is concealed with the same `NotFound` result as missing media.

The signed capability contains an asset UUID, audience, expiry, version, and nonce, but no storage key. Public and personal audiences are minted according to the selected asset. Nginx delegates every request to Canopy's private authorizer, which rechecks current PostgreSQL policy before returning an internal media redirect. Personal authorization additionally requires the asset owner to remain the currently configured instance owner, so ownership changes revoke unexpired personal capabilities. Nginx then serves the bytes with native range support. External provider URL generation has been removed; RustFS is inactive Compose infrastructure reserved for possible future storage work.

### Playback Session Controls

`Play`, `Pause`, `Seek`, `SetPlaybackSpeed`, and `Stop` mutate lightweight session state through the `SessionRepository` port. Requests may provide a `session_id`; empty IDs resolve to the backward-compatible `default` session, and `PlayResponse` returns the session that was updated. `ResolvePlayback` also accepts `session_id` and loads the resolved track into that session after the stream URL is minted, which keeps anonymous playback state synchronized even when the client resolves URLs separately from `Play`.

---

### Provider Adapters

Provider adapters ingest catalog content from external sources — Musopen, Pixabay Music, Internet Archive, and future providers — and are responsible for metadata extraction, license verification, and ongoing catalog synchronization. Every track ingested through a provider adapter carries a license record; a track with no resolvable license is not added to the catalog.

The legacy ingestion boundary is `CatalogIngest`. Inserts and re-ingests are idempotent, but both explicitly set `visibility='quarantined'` and `ingest_status='quarantined'`. Imported content cannot reach anonymous browse, search, discovery, or playback without a separate review and promotion step.

The local importer validates and fingerprints real MP3 files, writes managed content-addressed library paths, and records `local_admin` provenance. It is a separate owner-only administrative path rather than an extension of provider ingest.

---

## Storage Architecture

```mermaid
flowchart LR
    IMPORT[Canopy import CLI]
    CAN[Canopy gRPC control plane]
    DB[(PostgreSQL)]
    NGINX[Nginx protected HTTPS]
    MEDIA[(Canopy media volume)]
    IMPORT --> MEDIA
    IMPORT --> DB
    CAN --> DB
    CAN --> NGINX
    NGINX --> MEDIA
```

PostgreSQL stores metadata, ownership, visibility, ingest state, provenance, and license review. Binary media is never stored in PostgreSQL. The managed filesystem layout is:

```text
/srv/canopy/media/
|-- staging/
|-- library/
|   |-- audio/
|   `-- artwork/
|-- originals/
`-- quarantine/
```

`audio_assets.storage_key` and artwork storage keys are validated relative paths under this managed root. Nginx, not the gRPC process, will serve files after Canopy authorizes a public or owner-scoped request.

The schema foundation, managed directory creation, import CLI, public playback cutover, and Nginx protected locations are implemented. Supabase runtime code and direct-object-URL compatibility code are removed. Owner-scoped playback delivery is implemented; reconciliation remains; RustFS is dormant Compose infrastructure and is not used by active playback.

### PostgreSQL

Core metadata and policy tables include `tracks`, `audio_assets`, `licenses`, `profiles`, and singleton `instance_settings` alongside durable profile-state tables. Tracks default to quarantined. Anonymous paths require `visibility='release_safe'` and `ingest_status='ready'`; personal paths require the matching `owner_profile_id`.

---

## Database Schema

```mermaid
erDiagram

    ARTISTS ||--o{ ALBUMS : owns

    ARTISTS ||--o{ TRACKS : performs

    ALBUMS ||--o{ TRACKS : contains

    TRACKS ||--o{ AUDIO_ASSETS : has

    LICENSES ||--o{ TRACKS : governs

    ARTISTS {
        uuid id
        string name
        string sort_name
    }

    ALBUMS {
        uuid id
        string title
        uuid artist_id
        int release_year
    }

    TRACKS {
        uuid id
        string title
        uuid artist_id
        uuid album_id
        int duration_ms
        uuid owner_profile_id
        uuid composition_license_id
        uuid recording_license_id
        string visibility
        string ingest_status
    }

    AUDIO_ASSETS {
        uuid id
        uuid track_id
        string codec
        string content_type
        string storage_key
        bigint size_bytes
        string checksum_sha256
    }

    LICENSES {
        uuid id
        string license_type
        string source_url
        string attribution_text
        string review_status
        timestamp reviewed_at
    }
```

---

## Playback Flow

```mermaid
sequenceDiagram
    participant User
    participant PandaWave
    participant PandaEngine
    participant Canopy
    participant PostgreSQL
    participant Nginx

    User->>PandaWave: Play track
    PandaWave->>PandaEngine: play(trackId)
    PandaEngine->>Canopy: ResolvePlayback(trackId)
    Canopy->>PostgreSQL: Authorize scope and select asset
    PostgreSQL-->>Canopy: Track and storage key
    Canopy-->>PandaEngine: PlaybackSource with short-lived HTTPS route
    PandaEngine-->>PandaWave: PlaybackSource
    PandaWave->>Nginx: HTTP GET with Range
    Nginx-->>PandaWave: 206 Partial Content
    PandaWave-->>User: Playback
```

Canopy remains the policy authority while Nginx handles byte delivery. Every public stream request passes through Nginx `auth_request`; Canopy validates the opaque capability and current PostgreSQL policy before returning an internal X-Accel redirect. Nginx then serves the managed file with native byte-range support.

---

## Streaming Architecture

```mermaid
sequenceDiagram
    participant ExoPlayer
    participant Canopy
    participant Nginx
    participant MediaVolume

    ExoPlayer->>Canopy: Request playback route
    Canopy-->>ExoPlayer: Short-lived authorized HTTPS URL
    ExoPlayer->>Nginx: GET with Range header
    Nginx->>MediaVolume: Read authorized storage key
    MediaVolume-->>Nginx: File bytes
    Nginx-->>ExoPlayer: 206 Partial Content
```

---

## PlaybackSource Contract

```proto
message PlaybackSource {
    string track_id = 1;

    string stream_url = 2;

    string content_type = 3;

    string codec = 4;

    uint64 duration_ms = 5;

    uint64 expires_at_epoch_ms = 6;
}
```

Example:

```json
{
  "track_id": "trk_123",
  "stream_url": "https://media.example.com/stream/eyJ2IjoxLCJhaWQiOiIuLi4ifQ.signature",
  "content_type": "audio/mpeg",
  "codec": "mp3",
  "duration_ms": 245000,
  "expires_at_epoch_ms": 1750200000000
}
```

---

## Discovery Channel Flow

```mermaid
sequenceDiagram
    participant PandaWave
    participant PandaEngine
    participant Canopy
    participant PostgreSQL
    participant Nginx

    PandaWave->>PandaEngine: GetDiscoveryFeed() / GetForYouFeed() / GetRecommendations()
    PandaEngine->>Canopy: DiscoveryService feed request
    Canopy->>PostgreSQL: Read release-safe discovery pool
    PostgreSQL-->>Canopy: Selected track
    Canopy-->>PandaEngine: Feed response
    PandaEngine->>Canopy: ResolvePlayback()
    Canopy-->>PandaEngine: PlaybackSource
    PandaEngine-->>PandaWave: PlaybackSource
    PandaWave->>Nginx: Stream authorized audio
    Nginx-->>PandaWave: Audio stream
```

---

## Audio Asset Strategy

```text
audio_assets
├── track_id
├── codec
├── storage_key
├── content_type
├── size_bytes
└── checksum_sha256
```

Each track has one audio asset per codec it's available in. A track is never duplicated across storage keys for the same codec.

```text
Track:
    Beethoven Symphony No. 5

Audio Assets:

    MP3
    storage_key=library/audio/trk_123.mp3

Future:

    Opus
    storage_key=library/audio/trk_123.opus

    FLAC
    storage_key=library/audio/trk_123.flac
```

---

## Observability

Canopy initializes structured `tracing` instrumentation for server lifecycle and application events. End-to-end correlation-ID propagation from PandaEngine, OpenTelemetry export, and Prometheus request/dependency metrics remain roadmap work; current logs must not be described as a complete distributed trace.

---

## Health Checks

Canopy's health endpoint reports dependency readiness, not only process liveness. It checks PostgreSQL connectivity and verifies that the managed `library/` path exists, is a directory, and can be opened. Nginx exposes a separate token-free `/nginx-health` endpoint for container orchestration.

---

## CI / Verification

The CI pipeline is implemented via **GitHub Actions** (`.github/workflows/ci.yml`). Every change to `master` and every pull request is gated by:

```text
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-features --tests --locked -- -D warnings
cargo fmt --all -- --check
bash scripts/test-pg.sh         # disposable PostgreSQL policy and migration suite
bash scripts/test-streaming.sh  # real Nginx ranges, denial, and revocation
cargo build --workspace --release --locked
```

The workflow installs stable Rust components and uses `Swatinem/rust-cache`. Authenticated Buf Cargo registry access resolves the exact generated SDK pins, while `Cargo.lock` prevents dependency drift.

The PostgreSQL harness starts an isolated PostgreSQL 18.4 Compose project, applies the migration chain, runs feature tests serially, and destroys the stack. `scripts/test-streaming.sh` adds a real Nginx container and synthetic MP3, verifies the served HTTP OpenAPI document, `206` range responses, denial behavior, and immediate policy revocation. Canopy uses lockfile-enforced Cargo commands; the canonical `canopy-api` repository owns Buf format, lint, compatibility, and publication gates. See [Canopy API Consumption](docs/canopy-api-consumption.md) for backend dependency pins and [Client Integration Handoff](docs/client-integration.md) for deployment-provided client connection values.

---

## Local Development

Canopy ships with a full `docker-compose.yml` stack for local development and integration testing. The stack includes:

| Service | Image | Role | Port |
| ------- | ----- | ---- | ---- |
| PostgreSQL | `postgres:18.4-alpine` | Metadata persistence (catalog, sessions, playback history) | 5432 |
| Redis | `redis:8` | Reserved cache infrastructure for the future JadeCache adapter; not yet wired into Canopy | 6379 |
| RustFS | `rustfs/rustfs:latest` | Optional inactive S3-compatible storage reserved for future experiments; not used by active playback | 9000 |
| Adminer | `adminer` | Database management UI (dev-only, opt-in) | 8080 |

### Quick Start

```bash
# 1. Copy the environment template and adjust as needed
cp .env.example .env

# 2. Start the full stack
docker compose up -d

# 3. Apply database migrations (install sqlx-cli once)
cargo install sqlx-cli --no-default-features --features native-tls,postgres
sqlx migrate run

# 4. Run the server with PostgreSQL persistence
cargo run --bin canopy --features canopy-server/pg
```

To start only the active database and cache services (without inactive RustFS or Adminer):

```bash
docker compose up -d postgres redis
```

To start Adminer (optional, for browsing the database via web UI):

```bash
docker compose --profile adminer up -d
# Visit http://localhost:8080
```

### Complete Local Integration Environment

The backend-owned local reference runs Canopy as a native WSL process and
PostgreSQL, Mailpit, and Nginx in the scoped `canopy-local-integration`
Compose project. It requires Docker Compose v2 with Linux host networking,
OpenSSL, the repository Rust toolchain, `curl`, `sqlx-cli`, and standard WSL
utilities (`ss`, `awk`, `readlink`, and `nohup`).

Start and inspect an interactive environment, then remove it with:

```bash
./scripts/local-integration.sh up
./scripts/local-integration.sh status
./scripts/local-integration.sh down
```

The developer endpoints are gRPC at `http://127.0.0.1:50051`, streaming at
`http://127.0.0.1:8080`, OpenAPI at
`http://127.0.0.1:8080/openapi.json`, and the operator-only Mailpit inbox at
`http://127.0.0.1:8025`.

Run the clean end-to-end authentication and playback smoke flow with:

```bash
./scripts/local-integration.sh test
```

The script generates ephemeral credentials and certificates under the ignored
target/local-integration/` directory. The environment is loopback-only and
disposable; it is not a production deployment. Mailpit, PostgreSQL, SMTP, and
the private stream authorization listener are never client handoff surfaces.

### Service Configuration

All services are configurable via environment variables. A `.env.example` is included in the repo; copy it to `.env` and override values as needed.

| Variable | Default | Description |
| ---------- | ------- | ----------- |
| `CANOPY_POSTGRES_USER` | `canopy` | PostgreSQL username |
| `CANOPY_POSTGRES_PASSWORD` | `canopy` | PostgreSQL password |
| `CANOPY_POSTGRES_DB` | `canopy` | PostgreSQL database name |
| `CANOPY_POSTGRES_PORT` | `5432` | PostgreSQL host port |
| `CANOPY_REDIS_PORT` | `6379` | Redis host port |
| `CANOPY_AUTH_TOKEN_SECRET` | `canopy-auth-secret` | Shared secret used to verify logged-in profile tokens |
| `CANOPY_RUSTFS_BUCKET` | `pandawave-media` | Reserved RustFS bucket name for future optional storage work; inactive today |
| `CANOPY_RUSTFS_ACCESS_KEY` | `canopy` | Reserved RustFS access key for inactive local experiments |
| `CANOPY_RUSTFS_SECRET_KEY` | `canopy-secret` | Reserved RustFS secret key for inactive local experiments |
| `CANOPY_RUSTFS_PORT` | `9000` | RustFS host port when the optional inactive service is started |
| `CANOPY_ADMINER_PORT` | `8080` | Adminer host port |
| `CANOPY_GRPC_ADDR` | `[::1]:50051` | gRPC server bind address |
| `CANOPY_DATABASE_URL` | `postgres://canopy:canopy@localhost:5432/canopy` | PostgreSQL connection string; required explicitly by `canopy-admin` |
| `CANOPY_MEDIA_ROOT` | unset | Managed media root; required by the server and `canopy-admin` |
| `CANOPY_STREAM_PUBLIC_BASE_URL` | unset | Required public Nginx base URL; HTTPS except loopback development |
| `CANOPY_STREAM_TOKEN_SECRET` | unset | Required HMAC capability secret of at least 32 bytes |
| `CANOPY_STREAM_TOKEN_TTL_SECS` | `600` | Positive public capability lifetime in seconds |
| `CANOPY_STREAM_AUTH_ADDR` | `127.0.0.1:8081` | Private HTTP listener used only by Nginx `auth_request` |
| `CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64` | unset | Required 32-byte Ed25519 signing key seed for native identity access tokens in PostgreSQL mode |
| `CANOPY_AUTH_OUTBOX_SEALING_KEY` | unset | Required with SMTP: base64-encoded 32-byte AES-GCM key for persisted authentication email payloads |
| `CANOPY_SMTP_HOST` | unset | SMTP relay host; required unless the development-only undelivered-email escape hatch is enabled |
| `CANOPY_SMTP_PORT` | `465` or `587` | Relay port; defaults to 465 for implicit TLS and 587 for STARTTLS |
| `CANOPY_SMTP_TLS_MODE` | `implicit` | `implicit` or required `starttls`; plaintext SMTP is rejected |
| `CANOPY_SMTP_CA_CERT_PATH` | unset | Optional readable PEM CA bundle added to verified SMTP TLS trust; it does not disable hostname or certificate validation |
| `CANOPY_SMTP_USERNAME` | unset | Required SMTP authentication username |
| `CANOPY_SMTP_PASSWORD` | unset | Required SMTP authentication password |
| `CANOPY_SMTP_FROM_ADDRESS` | unset | Required validated sender address |
| `CANOPY_SMTP_FROM_NAME` | unset | Required sender display name |
| `CANOPY_AUTH_PUBLIC_BASE_URL` | unset | Required base URL for verification/reset links; HTTPS except loopback development |
| `CANOPY_SMTP_TIMEOUT_SECS` | `30` | Positive SMTP connection/send timeout |
| `CANOPY_AUTH_EMAIL_POLL_INTERVAL_SECS` | `2` | Positive worker poll interval |
| `CANOPY_AUTH_EMAIL_LEASE_SECS` | `60` | Positive row lease; must be at least the SMTP timeout |
| `CANOPY_AUTH_EMAIL_BATCH_SIZE` | `20` | Maximum rows claimed per pass |
| `CANOPY_AUTH_EMAIL_MAX_ATTEMPTS` | `8` | Maximum delivery attempts before terminal failure |
| `CANOPY_AUTH_EMAIL_INITIAL_RETRY_SECS` | `5` | Initial deterministic-jitter retry delay |
| `CANOPY_AUTH_EMAIL_MAX_RETRY_SECS` | `900` | Retry delay cap; must be at least the initial delay |
| `CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL` | `false` | Development-only escape hatch that leaves messages queued and reports degraded readiness |
| `CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY` | `false` | Explicit local-development escape hatch for generated native identity access-token keys |
| `CANOPY_IDENTITY_ACCESS_TOKEN_ISSUER` | `canopy` | Issuer claim for native identity access tokens |
| `CANOPY_IDENTITY_ACCESS_TOKEN_AUDIENCE` | `pandawave` | Audience claim for native identity access tokens |
| `CANOPY_IDENTITY_ACCESS_TOKEN_KEY_ID` | `identity-access-v1` | Key identifier embedded in native identity access tokens |
| `CANOPY_IDENTITY_ACCESS_TOKEN_TTL_SECS` | `900` | Positive native identity access-token lifetime in seconds |
| `CANOPY_GOOGLE_OIDC_CLIENT_IDS` | unset | Comma-separated Google OAuth client IDs accepted in ID-token `aud` claims; enables Google login/linking when set |
| `CANOPY_GOOGLE_OIDC_TOKENINFO_URL` | `https://oauth2.googleapis.com/tokeninfo` | HTTPS Google tokeninfo endpoint used to validate ID tokens |
| `CANOPY_MAX_AUDIO_BYTES` | `2147483648` | Maximum MP3 import size for `canopy-admin` |
| `CANOPY_MAX_ARTWORK_BYTES` | `20971520` | Maximum embedded or sidecar artwork size for `canopy-admin` |
| `CANOPY_PROVIDER_FIXTURE_PATH` | unset | Optional provider fixture JSON to ingest at startup when running with `canopy-server/pg` |
| `CANOPY_REDIS_URL` | `redis://localhost:6379` | Reserved JadeCache connection setting; Redis is not yet wired into the server |

### sqlx Compile-Time Checks

The PostgreSQL repositories currently use runtime-checked `sqlx::query` calls so normal builds do not require a live database. Once the schema stabilizes further, the hot queries can move to `sqlx::query!` / `query_as!` with offline metadata. To prepare that mode:

```bash
# Ensure the database is running and migrations are applied
docker compose up -d postgres
sqlx migrate run

# Generate offline query data
cargo sqlx prepare --workspace

# Check the generated .sqlx/ files into git
git add .sqlx
```

A future compile-time-query pass can check generated `.sqlx/` metadata into the repository and enable `SQLX_OFFLINE=true`. Current CI instead exercises the complete migration chain and PostgreSQL-backed behavior through the disposable `scripts/test-pg.sh` harness.

### Running the Server

```bash
mkdir -p /tmp/canopy-media/library
export CANOPY_MEDIA_ROOT=/tmp/canopy-media
export CANOPY_STREAM_PUBLIC_BASE_URL=http://127.0.0.1:8080
export CANOPY_STREAM_TOKEN_SECRET=0123456789abcdef0123456789abcdef
export CANOPY_STREAM_AUTH_ADDR=127.0.0.1:8081

# In-memory development mode.
cargo run -p canopy-server --bin canopy

# Production-style PostgreSQL mode. Connection failure is fatal.
# Use CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64 for real deployments.
export CANOPY_DATABASE_URL=postgres://canopy:canopy@localhost:5432/canopy
export CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY=true
# Development only: keep auth messages queued without SMTP.
export CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true
cargo run -p canopy-server --features pg --bin canopy
```

The checked-in local client reference uses
`CANOPY_GRPC_ADDR=127.0.0.1:50051` and a separately served Nginx origin at
`http://127.0.0.1:8080`. See the
[Client Integration Handoff](docs/client-integration.md) before sharing an
environment with a client team.

When the `pg` feature is enabled, PostgreSQL is mandatory. Startup fails before either listener serves traffic if the database connection, stream configuration, media root, or private listener bind is invalid. Non-PG builds retain isolated in-memory stores for tests and local experimentation only.

If `CANOPY_PROVIDER_FIXTURE_PATH` is set in PostgreSQL mode, Canopy reads the fixture through `TestFixtureProvider` and ingests it with `CatalogIngest` before starting the gRPC server. The operation is idempotent by `provider_tracks(provider, provider_track_id)`, so the same fixture can be replayed during local development.

The `HealthService` returns `healthy`, `version`, aggregate `status`, and per-dependency details. Only PostgreSQL and the managed media library participate in Canopy readiness.

### PostgreSQL Integration Tests

The `canopy-server/pg` feature includes database-backed integration coverage for the complete migration chain, provider ingest, catalog reads, profile state, consent-safe history, and profile-owned playlists. The recommended harness creates an isolated PostgreSQL database on `127.0.0.1:55432`, runs the complete suite, and removes the container and network even when tests fail.

```bash
# Recommended: start, test, and destroy isolated PostgreSQL automatically
bash scripts/test-pg.sh

# Advanced: use an explicitly managed test database
CANOPY_TEST_DATABASE_URL=postgres://user:password@localhost:5432/canopy_test \
  bash scripts/test-pg.sh
```

Direct `cargo test --workspace --features canopy-server/pg` runs require `CANOPY_TEST_DATABASE_URL`. PostgreSQL tests never fall back to `DATABASE_URL` and intentionally fail when the test database is missing or unreachable. Default `cargo test --workspace` runs remain database-free.


### Streaming Integration Tests

The streaming harness creates disposable PostgreSQL 18.4 and Nginx containers on localhost, mounts a temporary read-only media library, starts the real Canopy authorization adapter in the test process, and removes all containers and files on exit.

```bash
bash scripts/test-streaming.sh
```

The bundled Nginx configuration intentionally contains no TLS directives. Deploy it behind the external HTTPS reverse proxy, keep `CANOPY_STREAM_AUTH_ADDR` private, and never expose `/_canopy_auth` or `/_canopy_media/`; both locations are marked `internal`.

### Supabase Removed

Supabase runtime modules, startup synchronization, configuration parsing, and tests have been removed. Current imports go through `canopy-admin`, metadata lives in PostgreSQL, and playable MP3/artwork files live under the managed media root served by Nginx after Canopy authorization.

### RustFS Reserved

RustFS remains in the local Compose stack only as inactive, future-reserved S3-compatible infrastructure. Canopy has no RustFS runtime configuration, readiness probe, or playback provider, and imported media should be placed through `canopy-admin` into the managed local library. If RustFS becomes useful later, it should sit behind the same Canopy policy and Nginx authorization boundary instead of becoming a direct client-facing source.

### Redis (JadeCache)

Redis is started by default but is not yet wired into the server. The `JadeCache` layer will use it for:
- Cached `Browse` responses (keyed by path + pagination params)
- Pre-shuffled discovery pool snapshots
- Capability rate-limit counters and abuse controls

Redis configuration is defined in `docker-compose.yml` with `allkeys-lru` eviction and AOF persistence. `CANOPY_REDIS_URL` reserves the connection setting, but the current server does not connect to Redis until the JadeCache adapter is implemented.

---

## Recommended Technology Stack

```text
Frontend
├── Android Automotive OS
├── Media3
├── ExoPlayer
└── BambooUI

Middleware
├── Rust
├── Tokio
└── tonic

Backend
├── Rust
├── tonic
├── axum
├── sqlx
└── tracing

Storage
- PostgreSQL
- Managed local media volume + Nginx
- RustFS (inactive, future-reserved)
- Redis (JadeCache - planned)

Observability
├── OpenTelemetry
├── tracing
└── Prometheus (future)

Infrastructure
├── Docker
├── Docker Compose
│   ├── postgres:18.4-trixie
│   ├── redis:8
│   ├── rustfs/rustfs:latest
│   └── adminer (dev profile)
└── GitHub Actions
```
