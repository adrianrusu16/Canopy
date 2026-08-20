# Canopy Authentication Design

## Status

Approved design for implementation. This document defines the first complete
Canopy identity lifecycle: native email/password authentication, Google OIDC,
per-device sessions, verification and recovery, and immediate revocation for
identity-bearing operations.

## Goals

- Keep anonymous catalog browsing, search, and public playback available.
- Require an active, verified account for durable and owner-scoped features.
- Keep identity, credentials, and sessions separate from profile/application
  state.
- Support native email/password and Google login without making Google email an
  identity key.
- Provide rotation, revocation, recovery, deletion, abuse controls, and
  production-ready operational behavior.
- Keep Protobuf canonical and publish the additive contract through the Buf
  Schema Registry (BSR).

## Non-Goals

- Player state, queues, playback controls, history, libraries, playlists,
  preferences, recommendations, and media ownership do not belong to the
  authentication module.
- Passkeys, Apple login, multi-factor authentication, and compromised-password
  blocklisting are deferred.
- OpenAPI remains documentation/tooling output and is not a REST client contract.

## Architecture

Authentication is a bounded module inside the Canopy modular monolith. A new
`canopy.v1.AuthService` exposes the wire contract. Its gRPC adapter translates
messages and canonical statuses only; `IdentityService` owns policy and
orchestration.

The domain depends on narrow interfaces:

- `IdentityRepository`
- `PasswordHasher`
- `TokenIssuer`
- `OidcVerifier`
- `EmailSender`
- `RateLimiter`
- `Clock`

PostgreSQL adapters implement durable identity state and transactions. SMTP is
the initial email adapter. Google is the initial OIDC provider behind a
provider-neutral verifier. These interfaces keep tests deterministic and allow
future adapters without changing domain policy.

Profiles remain user-facing application state. Activating a new account creates
or claims its profile transactionally. Authentication code must not reach into
history, library, playlist, preference, recommendation, or media modules.

## Data Model

### `accounts`

- `id`
- `status`: `pending_email_verification`, `active`, `disabled`,
  `deletion_pending`, or `deleted`
- `created_at`
- `activated_at`
- `disabled_at`
- `deleted_at`

Account state transitions are explicit. Only active accounts may create or use
normal sessions.

### `account_emails`

- `id`
- `account_id`
- `normalized_email`
- `is_primary`
- `verified_at`
- `created_at`
- `deleted_at`

Only one non-deleted email record may claim a normalized email, and each account
has at most one non-deleted primary email. Case normalization is deterministic
and happens before persistence.

### `password_credentials`

- `account_id`
- `password_hash_phc`
- `policy_version`
- `created_at`
- `updated_at`
- `password_changed_at`

Passwords use Argon2id. The PHC string is canonical and already contains the
algorithm, version, parameters, salt, and hash. A successful login rehashes the
password when the stored policy is outdated. Passwords have a minimum of 15
characters and accept at least 64 characters, with no composition rules or
forced periodic rotation. A compromised-password blocklist is a later policy
extension.

### `external_identities`

- `id`
- `account_id`
- `provider`
- `provider_subject`
- `provider_email_at_link_time`
- `created_at`

`(provider, provider_subject)` is unique. Google `sub`, never email, is the
external identity key.

### `auth_sessions`

- `id`
- `account_id`
- `token_family_id`
- `device_label`
- `created_at`
- `last_used_at`
- `expires_at`
- `revoked_at`
- `revocation_reason`

Each row represents one device session and refresh-token family.

### `auth_session_tokens`

- `id`
- `session_id`
- `token_hash`
- `created_at`
- `expires_at`
- `consumed_at`

Only token hashes are stored. Retaining consumed hashes allows replay detection.
Reuse of any consumed refresh token revokes the entire family.

### `auth_challenges`

- `id`
- `account_id`
- `email_id`
- `type`: `email_verification`, `password_reset`, `email_change`,
  `google_login_nonce`, or `google_link`
- `token_hash`
- `attempts`
- `created_at`
- `expires_at`
- `consumed_at`

Challenges are random, high entropy, short lived, hashed at rest, single use,
typed, and atomically consumed.

### `auth_outbox`

- `id`
- `kind`
- `encrypted_payload`
- `created_at`
- `available_at`
- `attempts`
- `delivered_at`

Verification and reset transactions enqueue an outbox item. Sensitive delivery
content is protected with authenticated encryption under a separately managed
outbox key, so no raw challenge token exists at rest. SMTP delivery occurs only
after commit, decrypts only in worker memory, and can be retried after process
failure. Successful delivery removes the encrypted secret material.

## Session And Token Model

Access tokens are short lived (target 10-15 minutes) and signed. They include
issuer, audience, account ID (`sub`), session ID (`sid`), issued time, expiry,
token ID, and signing-key ID. Signing keys support rotation and have no unsafe
production default.

Refresh tokens are opaque, random, high entropy, and longer lived. Only their
cryptographic hashes are persisted. A refresh transaction locks the relevant
session/family, validates the presented hash, consumes that token, inserts the
replacement hash, and commits atomically.

Signature validation is local. Every durable or owner-scoped authenticated RPC
also confirms that the account is active and the session is unrevoked. This
provides immediate logout, disablement, and deletion enforcement where identity
matters. Anonymous catalog, search, and public playback paths do not perform a
session lookup.

## Authentication Flows

### Password Registration And Verification

Registration atomically creates a pending account, primary unverified email,
Argon2id credential, verification challenge, and outbox item. It issues no
session. Responses are generic where account existence is sensitive.

Verification atomically consumes the valid challenge, activates the account,
marks the email verified, creates or claims the profile, and creates a device
session. The response contains a session envelope.

Verification resend is rate limited and returns a generic response. It may
replace or supersede prior live verification challenges according to repository
policy.

### Password Login And Rehash

Login is rate limited before expensive hashing. It uses generic errors for
unknown accounts, wrong passwords, and other sensitive failures. A successful
verification may upgrade an outdated Argon2id hash without failing login. Only
active, verified accounts receive a session.

### Google Login

`BeginGoogleLogin` creates a one-time nonce challenge. `CompleteGoogleLogin`
verifies the Google ID token signature through Google JWKS and validates issuer,
audience, expiry, `sub`, `email_verified`, and an exact nonce match.

An existing `(google, sub)` identity logs into its account. A new verified email
creates a new active account, email, external identity, profile, and session.
When the verified Google email matches an existing account, Canopy does not
merge automatically. It returns a generic `AccountLinkRequired` containing only
a short-lived link challenge ID. Linking requires additional proof from an
already authenticated existing account.

Explicit Google link/unlink operations require an active session. Unlinking is
rejected when it would remove the account's final login method.

### Password Reset And Change

Reset requests are rate limited and always return a generic response. A valid
reset challenge is atomically consumed. Completing a reset updates the Argon2id
credential and immediately revokes every session. Password changes require an
active session and appropriate credential proof, update the credential, revoke
every session, and require a fresh login.

### Logout And Session Management

Logout idempotently revokes the current session. Logout-all revokes every
session for the account. Users may list their sessions and revoke one of their
own sessions. Attempts to manage another account's session fail without leaking
that session's existence.

### Account Deletion

Deletion is idempotent. It immediately revokes every session, transitions the
account to deletion state, and invokes the existing profile/application-data
deletion boundary. Repeated deletion requests produce the same successful
external result.

## Consistency And Security Invariants

- Challenge consumption is a conditional atomic transaction: correct hash and
  type, unexpired, unconsumed, and within the attempt limit.
- Refresh rotation and refresh-token replay handling are transactional.
- Email is sent only after the originating state transaction commits, through
  the transactional outbox.
- Raw access, refresh, challenge, reset, Google ID, authorization-code, and nonce
  values are never logged or emitted to telemetry.
- Raw refresh and challenge tokens are never stored.
- Google external identities use `sub`, never email, as their stable key.
- Matching email never causes automatic Google account linking.
- Deletion and revocation operations are idempotent.
- Session revocation is immediate on refresh and all durable/owner-scoped calls.
- Rate limits apply per relevant account/email, IP, device, and operation before
  expensive password or network work.
- Authentication responses and logs resist account enumeration.

## gRPC Contract

The additive `canopy.v1.AuthService` contract contains:

```proto
service AuthService {
  rpc RegisterPassword(RegisterPasswordRequest) returns (GenericAuthResponse);
  rpc ResendVerification(ResendVerificationRequest) returns (GenericAuthResponse);
  rpc VerifyEmail(VerifyEmailRequest) returns (SessionEnvelope);
  rpc LoginPassword(LoginPasswordRequest) returns (SessionEnvelope);
  rpc RequestPasswordReset(RequestPasswordResetRequest) returns (GenericAuthResponse);
  rpc CompletePasswordReset(CompletePasswordResetRequest) returns (GenericAuthResponse);
  rpc ChangePassword(ChangePasswordRequest) returns (GenericAuthResponse);
  rpc BeginGoogleLogin(BeginGoogleLoginRequest) returns (BeginGoogleLoginResponse);
  rpc CompleteGoogleLogin(CompleteGoogleLoginRequest) returns (GoogleLoginResponse);
  rpc LinkGoogle(LinkGoogleRequest) returns (GenericAuthResponse);
  rpc UnlinkGoogle(UnlinkGoogleRequest) returns (GenericAuthResponse);
  rpc RefreshSession(RefreshSessionRequest) returns (SessionEnvelope);
  rpc Logout(LogoutRequest) returns (GenericAuthResponse);
  rpc LogoutAll(LogoutAllRequest) returns (GenericAuthResponse);
  rpc ListSessions(ListSessionsRequest) returns (ListSessionsResponse);
  rpc RevokeSession(RevokeSessionRequest) returns (GenericAuthResponse);
  rpc GetAccount(GetAccountRequest) returns (GetAccountResponse);
  rpc DeleteAccount(DeleteAccountRequest) returns (GenericAuthResponse);
}
```

`GoogleLoginResponse` uses a `oneof` containing either `SessionEnvelope` or a
generic `AccountLinkRequired` with only a link challenge ID.

`SessionEnvelope` contains the access token, refresh token, both expirations,
account summary, and session summary. A refresh token may appear only in:

- successful email verification;
- successful password login;
- successful Google login; and
- successful session refresh.

It must not appear in account/session reads, password changes, linking, logout,
or other responses.

## Error Model

- `InvalidArgument`: malformed input, invalid password policy, invalid token
  shape.
- `Unauthenticated`: invalid login, expired/invalid access token, invalid refresh
  token, and generic sensitive failures.
- `PermissionDenied`: authenticated principal is not allowed to perform the
  operation.
- `NotFound`: non-enumeration-sensitive resources only.
- `FailedPrecondition`: disabled account, unverified account where activation is
  required, or removal of the final login method.
- `AlreadyExists`: only when existence is not sensitive.
- `ResourceExhausted`: rate limiting, with retry metadata.
- `Unavailable`: required PostgreSQL, SMTP/outbox, or Google dependency is
  unavailable.
- `Internal`: unexpected server fault with no internal detail exposed.

Messages remain deliberately generic. Enumeration-sensitive request/reset/
resend operations return generic success, while sensitive authentication
failures return generic `Unauthenticated`.

## Configuration And Operations

Authentication configuration is environment driven and validated before
listeners serve traffic. It includes:

- signing keys and active key ID;
- issuer, audience, access lifetime, refresh lifetime;
- Argon2id policy;
- Google enablement, client ID, issuer/discovery/JWKS settings;
- SMTP enablement and connection/from-address settings;
- challenge lifetimes and rate-limit policies;
- production/development mode.

Production fails closed when enabled flows lack required configuration. If
Google authentication is enabled, missing Google configuration prevents
startup. If password registration/recovery is enabled, SMTP/outbox must either
be configured or those flows must be explicitly disabled.

Process health reports whether the service is alive. Readiness separately
reflects PostgreSQL, signing-key availability, outbox/SMTP readiness, and Google
discovery/JWKS readiness when enabled. Outbound dependency failures are traced
without sensitive values.

## Contract Publication

The `.proto` source in `canopy-api` remains canonical. OpenAPI is regenerated as
a readable/tooling companion only. The contract change is additive within
`canopy.v1`, so it becomes the next `v0.x` release rather than `v2`.

The release flow is:

1. Update and test `canopy-api`.
2. Run formatting, lint, build, and Buf breaking checks.
3. Publish the schema to the private BSR module.
4. Label the immutable commit with the next `v0.x` version.
5. Pin Canopy to the generated immutable Prost/Tonic SDK versions.
6. Regenerate and validate OpenAPI documentation.

PandaEngine may pin the same immutable SDK later; it is outside this
implementation phase.

## Testing Strategy

Implementation follows test-driven development. Coverage includes:

- account-state transition and password-policy unit tests;
- Argon2id verification, outdated-policy rehash, and malformed PHC handling;
- atomic challenge consumption and replay under concurrency;
- concurrent refresh rotation and consumed-token family revocation;
- immediate rejection of revoked sessions on protected RPCs;
- Google JWKS/signature, issuer, audience, expiry, subject, verified-email, and
  nonce failures;
- explicit account-linking conflicts and final-login-method protection;
- generic enumeration-resistant responses;
- rate-limit behavior and retry metadata;
- outbox commit ordering, retry, and idempotent delivery;
- token and credential redaction in logs/traces;
- idempotent logout, revocation, and account deletion;
- PostgreSQL migration constraints and integration transactions;
- gRPC contract tests proving refresh tokens appear only in allowed responses;
- startup/readiness tests for missing or unavailable dependencies.

## Delivery Sequence

Implementation should proceed in bounded increments:

1. Publish the additive AuthService contract and pin its generated SDK.
2. Add the schema, repository interfaces, and PostgreSQL constraints.
3. Implement token/password/challenge primitives with tests.
4. Implement registration, verification, password login, and refresh rotation.
5. Add session management and immediate protected-call revocation checks.
6. Add SMTP outbox delivery and password reset/change flows.
7. Add Google begin/complete and explicit link/unlink flows.
8. Add account retrieval/deletion, abuse controls, readiness, observability, and
   documentation.

