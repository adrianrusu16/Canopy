# Project Status and Roadmap

Last updated: 2026-08-20

This page records Canopy's current backend capabilities, known limitations, and
next engineering priorities. Historical designs and implementation plans are
kept in the [documentation archive](archive/README.md).

## Current Capabilities

- A three-crate Cargo workspace separates the generated API facade, domain
  model and repository ports, and server adapters.
- Tonic services expose the audited canopy.v1 control-plane contract.
- PostgreSQL-backed JadeStore adapters persist catalog, identity, ownership,
  media policy, and profile-owned state.
- Native identity supports password registration and recovery, verified email,
  rotating device sessions, revocation, Google login/linking, and account
  lifecycle operations.
- Profiles, opt-in history, library entries, likes, preferences, and private
  playlists require verified identity. Track relationships use the same current
  access scope as catalog and playback, hide revoked items, and retain cleanup.
- Search uses PostgreSQL trigram matching in persistent mode, with a lightweight
  in-memory implementation for tests and demonstrations.
- Catalog browse and search plus Discovery, For You, and recommendations apply
  one caller scope before pagination: public-ready tracks for anonymous and
  non-owner callers, plus personal-ready tracks for the configured owner. The
  discovery feed names still share the same diversity rules and ranking.
- The configured instance owner receives personal-ready playback first with
  release-safe public fallback. Other callers receive public media only.
- canopy-admin imports MP3 files and artwork into content-addressed managed
  storage with checksum deduplication and fail-closed visibility.
- Opaque short-lived capabilities, current-policy revalidation, and internal
  Nginx redirects protect byte-range media delivery.
- CI and local harnesses cover formatting, Rust tests, PostgreSQL policy,
  authentication, client handoff, OpenAPI, and real Nginx streaming.

## Current Limitations

- Discovery feed names do not yet have distinct personalized ranking.
- Search has not been benchmarked against a production-size catalog.
- Media imports lack a reconciliation command for pending rows, missing files,
  finalized-but-unmarked files, and orphaned staging directories.
- Managed artwork has no complete client-facing authorization and universal
  fallback contract.
- Tracing is local structured logging only; correlation propagation, metrics,
  distributed export, and service objectives are not implemented.
- Backup, restore, secret rotation, resource limits, and disaster-recovery
  procedures are not yet documented or rehearsed.
- Redis and RustFS are reserved infrastructure, not active Canopy adapters.

## Priorities

1. **Media reconciliation and recovery.** Add an idempotent administrative
   command for pending database rows, staging directories, finalized files,
   missing files, and checksum-aware repair. Destructive cleanup must be
   explicit.
2. **Artwork delivery and fallback.** Define a client-visible artwork
   contract, authorize managed artwork without exposing storage keys, and
   provide a stable fallback for tracks without embedded or sidecar artwork.
3. **Contract compatibility gates.** Keep Buf breaking checks and publication
   policy in the canonical canopy-api repository while Canopy verifies the
   exact released SDK it consumes.
4. **Observability.** Propagate correlation metadata, attach it to Tonic and
   SQL spans, export bounded request/dependency metrics, and define latency and
   error-rate objectives without collecting undeclared listening behavior.
5. **Evidence-driven search.** Measure pg_trgm quality and cost. Add PostgreSQL
   full-text vectors and indexes only when measurements justify them, before
   considering a separate search service.
6. **Evidence-gated JadeCache.** Introduce a cache port only after measuring
   database load. Authorization and capability validity must continue to rely
   on signed claims plus current PostgreSQL state.
7. **Operational hardening.** Document and rehearse database/media backup and
   restore, authentication and stream-secret rotation, deployment network
   isolation, and resource limits.

## Architectural Guardrails

- gRPC is the public control plane; HTTP serves documented support routes and
  authorized media.
- PandaEngine owns playback and queue state.
- PostgreSQL is the metadata and policy authority.
- Anonymous behavior is not durable backend user data.
- Personal media is never enumerable to unauthorized callers.
- Storage keys, private media paths, signing material, database addresses, and
  SMTP credentials never enter client-visible contracts.
- Supabase runtime integration has been removed.
- RustFS remains inactive and Redis remains unwired until evidence supports a
  concrete adapter.
