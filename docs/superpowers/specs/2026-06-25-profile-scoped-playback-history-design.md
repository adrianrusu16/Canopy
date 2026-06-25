# Profile-Scoped Playback History Design

## Goal

Add durable playback history for real logged-in users only. Anonymous users can continue to browse, search, resolve playback, and play through lightweight sessions, but Canopy must not persist anonymous history.

## Context

Canopy now has a real profile boundary: `UpsertProfile` verifies a login token and persists a profile keyed by `external_user_id`. The original schema still contains older `users` and `playback_history` tables, but new user-specific features should hang from `profiles`, not anonymous sessions or legacy user rows.

## Chosen Approach

Create a profile-scoped playback history vertical slice. The slice adds a `PlaybackHistoryRepository` port, in-memory and PostgreSQL adapters, a `HistoryService`, and a focused authenticated RPC named `RecordPlaybackHistory`.

The RPC accepts an auth token plus playback facts: `track_id`, `duration_ms`, and `completion_pct`. The service verifies the token, loads the profile by external user ID, checks `history_enabled`, validates the playback facts, and records history only when the profile exists and history is enabled.

## Boundaries

Anonymous playback remains available through existing session and resolver paths. These paths must not automatically create durable history. The client may keep anonymous local history, but Canopy does not.

`RecordPlaybackHistory` is intentionally explicit. It records an event when the authenticated client decides a play should count. Future automatic recording from `ResolvePlayback` or `Play` can be added later, but only after the client/server playback lifecycle is clearer.

The auth token will remain in the request body for this slice to match `UpsertProfile`. A later auth-hardening slice can move authenticated RPCs to gRPC metadata or interceptors consistently.

## Data Model

Add a new migration that makes `playback_history` profile-scoped. Because the project is still early, the migration can alter the existing table rather than preserve legacy `users` semantics. The table should reference `profiles(id)`, store `track_id`, `played_at`, `duration_ms`, and `completion_pct`, and index by `profile_id` and `(profile_id, played_at DESC)`.

The repository method should accept the internal profile ID, not the external user ID. This keeps identity resolution inside `HistoryService` and leaves storage adapters working with stable relational keys.

## API Contract

Add proto messages:

```proto
message RecordPlaybackHistoryRequest {
  string auth_token = 1;
  string track_id = 2;
  int64 duration_ms = 3;
  float completion_pct = 4;
}

message RecordPlaybackHistoryResponse {
  bool recorded = 1;
}
```

Add service RPC:

```proto
rpc RecordPlaybackHistory(RecordPlaybackHistoryRequest)
    returns (RecordPlaybackHistoryResponse);
```

If history is disabled, the RPC returns success with `recorded=false`. If the token is invalid, the profile does not exist, or the playback facts are invalid, it returns an error.

## Error Handling

Invalid or expired tokens map to `Unauthenticated`. Missing profiles map to `Unauthenticated` because a login token alone is not a durable Canopy user until `UpsertProfile` has created the profile. Invalid playback facts map to `InvalidArgument`: empty `track_id`, negative duration, or completion outside `0.0..=1.0`. Storage failures propagate through the existing `CanopyError` to gRPC status mapping.

## Testing

Add focused unit tests for `HistoryService`:

- invalid token is rejected
- missing profile is rejected
- disabled history returns `recorded=false` and does not persist
- enabled history records the event
- invalid playback facts are rejected

Add PostgreSQL integration coverage that runs migrations, creates a profile, records playback history, and verifies the row is profile-scoped. Existing default and `canopy-server/pg` test commands remain the verification gate.

## Documentation

Update `README.md` status and Auth/Playback sections to state that backend history is logged-in and opt-in only. Update `docs/openapi.json` with the new RPC, schemas, and `x-canopy-docs` notes.

## Non-Goals

This slice does not add library saves, likes, preferences, recommendations, history retrieval, delete/export controls, or gRPC metadata-based auth. Those are later profile-scoped slices.
