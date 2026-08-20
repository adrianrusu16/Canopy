# Canopy Authentication Implementation Plan

> **Status (2026-07-11): Superseded.** This original plan is retained as historical design context. Its unchecked boxes do not describe current implementation status. Use [`2026-07-09-authentication-completion.md`](2026-07-09-authentication-completion.md) for the reconciled status and remaining SMTP delivery work.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add production-grade native password and Google authentication to Canopy with verified accounts, rotating per-device sessions, immediate revocation, recovery email, explicit account linking, and an additive BSR contract.

**Architecture:** Authentication is a bounded module inside the Canopy modular monolith. `canopy-api` owns the Protobuf contract; `canopy-core` owns identity types and repository ports; `canopy-server` owns policy services, PostgreSQL/SMTP/Google adapters, gRPC translation, and runtime wiring. Profiles remain application state and are created only when an account activates.

**Tech Stack:** Rust 2024, tonic/prost from BSR, SQLx/PostgreSQL, Argon2id, Ed25519 JWT access tokens, opaque rotating refresh tokens, SHA-256 token digests, AEAD-encrypted outbox payloads, lettre SMTP, reqwest Google OIDC/JWKS, Tokio, tracing.

**Git rule:** Do not stage, commit, push, or create branches. The user owns every Git checkpoint.

---

## File Map

### Canonical contract repository: `/home/catalina/projects/canopy-api`

- Modify `proto/canopy/v1/canopy.proto`: add `AuthService`, account/session resources, requests, and responses.
- Modify `openapi/openapi.json`: regenerate the readable companion after Protobuf approval.
- Modify `CHANGELOG.md`: record the additive `canopy.v1` authentication contract.

### Canopy core

- Create `crates/canopy-core/src/identity.rs`: account, email, credential, session, challenge, and auth-result domain types.
- Create `crates/canopy-core/src/identity_repository.rs`: transaction-oriented identity repository port.
- Modify `crates/canopy-core/src/error.rs`: identity-safe domain errors mapped later to canonical gRPC statuses.
- Modify `crates/canopy-core/src/lib.rs`: export the identity boundary.

### Canopy server

- Create `crates/canopy-server/src/identity/mod.rs`: module exports and dependency bundle.
- Create `crates/canopy-server/src/identity/service.rs`: authentication use cases and policy.
- Create `crates/canopy-server/src/identity/password.rs`: Argon2id adapter and password policy.
- Create `crates/canopy-server/src/identity/tokens.rs`: access/refresh/challenge token generation and verification.
- Create `crates/canopy-server/src/identity/google.rs`: Google discovery/JWKS and ID-token verification.
- Create `crates/canopy-server/src/identity/email.rs`: `EmailSender`, encrypted outbox payload, and SMTP adapter.
- Create `crates/canopy-server/src/identity/rate_limit.rs`: rate-limit policy and PostgreSQL-backed adapter.
- Create `crates/canopy-server/src/jade_store/pg_identity.rs`: SQLx identity repository and atomic transactions.
- Create `crates/canopy-server/src/api/grpc/auth.rs`: bounded `AuthService` adapter.
- Modify `crates/canopy-server/src/api/grpc/mod.rs`: export adapter and session-aware principal extraction.
- Modify `crates/canopy-server/src/auth.rs`: replace legacy HMAC token semantics with access-token verification facade.
- Modify `crates/canopy-server/src/config.rs`: fail-closed auth, Google, SMTP, key, lifetime, and rate-limit config.
- Modify `crates/canopy-server/src/health.rs`: auth dependency readiness.
- Modify `crates/canopy-server/src/lib.rs`: dependency construction, AuthService registration, and outbox worker lifecycle.
- Modify `crates/canopy-server/Cargo.toml`: add security/OIDC/SMTP dependencies.

### Persistence, tests, and docs

- Create `migrations/20260703000002_identity_auth.sql`: identity schema, constraints, indexes, and profile relationship.
- Create `crates/canopy-server/tests/auth_contract.rs`: public gRPC contract and token-exposure tests.
- Create `crates/canopy-server/tests/auth_domain.rs`: native/Google/session lifecycle integration tests.
- Modify `crates/canopy-server/tests/pg_integration.rs`: transactional race and database-constraint tests.
- Modify `crates/canopy-server/tests/proto_contract.rs`: verify AuthService registration and additive contract shape.
- Modify `scripts/test-pg.sh`: run the auth database suite.
- Modify `README.md`: authentication setup, flows, environment, and operator guidance.
- Modify `docs/openapi.json`: sync the companion contract.

---

### Task 1: Publish The Additive Auth Contract

**Files:**
- Modify: `/home/catalina/projects/canopy-api/proto/canopy/v1/canopy.proto`
- Modify: `/home/catalina/projects/canopy-api/CHANGELOG.md`
- Test: `/home/catalina/projects/canopy-api/proto/canopy/v1/canopy.proto`

- [ ] **Step 1: Add a failing contract assertion in Canopy**

Create `crates/canopy-server/tests/auth_contract.rs` with a compile-time import that is absent from the current SDK:

```rust
use canopy_proto::auth_service_client::AuthServiceClient;
use canopy_proto::{AccountSummary, SessionEnvelope};

#[test]
fn auth_contract_exports_service_and_session_resources() {
    fn accepts_client<T>(_client: Option<AuthServiceClient<T>>) {}
    accepts_client::<tonic::transport::Channel>(None);
    assert_eq!(AccountSummary::default().id, "");
    assert_eq!(SessionEnvelope::default().refresh_token, "");
}
```

- [ ] **Step 2: Verify the contract test fails for the missing SDK symbols**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_contract --locked
```

Expected: compilation fails because `AuthServiceClient`, `AccountSummary`, and `SessionEnvelope` do not exist.

- [ ] **Step 3: Add the complete Protobuf contract**

Add `AuthService` with these RPCs:

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

Use `oneof` for `GoogleLoginResponse`, return only an opaque link challenge in `AccountLinkRequired`, and place `refresh_token` only in `SessionEnvelope`. Use UUID strings and epoch milliseconds consistently with the current contract.

- [ ] **Step 4: Validate the canonical module and breaking compatibility**

Run from `/home/catalina/projects/canopy-api`:

```bash
/home/catalina/.local/bin/buf format -w
/home/catalina/.local/bin/buf lint
/home/catalina/.local/bin/buf build
/home/catalina/.local/bin/buf breaking --against buf.build/pandawave/canopy-api:v0.1.0
```

Expected: all commands exit 0 and the update is additive.

- [ ] **Step 5: Publish only after explicit external-write approval**

Push the private module with the next release label, record the returned immutable BSR commit, and update `CHANGELOG.md`. Do not attach Git metadata unless separately approved.

- [ ] **Step 6: Pin the generated Prost and Tonic SDKs in Canopy**

Update `crates/canopy-proto/Cargo.toml` to exact generated versions for the new BSR commit, then run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_contract --locked
```

Before running the test, copy the exact Prost and Tonic versions displayed by
the BSR generated-SDK page into `crates/canopy-proto/Cargo.toml`, then run
`cargo update` without `--precise` to refresh the lockfile from those exact
manifest pins. Expected: the contract test compiles and passes. Record the same
immutable versions in `docs/canopy-api-bsr-design.md`.

### Task 2: Add The Identity Schema And Constraints

**Files:**
- Create: `migrations/20260703000002_identity_auth.sql`
- Modify: `crates/canopy-server/tests/pg_integration.rs`
- Modify: `scripts/test-pg.sh`

- [ ] **Step 1: Add failing PostgreSQL schema tests**

Add tests that assert:

```rust
assert!(insert_duplicate_active_normalized_email(&pool).await.is_err());
assert!(insert_duplicate_google_subject(&pool).await.is_err());
assert!(insert_second_primary_email(&pool).await.is_err());
assert!(insert_refresh_token_with_wrong_digest_length(&pool).await.is_err());
```

Use repository test helpers rather than embedding production SQL in assertions.

- [ ] **Step 2: Verify the schema tests fail before migration**

Run:

```bash
bash scripts/test-pg.sh pg_identity_schema
```

Expected: failure because identity tables do not exist.

- [ ] **Step 3: Create enums and identity tables**

The migration must create account/challenge enums and the tables defined by the approved design: `accounts`, `account_emails`, `password_credentials`, `external_identities`, `auth_sessions`, `auth_session_tokens`, `auth_challenges`, `auth_outbox`, and `auth_rate_limits`.

Use partial unique indexes:

```sql
CREATE UNIQUE INDEX account_emails_active_email_uq
ON account_emails (normalized_email)
WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX account_emails_one_primary_uq
ON account_emails (account_id)
WHERE is_primary AND deleted_at IS NULL;

CREATE UNIQUE INDEX external_identities_provider_subject_uq
ON external_identities (provider, provider_subject);
```

Store `password_hash_phc`, token digests, and encrypted outbox/challenge payloads as bounded text or byte arrays. Add checks for nonempty digest/ciphertext lengths, expiry after creation, nonnegative attempts, and valid lifecycle timestamps.

- [ ] **Step 4: Connect accounts to profiles without merging responsibilities**

Add a nullable unique `profiles.account_id` foreign key for migration compatibility. New activation transactions populate it. Preserve existing profile IDs and application-data ownership.

- [ ] **Step 5: Run migration and constraint tests**

Run:

```bash
bash scripts/test-pg.sh pg_identity_schema
```

Expected: PASS, including duplicate-email, duplicate-subject, primary-email, and foreign-key checks.

### Task 3: Define Core Identity Types And Repository Ports

**Files:**
- Create: `crates/canopy-core/src/identity.rs`
- Create: `crates/canopy-core/src/identity_repository.rs`
- Modify: `crates/canopy-core/src/error.rs`
- Modify: `crates/canopy-core/src/lib.rs`

- [ ] **Step 1: Write failing lifecycle tests**

Define desired behavior first:

```rust
#[test]
fn only_pending_accounts_can_activate() {
    assert_eq!(AccountStatus::PendingEmailVerification.activate(), Ok(AccountStatus::Active));
    assert!(AccountStatus::Disabled.activate().is_err());
}

#[test]
fn session_envelope_requires_active_account_and_session() {
    let result = AuthenticatedSession::new(disabled_account(), active_session(), token_pair());
    assert!(matches!(result, Err(CanopyError::FailedPrecondition(_))));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-core identity --locked
```

Expected: compile failure because identity types do not exist.

- [ ] **Step 3: Implement focused domain types**

Create strongly typed IDs and enums for account status, challenge type, provider, session/revocation state, public account/session summaries, and token pair. Keep raw secrets in zeroizable/newtype wrappers that do not implement `Debug` or `Display`.

- [ ] **Step 4: Define transaction-oriented repository methods**

The port should expose use-case operations rather than generic CRUD:

```rust
#[async_trait::async_trait]
pub trait IdentityRepository: Send + Sync {
    async fn register_password(&self, command: RegisterPasswordRecord) -> CanopyResult<()>;
    async fn activate_email(&self, command: ActivateEmailCommand) -> CanopyResult<ActivatedAccount>;
    async fn rotate_refresh_token(&self, command: RotateRefreshTokenCommand) -> CanopyResult<RotatedSession>;
    async fn validate_active_session(&self, account_id: &AccountId, session_id: &SessionId) -> CanopyResult<()>;
    async fn revoke_session(&self, account_id: &AccountId, session_id: &SessionId) -> CanopyResult<()>;
    async fn revoke_all_sessions(&self, account_id: &AccountId, reason: RevocationReason) -> CanopyResult<()>;
}
```

Add the remaining password, Google, challenge, session-list, and deletion operations with similarly bounded commands/results.

- [ ] **Step 5: Make core tests pass**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-core --locked
```

Expected: PASS.

### Task 4: Implement Password And Token Primitives

**Files:**
- Create: `crates/canopy-server/src/identity/password.rs`
- Create: `crates/canopy-server/src/identity/tokens.rs`
- Create: `crates/canopy-server/src/identity/mod.rs`
- Modify: `crates/canopy-server/Cargo.toml`

- [ ] **Step 1: Write failing password-policy and rehash tests**

```rust
#[test]
fn password_policy_accepts_long_passphrases_without_composition_rules() {
    assert!(PasswordPolicy::default().validate("correct horse battery staple").is_ok());
}

#[test]
fn password_policy_rejects_fewer_than_fifteen_characters() {
    assert!(PasswordPolicy::default().validate("short password").is_err());
}

#[test]
fn successful_verify_requests_rehash_for_old_policy() {
    let result = hasher.verify(old_policy_hash(), "correct horse battery staple").unwrap();
    assert!(result.needs_rehash);
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server identity::password --locked
```

Expected: compile failure for missing policy/hasher.

- [ ] **Step 3: Implement Argon2id using PHC strings**

Use `argon2::Argon2` with configured memory/time/parallelism. Parse and verify PHC strings through `password-hash`; compare policy parameters to set `needs_rehash`. Run hashing and verification with `spawn_blocking` so CPU work does not block Tokio workers.

- [ ] **Step 4: Write failing token tests**

Cover Ed25519 access claims, wrong issuer/audience/key ID, expiry, opaque 256-bit refresh/challenge tokens, stable digests, and debug redaction.

- [ ] **Step 5: Implement token primitives**

Access claims must include `iss`, `aud`, `sub`, `sid`, `iat`, `exp`, `jti`, and `kid`. Refresh/challenge tokens use OS randomness and store only SHA-256 digests. Secret wrappers must render as `[REDACTED]` under `Debug`.

- [ ] **Step 6: Run primitive tests**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server identity::password --locked
/home/catalina/.cargo/bin/cargo test -p canopy-server identity::tokens --locked
```

Expected: PASS with no warnings.

### Task 5: Implement Atomic PostgreSQL Identity Operations

**Files:**
- Create: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/jade_store/mod.rs`
- Modify: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Add a concurrent challenge-consumption test**

Spawn two tasks against one verification challenge and assert exactly one activation succeeds while the other receives the generic consumed/invalid result.

- [ ] **Step 2: Add a concurrent refresh-rotation test**

Spawn two refreshes with one token. Assert one returns a replacement and the other detects reuse and leaves the token family revoked.

- [ ] **Step 3: Verify both tests fail**

Run:

```bash
bash scripts/test-pg.sh pg_identity_atomicity
```

Expected: failure for missing repository operations.

- [ ] **Step 4: Implement conditional challenge consumption**

Use one transaction and a guarded update equivalent to:

```sql
UPDATE auth_challenges
SET consumed_at = now(), attempts = attempts + 1
WHERE id = $1
  AND token_hash = $2
  AND challenge_type = $3
  AND consumed_at IS NULL
  AND expires_at > now()
  AND attempts < $4
RETURNING account_id, email_id, encrypted_payload;
```

Perform activation/profile creation and challenge consumption in the same transaction.

- [ ] **Step 5: Implement locked refresh rotation and family replay revocation**

Lock the session and token row with `FOR UPDATE`. If the matching token was already consumed, revoke `auth_sessions.revoked_at` and every live family token before returning generic `Unauthenticated`. Otherwise consume it and insert exactly one replacement before commit.

- [ ] **Step 6: Run PostgreSQL atomicity tests**

Run:

```bash
bash scripts/test-pg.sh pg_identity_atomicity
```

Expected: PASS repeatedly, including under `--test-threads=1` and normal scheduling.

### Task 6: Implement Native Registration, Verification, Login, And Refresh

**Files:**
- Create: `crates/canopy-server/src/identity/service.rs`
- Create: `crates/canopy-server/src/api/grpc/auth.rs`
- Modify: `crates/canopy-server/src/api/grpc/mod.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Test: `crates/canopy-server/tests/auth_domain.rs`

- [ ] **Step 1: Write failing lifecycle tests**

Cover registration without a session, generic duplicate registration, verification creating profile/session, unverified login denial, successful login, old-hash upgrade, refresh rotation, and replay family revocation.

- [ ] **Step 2: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain native_ --locked
```

Expected: compilation or assertion failure because `IdentityService` is not implemented.

- [ ] **Step 3: Implement use cases without transport types**

`IdentityService` accepts domain commands and returns domain results. It normalizes email, checks rate limits, invokes `spawn_blocking` password work, calls transaction-oriented repository methods, and mints tokens only after committed state transitions.

- [ ] **Step 4: Implement the bounded gRPC adapter**

Map Protobuf requests to domain commands and domain errors to canonical statuses. Keep duplicate registration/resend/reset responses generic. Never include a refresh token outside `VerifyEmail`, `LoginPassword`, `CompleteGoogleLogin`, and `RefreshSession` success.

- [ ] **Step 5: Register AuthService on the existing Tonic listener**

Construct dependencies once in `run`, add `AuthServiceServer::new(AuthGrpc(...))`, and preserve coordinated graceful shutdown.

- [ ] **Step 6: Run native lifecycle and contract tests**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain native_ --locked
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_contract --locked
```

Expected: PASS.

### Task 7: Enforce Immediate Revocation On Protected Calls

**Files:**
- Modify: `crates/canopy-server/src/auth.rs`
- Modify: `crates/canopy-server/src/api/grpc/mod.rs`
- Modify: protected adapters under `crates/canopy-server/src/api/grpc/`
- Test: `crates/canopy-server/tests/auth_domain.rs`

- [ ] **Step 1: Write a failing revoked-session access test**

Authenticate, revoke the session, then invoke profile/library/history and owner-personal playback RPCs with the still-unexpired access token. Assert `Unauthenticated`. Assert anonymous public catalog/search/playback still succeeds without a session lookup.

- [ ] **Step 2: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain revoked_session --locked
```

Expected: protected calls incorrectly accept the locally valid token.

- [ ] **Step 3: Replace legacy HMAC identity extraction**

Verify access-token signature/claims locally, then call `validate_active_session` for every durable or owner-scoped operation. Return a principal containing both `account_id` and `session_id`. Keep identity absent for anonymous public operations.

- [ ] **Step 4: Run revocation and existing anonymous-boundary tests**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain revoked_session --locked
/home/catalina/.cargo/bin/cargo test -p canopy-server --test domain --locked
```

Expected: PASS with existing anonymous behavior preserved.

### Task 8: Add Encrypted Outbox, SMTP, Reset, And Password Change

**Files:**
- Create: `crates/canopy-server/src/identity/email.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Modify: `crates/canopy-server/tests/auth_domain.rs`

- [ ] **Step 1: Write failing outbox tests**

Assert that rollback produces no outbox row, committed registration produces ciphertext that does not contain the raw token/email body, worker retries are idempotent, and successful delivery clears encrypted secret material.

- [ ] **Step 2: Write failing reset/change tests**

Assert generic reset requests, atomic single-use completion, all-session revocation after reset, and all-session revocation plus fresh-login requirement after authenticated password change.

- [ ] **Step 3: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain email_ --locked
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain password_reset --locked
```

Expected: failure for missing outbox and recovery behavior.

- [ ] **Step 4: Implement AEAD outbox encryption and SMTP adapter**

Encrypt sensitive payloads with a versioned key ID, random nonce, and authenticated context binding the outbox ID/kind. Decrypt only in worker memory. Use lettre over TLS; never log recipients with tokens or message bodies.

- [ ] **Step 5: Implement a supervised outbox worker**

Claim rows with `FOR UPDATE SKIP LOCKED`, send after commit, mark delivered idempotently, clear ciphertext, and use bounded exponential retry. Worker failure degrades readiness rather than silently exiting.

- [ ] **Step 6: Implement reset and change use cases**

Consume reset challenges atomically, update PHC credential, and revoke all sessions in the same transaction. Password change requires the current password, performs the same credential/session transaction, and returns generic success without tokens.

- [ ] **Step 7: Run email and recovery tests**

Run the two focused commands from Step 3. Expected: PASS.

### Task 9: Add Google OIDC And Explicit Linking

**Files:**
- Create: `crates/canopy-server/src/identity/google.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/tests/auth_domain.rs`

- [ ] **Step 1: Write failing verifier tests with local JWKS fixtures**

Cover valid signature and every required rejection independently: unknown key ID, bad signature, issuer, audience, expiry, missing/empty `sub`, `email_verified=false`, and nonce mismatch.

- [ ] **Step 2: Write failing identity/link tests**

Cover login by existing `(google, sub)`, new-account creation for an unused verified email, generic account-link-required for an existing email, explicit authenticated linking, duplicate-sub conflict, and refusal to unlink the final method.

- [ ] **Step 3: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain google_ --locked
```

Expected: failure for missing Google adapter/use cases.

- [ ] **Step 4: Implement discovery/JWKS verification**

Cache keys with bounded TTL, refresh once on unknown `kid`, enforce configured issuer/client audience, and compare the ID-token nonce digest against the atomically consumed begin challenge. Never store or log the ID token.

- [ ] **Step 5: Implement explicit linking**

Store pending link claims only as authenticated ciphertext bound to a high-entropy link challenge. `LinkGoogle` requires both an active existing-account session and atomic consumption of that challenge. Email equality alone never links identities.

- [ ] **Step 6: Run Google tests**

Run the focused command from Step 3. Expected: PASS.

### Task 10: Add Session Management And Idempotent Account Deletion

**Files:**
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/src/api/grpc/auth.rs`
- Modify: `crates/canopy-server/tests/auth_domain.rs`

- [ ] **Step 1: Write failing session-management tests**

Cover list-own-sessions, revoke-own-session, non-enumerating foreign-session rejection, current logout, logout-all, repeated revocation, and immediate protected-call denial.

- [ ] **Step 2: Write failing deletion tests**

Assert first deletion revokes all sessions and invokes profile deletion, repeated deletion returns the same external success, and deleted accounts cannot refresh or authenticate.

- [ ] **Step 3: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain session_ --locked
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain delete_account --locked
```

Expected: failing assertions for missing use cases.

- [ ] **Step 4: Implement session/account operations transactionally**

Scope every query by authenticated `account_id`. Deletion locks the account, treats `deletion_pending/deleted` as success, revokes sessions, and calls the existing profile deletion boundary without duplicating application-data policy.

- [ ] **Step 5: Run focused tests**

Run both commands from Step 3. Expected: PASS.

### Task 11: Add Rate Limits, Configuration, Readiness, And Redaction

**Files:**
- Create: `crates/canopy-server/src/identity/rate_limit.rs`
- Modify: `crates/canopy-server/src/config.rs`
- Modify: `crates/canopy-server/src/health.rs`
- Modify: `crates/canopy-server/src/observability.rs`
- Modify: `crates/canopy-server/tests/auth_domain.rs`

- [ ] **Step 1: Write failing rate-limit tests**

Assert limits by operation and privacy-preserving digests of email/IP/device, enforcement before Argon2/Google work, window rollover, and `ResourceExhausted` retry metadata.

- [ ] **Step 2: Write failing configuration/readiness tests**

Assert production startup fails without signing keys; Google-enabled startup fails without client/issuer/JWKS config; password-flow startup fails without SMTP/outbox key; process health remains separate from dependency readiness.

- [ ] **Step 3: Verify RED**

Run:

```bash
/home/catalina/.cargo/bin/cargo test -p canopy-server config::tests::auth_ --locked
/home/catalina/.cargo/bin/cargo test -p canopy-server --test auth_domain rate_limit --locked
```

Expected: failure because auth configuration/rate limiting is absent.

- [ ] **Step 4: Implement PostgreSQL rate-limit buckets**

Use atomic upsert/increment on HMAC-digested dimensions. Never persist raw email/IP/device identifiers. Return retry-after duration without exposing the matched dimension.

- [ ] **Step 5: Implement fail-closed configuration and readiness**

Add explicit enable flags and validated secret/key material. No production signing, encryption, or SMTP secret may have a default. Readiness checks PostgreSQL, signing keys, outbox worker, SMTP configuration, and Google JWKS when enabled.

- [ ] **Step 6: Add redaction assertions**

Capture tracing output in tests and assert representative raw access, refresh, challenge, password, Google ID, and nonce values do not appear.

- [ ] **Step 7: Run focused operational tests**

Run the Step 3 commands plus redaction tests. Expected: PASS.

### Task 12: Synchronize Documentation And Run The Full Gate

**Files:**
- Modify: `README.md`
- Modify: `docs/openapi.json`
- Modify: `docs/canopy-api-bsr-design.md`
- Modify: `/home/catalina/projects/canopy-api/openapi/openapi.json`
- Modify: `/home/catalina/projects/canopy-api/CHANGELOG.md`

- [ ] **Step 1: Update operator and client documentation**

Document anonymous versus authenticated behavior, registration/verification, Google begin/complete/linking, session rotation/revocation, password recovery, deletion, all environment variables, SMTP/outbox behavior, and readiness semantics. State explicitly that Protobuf is canonical and OpenAPI is companion-only.

- [ ] **Step 2: Regenerate readable OpenAPI**

Regenerate both companion copies from the approved Protobuf tooling, format JSON, and assert no removed `canopy.v1` operations or stale Supabase/session-control material appears.

- [ ] **Step 3: Run formatting**

```bash
/home/catalina/.cargo/bin/cargo fmt --all -- --check
```

Expected: exit 0.

- [ ] **Step 4: Run locked Clippy**

```bash
/home/catalina/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 5: Run the locked workspace suite**

```bash
/home/catalina/.cargo/bin/cargo test --workspace --locked
```

Expected: all unit, integration, contract, and doc tests pass.

- [ ] **Step 6: Run PostgreSQL and streaming harnesses**

```bash
bash scripts/test-pg.sh
bash scripts/test-streaming.sh
```

Expected: both harnesses pass, including revoked-session personal-media checks.

- [ ] **Step 7: Revalidate the canonical contract**

From `/home/catalina/projects/canopy-api`:

```bash
/home/catalina/.local/bin/buf format --diff --exit-code
/home/catalina/.local/bin/buf lint
/home/catalina/.local/bin/buf build
/home/catalina/.local/bin/buf breaking --against buf.build/pandawave/canopy-api:v0.1.0
```

Expected: all commands exit 0.

- [ ] **Step 8: Present the unstaged changes to the user**

Run `git status --short --branch` and `git diff --stat` in both repositories. Confirm `git diff --cached --name-only` is empty. Do not stage, commit, tag Git, or push; the user handles repository history.

