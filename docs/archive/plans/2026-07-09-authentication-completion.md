# Authentication Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Completed steps are checked; remaining steps use unchecked boxes.

**Goal:** Finish Canopy authentication so native identity is production-ready and becomes the auth boundary for all durable user-state gRPC calls.

**Architecture:** Keep identity, credentials, sessions, and challenges inside the `identity` bounded module. Keep profiles, history, library, likes, preferences, playlists, media ownership, and recommendations outside identity. The gRPC adapter translates protobuf and metadata only; `IdentityService` owns authentication policy; PostgreSQL repositories own atomic state transitions.

**Tech Stack:** Rust, Tokio, Tonic gRPC, Prost generated SDK from BSR `canopy-api`, SQLx PostgreSQL, Argon2id, Ed25519 access tokens, opaque SHA-256-digested refresh/challenge tokens.

## Status (2026-07-14)

Tasks 1-9 are implemented and verified in Canopy. All `AuthService` RPCs are wired, native identity protects durable user-state RPCs, abuse controls are active, sensitive outbox payloads use authenticated encryption, and the supervised SMTP worker provides retry, idempotency, startup-policy, and readiness behavior. The backend authentication scope in this plan is complete.

Canonical protobuf resources and RPC semantics live in the sibling `../canopy-api` repository. Canopy owns runtime and deployment documentation, including `docs/openapi.json` for its real HTTP routes and the secret-free client integration handoff.

## Global Constraints

- Do not commit, stage, push, or switch branches; the user handles Git.
- `.proto` is the real contract; `docs/openapi.json` is documentation/tooling only.
- Anonymous users may browse, search, and play; anonymous users must not get backend history, libraries, likes, preferences, playlists, or durable sessions.
- Never log raw refresh tokens, challenge tokens, password-reset tokens, Google ID tokens, passwords, or password hashes.
- Never store raw refresh or challenge tokens.
- All challenge consumption must be atomic.
- All refresh rotation must be transactional.
- Email sending happens after commit.
- Use Google `sub`, not email, as the external identity key.
- Do not auto-link Google to an existing email without extra proof.
- Deletion must be idempotent.
- Session revocation must be immediate and checked on refresh and durable calls.
- Current implemented AuthService methods: `RegisterPassword`, `ResendVerification`, `VerifyEmail`, `LoginPassword`, `RequestPasswordReset`, `CompletePasswordReset`, `ChangePassword`, `BeginGoogleLogin`, `CompleteGoogleLogin`, `LinkGoogle`, `UnlinkGoogle`, `RefreshSession`, `Logout`, `LogoutAll`, `ListSessions`, `RevokeSession`, `GetAccount`, `DeleteAccount`.
- Current pending AuthService methods: none.
- Current production email delivery: a supervised TLS-only SMTP worker leases committed verification and password-reset messages, retries failures with bounded backoff, and reports readiness independently from liveness.

---

## File Structure

- Modify `crates/canopy-server/src/identity/service.rs`: domain policy for access-token validation, email verification resend, password reset/change, Google login/linking, account lookup/deletion, and durable-call principal creation.
- Modify `crates/canopy-server/src/identity/tokens.rs`: production-safe access-token config loading/verification improvements if needed.
- Create or modify `crates/canopy-server/src/identity/outbox.rs`: post-commit email outbox payload types and delivery boundary.
- Modify `crates/canopy-core/src/identity_repository.rs`: repository commands/results for resend, reset, change password, Google login/linking, account lookup, deletion, and identity-backed profile lookup.
- Modify `crates/canopy-server/src/jade_store/pg_identity.rs`: SQLx implementation of all identity repository methods.
- Modify `migrations/20260703000002_identity_auth.sql` only if the existing schema is insufficient; prefer additive migrations after it if already shared.
- Modify `crates/canopy-server/src/api/grpc/auth.rs`: wire remaining AuthService RPCs.
- Modify `crates/canopy-server/src/api/grpc/mod.rs` and durable adapters under `crates/canopy-server/src/api/grpc/*.rs`: replace legacy durable auth extraction with native identity principal extraction.
- Modify durable services/repositories only where the identity-to-profile boundary requires it.
- Modify `README.md`, `docs/openapi.json`, and `docs/canopy-api-bsr-design.md` after each externally visible auth change.
- Add/extend tests in `crates/canopy-server/tests/auth_domain.rs`, `crates/canopy-server/tests/pg_integration.rs`, `crates/canopy-server/tests/identity_schema.rs`, and relevant gRPC adapter tests.

---

### Task 1: Production Access Token Configuration

**Status: Complete.**

**Files:**
- Modify: `crates/canopy-server/src/config.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Modify: `crates/canopy-server/src/identity/tokens.rs`
- Test: `crates/canopy-server/tests/token_primitives.rs`
- Test: `crates/canopy-server/src/config.rs`

**Interfaces:**
- Consumes: `Ed25519AccessTokenIssuer::generate`, `Ed25519AccessTokenIssuer::verifier`, `AccessTokenConfig`.
- Produces: startup config that does not generate a throwaway signing key in production.

- [x] **Step 1: Write failing config tests**

Add tests proving startup rejects missing production signing material and accepts configured signing material. The minimum acceptable design is an env-backed Ed25519 signing key or a dev-only generated key guarded by an explicit development flag.

Run: `cargo test -p canopy-server config::tests::identity_access_token --locked`

Expected: FAIL because identity access-token configuration is not loaded yet.

- [x] **Step 2: Implement config parsing**

Add fields such as:

```rust
pub struct IdentityTokenConfig {
    pub issuer: String,
    pub audience: String,
    pub key_id: String,
    pub ttl_seconds: u64,
    pub signing_key_base64: Option<String>,
    pub allow_ephemeral_dev_key: bool,
}
```

Reject production startup unless signing key material is present.

- [x] **Step 3: Wire runtime construction**

Replace runtime-generated unconditional key creation in `crates/canopy-server/src/lib.rs` with config-driven construction.

- [x] **Step 4: Verify**

Run:

```bash
cargo fmt --all
cargo test -p canopy-server --test token_primitives --locked
cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings
```

---

### Task 2: Sealed Email Outbox Payloads And Verification Resend

**Status: Complete.** This task covers transactional enqueueing and sealed payloads. SMTP delivery is tracked separately in Task 9.

**Files:**
- Create: `crates/canopy-server/src/identity/outbox.rs`
- Modify: `crates/canopy-server/src/identity/mod.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-core/src/identity_repository.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/api/grpc/auth.rs`
- Test: `crates/canopy-server/tests/auth_domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `RegisterPasswordRecord`, `auth_outbox`, `auth_challenges`.
- Produces: `ResendVerification` behavior and encrypted/signed outbox payloads instead of placeholder bytes.

- [x] **Step 1: Write failing domain tests**

Add tests:

```rust
#[tokio::test]
async fn registration_queues_verification_email_payload_after_account_create() {}

#[tokio::test]
async fn resend_verification_rotates_challenge_for_pending_account_only() {}
```

Expected: FAIL because outbox payloads are placeholders and resend is unimplemented.

- [x] **Step 2: Define outbox payload types**

Create an internal payload model that includes email, challenge purpose, token template variables, and expiry. Do not include raw tokens in logs or debug output.

- [x] **Step 3: Add repository method**

Add an atomic method similar to:

```rust
async fn create_email_verification_challenge(
    &self,
    normalized_email: &str,
    token_hash: [u8; 32],
    expires_at_epoch_ms: u64,
    encrypted_outbox_payload: Vec<u8>,
    outbox_key_id: String,
) -> CanopyResult<()>;
```

It must only operate on pending accounts and must not reveal whether an email exists in public responses.

- [x] **Step 4: Wire `ResendVerification`**

`AuthGrpc::resend_verification` returns `GenericAuthResponse { accepted: true }` for both existing and non-existing emails.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p canopy-server --test auth_domain --locked
bash scripts/test-pg.sh
cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings
```

---

### Task 3: Password Reset And Change Password

**Status: Complete.**

**Files:**
- Modify: `crates/canopy-core/src/identity_repository.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/api/grpc/auth.rs`
- Test: `crates/canopy-server/tests/auth_domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `auth_challenges` with `password_reset`, password hasher, active session authentication.
- Produces: `RequestPasswordReset`, `CompletePasswordReset`, and `ChangePassword`.

- [x] **Step 1: Write failing tests**

Required behaviors:
- reset request always returns generic accepted
- reset token is single-use
- reset token expiry is enforced
- reset changes password only after valid token
- change password requires valid current access token and old password
- password hash metadata/policy version updates
- all sessions are revoked after password reset/change except the current session only if explicitly chosen by policy

- [x] **Step 2: Implement reset challenge creation**

Store only token digest, expiry, type, and encrypted outbox payload.

- [x] **Step 3: Implement atomic reset consumption**

Use one transaction to consume the challenge, update password hash, and revoke sessions.

- [x] **Step 4: Implement authenticated change password**

Verify access token, recheck session, verify current password, write new hash, revoke other sessions or all sessions according to the chosen policy.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p canopy-server --test auth_domain --locked
bash scripts/test-pg.sh
cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings
```

---

### Task 4: Google Login And Explicit Linking

**Status: Complete.**

**Files:**
- Modify: `crates/canopy-server/src/identity/service.rs`
- Create: `crates/canopy-server/src/identity/google.rs`
- Modify: `crates/canopy-core/src/identity_repository.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/api/grpc/auth.rs`
- Test: `crates/canopy-server/tests/auth_domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `external_identities(provider, provider_subject)`, `auth_challenges` with `google_login_nonce` and `google_link`.
- Produces: `BeginGoogleLogin`, `CompleteGoogleLogin`, `LinkGoogle`, `UnlinkGoogle`.

- [x] **Step 1: Add OIDC verifier trait**

```rust
#[async_trait]
pub trait OidcVerifier: Send + Sync {
    async fn verify_google_id_token(&self, id_token: &str, nonce: &str) -> CanopyResult<GoogleIdentity>;
}

pub struct GoogleIdentity {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}
```

- [x] **Step 2: Write failing tests**

Required behaviors:
- Google identity key is `sub`, not email
- known `(google, sub)` logs in
- unknown `(google, sub)` creates a new active account only when provider proof is valid
- same email with different `sub` returns account-link-required path, not auto-merge
- `LinkGoogle` requires an active existing account session plus link challenge consumption

- [x] **Step 3: Implement repository operations**

Use uniqueness on `(provider, provider_subject)`. Do not query by provider email as identity.

- [x] **Step 4: Wire gRPC**

Return `GoogleLoginResponse.session` for success or `GoogleLoginResponse.account_link_required` for explicit linking.

- [x] **Step 5: Verify**

Run:

```bash
cargo test -p canopy-server --test auth_domain --locked
bash scripts/test-pg.sh
cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings
```

---

### Task 5: Account Lookup And Idempotent Deletion

**Status: Complete.**

**Files:**
- Modify: `crates/canopy-core/src/identity_repository.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/api/grpc/auth.rs`
- Modify profile/media deletion boundaries only as needed.
- Test: `crates/canopy-server/tests/auth_domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: authenticated identity access token and `profiles.account_id`.
- Produces: `GetAccount` and `DeleteAccount`.

- [x] **Step 1: Write failing tests**

Required behaviors:
- `GetAccount` returns the authenticated account summary
- `DeleteAccount` is idempotent
- deletion revokes all sessions immediately
- deletion purges or delegates profile-owned app state according to current profile deletion policy
- deleting the instance owner is rejected if existing profile policy rejects it

- [x] **Step 2: Implement repository transaction**

Lock the account row. Treat `deletion_pending` and `deleted` as success. Revoke sessions and consume outstanding refresh tokens inside the same transaction.

- [x] **Step 3: Wire gRPC**

Require `authorization: Bearer <identity access token>` for both methods.

- [x] **Step 4: Verify**

Run:

```bash
cargo test -p canopy-server --test auth_domain --locked
bash scripts/test-pg.sh
cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings
```

---

### Task 6: Roll Native Identity Into Durable App RPCs

**Status: Complete.**

**Files:**
- Modify: `crates/canopy-server/src/api/grpc/mod.rs`
- Modify: `crates/canopy-server/src/api/grpc/profile.rs`
- Modify: `crates/canopy-server/src/api/grpc/history.rs`
- Modify: `crates/canopy-server/src/api/grpc/library.rs`
- Modify: `crates/canopy-server/src/api/grpc/playlist.rs`
- Modify: durable services only where profile lookup must move from external user id to `account_id`.
- Test: adapter/unit tests under `crates/canopy-server/src/api/grpc/*`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `IdentityService::authenticate_access_token`.
- Produces: one durable auth boundary for profile/history/library/likes/preferences/playlists.

- [x] **Step 1: Add native identity metadata extractor**

Create a helper that returns:

```rust
pub struct DurablePrincipal {
    pub account_id: String,
    pub session_id: String,
    pub profile_id: String,
}
```

It must verify token signature and recheck session revocation in PostgreSQL.

- [x] **Step 2: Write failing adapter tests**

For each durable adapter:
- missing metadata returns unauthenticated
- revoked session returns unauthenticated
- valid identity resolves to profile/account scope
- cross-profile access returns not found

- [x] **Step 3: Replace legacy verifier usage in durable adapters**

Keep anonymous browse/search/playback unchanged. Only durable state moves to native identity.

- [x] **Step 4: Decide legacy fallback sunset**

Either remove `x-canopy-auth-token`/request-body fallback from durable RPCs or explicitly keep it behind compatibility documentation. Prefer removal for state-of-the-art auth unless PandaEngine still needs it.

- [x] **Step 5: Verify**

Run:

```bash
cargo test --workspace --locked
bash scripts/test-pg.sh
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

---

### Task 7: Rate Limiting And Abuse Controls

**Status: Complete.**

**Files:**
- Modify: `crates/canopy-core/src/identity_repository.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Test: `crates/canopy-server/tests/auth_domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `auth_rate_limits`.
- Produces: throttling hooks for registration, login, resend, reset, Google failure, refresh abuse.

- [x] **Step 1: Write failing tests**

Required behaviors:
- repeated login failures for the same normalized email are throttled
- reset/resend requests are throttled with generic accepted responses where applicable
- refresh-token reuse still revokes immediately and records abuse signal

- [x] **Step 2: Implement rate-limit repository methods**

Use operation + subject hash + fixed window. Never store raw email/IP/device identifiers in the rate-limit table.

- [x] **Step 3: Wire service policy**

Apply checks before expensive password/OIDC verification where possible.

- [x] **Step 4: Verify**

Run:

```bash
cargo test -p canopy-server --test auth_domain --locked
bash scripts/test-pg.sh
cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings
```

---

### Task 8: Documentation And Contract Transparency

**Status: Complete for the implemented RPC contract.** Runtime SMTP delivery and documentation are complete under Task 9.

**Files:**
- Modify: `README.md`
- Modify: `../canopy-api/openapi/openapi.json`
- Modify: `docs/canopy-api-bsr-design.md`
- Modify: `docs/superpowers/specs/2026-07-03-authentication-design.md`
- Modify: `docs/superpowers/plans/2026-07-03-authentication-implementation.md` or mark completed/obsolete items.

**Interfaces:**
- Consumes: implemented AuthService behavior.
- Produces: docs that match runtime behavior and clarify pending work.

- [x] **Step 1: Update README status table**

Mark native auth as implemented only after Tasks 1-7 are done. Keep partial status if Google/reset/deletion/rate limiting are still pending.

- [x] **Step 2: Update OpenAPI companion**

Ensure every implemented AuthService RPC has request/response schemas, metadata security, and accurate error notes. Ensure OpenAPI says it is not the client-generation source.

- [x] **Step 3: Clean recommendations and plans**

Completed recommendations can be marked completed. Obsolete recommendations can be removed.

- [x] **Step 4: Verify docs**

Run:

```powershell
Get-Content -Path '../canopy-api/openapi/openapi.json' -Raw | ConvertFrom-Json | Out-Null
rg -n "Supabase|not implemented|TODO|Remaining: implement AuthService" README.md docs ../canopy-api
```

---

### Task 9: Supervised SMTP Outbox Delivery

**Status: Complete.**

**Detailed implementation plan:** [2026-07-11-auth-email-delivery-implementation.md](2026-07-11-auth-email-delivery-implementation.md)

**Files:**
- Create: `crates/canopy-server/src/identity/email.rs`
- Modify: `crates/canopy-server/src/identity/outbox.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/config.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Modify: `README.md`
- Test: `crates/canopy-server/tests/pg_integration.rs`
- Test: focused worker/config unit tests

**Interfaces:**
- Consumes: committed `auth_outbox` rows and `EmailOutboxPayload::open`.
- Produces: post-commit SMTP delivery, bounded retries, idempotent row claiming, and removal of encrypted secret material after success.

- [x] **Step 1: Confirm startup policy**

PostgreSQL mode requires SMTP configuration by default. `CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true` is the explicit development-only escape hatch that leaves mail queued and delivery readiness degraded. Production fails closed while password registration/recovery is enabled.

- [x] **Step 2: Write failing worker and repository tests**

Cover concurrent row claiming, retry scheduling, successful delivery, process restart, and clearing encrypted payload bytes after success. Assert logs and errors never contain recipients, tokens, or message bodies.

- [x] **Step 3: Add atomic outbox claim/complete/fail operations**

Use PostgreSQL row locking with `FOR UPDATE SKIP LOCKED` or an equivalent lease so multiple server processes cannot deliver the same row concurrently. Failed attempts must use bounded backoff; exhausted rows remain inspectable without retaining raw decrypted material.

- [x] **Step 4: Add the SMTP adapter and message templates**

Use TLS, validated from-address configuration, and separate verification/reset templates. Decrypt only in worker memory and never derive telemetry fields from sensitive template variables.

- [x] **Step 5: Supervise the worker and expose readiness**

Start the worker only after PostgreSQL connection and configuration validation. Coordinate shutdown with the server and report SMTP/outbox readiness separately from liveness.

- [x] **Step 6: Document and verify**

Document SMTP, retry, readiness, and development-mode settings in Canopy. No protobuf change is expected because delivery is internal runtime behavior.

Run:

```bash
cargo fmt --all
cargo test --workspace --locked
bash scripts/test-pg.sh
cargo clippy --workspace --all-features --tests --locked -- -D warnings
```

---

## Recommended Execution Order

1. Tasks 1-9: complete.
2. Downstream PandaEngine adoption and CI verification remain separate repository/integration work.

## Self-Review

- Spec coverage: covers production keys, email/resend, reset/change, Google/linking, account/deletion, durable RPC rollout, rate limiting, and docs.
- Placeholder scan: no intentional TBD placeholders; each task has concrete behavior and verification commands.
- Type consistency: task interfaces use existing names where already implemented and introduce new names only inside the task that owns them.

