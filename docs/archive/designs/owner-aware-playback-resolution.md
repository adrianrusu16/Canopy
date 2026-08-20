# Owner-Aware Playback Resolution

## Status

Approved design. Implementation is the next phase.

## Purpose

Canopy exposes one `ResolvePlayback` gRPC operation to every client. The server,
not the client, selects the appropriate media according to the caller's identity
and current access policy.

Anonymous callers can play release-safe public media. The configured instance
owner can play owner-scoped personal media and public media, with personal media
preferred whenever both are available.

## Scope

This phase changes playback resolution and stream authorization. It does not
change search or recommendations behavior. Those APIs are the immediate
follow-up and must adopt the same visibility and ordering policy.

## Principal Model

Playback resolution classifies each request into an internal principal:

- `Anonymous`: no authorization metadata was supplied.
- `Authenticated`: valid authentication for a non-owner profile.
- `Owner`: valid authentication for the profile currently configured as the
  instance owner.

The client always calls the same RPC. When the client has credentials, its gRPC
interceptor may attach them without changing the request shape or selecting a
different endpoint.

If authorization metadata is supplied but invalid, Canopy returns
`Unauthenticated`. It does not downgrade the request to anonymous access.

The principal is an internal policy input. It provides an extension point for
future subscription tiers, advertising, or playback restrictions without
placing policy logic in clients or changing the RPC contract.

## Resolution Policy

Resolution uses the following order.

### Owner

1. Find a ready personal asset for the requested track that belongs to the
   currently configured instance owner.
2. If one exists, issue a short-lived `Personal` stream capability.
3. Otherwise find a ready, release-safe public asset for the track.
4. If one exists, issue a short-lived `Public` stream capability.
5. Otherwise return `NotFound`.

### Anonymous And Authenticated Non-Owner

1. Find a ready, release-safe public asset for the requested track.
2. If one exists, issue a short-lived `Public` stream capability.
3. Otherwise return `NotFound`.

An authenticated non-owner currently receives the public policy. Keeping it as
a distinct principal permits future authenticated-user policies without
changing this design's external contract.

Personal-first selection applies only to media accessible to the owner. It does
not relax public release-safety rules or expose personal media to other callers.

Repository or infrastructure failures are returned as service errors. They must
not be interpreted as an absent asset and must not cause fallback to another
visibility class.

## Privacy And Error Semantics

Canopy returns the same `NotFound` result when:

- a track does not exist;
- a track has no ready eligible asset;
- an anonymous caller requests a personal-only track;
- an authenticated non-owner requests a personal-only track; or
- an owner requests personal media that belongs to another profile.

This prevents callers from using response differences to enumerate private
media. `PermissionDenied` is not returned for inaccessible personal media.

## Authorization Boundaries

Authorization is enforced twice:

1. **Capability issuance:** `ResolvePlayback` verifies the principal, asset
   visibility, readiness, ownership, and public release safety before minting a
   capability.
2. **Stream authorization:** when the media proxy presents that capability,
   Canopy rechecks that the asset remains eligible for its encoded audience.

A personal capability is valid only while the asset is ready, personal, owned
by the currently configured instance owner, and otherwise streamable. Changing
the configured owner, deleting the asset, or changing its eligibility revokes
subsequent stream authorization even if the capability has not expired.

Public capabilities likewise require the asset to remain ready and
release-safe.

## Components

- The gRPC adapter extracts optional authentication metadata and rejects invalid
  supplied credentials.
- A playback policy component maps identity and instance ownership to the
  internal principal.
- The resolver applies principal-specific lookup order and mints the matching
  capability audience.
- The playable-asset repository exposes explicit owner-scoped personal and
  release-safe public queries.
- The stream authorizer revalidates the capability audience against current
  repository state.

These boundaries keep transport parsing, identity policy, asset selection,
capability creation, and stream enforcement independently testable.

## Session Behavior

Session synchronization continues to use the source selected by
`ResolvePlayback`. Public and personal tracks may coexist in a session. The
client does not track capability scopes or call separate playback endpoints.

## Verification

Tests must cover:

- anonymous public playback;
- anonymous personal-media concealment;
- authenticated non-owner public playback and personal-media concealment;
- owner personal-first playback;
- owner fallback to public playback;
- invalid supplied credentials returning `Unauthenticated`;
- identical `NotFound` behavior for missing and inaccessible media;
- infrastructure failures not triggering visibility fallback;
- personal capability rejection after instance ownership changes;
- public and personal stream-time eligibility rechecks; and
- session synchronization with both capability audiences.

PostgreSQL integration tests must verify the ownership and visibility predicates
used for issuance and stream authorization. In-memory repositories must preserve
the same observable semantics.

## Follow-Up

After this phase, search and recommendations will be reviewed and updated to
work as designed:

- anonymous and non-owner callers receive public, release-safe media only;
- the owner receives owner-scoped personal and public media;
- personal results are ordered ahead of public results for the owner;
- private media remains non-enumerable to unauthorized callers; and
- duplicate-result behavior is defined consistently when personal and public
  representations refer to the same logical recording.
