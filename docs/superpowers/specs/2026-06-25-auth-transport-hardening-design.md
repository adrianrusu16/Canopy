# Auth Transport Hardening Design

## Goal

Move logged-in user authentication out of protobuf request bodies and into a consistent gRPC transport boundary, while preserving anonymous browse, search, playback, and playback resolution.

## Context

Canopy now has two authenticated profile-scoped operations: UpsertProfile and RecordPlaybackHistory. Both currently carry auth_token in their request message and both domain services verify that token directly. That works for the prototype, but it spreads transport concerns into application services and will become noisy as library, likes, preferences, and history retrieval arrive.

## Chosen Approach

Introduce a small gRPC auth context extractor in the server API layer. Authenticated RPCs read a bearer token from request metadata, verify it with AuthService, and pass the resulting UserIdentity into application services. Domain services stop parsing tokens and instead accept an already-authenticated identity.

The preferred metadata key is authorization with value Bearer <token>. A secondary x-canopy-auth-token metadata key is accepted for clients that cannot easily set authorization metadata. During this transition, request-body auth_token remains as a compatibility fallback for UpsertProfile and RecordPlaybackHistory, but README and OpenAPI mark metadata as preferred and body token as deprecated.

## Boundaries

Anonymous RPCs remain ungated: browse, search, get media, playback controls, session operations, discovery, health, and playback resolution keep working without authentication. Auth extraction is invoked only by profile-scoped durable-state RPCs.

This slice does not add service-to-service authentication for PandaEngine itself. It hardens end-user identity transport only. Future service identity can use mTLS, signed client credentials, or gateway validation without changing the profile/history service contracts.

## Service Changes

ProfileService::upsert_profile should accept &UserIdentity, display_name, and history_enabled; it should no longer own AuthService.

HistoryService::record_playback should accept &UserIdentity, track_id, duration_ms, and completion_pct; it should no longer own AuthService. It still loads the profile by identity.user_id, enforces profile existence, validates playback facts, and returns recorded=false when history is disabled.

GrpcApi owns or receives the AuthService alongside the other domain services. Its authenticated RPC methods extract identity before calling the service.

## API Contract

Proto messages keep their existing auth_token fields for compatibility in this slice. Comments and docs should state that metadata auth is preferred and request-body token is deprecated. A later breaking-contract cleanup can remove the fields once PandaEngine has migrated.

Metadata precedence is explicit:

1. authorization: Bearer <token>
2. x-canopy-auth-token: <token>
3. request body auth_token compatibility fallback

If no token is present, return Unauthenticated. If authorization metadata is present but does not use Bearer, return Unauthenticated.

## Error Handling

Malformed, missing, expired, or invalid tokens map to CanopyError::Unauthenticated, then through the existing gRPC to_status mapping. Invalid profile/history payloads continue to map to InvalidArgument. Missing profile remains Unauthenticated because a verified token is not a durable Canopy profile until UpsertProfile has created the profile.

## Testing

Add unit tests for the metadata extractor covering bearer metadata, direct token metadata, malformed bearer metadata, missing metadata, and fallback token behavior. Update profile/history service tests to construct UserIdentity directly. Add gRPC adapter tests if practical; otherwise compile-level coverage plus service/extractor unit tests are acceptable for this slice because the tonic adapter wiring is generated and covered by all-feature compile checks.

Run the existing full verification gate: format, all-feature Clippy, default tests, and PostgreSQL-feature tests.

## Documentation

Update README auth guidance and docs/openapi.json descriptions to say authenticated RPCs should use gRPC metadata. Keep documenting anonymous access clearly: no auth is required to browse, search, play, resolve playback, or use anonymous operational sessions.

## Non-Goals

This slice does not remove auth_token from protobuf messages, add refresh tokens, add login issuance endpoints, add mTLS/service credentials, add authorization roles, or implement library/preferences endpoints.
