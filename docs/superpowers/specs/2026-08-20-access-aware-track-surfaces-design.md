# Access-Aware Track Surfaces Design

## Context

Canopy stores two playable catalog partitions:

- `release_safe` + `ready` tracks are available to every caller.
- `personal` + `ready` tracks are available only when `tracks.owner_profile_id`
  is the profile currently configured as the instance owner.

Playback already applies an owner-first policy, but catalog browse, search,
metadata lookup, discovery, and profile-owned track relationships do not apply
one consistent rule. In particular, catalog calls return only public tracks,
while several library, like, history, and playlist queries can render a track
without rechecking whether the requesting profile may still access it.

This change establishes one backend-wide definition of track accessibility and
uses it for every API that reads or creates a reference to a track. The HMI is a
separate project. Canopy will publish integration guidance, but it will not
implement or assume an HMI pagination strategy.

## Goals

1. Return every track accessible to the caller from browse, search, metadata,
   discovery, playback, saved tracks, likes, history, and playlists.
2. Prevent pending, quarantined, or another owner's personal tracks from being
   revealed or referenced through any API surface.
3. Preserve anonymous access to release-safe media.
4. Preserve owner-only playback and stream authorization for personal media.
5. Apply filtering before pagination so page counts and continuation tokens
   describe the caller's accessible result set.
6. Keep the existing `canopy.v1` protobuf shapes and opaque page-token format.
7. Document the access matrix and correct client pagination behavior.

## Non-goals

- Changing PandaWave, PandaEngine, Media3, or another HMI implementation.
- Promoting personal or quarantined media to `release_safe`.
- Exposing storage keys, raw owner identifiers, visibility fields, or page-token
  internals to clients.
- Adding personalized recommendation ranking. The three discovery-family RPCs
  may continue to share one accessible diversified pool.
- Replacing the current offset encoded inside authenticated page tokens with
  keyset pagination.

## Access Model

Introduce a transport-independent `TrackAccessScope` in `canopy-core`:

```rust
pub enum TrackAccessScope {
    Public,
    Owner { profile_id: String },
}
```

The scope answers which track rows a request may observe. Authentication class
and track scope remain separate concepts: anonymous callers and authenticated
non-owners both receive `Public`; only the currently configured instance owner
receives `Owner`.

| Caller | Accessible tracks |
| --- | --- |
| No credentials | `release_safe` + `ready` |
| Valid authenticated non-owner | `release_safe` + `ready` |
| Valid configured instance owner | Public tracks plus `personal` + `ready` tracks whose `owner_profile_id` equals that profile |
| Invalid, malformed, expired, or revoked supplied credentials | Request fails as `Unauthenticated`; it never falls back to public |

`pending` and `quarantined` tracks are inaccessible to every runtime caller.
Being the historical `owner_profile_id` is insufficient after instance
ownership changes: the profile must also be the currently configured owner.

The existing principal classifier becomes a general request-principal policy
component rather than a playback-only concept. It must derive the same
`TrackAccessScope` for optional-auth public RPCs and for the verified profile
used by durable RPCs.

## Canonical Persistence Predicate

PostgreSQL queries will use one canonical policy predicate equivalent to:

```sql
t.ingest_status = 'ready'
AND (
    t.visibility = 'release_safe'
    OR (
        t.visibility = 'personal'
        AND :owner_profile_id IS NOT NULL
        AND t.owner_profile_id = :owner_profile_id
    )
)
```

`:owner_profile_id` is null for `Public` and is the configured owner's UUID for
`Owner`. The predicate is applied inside selection, count, and mutation SQL,
not after rows have been paginated. PostgreSQL integration tests must exercise
the complete access matrix so SQL behavior cannot drift from the in-memory
implementation.

The in-memory stores will use the equivalent `TrackAccessScope` helper against
`MediaVisibility`, `IngestStatus`, and `owner_profile_id`.

## API Behavior

### Catalog

`CatalogService.Browse`, `CatalogService.Search`, and
`CatalogService.GetMedia` accept optional authentication metadata. The gRPC
adapter verifies any supplied credential, classifies the principal, and passes
the resulting scope to the catalog/search services and repository.

Owner browse and search results are a single database result set containing
public and owned-personal tracks. Canopy must not fetch two independent pages
and merge them in application code, because that would make offsets, counts,
and continuation tokens unstable.

Browse ordering becomes deterministic with `created_at, id`. Search preserves
rank ordering and adds deterministic tie-breakers `title, id`. `GetMedia`
returns `NotFound` for both missing and inaccessible tracks.

### Discovery, For You, and Recommendations

The three discovery-family RPCs accept optional authentication and use the same
scope. Public callers retain the release-safe discovery pool. The owner pool
also includes owned-personal ready tracks and continues to apply explicit-track
filtering, caller exclusions, diversity ordering, and page limits.

The existing public materialized view may remain the optimized public path.
Owner discovery must query an access-filtered pool that includes personal
tracks; it must not append personal tracks after a fully ordered public pool in
a way that permanently starves them from early pages.

### Playback and Stream Authorization

`PlaybackService.ResolvePlayback` retains its current public/owner behavior but
uses the common principal/scope vocabulary. An owner can resolve an owned
personal track or a public track. Other callers can resolve only public tracks.
Missing and inaccessible tracks remain indistinguishable as `NotFound`.

Opaque stream capabilities retain their public or personal audience. The
private stream authorizer continues rechecking current visibility, readiness,
track ownership, and configured instance ownership on every request. No storage
metadata is added to client responses.

### Saved Tracks and Likes

Saving or liking a track validates accessibility transactionally with the
relationship write. Guessing another owner's track ID returns `NotFound` and
creates no row. Listing saved or liked tracks applies the same predicate before
ordering, counting, and pagination.

Removal and unlike operations remain idempotent for a profile-owned
relationship even if the referenced track later becomes inaccessible. This
allows cleanup without revealing track metadata.

### Playback History

Recording history validates that the profile may access the track in the same
transaction that rechecks consent and inserts the event. History lists filter
inaccessible tracks before count and pagination. Entry deletion and history
clearing remain profile-scoped cleanup operations and do not require current
track accessibility.

### Playlists

Adding a track to a playlist validates accessibility transactionally with the
membership write. Playlist-track listing filters before count and pagination.
Removal remains allowed for an owned playlist even if the track later becomes
inaccessible.

Reordering validates the complete currently accessible membership set. Hidden
legacy memberships are neither returned nor required in the request and remain
untouched. Positions of the accessible subset are updated deterministically;
the schema has no uniqueness constraint on `position`, so hidden rows do not
create a write conflict or leak through the response.

## Revocation and Existing Relationships

Track accessibility is evaluated at request time. License revocation,
quarantine, readiness changes, owner reassignment, or visibility changes take
effect immediately across metadata, discovery, playback, and relationship
lists.

Canopy will not destructively delete existing saved, liked, history, or
playlist rows merely because a track becomes inaccessible. Those rows are
retained but hidden. Profile-scoped cleanup operations remain available, and a
track becoming accessible again restores the relationship without recreating
it.

## Pagination Contract

The protobuf contract remains unchanged:

- Clients choose `PageRequest.page_size`; `0` means Canopy's default of 20.
- Canopy caps a requested page size at 100.
- A non-empty `PageInfo.next_page_token` means another page is available.
- Clients must pass that token back unchanged and continue until it is empty.
- Tokens are opaque, authenticated, deployment-specific continuation values.
  Clients must not parse, edit, synthesize, or persist them as durable state.
- Changing the authenticated user, query, parent, genres, or feed exclusions
  starts a new pagination sequence.

The integration documentation will make explicit that receiving one page does
not mean the catalog is complete.

## Error and Privacy Semantics

- Invalid supplied authentication returns `Unauthenticated`.
- Missing and inaccessible track identifiers return the same `NotFound`
  behavior on reads and relationship creation.
- Empty or malformed identifiers retain existing `InvalidArgument` behavior.
- Repository failures remain internal errors and must not downgrade an owner
  request to public scope.
- Responses never reveal whether an inaccessible track is personal, pending,
  quarantined, owned by another profile, or absent.

## Testing Strategy

Tests will be written before implementation and will cover:

1. The domain access matrix for public, owner, other-owner, pending, and
   quarantined tracks.
2. In-memory browse, search, get, and discovery behavior for both scopes.
3. PostgreSQL browse/search ordering, counts, and multi-page continuation over
   a mixed public/personal catalog.
4. Optional-auth gRPC behavior: anonymous public results, owner union results,
   non-owner public results, and invalid-credential rejection.
5. Playback and stream authorization regression coverage.
6. Transactional rejection of inaccessible save, like, history, and playlist
   writes.
7. Filtered counts and pagination for saved, liked, history, and playlist
   reads, including retained-but-hidden stale relationships.
8. Reordering the complete accessible playlist membership while hidden legacy
   rows exist.
9. Documentation contract tests for the access matrix and opaque continuation
   loop.

Focused tests will run first during each red/green cycle. Completion requires
the full locked workspace test suite, PostgreSQL integration tests, formatting,
and lint checks used by the repository.

## Documentation Deliverables

Update `README.md` and `docs/client-integration.md` to state:

- which tracks each caller class can observe;
- which RPCs accept optional authentication;
- that supplied invalid credentials fail rather than becoming anonymous;
- that every track-returning or track-referencing surface uses the same access
  policy;
- how to follow `next_page_token` until exhaustion; and
- that clients must treat `NotFound` as intentionally concealing absent and
  inaccessible tracks.

No HMI source changes are part of this work.

