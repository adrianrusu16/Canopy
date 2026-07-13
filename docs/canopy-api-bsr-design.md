# Canopy API Registry Design

## Status

Canopy consumes the published contract. All native authentication and session-management RPCs are wired, durable user-state RPCs enforce native identity, and the persisted authentication outbox is delivered by a supervised TLS-only SMTP worker with leased at-least-once processing and readiness reporting.

- Private BSR module: `buf.build/pandawave/canopy-api`
- Baseline release: `v0.1.0` at `78c41b267ad04168b6a08951de30e415`
- Authentication release: `v0.2.0` at `145678c1d73e45b7bbaebf7e16ee4d64`
- Canopy Prost SDK: `0.5.0-00000000000000-145678c1d73e.2`
- Canopy Tonic SDK: `0.5.0-00000000000000-145678c1d73e.4`
- Implemented or wired in Canopy: native password registration, email verification session issuance, password login, refresh rotation, verification resend, password reset/change, Google login/linking, account lookup/deletion, logout, logout-all, list sessions, revoke session, and native identity enforcement for durable user-state RPCs.
- Remaining in Canopy: production email delivery outside the persisted outbox boundary, CI verification with Buf registry credentials, and full downstream PandaEngine adoption.

## Purpose

The PandaWave ecosystem will move its canonical gRPC contract out of the
Canopy backend repository and into a dedicated, language-neutral API source
repository published through the Buf Schema Registry (BSR).

The protobuf schema is the executable contract. OpenAPI remains a
documentation and tooling companion and must never be used to generate the
PandaEngine client.

## Ownership

- Git source repository: `canopy-api`
- Buf organization: `pandawave`
- Private BSR module: `buf.build/pandawave/canopy-api`
- Initial protobuf package: `canopy.v1`

The Git repository is where changes are authored, reviewed, and released. The
BSR is the publication, compatibility, documentation, and SDK distribution
layer. It complements Git rather than replacing it.

## Repository Layout

```text
canopy-api/
|-- proto/
|   `-- canopy/v1/
|       `-- canopy.proto
|-- openapi/
|   `-- openapi.json
|-- docs/
|-- buf.yaml
|-- buf.lock
|-- CHANGELOG.md
`-- README.md
```

The module root and import paths mirror the protobuf package namespace.

## Canonical Contract

`proto/canopy/v1/canopy.proto` is the only canonical wire contract.

```proto
syntax = "proto3";

package canopy.v1;
```

The `v1` package is the compatibility boundary. Backward-compatible changes
remain in `canopy.v1`. A deliberately incompatible redesign is introduced in
`canopy.v2`, allowing both versions to coexist during migration.

Removing fields requires reserving their numbers and names. Existing field
numbers must never be reused. Additive fields and methods must preserve proto3
default-value behavior for older clients.

## Publication Model

After the initial bootstrap release, pull requests in `canopy-api` run:

```bash
buf format --diff --exit-code
buf lint
buf build
buf breaking --against buf.build/pandawave/canopy-api:main
```

Preview branches may be published under temporary BSR labels so Canopy and
PandaEngine can compile against a proposed contract before release.

After approval, CI publishes the schema to the BSR default label and applies a
release label such as `v0.1.0`. Publications include Git metadata only when that disclosure is explicitly approved;
otherwise the immutable BSR commit and Git revision are recorded side by side.

The bootstrap pull request runs format, lint, and build checks but has no prior BSR release to compare. Compatibility enforcement starts immediately after `v0.1.0` is published.

Release labels support human communication and changelog history. Consumers
pin immutable generated SDK versions, not mutable labels.

## Rust SDK Consumption

The BSR generates separate Prost and Tonic Cargo packages from the published
module using explicitly pinned plugin versions.

Canopy and PandaEngine each retain a small local `canopy-proto` facade crate.
The facade:

- depends on exact BSR-generated SDK versions;
- re-exports the generated message and service modules behind a stable local
  crate name;
- contains no independent message definitions; and
- prevents BSR package naming from leaking throughout either codebase.

Canopy consumes the generated server surface. PandaEngine consumes the
generated client surface. Both projects pin the same schema commit and plugin
versions through their manifests and `Cargo.lock` files.

Local `protoc` and duplicate source generation are removed only after both
consumers build successfully against the generated SDK.

## Authentication

The BSR repository is private.

- Developers authenticate locally with personal Buf/Cargo registry tokens.
- CI uses a least-privilege Buf bot token stored as an encrypted secret.
- Tokens are never committed, printed, placed in Docker build arguments, or
  persisted in image layers.
- Dependency caches may contain downloaded artifacts but never credentials.

Canopy and PandaEngine CI fail clearly when registry authentication is absent
or invalid.

## Documentation

BSR-generated documentation is the primary reference for protobuf packages,
messages, services, and methods. Protobuf comments are therefore treated as
versioned API documentation.

`openapi/openapi.json` is explicitly noncanonical:

- actual HTTP endpoints, including Canopy's private stream authorization
  endpoint, remain genuine OpenAPI operations;
- gRPC-shaped entries are documentation projections only;
- the file states that protobuf is authoritative; and
- CI validates projected gRPC operation names against a protobuf descriptor to
  prevent silent drift.

The Android application does not consume this OpenAPI document.

## Audited V1 Contract

The initial release replaces the prototype monolith with bounded services on
the same gRPC listener and HTTP/2 connection:

- `CatalogService`: browse, search, and media lookup;
- `PlaybackService`: playback-source resolution only;
- `DiscoveryService`: discovery feeds and recommendations;
- `ProfileService`: profile lifecycle and ordinary UI preferences;
- `HistoryService`: consent, recording, listing, and deletion;
- `LibraryService`: saved tracks and likes;
- `PlaylistService`: playlist lifecycle, membership, and ordering;
- `SystemService`: rich application and dependency status; and
- standard `grpc.health.v1.Health`: infrastructure health checking.

These are contract boundaries, not separate deployments. Canopy registers all
services on one server, and PandaEngine uses one generated client channel.

### Local Player Ownership

PandaEngine owns local playback execution and queue state. The prototype
`Play`, `Pause`, `Seek`, `SetPlaybackSpeed`,
`Stop`, `GetSession`, `UpdateSession`, and
`EndSession` RPCs are removed. Playback resolution has no
`session_id`.

Canopy resolves authorized media sources but does not shadow anonymous player
state. PandaEngine controls Media3/ExoPlayer through the local Android bridge.
A future cross-device control feature receives a dedicated synchronized
service.

### Authentication And Privacy

Authentication is carried only in gRPC metadata. Deprecated request-body token
fields are removed.

- Anonymous-capable methods accept absent authorization metadata.
- Invalid supplied credentials return `UNAUTHENTICATED` and never
  downgrade to anonymous.
- Profile-owned operations require valid metadata and an existing profile.
- Private-resource concealment uses `NOT_FOUND` where revealing
  existence would leak information.

Anonymous playback remains stateless and cannot create durable history,
libraries, likes, preferences, or playlists.

### Catalog, Search, And Discovery

Catalog, search, discovery, and recommendation methods use unary paginated
gRPC. Opaque page tokens replace offsets. A zero page size selects the server
default, excessive sizes are clamped, and clients never interpret tokens.
`DiscoveryNext` becomes a batched discovery feed.

Anonymous and non-owner callers receive public release-safe results. The owner
may receive owner-scoped personal results first followed by public results.

### Media Resources

Catalog summaries use stable track, artist, album, and artwork identifiers.
Android `content://` URIs are not part of the backend contract.
PandaEngine resolves and caches artwork before exposing a local URI to
PandaWave. Codec, bitrate, and content type are playback-asset properties.
Missing artwork is represented by an absent artwork reference.

### Common Protobuf Conventions

- Errors use canonical gRPC statuses, not success booleans or error strings.
- Persisted timestamps use `google.protobuf.Timestamp`.
- Media positions and durations use unsigned millisecond fields.
- Partial resource updates use `google.protobuf.FieldMask`.
- Empty successful deletes use `google.protobuf.Empty`.
- Presence-sensitive scalars use proto3 `optional`.
- Create and update methods return the resulting resource.
- IDs and page sizes are validated server-side.

The contract uses `INVALID_ARGUMENT`, `UNAUTHENTICATED`,
`PERMISSION_DENIED`, `NOT_FOUND`, `ALREADY_EXISTS`,
`FAILED_PRECONDITION`, `ABORTED`,
`RESOURCE_EXHAUSTED`, `INTERNAL`, and `UNAVAILABLE`
according to their canonical meanings.

### Profiles, History, And Preferences

`ProfileService` owns upsert, get, update, and delete operations plus
ordinary UI preferences. Cross-device UI preferences use
`google.protobuf.Struct` rather than a JSON string. Security, privacy,
authorization, and subscription policy use typed messages and methods.

`HistoryService` exposes explicit history settings. Disabling history
atomically purges existing events and reports the deleted count.

Profile deletion returns `FAILED_PRECONDITION` while the profile is
the configured instance owner or owns personal media. Personal media requires
an explicit transfer or administrative deletion workflow.

### Durable Collections

Saved-track, like, and membership mutations are idempotent. Repeating a save or
like returns the existing relationship; removing an absent relationship
succeeds.

Playlist ordering is server-authoritative. Reorder requests contain the full
ordered membership and an expected playlist revision. A stale revision returns
`ABORTED` instead of overwriting a concurrent edit.
## Consumer Architecture

```text
PandaWave Android UI
        |
        | local bridge / FFI
        v
PandaEngine Rust middleware
        |
        | generated canopy.v1 gRPC client
        v
Canopy backend
        |
        | generated canopy.v1 gRPC server
        v
Nginx private authorization and media delivery
```

PandaWave is isolated from backend transport details. PandaEngine owns the
remote gRPC client and local application-facing bridge.

## Migration

1. Create the `pandawave` Buf organization and private
   `canopy-api` module.
2. Audit and rewrite the prototype as the clean `canopy.v1` contract.
   No deployed consumer depends on the prototype, so the initial release may
   intentionally change package paths and method shapes.
3. Update Canopy against the locally generated audited contract and run all
   backend quality gates before publication.
4. Create the `canopy-api` Git repository, add the audited protobuf
   and companion documentation, and publish a private preview BSR commit.
5. Verify generated Prost and Tonic SDKs, including plugin/runtime
   compatibility.
6. Replace Canopy's embedded generation input with exact BSR SDK dependencies
   behind its existing `canopy-proto` facade.
7. Run Canopy formatting, Clippy, unit, PostgreSQL, and streaming gates again.
8. Update PandaEngine to the same immutable SDK versions and run its complete
   client and bridge test suites.
9. Publish the approved commit under `main` and `v0.1.0`,
   then enforce `FILE` compatibility against `main`.
10. Remove embedded protobuf sources and local generation only after both
    consumers pass against the published release.

The migration never requires an unverified consumer or server deployment.
## Failure And Rollback

- A schema that fails Buf build, lint, or compatibility checks is not
  published.
- A generated SDK that fails either consumer remains on a preview label.
- Consumer dependency upgrades are ordinary manifest and lockfile changes and
  can be reverted to the previous immutable SDK version.
- The embedded Canopy protobuf source remains available until both consumers
  have completed migration.
- Registry unavailability cannot alter an already locked and cached build
  artifact, but clean online builds require BSR access.

## Verification

The completed migration must prove:

- the source schema builds and passes Buf lint;
- no unintended `FILE` breaking change exists against the previous release;
- Canopy and PandaEngine resolve the same immutable schema commit;
- generated plugin versions are pinned;
- Canopy exposes the expected `canopy.v1` server methods;
- PandaEngine invokes the expected `canopy.v1` client methods;
- the protobuf descriptor and companion OpenAPI operation list agree;
- private credentials are absent from source, logs, and images; and
- existing Canopy PostgreSQL and Nginx streaming behavior remains unchanged.
