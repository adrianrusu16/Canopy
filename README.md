# PandaWave Architecture

> This document describes the architecture of the **PandaWave** ecosystem as a whole. It lives in the **Canopy** repository — the Rust backend — because Canopy is the central control-plane component. Sections below cover the full system; the **Canopy Backend Structure** and **Canopy Responsibilities** sections are specific to this repo.

## Status

This document is the **target architecture**. Most of it is not yet implemented — the current codebase is an early prototype. The table below tracks reality so the design and the code stay honest with each other. The code is organized as a Cargo workspace (`canopy-proto` / `canopy-core` / `canopy-server`) with domain-oriented modules and repository ports, so the structure already matches the design even where the behavior is still a stub.

| Area                      | Status         | Notes                                                                                       |
| ------------------------- | -------------- | ------------------------------------------------------------------------------------------- |
| Workspace / modularization | ✅ Implemented | Cargo workspace: `canopy-proto` (wire contract), `canopy-core` (domain model, `CanopyError`, repository ports), `canopy-server` (domain services + `api::grpc` adapter + `jade_store`). |
| gRPC server (`tonic`)     | 🟡 Prototype   | Catalog, session playback controls, playback resolution, discovery, and authenticated profile-state RPCs including private playlists are wired through the shared proto contract. `Search` and `Browse` still use the unary demo response shape (not streaming `SearchResult` yet). |
| Configuration             | ✅ Implemented | Env-driven `Config` (`CANOPY_GRPC_ADDR`, `CANOPY_DATABASE_URL`) with sensible defaults. |
| Catalog service           | 🟡 Prototype   | `CatalogService` over the `CatalogRepository` port; in-memory and PostgreSQL `jade_store` implementations are available, with browse/get/search backed by PostgreSQL when the `pg` feature is enabled. |
| Session handling          | 🟡 Prototype   | `PlaybackService` over the `SessionRepository` port; `play`/`pause`/`seek`/`stop`/speed RPCs now mutate persisted session state. Queue semantics and multi-device conflict handling are still planned. |
| Search (`pg_trgm`)        | 🟡 Prototype   | Dedicated `SearchService` over the `CatalogRepository` port: query normalization + page-size clamping. PostgreSQL mode uses trigram similarity over tracks, artists, and albums; in-memory mode keeps the lightweight demo matcher. |
| Discovery service         | 🟡 Prototype   | `DiscoveryService` over the `DiscoveryRepository` port: recently-played exclusion, artist-diversity reordering, limit clamping, and `DiscoveryNext` gRPC RPC. PostgreSQL mode reads `mv_discovery_pool`, a pre-shuffled materialized view with one representative asset per track. |
| Playback Resolver         | 🟡 Prototype   | `ResolverService` over `AudioAssetRepository` plus pluggable URL providers: codec-preference asset selection, TTL expiry, RustFS HMAC URLs, or Supabase Storage signed URLs. `ResolvePlayback` also synchronizes the anonymous session when called. |
| Auth / Profiles           | 🟡 Partial     | Browse/search/playback remain anonymous-compatible. Durable state is profile-owned; history supports chronological reads and deletion, and disabling consent atomically purges it. Anonymous users receive no backend history, library, likes, preferences, or playlists. |
| Provider Adapters         | 🟡 Partial     | Provider-facing ports and fixture adapter exist; PostgreSQL `CatalogIngest` transactionally upserts provider tracks, and `CANOPY_PROVIDER_FIXTURE_PATH` can ingest a local fixture at startup in PostgreSQL mode. Musopen/Pixabay/Internet Archive adapters are still planned. |
| Persistence (PostgreSQL)  | 🟡 Partial     | `sqlx` migrations, Docker Compose, typed repository ports, catalog/session/profile-state adapters, profile-owned ordered playlists, and transactional provider ingest are implemented. PostgreSQL mode auto-detects the DB under the `pg` feature and falls back to in-memory stores on connection failure. |
| Music storage             | 🟡 Partial     | RustFS HMAC URL generation and Supabase Storage signed URL fetching are available. Canopy stays out of the byte-serving path after `ResolvePlayback`. |
| Observability             | 🟡 Partial     | `tracing` initialized; no correlation-ID propagation or Prometheus metrics.                 |
| Health checks             | 🟡 Partial     | `HealthService` reports liveness, version, aggregate status, dependency details, PostgreSQL connectivity, and optional RustFS TCP reachability via `CANOPY_HEALTH_CHECK_RUSTFS=true`. |
| CI / Verification         | ✅ Implemented | GitHub Actions gates `master` with fmt, all-feature Clippy, default tests, a fail-closed disposable PostgreSQL integration harness, and a release build. Proto compatibility gates are still planned. |

Legend: ✅ Implemented · 🟡 Partial / prototype · 🔴 Planned

## Ecosystem Overview

```mermaid
flowchart TD

    PW[PandaWave<br/>AAOS Media App]

    PE[PandaEngine<br/>Rust Middleware]

    CAN[Canopy<br/>Rust Backend]

    DB[(PostgreSQL)]
    FS[(RustFS)]

    PW -->|AIDL| PE
    PE -->|gRPC / tonic| CAN

    CAN --> DB
    CAN --> FS
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
    FS[(RustFS)]

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
    RES --> FS
```

---

## Canopy Backend Structure

Canopy is a Cargo **workspace**. The wire contract lives in its own crate so
PandaEngine can depend on it without pulling in the backend, the domain is kept
transport- and storage-agnostic in `canopy-core`, and the server crate holds the
adapters and wiring. Dependencies point inwards only: `canopy-server →
canopy-core` and `canopy-server → canopy-proto`.

```text
canopy/                         # workspace root
├── crates/
│   ├── canopy-proto/           # generated gRPC types (single source of truth)
│   │   ├── proto/canopy.proto
│   │   └── build.rs
│   │
│   ├── canopy-core/            # transport-agnostic domain
│   │   ├── model.rs            # MediaItem, AudioAsset, PlaybackSource, Session, Page, ...
│   │   ├── error.rs            # CanopyError / CanopyResult
│   │   ├── repository.rs       # Catalog / Discovery / AudioAsset / Session ports
│   │   └── signing.rs          # UrlSigner port (presigned-URL signing)
│   │
│   └── canopy-server/          # adapters + wiring (lib + `canopy` binary)
│       ├── api/                # driving adapters
│       │   ├── grpc.rs         # gRPC ⇄ domain, CanopyError → Status
│       │   └── http.rs         # (planned)
│       │
│       ├── auth/               # (planned)
│       ├── catalog/            # CatalogService
│       ├── search/             # SearchService (normalization; pg_trgm planned)
│       ├── discovery/          # DiscoveryService (diversity/exclusion; pre-shuffled view planned)
│       ├── playback/           # PlaybackService (sessions) + ResolverService (presigned URLs)
│       ├── signing/            # HmacUrlSigner (UrlSigner impl)
│       ├── providers/          # provider adapter contracts + fixture ingestion
│       ├── jade_store/         # persistence layer (in-memory + PostgreSQL adapters)
│       ├── observability/      # tracing init (metrics planned)
│       ├── health/             # HealthService
│       └── config.rs
```

> **Naming note.** The persistence layer is `jade_store` in code, matching the
> **JadeStore** entry in the Naming Hierarchy above, rather than a generic
> `storage`. Domain services depend on the repository **ports** in `canopy-core`,
> so the in-memory store and the future PostgreSQL / RustFS backends are
> interchangeable behind the same interfaces.

---

## Canopy Responsibilities

### gRPC API Layer

The gRPC API is Canopy's single control-plane contract with PandaEngine. It owns search, browse, discovery, session playback controls, playback resolution, metadata retrieval, and authenticated profile state. Audio bytes never travel over this channel — gRPC resolves *what* to play and *where* to get it; HTTP handles the actual streaming.

```proto
rpc Search(SearchRequest)
    returns (stream SearchResult);

rpc Browse(BrowseRequest)
    returns (BrowseResponse);

rpc ResolvePlayback(PlaybackRequest)
    returns (PlaybackSource);

rpc DiscoveryNext(DiscoveryRequest)
    returns (DiscoveryTrack);

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

Durable user state starts at `UpsertProfile`. Authenticated profile-scoped RPCs send the end-user token in gRPC metadata as `authorization: Bearer <token>`; `x-canopy-auth-token` is accepted for clients that cannot set authorization metadata. Legacy request-body `auth_token` fields remain only on older profile/history RPCs as a compatibility fallback. New durable-state RPCs, including playlists, accept metadata auth only. Canopy verifies the token with `AuthService`, and the resulting external user identity creates or updates a `profiles` row. The profile includes `history_enabled`, so even logged-in playback history remains an explicit opt-in.

`RecordPlaybackHistory` records one append-only event only while `history_enabled=true`; disabled history returns `recorded=false`. Authenticated clients can list repeated events newest first, delete one event idempotently, or clear all history. Each listed event includes its ID, timestamp, listening facts, and renderable media metadata. Disabling history is destructive: PostgreSQL purges the profile's rows inside the profile-update transaction, and consent-safe recording prevents a concurrent request from repopulating them.

Saved library items, track likes, preferences, and playlists follow the same boundary: they require metadata auth, resolve the verified identity to a profile, and persist only under `profiles.id`. Anonymous clients may cache these locally, but Canopy does not store them until the user logs in and calls `UpsertProfile`.

Private playlists support metadata updates, deletion, pagination, idempotent track membership, explicit ordering, and ordered track listing. Reordering requires the complete current set of track IDs, and cross-profile access is reported as not found.

---

### Search Service

Search runs on PostgreSQL full-text search with `pg_trgm` trigram similarity. Queries are normalized, matched via `ILIKE` against trigram-indexed columns, and results are paginated and streamed back to the client as they're found. This is the entire search implementation — there is no separate search engine or index to operate.

Ranking and personalized suggestions are intentionally out of scope for the initial implementation. They are addressed only once trigram search has been observed to be insufficient for the catalog's actual size and query patterns, at which point a dedicated search engine becomes a deliberate, evidence-driven addition rather than a default.

---

### Catalog Service

The catalog service owns artists, albums, and tracks and serves the hierarchical browsing experience used for discovery navigation. User-created playlists are a separate profile-owned domain and never attach to anonymous sessions.

---

### Discovery Service

Discovery serves the shuffle channel: randomized playback with diversity filtering and exclusion of recently played tracks. Track selection is sourced from a pre-shuffled materialized view rather than an `ORDER BY random()` query against the live catalog table, so selection cost stays flat as the catalog grows. The materialized view is refreshed on a schedule independent of request traffic.

A recommendation engine is a future layer on top of this service; the initial implementation is uniform random selection with the diversity and exclusion rules above.

---

### Playback Resolver

The playback resolver selects the correct audio asset for a track, generates a short-lived stream URL, and returns it in `PlaybackSource`. `CANOPY_MUSIC_SOURCE=rustfs` uses Canopy's HMAC signer for RustFS/S3-compatible object keys. `CANOPY_MUSIC_SOURCE=supabase` asks Supabase Storage for a signed URL and returns that URL to the player. In both modes Canopy is control-plane only: it resolves what to stream, then the player fetches bytes directly from object storage.

### Playback Session Controls

`Play`, `Pause`, `Seek`, `SetPlaybackSpeed`, and `Stop` mutate lightweight session state through the `SessionRepository` port. Requests may provide a `session_id`; empty IDs resolve to the backward-compatible `default` session, and `PlayResponse` returns the session that was updated. `ResolvePlayback` also accepts `session_id` and loads the resolved track into that session after the stream URL is minted, which keeps anonymous playback state synchronized even when the client resolves URLs separately from `Play`.

---

### Provider Adapters

Provider adapters ingest catalog content from external sources — Musopen, Pixabay Music, Internet Archive, and future providers — and are responsible for metadata extraction, license verification, and ongoing catalog synchronization. Every track ingested through a provider adapter carries a license record; a track with no resolvable license is not added to the catalog.

The core ingestion boundary is `CatalogIngest`: adapters produce `ProviderTrack` records and the PostgreSQL implementation persists them in one transaction. The transaction upserts artist, license, album, track metadata, per-codec `audio_assets`, and the `provider_tracks(provider, provider_track_id)` dedupe mapping. Re-ingesting the same provider track updates metadata and assets in place, which makes provider sync idempotent.

---

## Storage Architecture

```mermaid
flowchart LR

    CAN[Canopy]

    DB[(PostgreSQL)]
    FS[(RustFS)]

    CAN --> DB
    CAN --> FS

    DB --> META[Metadata]

    FS --> AUDIO[Audio Files]
    FS --> ART[Artwork]
    FS --> LIC[Licenses]
```

PostgreSQL stores metadata; RustFS stores bytes. Canopy itself does not proxy audio data — `ResolvePlayback` returns a presigned RustFS URL with the expiry embedded, and ExoPlayer streams directly from RustFS using that URL. This keeps Canopy's own request path free of the bandwidth and CPU cost of serving audio, and isolates the streaming hot path to RustFS, which is the system actually responsible for serving bytes.

RustFS is chosen for its S3-compatible API and Apache 2.0 license — fully permissive, with no copyleft or network-use obligations, which matters for a proprietary product. It is run as a single Rust-native binary alongside the rest of the stack, with no foreign runtime in the deployment.

---

### PostgreSQL

Stores metadata only.

```text
artists
albums
tracks
audio_assets
licenses
playlists
users
playback_history
```

No binary audio data stored in PostgreSQL.

---

### RustFS

Bucket:

```text
pandawave-media
```

Structure:

```text
audio/
└── tracks/
    └── musopen/
        ├── trk_001.mp3
        ├── trk_002.mp3
        └── trk_003.mp3

artwork/
├── artists/
├── albums/
└── tracks/

licenses/
└── musopen/
```

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
        uuid license_id
    }

    AUDIO_ASSETS {
        uuid id
        uuid track_id
        string codec
        string content_type
        string object_key
        bigint size_bytes
        string checksum_sha256
    }

    LICENSES {
        uuid id
        string license_type
        string source_url
        string attribution_text
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
    participant RustFS

    User->>PandaWave: Play Track

    PandaWave->>PandaEngine: play(trackId)

    PandaEngine->>Canopy: ResolvePlayback(trackId)

    Canopy->>PostgreSQL: Lookup metadata

    PostgreSQL-->>Canopy: Track + Asset

    Canopy-->>PandaEngine: PlaybackSource (presigned RustFS URL)

    PandaEngine-->>PandaWave: PlaybackSource

    PandaWave->>RustFS: HTTP GET (presigned URL)

    RustFS-->>PandaWave: HTTP Stream

    PandaWave-->>User: Playback
```

---

## Streaming Architecture

```mermaid
sequenceDiagram

    participant ExoPlayer
    participant RustFS

    ExoPlayer->>RustFS: GET (presigned URL)
    Note over ExoPlayer,RustFS: Includes Range header

    RustFS->>RustFS: Validate URL signature + expiry

    RustFS-->>ExoPlayer: 206 Partial Content

    Note over ExoPlayer: Buffer, Seek, Decode
```

Canopy is not in this path. Once `ResolvePlayback` has returned a signed URL, every subsequent byte of audio is served directly by RustFS or Supabase Storage to ExoPlayer.

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
  "stream_url": "https://project.supabase.co/storage/v1/object/sign/pandawave-media/audio/tracks/trk_123.mp3?token=abc123",
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
    participant RustFS

    PandaWave->>PandaEngine: DiscoveryNext()

    PandaEngine->>Canopy: DiscoveryNext()

    Canopy->>PostgreSQL: Read pre-shuffled materialized view

    PostgreSQL-->>Canopy: Selected track

    Canopy-->>PandaEngine: DiscoveryTrack

    PandaEngine->>Canopy: ResolvePlayback()

    Canopy-->>PandaEngine: PlaybackSource

    PandaEngine-->>PandaWave: PlaybackSource

    PandaWave->>RustFS: Stream Audio (presigned URL)

    RustFS-->>PandaWave: Audio Stream
```

---

## Audio Asset Strategy

```text
audio_assets
├── track_id
├── codec
├── object_key
├── content_type
├── size_bytes
└── checksum_sha256
```

Each track has one audio asset per codec it's available in. A track is never duplicated across object keys for the same codec.

```text
Track:
    Beethoven Symphony No. 5

Audio Assets:

    MP3
    object_key=audio/tracks/musopen/trk_123.mp3

Future:

    Opus
    object_key=audio/tracks/musopen/trk_123.opus

    FLAC
    object_key=audio/tracks/musopen/trk_123.flac
```

---

## Observability

PandaEngine and Canopy share a single trace per request. A correlation ID is generated at the earliest possible point — the FFI boundary in PandaEngine — and propagated across the gRPC call to Canopy via a `tonic` interceptor, the same mechanism already used for `x-client-name` metadata. Every span from the Android call site down to Canopy's database queries is part of one trace, not two disconnected ones.

`tracing` is the instrumentation layer on both sides. Prometheus metrics are layered on top once the service is running in production.

---

## Health Checks

Canopy's health endpoint reports actual dependency health, not process liveness. Before responding, it checks PostgreSQL connectivity and RustFS reachability, and distinguishes a fully healthy state from a degraded-but-functional one (for example, RustFS reachable but slow) rather than collapsing every condition into a binary up/down signal. PandaEngine's client maps these states directly to the `HEALTHY` / `REACHABLE` / `DEGRADED` values it already expects.

---

## CI / Verification

The CI pipeline is implemented via **GitHub Actions** (`.github/workflows/ci.yml`). Every change to `master` and every pull request is gated by:

```text
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-features --tests -- -D warnings
cargo fmt --all -- --check
bash scripts/test-pg.sh  # disposable PostgreSQL; migrations and pg tests must run
cargo build --workspace --release
```

The workflow installs `protoc` so that the `canopy-proto` crate's `build.rs` compiles successfully in CI, and uses `Swatinem/rust-cache` for fast incremental builds.

The PostgreSQL step uses the same `scripts/test-pg.sh` harness locally and in CI. It starts an isolated PostgreSQL 18.4 Compose project, waits for database health, applies the complete migration chain, runs the feature tests serially, and destroys the test stack. Startup, connection, migration, or test failures fail the job; database tests cannot report success by skipping their bodies. RustFS integration tests and proto wire-compatibility gates are still planned.

---

## Local Development

Canopy ships with a full `docker-compose.yml` stack for local development and integration testing. The stack includes:

| Service | Image | Role | Port |
| ------- | ----- | ---- | ---- |
| PostgreSQL | `postgres:18.4-trixie` | Metadata persistence (catalog, sessions, playback history) | 5432 |
| Redis | `redis:8` | Cache layer (JadeCache: browse results, discovery pools, presigned-URL TTL) | 6379 |
| RustFS | `rustfs/rustfs:latest` | Object storage (audio bytes, artwork, licenses) | 9000 |
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

To start only the database and cache (without RustFS or Adminer):

```bash
docker compose up -d postgres redis
```

To start Adminer (optional, for browsing the database via web UI):

```bash
docker compose --profile adminer up -d
# Visit http://localhost:8080
```

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
| `CANOPY_RUSTFS_BUCKET` | `pandawave-media` | RustFS media bucket |
| `CANOPY_RUSTFS_ACCESS_KEY` | `canopy` | RustFS access key |
| `CANOPY_RUSTFS_SECRET_KEY` | `canopy-secret` | RustFS secret key |
| `CANOPY_RUSTFS_PORT` | `9000` | RustFS host port |
| `CANOPY_MUSIC_SOURCE` | `rustfs` | Playback URL source: `rustfs` or `supabase` |
| `CANOPY_SUPABASE_URL` | unset | Supabase project URL when using Supabase Storage |
| `CANOPY_SUPABASE_KEY` | unset | Supabase anon/service key used by Canopy to request signed URLs |
| `CANOPY_SUPABASE_STORAGE_BUCKET` | `pandawave-media` | Supabase Storage bucket for music objects |
| `CANOPY_SUPABASE_SIGNED_URL_TTL_SECS` | `900` | Supabase signed URL lifetime |
| `CANOPY_SUPABASE_CATALOG_TABLE` | `tracks` | Supabase REST table/view used by the catalog adapter |
| `CANOPY_SUPABASE_SYNC_ON_START` | `false` | In PostgreSQL mode, fetch and ingest Supabase catalog rows during startup |
| `CANOPY_ADMINER_PORT` | `8080` | Adminer host port |
| `CANOPY_GRPC_ADDR` | `[::1]:50051` | gRPC server bind address |
| `CANOPY_DATABASE_URL` | `postgres://canopy:canopy@localhost:5432/canopy` | PostgreSQL connection string |
| `CANOPY_PROVIDER_FIXTURE_PATH` | unset | Optional provider fixture JSON to ingest at startup when running with `canopy-server/pg` |

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

CI builds can then set `SQLX_OFFLINE=true` to skip the live database requirement for compile-time checked queries. The GitHub Actions workflow will be updated in a future iteration to spin up a PostgreSQL service container and run `sqlx migrate run` before `cargo check`.

### Running the Server

```bash
# Default: in-memory stores (no external dependencies, no database required)
cargo run --bin canopy

# With the full persistent stack (PostgreSQL + RustFS + Redis on the roadmap):
# 1. Start the stack
#    docker compose up -d
#
# 2. Run migrations
#    sqlx migrate run
#
# 3. Run with the `pg` feature — the server auto-detects PostgreSQL and falls
#    back to in-memory stores only if the connection fails.
cargo run --bin canopy --features canopy-server/pg

# Override the database URL:
# CANOPY_DATABASE_URL=postgres://canopy:canopy@localhost:5432/canopy \
#   cargo run --bin canopy --features canopy-server/pg
```

When the `pg` feature is enabled, the server attempts to connect to the database URL configured in `CANOPY_DATABASE_URL`. If the connection succeeds, `PgCatalogRepository`, `PgSessionRepository`, and `PgAudioAssetRepository` are used; otherwise it logs a warning and transparently falls back to the in-memory demo stores. This lets the server start standalone without a database for quick iteration, while production and integration-test deployments use the persistent backend.

If `CANOPY_PROVIDER_FIXTURE_PATH` is set in PostgreSQL mode, Canopy reads the fixture through `TestFixtureProvider` and ingests it with `CatalogIngest` before starting the gRPC server. The operation is idempotent by `provider_tracks(provider, provider_track_id)`, so the same fixture can be replayed during local development.

The `HealthService` returns `healthy`, `version`, aggregate `status`, and per-dependency details. It checks PostgreSQL connectivity when a pool is present; set `CANOPY_HEALTH_CHECK_RUSTFS=true` to include a RustFS TCP reachability probe. Supabase playback URL signing is checked lazily when `ResolvePlayback` asks Supabase Storage for a signed URL.

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

### Supabase Music Source

Set `CANOPY_MUSIC_SOURCE=supabase` when Supabase Storage should be the music source. Canopy expects `audio_assets.object_key` values to match paths inside `CANOPY_SUPABASE_STORAGE_BUCKET`; `ResolvePlayback` selects the best asset, requests a signed Supabase Storage URL, returns it to the client, and updates the supplied anonymous session. Set `CANOPY_SUPABASE_SYNC_ON_START=true` in PostgreSQL mode to fetch normalized catalog rows from Supabase REST and ingest them before serving gRPC.

Required runtime values:

```bash
CANOPY_MUSIC_SOURCE=supabase
CANOPY_SUPABASE_URL=https://<project-ref>.supabase.co
CANOPY_SUPABASE_KEY=<anon-or-service-role-key>
CANOPY_SUPABASE_STORAGE_BUCKET=pandawave-media
CANOPY_SUPABASE_SIGNED_URL_TTL_SECS=900
CANOPY_SUPABASE_CATALOG_TABLE=tracks
CANOPY_SUPABASE_SYNC_ON_START=true
```

Use the anon key only when Supabase Storage policies permit signing the relevant objects. Use a service role key for server-side private-bucket signing and keep it out of client builds.

Expected Supabase catalog row shape:

```json
{
  "id": "song-1",
  "title": "Soft Signal",
  "artist": "Canopy Test",
  "album": "Backend Sessions",
  "release_year": 2026,
  "duration_ms": 181000,
  "is_explicit": false,
  "license_type": "Private",
  "license_url": "https://example.test/license",
  "attribution": "Canopy Test",
  "assets": [
    {
      "codec": "mp3",
      "content_type": "audio/mpeg",
      "object_key": "audio/song-1.mp3",
      "size_bytes": 1234,
      "checksum_sha256": "abc",
      "duration_ms": 181000
    }
  ],
  "artwork_key": "artwork/song-1.png"
}
```


### RustFS Setup

RustFS is an S3-compatible object store that serves audio bytes and artwork directly to ExoPlayer via presigned URLs. Once the container is running, create the `pandawave-media` bucket and upload demo assets:

```bash
# Create the media bucket (using the default credentials from .env.example)
# Adjust host/port if you've overridden CANOPY_RUSTFS_PORT.
mc alias set canopy-rustfs http://localhost:9000 canopy canopy-secret
mc mb canopy-rustfs/pandawave-media

# Upload demo audio and artwork
mc cp demo/audio/tracks/demo-1.m4a canopy-rustfs/pandawave-media/audio/tracks/demo-1.m4a
mc cp demo/artwork/albums/demo-album.png canopy-rustfs/pandawave-media/artwork/albums/demo-album.png
mc cp demo/artwork/tracks/demo-1.png canopy-rustfs/pandawave-media/artwork/tracks/demo-1.png
```

> **Note:** The `mc` (MinIO Client) tool is used here because RustFS is S3-compatible. Any S3-compatible client (AWS CLI, `rclone`, `aws-sdk-s3`) works. Replace credentials and endpoint with your `.env` values if you changed them.

### Redis (JadeCache)

Redis is started by default but is not yet wired into the server. The `JadeCache` layer will use it for:
- Cached `Browse` responses (keyed by path + pagination params)
- Pre-shuffled discovery pool snapshots
- Presigned URL TTL tracking and rate-limiting

Redis configuration is defined in `docker-compose.yml` with `allkeys-lru` eviction and AOF persistence. The server will auto-connect to `redis://localhost:6379` when the cache layer is implemented.

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
├── PostgreSQL
├── RustFS
└── Redis (JadeCache — planned)

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
