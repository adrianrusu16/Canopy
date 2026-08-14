# For You and Recommendations Design

## Goal

Add distinct `For You` and `Recommendations` gRPC endpoints to the canonical
`canopy.v1` API. Initially, both endpoints return the same feed as public
discovery so clients can integrate stable endpoint names before personalized
ranking exists.

This change also closes the PostgreSQL discovery fallback leak that can return
media outside the public release-safe catalog when the discovery materialized
view is empty.

## Scope

The work spans two repositories:

- `/home/catalina/projects/canopy-api` owns the canonical protobuf schema and
  publishes immutable generated SDK packages through Buf Schema Registry.
- `/home/catalina/projects/Canopy` consumes those generated packages and owns
  the discovery application service, PostgreSQL repository, and gRPC adapter.

Personalized ranking, model training, scoring, durable recommendation state,
and changes to playback history or preference storage are outside this phase.

## Contract Design

`DiscoveryService` gains two additive RPCs:

```proto
rpc GetForYouFeed(GetForYouFeedRequest) returns (GetForYouFeedResponse);
rpc GetRecommendations(GetRecommendationsRequest) returns (GetRecommendationsResponse);
```

Each request has the same initial wire shape as discovery:

```proto
repeated string exclude_track_ids = 1;
PageRequest page = 2;
```

Each response has the same initial wire shape as discovery:

```proto
repeated TrackSummary tracks = 1;
PageInfo page_info = 2;
```

The RPCs use dedicated request and response messages instead of reusing the
discovery messages. This small amount of schema duplication lets each feed
evolve independently without changing method names or overloading fields with
different semantics.

The additions are wire-compatible within `canopy.v1`. Existing RPCs and field
numbers remain unchanged.

## Authentication and Visibility

All three feed RPCs remain anonymous-capable. A bearer token may be present,
but it does not alter results in this phase. `For You` is therefore a stable
product endpoint name, not a claim that the initial response is personalized.

Every feed returns only tracks satisfying all of these conditions:

- `visibility = 'release_safe'`
- `ingest_status = 'ready'`
- `is_explicit = FALSE`

Caller-supplied `exclude_track_ids`, page size, page token, and artist
diversification behave identically across the three RPCs.

## Canopy Backend Design

The existing `DiscoveryService::feed` remains the single application-level
implementation. `DiscoveryGrpc` adds two transport methods that translate
their dedicated protobuf messages, call `DiscoveryService::feed`, and map the
result to their dedicated response messages. No new domain service or ranking
abstraction is introduced.

The PostgreSQL `shuffle_pool` fallback is retained for resilience when
`mv_discovery_pool` is empty, but its base-table query gains the same public,
ready, and non-explicit predicates as the materialized view. The fallback must
never widen visibility relative to the primary query.

## Publication and Consumption

The canonical schema change is formatted, linted, built, checked for breaking
changes, and checked against the repository boundary rules. The existing
`canopy-api` release workflow publishes a new immutable Buf commit and
generated Rust SDK packages.

After publication, Canopy pins the exact new Prost and Tonic package versions.
The local reference protobuf in Canopy is updated to match the canonical
schema, but it remains documentation rather than a generated-code source.

Canopy must not claim the endpoints are implemented until it compiles and its
tests pass against the newly published generated SDK versions.

## Error Handling

The new methods use the existing page-token decoder, page-size limits, domain
error mapping, and tonic status behavior. Invalid page tokens fail exactly as
they do for `GetDiscoveryFeed`; repository failures remain `Unavailable` via
the existing transport mapping.

An empty eligible catalog produces a successful empty response. It must not
fall back to ineligible tracks.

## Testing

Contract verification covers:

- Buf formatting, linting, build, and breaking-change checks.
- Boundary validation for the canonical `canopy.v1` schema.
- Generated Rust clients and servers exposing all three discovery methods.

Canopy verification covers:

- A PostgreSQL regression test in which the materialized view is empty while
  public-ready, personal, quarantined, pending, and explicit base rows exist;
  only the eligible public-ready non-explicit row may be returned.
- gRPC adapter tests showing discovery, `For You`, and recommendations return
  equivalent tracks and pagination for equivalent requests.
- Existing discovery behavior tests for exclusions, diversification, default
  page size, maximum page size, and page-token handling.
- The complete PostgreSQL test harness after the targeted tests pass.

## Documentation

`canopy-api` release notes and consumer documentation describe the two new
anonymous-capable endpoints and their deliberately non-personalized initial
behavior. Canopy's README status table distinguishes implemented endpoint
aliases from future personalized recommendation ranking.

## Future Evolution

Later work may add authenticated ranking based on consented history, likes,
library state, and preferences behind the dedicated endpoints. That work may
change internal services and response ordering but must preserve the existing
RPC names, field numbers, visibility guarantees, pagination semantics, and
anonymous fallback behavior unless a separately reviewed contract change says
otherwise.
