# Recommendations for Canopy

> This document captures architectural recommendations, short-term priorities, and longer-term considerations for the **Canopy** backend and the broader **PandaWave** ecosystem. It is a living document — items should be promoted or removed as the codebase evolves.

---

## Quick Wins (Next 1–2 Sprints)

These items deliver the highest value for the lowest risk and should be prioritized before the codebase grows much larger.

### 1. Wire Up PostgreSQL + `sqlx`

The in-memory `jade_store` implementation has validated the repository port design, but the real complexity of a media catalog service — connection pooling, query optimization, transaction boundaries, and migration discipline — has not been exercised yet. Start with:

- A `docker-compose.yml` for local PostgreSQL.
- A minimal `sqlx` migration set (users, artists, albums, tracks, audio_assets, licenses, playback_history).
- A `PgCatalogRepository` behind the `CatalogRepository` port, even if it only implements read/browse for now.
- `sqlx prepare` checked into the repo so CI can compile in offline mode.

### 2. Implement Health Check Dependency Probes

The `HealthService` already exists but the dependency-aware checks (PostgreSQL connectivity, RustFS reachability) are marked as planned. Add them now:

- A lightweight PostgreSQL ping (`SELECT 1`).
- A RustFS/S3-compatible `HEAD` or `ListBuckets` probe.
- Return `HEALTHY` / `REACHABLE` / `DEGRADED` so PandaEngine can react appropriately.

This is a small amount of code that prevents cascading failures in a Docker Compose or Kubernetes deployment.

### 3. Add a Skeleton Provider Adapter

Even a minimal adapter for a single source (e.g., Musopen or a static JSON test fixture) forces the full ingestion pipeline to be exercised end-to-end:

- Metadata extraction and normalization.
- License record creation and validation.
- `CatalogService` → `JadeStore` write path.
- Provider-specific ID tracking (so you don't re-import the same track).

A test fixture adapter is especially useful because it gives you deterministic data for integration tests without relying on external API availability.

### 4. Explicit Error Taxonomy

Define and document the `CanopyError` → `tonic::Status` mapping explicitly. The middleware (`PandaEngine`) needs to react differently to:

- `NOT_FOUND` → skip or surface to user.
- `UNAVAILABLE` → retry with backoff.
- `PERMISSION_DENIED` → re-authenticate or fail hard.
- `INVALID_ARGUMENT` → client bug, do not retry.

Document this in a short markdown table or code comment block in `api/grpc.rs` so the two sides stay in sync.

---

## Medium-Term Priorities (2–6 Weeks)

### 5. Implement Auth Stubs

Retrofitting authentication into a working gRPC service is harder than building it when the surface is smaller. Even a naive implementation forces interceptors to be wired correctly:

- **Service identity:** a static mTLS check or a simple bearer-token validation for the PandaEngine → Canopy gRPC channel.
- **End-user identity:** a JWT or session-token check scoped to playback history and personalization.
- Both checks should run on every request, as designed.

The stub can be permissive in development (e.g., bypass with an env flag), but the interceptor plumbing should be real.

### 6. Complete the Database Schema

The ER diagram is missing operational tables that are mentioned in the text:

- **`playback_history`** — needed for the Discovery service's "exclude recently played" feature.
- **`users`** — needed for end-user identity and session scoping.
- **Provider sync state** — a `provider_sync_logs` or `external_ids` table to track last ingestion time and provider-specific IDs.

Update the schema diagram and the `sqlx` migrations to include these before the discovery and personalization features are implemented.

### 7. Add `JadeCache` for Read-Heavy Paths

`JadeCache` is named in the hierarchy but not in the architecture. For read-heavy operations like `Browse` and `DiscoveryNext`, a short-lived cache (e.g., Redis, Valkey, or even an in-memory LRU in the server process) can dramatically reduce database load:

- `Browse` responses are cacheable by path + pagination params.
- `DiscoveryNext` is less cacheable due to the exclusion/diversity logic, but the underlying pre-shuffled materialized view can be cached.
- Keep the cache invalidation simple: TTL-based for now, explicit invalidation later when provider adapters write new data.

### 8. Load-Test the Presigned URL Flow

ExoPlayer's behavior during seeking and buffering generates many concurrent HTTP `Range` requests against the same presigned URL. Verify that:

- The presigned URL validation in RustFS is stateless and fast (no DB lookup).
- RustFS handles concurrent small-range requests without connection exhaustion.
- The URL expiry is generous enough to cover a full track duration plus buffering, but short enough to limit abuse.

A simple `k6` or `oha` script against RustFS with Range headers is sufficient for an early validation.

---

## Longer-Term / Architectural Considerations

### 9. Search: Delayed but Deliberate Upgrade Path

The current strategy — PostgreSQL `pg_trgm` + `ILIKE` — is correct for the prototype stage. When the catalog grows and query patterns demand more, consider an intermediate step before jumping to a dedicated search engine:

- A materialized `search_vectors` table in PostgreSQL with pre-combined artist + album + track tsvectors.
- GIN indexing on that vector.
- Only move to Elasticsearch/OpenSearch when PostgreSQL full-text search has been observed to be insufficient with real data and real query logs.

This is a deliberate, evidence-driven addition rather than a default.

### 10. Storage: RustFS vs. Alternatives

The choice of **RustFS** (a Rust-native, S3-compatible store) is defensible, especially given licensing concerns with MinIO. However, before committing to operating RustFS in production:

- Verify it has been tested for concurrent range-request workloads (see #8).
- Confirm durability guarantees, replication strategy, and corruption detection.
- Evaluate whether a cloud object store (S3, R2, GCS) with presigned URLs is a viable bridge during early deployment, with RustFS as a later self-hosted target.

The "no foreign runtime" constraint is valuable, but not if it comes at the cost of data durability or operational simplicity.

### 11. Observability: Correlation IDs and Metrics

`tracing` is initialized, but distributed tracing across the FFI/gRPC boundary is one of the hardest parts of a three-tier system (Android app → Rust middleware → Rust backend). The next observability milestone should be:

- **Correlation ID propagation:** generated at the FFI boundary in PandaEngine, passed via `tonic` metadata to Canopy, and attached to every span and log line.
- **Prometheus metrics:** request count, latency histograms, error rates per RPC method, and dependency health (PostgreSQL, RustFS) as metrics, not just logs.

### 12. CI Expansion: Integration Tests and Proto Compatibility

The CI pipeline now covers the basics. Future gates should include:

- **Database migration tests:** apply migrations against a fresh PostgreSQL container in CI, roll back one step, and verify the application still compiles (`sqlx` offline check).
- **Integration tests:** spin up real PostgreSQL and RustFS containers in CI, run tests against them rather than mocks alone.
- **Proto wire-compatibility check:** verify that changes to `canopy-proto` don't break the expected gRPC contract with PandaEngine before merge. This can be a simple `buf breaking` check or a contract test.

### 13. Schema Evolution for Audio Assets

The current asset strategy is one row per codec per track. As the catalog grows, you may want to add:

- **Bitrate / quality tiers** within a codec (e.g., 128 kbps MP3 vs. 320 kbps MP3 vs. V0).
- **Regional availability** or provider-specific restrictions.
- **Asset priority / fallback order** so the resolver can select the best available codec for the client's declared capabilities.

Keep the `audio_assets` table flexible — a `metadata` JSONB column or a normalized `asset_attributes` table can accommodate this without breaking the core resolver logic.

---

## License Note

MinIO's licensing change (AGPLv3 / SSPL) makes it a poor fit for a proprietary product. The current choice of **RustFS** avoids this issue entirely. Continue to verify that any future storage dependencies remain permissively licensed (Apache 2.0, MIT, BSD) before integration.

---

*Last updated: 2025-06-17*
