# Auth Transport Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move authenticated profile-scoped RPCs to gRPC metadata auth while preserving request-body token fallback during migration.

**Architecture:** Add a focused auth extractor in `api::grpc` that reads `authorization: Bearer <token>`, then `x-canopy-auth-token`, then optional request-body fallback. `GrpcApi` owns `AuthService`, verifies tokens at the transport boundary, and passes `UserIdentity` into `ProfileService` and `HistoryService`. Anonymous RPCs keep bypassing auth extraction.

**Tech Stack:** Rust, Tokio, tonic metadata, async-trait, protobuf, Cargo workspace (`canopy-core`, `canopy-server`, `canopy-proto`).

---

## File Structure

- Modify `crates/canopy-server/src/api/grpc.rs`: add auth extractor helpers, tests, `AuthService` field, and authenticated RPC wiring.
- Modify `crates/canopy-server/src/profile.rs`: remove `AuthService` dependency and accept `UserIdentity`.
- Modify `crates/canopy-server/src/history.rs`: remove `AuthService` dependency and accept `UserIdentity`.
- Modify `crates/canopy-server/src/lib.rs`: pass `AuthService` to `GrpcApi` instead of profile/history services.
- Modify `crates/canopy-proto/proto/canopy.proto`: mark `auth_token` fields as compatibility fallback in comments.
- Modify `README.md`: document metadata auth and anonymous availability.
- Modify `docs/openapi.json`: document preferred metadata auth and deprecated body token fallback.

---

### Task 1: Auth Extractor

**Files:**
- Modify: `crates/canopy-server/src/api/grpc.rs`

- [ ] **Step 1: Add extractor tests**

Add a `#[cfg(test)] mod tests` block near the bottom of `crates/canopy-server/src/api/grpc.rs` with tests for metadata precedence and error behavior:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use tonic::metadata::MetadataValue;

    fn auth() -> AuthService {
        AuthService::new("secret")
    }

    fn token_for(user_id: &str) -> String {
        auth()
            .mint_at(user_id, std::time::Duration::from_secs(3600), 1_000)
            .unwrap()
    }

    #[test]
    fn extract_identity_reads_bearer_authorization_metadata() {
        let mut request = Request::new(());
        let token = token_for("user-1");
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::try_from(format!("Bearer {token}")).unwrap());

        let identity = extract_identity(request.metadata(), "", &auth()).unwrap();

        assert_eq!(identity.user_id, "user-1");
    }

    #[test]
    fn extract_identity_reads_direct_token_metadata() {
        let mut request = Request::new(());
        let token = token_for("user-2");
        request
            .metadata_mut()
            .insert("x-canopy-auth-token", MetadataValue::try_from(token).unwrap());

        let identity = extract_identity(request.metadata(), "", &auth()).unwrap();

        assert_eq!(identity.user_id, "user-2");
    }

    #[test]
    fn extract_identity_prefers_authorization_over_direct_token_metadata() {
        let mut request = Request::new(());
        let bearer = token_for("bearer-user");
        let direct = token_for("direct-user");
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::try_from(format!("Bearer {bearer}")).unwrap());
        request
            .metadata_mut()
            .insert("x-canopy-auth-token", MetadataValue::try_from(direct).unwrap());

        let identity = extract_identity(request.metadata(), "", &auth()).unwrap();

        assert_eq!(identity.user_id, "bearer-user");
    }

    #[test]
    fn extract_identity_uses_body_fallback_when_metadata_is_absent() {
        let token = token_for("fallback-user");

        let identity = extract_identity(&tonic::metadata::MetadataMap::new(), &token, &auth()).unwrap();

        assert_eq!(identity.user_id, "fallback-user");
    }

    #[test]
    fn extract_identity_rejects_malformed_bearer_metadata() {
        let mut request = Request::new(());
        request
            .metadata_mut()
            .insert("authorization", MetadataValue::from_static("Token abc"));

        let err = extract_identity(request.metadata(), "", &auth()).unwrap_err();

        assert!(matches!(err, canopy_core::CanopyError::Unauthenticated(_)));
    }

    #[test]
    fn extract_identity_rejects_missing_token() {
        let err = extract_identity(&tonic::metadata::MetadataMap::new(), "", &auth()).unwrap_err();

        assert!(matches!(err, canopy_core::CanopyError::Unauthenticated(_)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test -p canopy-server api::grpc::tests::extract_identity
```

Expected: compile fails because `extract_identity` does not exist yet.

- [ ] **Step 3: Implement extractor helpers**

In `crates/canopy-server/src/api/grpc.rs`, import `AuthService`, `CanopyError`, `CanopyResult`, and `UserIdentity`, then add helpers near `current_epoch_ms`:

```rust
fn extract_identity(
    metadata: &tonic::metadata::MetadataMap,
    body_token: &str,
    auth: &AuthService,
) -> CanopyResult<UserIdentity> {
    if let Some(raw) = metadata.get("authorization") {
        let value = raw
            .to_str()
            .map_err(|_| CanopyError::unauthenticated("invalid authorization metadata"))?;
        let Some(token) = value.strip_prefix("Bearer ") else {
            return Err(CanopyError::unauthenticated("authorization must use Bearer token"));
        };
        return auth.verify(token.trim());
    }

    if let Some(raw) = metadata.get("x-canopy-auth-token") {
        let token = raw
            .to_str()
            .map_err(|_| CanopyError::unauthenticated("invalid x-canopy-auth-token metadata"))?;
        return auth.verify(token.trim());
    }

    let body_token = body_token.trim();
    if !body_token.is_empty() {
        return auth.verify(body_token);
    }

    Err(CanopyError::unauthenticated("missing auth token"))
}
```

- [ ] **Step 4: Run extractor tests**

Run the same command as Step 2.

Expected: extractor tests pass.

---

### Task 2: Move Token Verification Out Of Services

**Files:**
- Modify: `crates/canopy-server/src/profile.rs`
- Modify: `crates/canopy-server/src/history.rs`

- [ ] **Step 1: Update ProfileService tests first**

Change profile tests to pass `UserIdentity` instead of tokens. The invalid-token test should be removed because token validity becomes an API adapter concern. Keep a test named `upsert_profile_persists_real_user_preferences` with this shape:

```rust
let service = ProfileService::new(Arc::new(InMemoryProfileStore::default()));
let identity = canopy_core::UserIdentity { user_id: "user-123".into() };
let profile = service.upsert_profile(&identity, "Ada", true).await.unwrap();
```

Expected assertions stay the same.

- [ ] **Step 2: Update ProfileService implementation**

Remove the `AuthService` field and constructor argument. Update `upsert_profile` signature to:

```rust
pub async fn upsert_profile(
    &self,
    identity: &UserIdentity,
    display_name: &str,
    history_enabled: bool,
) -> CanopyResult<UserProfile>
```

Use `identity.user_id` in the repository call.

- [ ] **Step 3: Update HistoryService tests first**

Remove invalid-token test from `history.rs`. Keep missing-profile, disabled-history, enabled-history, and invalid-playback-facts tests, but construct this identity directly:

```rust
fn identity() -> canopy_core::UserIdentity {
    canopy_core::UserIdentity { user_id: "user-123".into() }
}
```

Call `record_playback(&identity(), "track-1", 1000, 0.5)`.

- [ ] **Step 4: Update HistoryService implementation**

Remove the `AuthService` field and constructor argument. Update `record_playback` signature to:

```rust
pub async fn record_playback(
    &self,
    identity: &UserIdentity,
    track_id: &str,
    duration_ms: i64,
    completion_pct: f32,
) -> CanopyResult<bool>
```

Remove `self.auth.verify(...)` and keep the profile lookup by `identity.user_id`.

- [ ] **Step 5: Run service tests**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test -p canopy-server profile::tests history::tests
```

Expected: profile/history service tests pass.

---

### Task 3: Wire AuthService Into GrpcApi

**Files:**
- Modify: `crates/canopy-server/src/api/grpc.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Add AuthService to GrpcServices and GrpcApi**

Add `pub auth: AuthService` to `GrpcServices` and `auth: AuthService` to `GrpcApi`. Assign it in `GrpcApi::new`.

- [ ] **Step 2: Use extractor in authenticated RPCs**

In `upsert_profile`, keep the request metadata before `into_inner`:

```rust
let metadata = request.metadata().clone();
let req = request.into_inner();
let identity = extract_identity(&metadata, &req.auth_token, &self.auth).map_err(to_status)?;
let profile = self
    .profile
    .upsert_profile(&identity, &req.display_name, req.history_enabled)
    .await
    .map_err(to_status)?;
```

In `record_playback_history`, do the same and call:

```rust
.record_playback(&identity, &req.track_id, req.duration_ms, req.completion_pct)
```

- [ ] **Step 3: Update server construction**

In `crates/canopy-server/src/lib.rs`, remove `AuthService` construction from `ProfileService::new` and `HistoryService::new`. Add `auth: AuthService::new(config.auth_token_secret.clone())` to the `GrpcServices` construction.

- [ ] **Step 4: Run adapter compile check**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' check -p canopy-server --all-features
```

Expected: compile succeeds.

---

### Task 4: Proto And Docs

**Files:**
- Modify: `crates/canopy-proto/proto/canopy.proto`
- Modify: `README.md`
- Modify: `docs/openapi.json`

- [ ] **Step 1: Update proto comments**

Change `auth_token` comments in `UpsertProfileRequest` and `RecordPlaybackHistoryRequest` to:

```proto
string auth_token = 1;       // deprecated fallback; prefer authorization metadata
```

- [ ] **Step 2: Update README**

In the Auth section, add a paragraph:

```markdown
Authenticated profile-scoped RPCs should send the end-user token in gRPC metadata as `authorization: Bearer <token>`. `x-canopy-auth-token` is accepted for clients that cannot set authorization metadata. Existing `auth_token` request fields remain as a temporary compatibility fallback, but new clients should not depend on them.
```

Keep the existing anonymous access paragraph unchanged.

- [ ] **Step 3: Update OpenAPI docs**

In `docs/openapi.json`, update descriptions for `UpsertProfile`, `RecordPlaybackHistory`, and `x-canopy-docs.authModel` to mention metadata auth. Update `auth_token` schema descriptions to say deprecated fallback.

- [ ] **Step 4: Validate docs**

Run:

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy python3 -m json.tool docs/openapi.json
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git diff --check
```

Expected: JSON validation exits 0; `rg` exits 1 with no matches.

---

### Task 5: Full Verification And Commit

**Files:**
- All files modified in Tasks 1-4.

- [ ] **Step 1: Run full verification**

Run:

```powershell
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' fmt --all
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' clippy --workspace --all-features --tests -- -D warnings
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test
$env:CARGO_INCREMENTAL='0'; $env:CARGO_TARGET_DIR='C:\Users\Catalina\AppData\Local\Temp\canopy-cargo-target'; & 'C:\Users\CÄƒtÄƒlina\.cargo\bin\cargo.exe' test --features canopy-server/pg
```

Expected: every command exits 0.

- [ ] **Step 2: Inspect diff hygiene**

Run:

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git diff --check
& 'C:\Program Files\Git\cmd\git.exe' diff --stat
& 'C:\Program Files\Git\cmd\git.exe' status --short --branch
```

Expected: `git diff --check` exits 0 and status contains only intended auth-transport files.

- [ ] **Step 3: Commit**

Run:

```powershell
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git add README.md docs/openapi.json crates/canopy-proto/proto/canopy.proto crates/canopy-server/src/api/grpc.rs crates/canopy-server/src/history.rs crates/canopy-server/src/lib.rs crates/canopy-server/src/profile.rs
wsl.exe -d Ubuntu-24.04 --cd /home/catalina/projects/Canopy git commit -m "Harden authenticated gRPC transport"
```

Expected: commit succeeds.
