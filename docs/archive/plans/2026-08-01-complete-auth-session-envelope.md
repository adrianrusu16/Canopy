# Complete Auth Session Envelope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Return authoritative access-token expiry and complete account/session timestamps from every Canopy authentication response.

**Architecture:** Token issuance returns a structured value containing both the JWT and its expiry, which the identity service carries in its domain envelope. Persistent session timestamps are added to `AuthSession`, populated by every PostgreSQL session projection, and mapped without defaults into protobuf responses shared by envelopes, account lookup, and session listing.

**Tech Stack:** Rust, Tokio, Tonic/prost, SQLx, PostgreSQL 17, Cargo tests

## Global Constraints

- Keep the existing protobuf wire contract and field numbers unchanged.
- Keep the existing PostgreSQL schema unchanged.
- Preserve PandaEngine's fail-closed handling; no client changes are in scope.
- Access expiry must equal the JWT `exp` claim expressed in epoch milliseconds.
- Account and session protobuf timestamps must be present on successful responses.
- Follow red-green-refactor and do not change production code before the relevant failing test.
- Preserve unrelated working-tree changes in `scripts/`, `.serena/`, and `docs/`.

---

## File Structure

- `crates/canopy-server/src/identity/tokens.rs`: define `IssuedAccessToken` and make issuance return token plus authoritative expiry.
- `crates/canopy-server/src/identity/mod.rs`: re-export `IssuedAccessToken` with the other identity token types.
- `crates/canopy-server/src/identity/service.rs`: carry access expiry in the domain `SessionEnvelope` for all issuance paths.
- `crates/canopy-server/tests/token_primitives.rs`: prove returned expiry matches verified JWT claims.
- `crates/canopy-core/src/identity.rs`: add persistent creation and last-use timestamps to `AuthSession`.
- `crates/canopy-core/tests/identity.rs`: update session fixtures while preserving lifecycle assertions.
- `crates/canopy-server/src/jade_store/pg_identity.rs`: populate timestamps from each SQL session projection.
- `crates/canopy-server/tests/pg_integration.rs`: verify creation, refresh, and listing preserve stored timestamps.
- `crates/canopy-server/tests/auth_domain.rs`: update fake sessions and assert complete gRPC envelopes/account/session summaries.
- `crates/canopy-server/src/api/grpc/auth.rs`: map all authoritative expiry and timestamp values.
- `crates/canopy-server/tests/local_integration.rs`: reject incomplete live verification, login, refresh, and list-session responses.

---

### Task 1: Return Authoritative Access-Token Expiry

**Files:**
- Modify: `crates/canopy-server/tests/token_primitives.rs`
- Modify: `crates/canopy-server/src/identity/tokens.rs`
- Modify: `crates/canopy-server/src/identity/mod.rs`
- Modify: `crates/canopy-server/src/identity/service.rs`
- Modify: `crates/canopy-server/tests/auth_domain.rs`

**Interfaces:**
- Produces: `IssuedAccessToken { token: String, expires_at_epoch_ms: u64 }`
- Changes: `Ed25519AccessTokenIssuer::issue(...) -> CanopyResult<IssuedAccessToken>`
- Changes: domain `SessionEnvelope` gains `access_expires_at_epoch_ms: u64`

- [ ] **Step 1: Write the failing token and envelope assertions**

Change the first token primitive test to consume a structured issuance result and assert exact expiry:

```rust
let issued = issuer.issue("account-1", "session-1", 1_000).unwrap();
let claims = issuer.verify(&issued.token, 1_100).unwrap();

assert_eq!(issued.expires_at_epoch_ms, 1_900_000);
assert_eq!(
    issued.expires_at_epoch_ms,
    claims.expires_at_epoch_seconds * 1_000
);
```

Update remaining token tests to pass `&issued.token` to `verify`. In
`refresh_session_rotates_presented_token_and_returns_replacement`, add:

```rust
assert_eq!(envelope.access_expires_at_epoch_ms, (NOW_MS / 1_000 + 900) * 1_000);
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test -p canopy-server --test token_primitives --test auth_domain
```

Expected: compilation fails because `issue` still returns `String` and the domain envelope has no `access_expires_at_epoch_ms` field.

- [ ] **Step 3: Implement structured issuance**

Add beside `AccessTokenClaims`:

```rust
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IssuedAccessToken {
    pub token: String,
    pub expires_at_epoch_ms: u64,
}
```

In `issue`, calculate one authoritative expiry and use it for both outputs:

```rust
let expires_at_epoch_seconds = now_epoch_seconds + self.config.ttl_seconds;
// payload.exp = expires_at_epoch_seconds
Ok(IssuedAccessToken {
    token: format!("{signing_input}.{signature}"),
    expires_at_epoch_ms: expires_at_epoch_seconds * 1_000,
})
```

Re-export `IssuedAccessToken` from `identity/mod.rs`. Add
`access_expires_at_epoch_ms: u64` to the domain envelope. In both
`IdentityService::session_envelope` and `IdentityService::verify_email`, destructure the issuance result and place its token and expiry in the returned envelope.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run:

```powershell
cargo test -p canopy-server --test token_primitives --test auth_domain
```

Expected: both test binaries pass with no warnings.

- [ ] **Step 5: Commit the token-expiry slice**

```powershell
git add crates/canopy-server/src/identity/tokens.rs crates/canopy-server/src/identity/mod.rs crates/canopy-server/src/identity/service.rs crates/canopy-server/tests/token_primitives.rs crates/canopy-server/tests/auth_domain.rs
git commit -m "fix: propagate identity access token expiry"
```

---

### Task 2: Preserve Persistent Session Timestamps

**Files:**
- Modify: `crates/canopy-core/src/identity.rs`
- Modify: `crates/canopy-core/tests/identity.rs`
- Modify: `crates/canopy-server/src/jade_store/pg_identity.rs`
- Modify: `crates/canopy-server/tests/auth_domain.rs`
- Modify: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Changes: `AuthSession` gains required `created_at_epoch_ms: u64` and `last_used_at_epoch_ms: u64`
- Preserves: `IdentityRepository` method signatures and `StoredAuthenticatedSession`

- [ ] **Step 1: Write failing PostgreSQL timestamp assertions**

In the session-management integration test, assert every newly returned and listed session has ordered, non-zero stored timestamps and the requested expiry:

```rust
assert!(activated.session.created_at_epoch_ms > 0);
assert!(activated.session.last_used_at_epoch_ms >= activated.session.created_at_epoch_ms);
assert_eq!(activated.session.expires_at_epoch_ms, expires_at);

assert!(listed.iter().all(|session| session.created_at_epoch_ms > 0));
assert!(listed
    .iter()
    .all(|session| session.last_used_at_epoch_ms >= session.created_at_epoch_ms));
assert!(listed
    .iter()
    .all(|session| session.expires_at_epoch_ms == expires_at));
```

In the refresh integration test, retain the successful rotation result and assert:

```rust
let refreshed = match (first, second) {
    (Ok(session), Err(_)) | (Err(_), Ok(session)) => session,
    (Ok(_), Ok(_)) => panic!("refresh token was rotated more than once"),
    (Err(first), Err(second)) => {
        panic!("both refresh attempts failed: {first:?}; {second:?}")
    }
};
assert_eq!(refreshed.session.last_used_at_epoch_ms, now);
assert_eq!(refreshed.session.expires_at_epoch_ms, expires_at);
```

- [ ] **Step 2: Run PostgreSQL integration and verify RED**

Run:

```powershell
cargo test -p canopy-server --features pg --test pg_integration postgres_identity_sessions_can_be_listed_and_revoked -- --ignored
```

Expected: compilation fails because `AuthSession` lacks `created_at_epoch_ms` and `last_used_at_epoch_ms`.

- [ ] **Step 3: Extend `AuthSession` and all test fixtures**

Add the required fields before expiry:

```rust
/// Creation time in Unix epoch milliseconds.
pub created_at_epoch_ms: u64,
/// Most recent successful refresh/use time in Unix epoch milliseconds.
pub last_used_at_epoch_ms: u64,
```

Update all literal fixtures in `canopy-core/tests/identity.rs` and
`canopy-server/tests/auth_domain.rs` with deterministic values such as:

```rust
created_at_epoch_ms: NOW_MS,
last_used_at_epoch_ms: NOW_MS,
```

- [ ] **Step 4: Populate timestamp fields from PostgreSQL**

Update `session_from_row`:

```rust
created_at_epoch_ms: epoch_ms(row, "session_created_at_epoch_ms"),
last_used_at_epoch_ms: epoch_ms(row, "last_used_at_epoch_ms"),
expires_at_epoch_ms: epoch_ms(row, "expires_at_epoch_ms"),
```

In each of the five queries consumed by `session_from_row`—activation,
creation/login, refresh, listing, and Google account creation—select:

```sql
floor(extract(epoch from s.created_at) * 1000)::bigint AS session_created_at_epoch_ms,
floor(extract(epoch from s.last_used_at) * 1000)::bigint AS last_used_at_epoch_ms,
floor(extract(epoch from s.expires_at) * 1000)::bigint AS expires_at_epoch_ms,
```

- [ ] **Step 5: Run core, domain, and PostgreSQL tests and verify GREEN**

Run:

```powershell
cargo test -p canopy-core
cargo test -p canopy-server --test auth_domain
cargo test -p canopy-server --features pg --test pg_integration postgres_identity_sessions_can_be_listed_and_revoked -- --ignored
```

Expected: all commands pass without missing-column errors.

- [ ] **Step 6: Commit the persistent timestamp slice**

```powershell
git add crates/canopy-core/src/identity.rs crates/canopy-core/tests/identity.rs crates/canopy-server/src/jade_store/pg_identity.rs crates/canopy-server/tests/auth_domain.rs crates/canopy-server/tests/pg_integration.rs
git commit -m "fix: preserve identity session timestamps"
```

---

### Task 3: Map Complete gRPC Authentication Resources

**Files:**
- Modify: `crates/canopy-server/tests/auth_domain.rs`
- Modify: `crates/canopy-server/src/api/grpc/auth.rs`
- Modify: `crates/canopy-server/tests/local_integration.rs`

**Interfaces:**
- Consumes: domain `SessionEnvelope.access_expires_at_epoch_ms`
- Consumes: `AuthSession.created_at_epoch_ms`, `last_used_at_epoch_ms`, and `expires_at_epoch_ms`
- Produces: complete protobuf `SessionEnvelope`, `AccountSummary`, and `SessionSummary`

- [ ] **Step 1: Strengthen gRPC regression assertions**

In `auth_grpc_maps_verify_email_to_session_envelope`, keep the nested resources before asserting them and add:

```rust
assert_eq!(response.access_expires_at_epoch_ms, (NOW_MS / 1_000 + 900) as i64 * 1_000);
assert_eq!(response.refresh_expires_at_epoch_ms, (NOW_MS + 86_400_000) as i64);

let account = response.account.expect("account is required");
assert_eq!(account.created_at, Some(timestamp_from_test_epoch_ms(NOW_MS)));

let session = response.session.expect("session is required");
assert_eq!(session.created_at, Some(timestamp_from_test_epoch_ms(NOW_MS)));
assert_eq!(session.last_used_at, Some(timestamp_from_test_epoch_ms(NOW_MS)));
assert_eq!(
    session.expires_at,
    Some(timestamp_from_test_epoch_ms(NOW_MS + 86_400_000))
);
```

Add a small test-only timestamp helper returning `prost_types::Timestamp`. In the session-management test, assert all three timestamp options are `Some` for every listed session. In the authenticated account test, assert `account.created_at.is_some()`.

- [ ] **Step 2: Run auth-domain tests and verify RED**

Run:

```powershell
cargo test -p canopy-server --test auth_domain auth_grpc_
```

Expected: assertions fail because the gRPC mapper still emits zero and `None`.

- [ ] **Step 3: Implement the complete mapper**

Replace transport defaults with domain values:

```rust
access_expires_at_epoch_ms: i64::try_from(envelope.access_expires_at_epoch_ms)
    .unwrap_or(i64::MAX),
created_at: Some(timestamp_from_epoch_ms(account.created_at_epoch_ms)),
```

and in `to_proto_session`:

```rust
created_at: Some(timestamp_from_epoch_ms(session.created_at_epoch_ms)),
last_used_at: Some(timestamp_from_epoch_ms(session.last_used_at_epoch_ms)),
expires_at: Some(timestamp_from_epoch_ms(session.expires_at_epoch_ms)),
```

- [ ] **Step 4: Run auth-domain tests and verify GREEN**

Run:

```powershell
cargo test -p canopy-server --test auth_domain auth_grpc_
```

Expected: all filtered gRPC tests pass.

- [ ] **Step 5: Strengthen the ignored live integration contract**

Add a shared assertion helper:

```rust
fn assert_complete_session_envelope(envelope: &SessionEnvelope) {
    assert!(envelope.access_expires_at_epoch_ms > 0);
    assert!(envelope.refresh_expires_at_epoch_ms > 0);
    assert!(envelope.account.as_ref().and_then(|value| value.created_at.as_ref()).is_some());
    let session = envelope.session.as_ref().expect("session is required");
    assert!(session.created_at.is_some());
    assert!(session.last_used_at.is_some());
    assert!(session.expires_at.is_some());
}
```

Call it for `verified`, `logged_in`, and `refreshed`. Capture the
`ListSessions` response and assert every listed session has all timestamps.

- [ ] **Step 6: Compile the live integration test**

Run:

```powershell
cargo test -p canopy-server --test local_integration --no-run
```

Expected: compilation succeeds. Do not launch or modify the local environment as part of this step.

- [ ] **Step 7: Commit the gRPC contract slice**

```powershell
git add crates/canopy-server/src/api/grpc/auth.rs crates/canopy-server/tests/auth_domain.rs crates/canopy-server/tests/local_integration.rs
git commit -m "fix: return complete auth session resources"
```

---

### Task 4: Full Verification

**Files:**
- Verify only; no planned production changes

**Interfaces:**
- Verifies all prior task outputs as one server contract

- [ ] **Step 1: Format and verify formatting**

Run:

```powershell
cargo fmt --all
cargo fmt --all --check
```

Expected: the check exits successfully with no output.

- [ ] **Step 2: Run focused unit and integration tests**

Run:

```powershell
cargo test -p canopy-core
cargo test -p canopy-server --test token_primitives
cargo test -p canopy-server --test auth_domain
cargo test -p canopy-server --test local_integration --no-run
```

Expected: all runnable tests pass and the ignored live test compiles.

- [ ] **Step 3: Run the server suite**

Run:

```powershell
cargo test -p canopy-server
```

Expected: all non-ignored server tests pass.

- [ ] **Step 4: Run Clippy with warnings denied**

Run:

```powershell
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: exit code zero with no warnings.

- [ ] **Step 5: Inspect the final diff and working tree**

Run:

```powershell
git diff --check
git status --short
```

Expected: no whitespace errors; only intentional auth fix files plus the user's pre-existing unrelated changes are present.
