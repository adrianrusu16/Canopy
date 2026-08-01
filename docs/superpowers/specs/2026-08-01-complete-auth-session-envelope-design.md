# Complete Auth Session Envelope Design

## Problem

Canopy successfully registers accounts, delivers verification mail, consumes
email-verification challenges, and creates authenticated sessions. The gRPC
adapter nevertheless returns an incomplete `SessionEnvelope`: access-token
expiry is hard-coded to zero, account creation time is omitted, and session
creation, last-use, and expiry timestamps are omitted. Clients that require a
complete atomic session snapshot must reject this response.

The database already stores the required account and session timestamps. The
defect spans token issuance metadata, the domain session projection, PostgreSQL
row projection, and the gRPC mapper.

## Scope

Repair every public authentication response that exposes account or session
metadata:

- `VerifyEmail`
- `LoginPassword`
- successful `CompleteGoogleLogin`
- `RefreshSession`
- `GetAccount`
- `ListSessions`

PandaEngine's fail-closed validation and persistence behavior remain unchanged.
No protobuf field numbers or database schema changes are required.

## Design

### Access-token issuance

`Ed25519AccessTokenIssuer::issue` will return an `IssuedAccessToken` containing
the encoded token and `expires_at_epoch_ms`. The issuer remains the single
source of truth for TTL calculation. `IdentityService` will place both values
in its domain `SessionEnvelope`; it will not recalculate or decode the token.

The encoded JWT `exp` claim remains expressed in epoch seconds. The returned
envelope expiry is that exact claim multiplied by 1,000, preserving the
contract's epoch-millisecond unit.

### Domain session projection

`AuthSession` will gain required `created_at_epoch_ms` and
`last_used_at_epoch_ms` fields. These are persistent session facts, not
transport-only decoration, and therefore belong in the repository/domain
projection used by both session envelopes and `ListSessions`.

All PostgreSQL queries consumed by `session_from_row` will select:

- `auth_sessions.created_at` as `created_at_epoch_ms`
- `auth_sessions.last_used_at` as `last_used_at_epoch_ms`
- `auth_sessions.expires_at` as `expires_at_epoch_ms`
- the existing optional revocation timestamp

Session creation naturally reports equal creation and last-use timestamps from
the database defaults. Refresh continues to update `last_used_at`, and the
returned refreshed envelope reports that stored value.

### gRPC mapping

The auth mapper will convert all required epoch-millisecond values to protobuf
timestamps:

- `SessionEnvelope.access_expires_at_epoch_ms` from the issued token metadata
- `SessionEnvelope.refresh_expires_at_epoch_ms` from session expiry
- `AccountSummary.created_at` from `AccountRecord.created_at_epoch_ms`
- `SessionSummary.created_at`, `last_used_at`, and `expires_at` from
  `AuthSession`

The timestamp fields will always be present for successfully projected records.
The existing `current` calculation for listed sessions remains unchanged.

## Error Handling and Compatibility

The fix does not add new client-visible failure modes. Existing conversion
behavior for protobuf integer expiry fields remains compatible. Timestamp
values originate from non-null PostgreSQL columns and are therefore required in
the domain types and response mapping.

Changing `Ed25519AccessTokenIssuer::issue` is an internal Rust API change. All
callers and tests will consume the structured result. The wire contract and
database schema remain backward compatible.

## Testing

Implementation follows red-green-refactor:

1. Add a token test proving the structured issuance expiry exactly matches the
   JWT `exp` claim in milliseconds.
2. Add mapper tests proving account and session protobuf timestamps are present
   and preserve millisecond precision.
3. Extend PostgreSQL integration assertions to prove session creation,
   refresh, and listing return stored creation/last-use/expiry values.
4. Strengthen the local gRPC integration flow so verification and refresh reject
   zero expiry or absent required timestamps.
5. Run formatting, focused tests, the server test suite, and Clippy with warnings
   denied before completion.

## Success Criteria

- Every successful session-producing RPC returns a non-zero access expiry.
- The access expiry equals the JWT `exp` claim expressed in epoch milliseconds.
- Account creation timestamps are present wherever account summaries appear.
- Session creation, last-use, and expiry timestamps are present in envelopes
  and `ListSessions`.
- Existing refresh rotation, revocation, and authentication behavior continues
  to pass its tests.
