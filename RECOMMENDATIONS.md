# Recommendations for Canopy

This document is the current engineering progress ledger and roadmap for Canopy. Completed foundations remain visible so recommendations are not repeatedly reopened; obsolete advice is removed.

## Completed Foundations

### Complete: PostgreSQL and Migration Discipline

Canopy has a PostgreSQL-backed JadeStore, versioned migrations, pool configuration, transaction boundaries for media import and policy changes, and a disposable integration harness that applies the complete migration chain.

### Complete: Dependency-Aware Health

Health reports PostgreSQL and managed-media-library readiness. External storage providers have no runtime health wiring. Nginx has an independent token-free health endpoint for orchestration.

### Complete: Deterministic Provider Ingestion

The fixture adapter exercises normalized, idempotent catalog ingestion without an external provider dependency. Imported local media is quarantined as personal and pending until finalization succeeds.

### Complete: Domain Error Taxonomy

CanopyError maps transport-independent failures to intentional tonic status codes, allowing clients to distinguish invalid requests, authentication failures, missing resources, conflicts, and unavailable dependencies.

### Complete: Authenticated Durable State

Profiles, opt-in history, libraries, likes, preferences, and private playlists are profile-owned and require verified login metadata. Anonymous users can browse, search, and play but never create durable backend state.

### Complete: Managed Local Media and Public Streaming

canopy-admin imports MP3 files and artwork into content-addressed managed storage. ResolvePlayback issues opaque short-lived capabilities, Canopy revalidates current PostgreSQL policy, and Nginx serves authorized files with byte-range support through internal X-Accel locations.

### Complete: Integration and CI Foundations

GitHub Actions runs formatting, checks, all-feature Clippy, default tests, the disposable PostgreSQL harness, the streaming harness, and release builds. Streaming tests cover ranges, generic denial, direct-path protection, and immediate policy revocation.

## Current Priorities

### 1. Owner-Only Personal Playback Issuance

Complete authenticated personal capability issuance for the configured instance owner. Reuse the existing personal token audience and fail-closed repository checks; do not broaden personal media access to every authenticated profile.

### 2. Media Reconciliation and Recovery

Add an administrative reconciliation command for pending rows, finalized files, missing files, and orphaned staging directories. Recovery must be idempotent, checksum-aware, and explicit about destructive actions.

### 3. Artwork Delivery and Universal Fallback

Define the client-facing artwork contract, authorize managed artwork without exposing storage keys, and provide a stable fallback when imported tracks have no embedded or sidecar image.

### 4. Proto Compatibility Gate

Add a wire-compatibility check, such as buf breaking, against the accepted PandaEngine contract. OpenAPI remains a companion document; protobuf remains canonical.

### 5. Observability

Propagate correlation metadata from PandaEngine, attach it to tonic and SQL spans, export request/dependency metrics, and define latency and error-rate objectives. Avoid adding telemetry that captures track history or user behavior beyond the service's declared persistence model.

### 6. JadeCache Only Where Evidence Supports It

Wire Redis behind a cache port after measuring PostgreSQL load. Start with bounded, TTL-based catalog reads or discovery snapshots; capability validity and authorization must continue to rely on signed claims plus current PostgreSQL policy, not cache presence.

### 7. Evidence-Driven Search Evolution

Measure the existing trigram search against real catalog size and query patterns. Add PostgreSQL full-text vectors and GIN indexes before considering a separate search service.

### 8. Operational Hardening

Document backup and restore for PostgreSQL and the managed media volume, rehearse recovery, rotate authentication and stream secrets safely, and define resource limits and deployment-specific network isolation.

## Guardrails

- Supabase runtime and configuration code has been removed.
- RustFS is inactive Compose infrastructure only; Canopy has no RustFS runtime or readiness wiring.
- gRPC is the client control plane. HTTP exists only for Nginx media delivery and its private authorization subrequest.
- Anonymous activity is not durable backend user data.
- Storage keys and internal media paths never enter client-visible contracts.

*Last updated: 2026-07-01*
