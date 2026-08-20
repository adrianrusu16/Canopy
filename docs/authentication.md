# Authentication

Canopy supports anonymous discovery and playback alongside native accounts.
Identity is separate from application profile state: an account proves who the
caller is, while a profile owns durable listening data and collections.

Authentication is available only in PostgreSQL builds. Non-PostgreSQL builds
retain in-memory domain stores for tests and demonstrations but do not register
the public AuthService.

## Access Model

Anonymous callers may browse, search, request discovery feeds, and resolve
release-safe playback. Anonymous activity is not a durable backend identity and
cannot own history, library items, likes, preferences, or playlists.

Protected calls receive lowercase gRPC metadata:

```text
authorization: Bearer <access-token>
```

Supplying invalid credentials is an authentication error. Canopy does not
downgrade a request with malformed or expired credentials to anonymous access.
Protected operations verify the Ed25519 access token and recheck the referenced
session in PostgreSQL so logout and revocation take effect immediately.

## Password Registration and Verification

RegisterPassword creates a pending account. Canopy stores an Argon2id password
hash and a digest of the single-use verification challenge; plaintext
passwords and challenge tokens are not stored.

Registration returns a generic accepted response to avoid account
enumeration. Canopy sends a verification action URL based on
CANOPY_AUTH_PUBLIC_BASE_URL. VerifyEmail atomically consumes the challenge,
activates the account, creates its profile, and returns the first complete
session envelope.

New and replacement passwords contain 8–64 Unicode characters. Login continues
to accept existing non-empty credentials that may predate the current creation
policy. Email input is normalized and validated before persistence.

Password hashing and verification run outside the asynchronous transport with
bounded concurrency. Saturation returns RESOURCE_EXHAUSTED rather than allowing
unbounded expensive work.

## Access and Refresh Sessions

Each device session has a short-lived signed access token and a rotating opaque
refresh token. RefreshSession performs a transactional rotation: it consumes
the presented refresh token and returns a replacement session envelope.

Reuse of a consumed refresh token revokes the session family. Clients must
serialize refresh operations and must not replay a token after an ambiguous
transport outcome.

Logout revokes the current session. LogoutAll, password reset, and account
deletion invalidate every session for the account. ListSessions and
RevokeSession operate only within the authenticated account, and revocation is
idempotent.

Production deployments provide a stable 32-byte Ed25519 signing seed through
CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64. The generated-key escape hatch
is for explicit local development only.

## Email Delivery

Registration, verification resend, and password-reset requests enqueue email
work inside the same PostgreSQL transaction as the identity change. Recipient
and template payloads are authenticated-encrypted at rest with an AES-GCM
sealing key.

A supervised worker leases committed outbox rows and sends through
authenticated, TLS-only SMTP. Supported modes are implicit TLS and required
STARTTLS; plaintext delivery and disabled certificate verification are not
accepted. At-least-once delivery is combined with deterministic message IDs to
reduce duplicate presentation. Successful delivery clears the encrypted
payload.

Retries use bounded backoff and terminal failure after the configured attempt
limit. The development-only undelivered-email mode leaves work queued and
reports degraded readiness; it is not a functional client bootstrap.

## Google Identity

Google login and explicit account linking are enabled only when
CANOPY_GOOGLE_OIDC_CLIENT_IDS is configured. Canopy validates the ID token
against the configured public client IDs and the HTTPS token-information
endpoint.

An existing Google subject can start a session. Matching an email address alone
does not silently link identities; linking requires an authenticated account
operation. Google client secrets and provider-validation credentials are never
client handoff values.

## Durable Profile State

Verified identity is resolved to a profile before Canopy reads or mutates
history, saved library items, likes, preferences, or private playlists.
Anonymous clients may keep local state, but Canopy persists none of it until
the user has an account and profile.

Playback history is opt-in through history_enabled. Recording while disabled
returns recorded=false. Disabling history deletes existing events in the
profile-update transaction, and consent-aware recording prevents a concurrent
request from repopulating them.

Private playlists are profile-scoped. Cross-profile access is concealed as not
found; membership changes are idempotent, and reordering requires the complete
current track set.

## Readiness and Failure Behavior

PostgreSQL-mode startup fails closed when required signing or email
configuration is invalid. SMTP outages affect readiness but not liveness or
unrelated RPCs. Authentication failures use canonical gRPC statuses; clients
must branch on status codes rather than message text.

Secrets, tokens, email addresses, message bodies, database URLs, and encryption
material must not be logged or included in client connection artifacts.

## Client Integration

The end-to-end registration, refresh, recovery, and logout sequence is in the
[Client Integration Handoff](client-integration.md). Environment requirements
and validation rules are in [Configuration](configuration.md).
