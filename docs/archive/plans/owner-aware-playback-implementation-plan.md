# Owner-Aware Playback Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing `ResolvePlayback` RPC select owner-scoped personal media before public media for the configured owner while preserving public-only anonymous behavior and privacy-preserving errors.

**Architecture:** A focused `PrincipalService` classifies optional authenticated identities using profile and instance-owner repositories. `ResolverService` consumes that principal, performs owner-first or public-only lookup, and mints a capability with the matching audience. Both capability issuance and stream authorization enforce current visibility, readiness, and configured ownership.

**Tech Stack:** Rust, Tokio, tonic gRPC, async-trait, SQLx, PostgreSQL, HMAC stream capabilities, in-memory test adapters.

**Repository rule:** Do not stage or commit changes. The repository owner will handle Git operations.

---

## File Map

- Create `crates/canopy-server/src/principal.rs`: request-principal model and identity/owner classification service.
- Modify `crates/canopy-server/src/lib.rs`: export and wire the principal service and a shared instance-settings repository.
- Modify `crates/canopy-core/src/repository.rs`: add owner-scoped personal playback lookup to `PlayableAssetRepository`.
- Modify `crates/canopy-server/src/jade_store/pg_stream.rs`: implement personal issuance and require the currently configured owner during personal stream authorization.
- Modify `crates/canopy-server/src/jade_store/memory.rs`: mirror PostgreSQL personal issuance and revocation semantics.
- Modify `crates/canopy-server/src/playback.rs`: apply owner-first selection and mint the correct capability audience.
- Modify `crates/canopy-server/src/api/grpc.rs`: parse optional credentials, classify the principal, and pass it to playback resolution.
- Modify repository fakes in `crates/canopy-server/src/stream/http.rs` and `crates/canopy-server/src/stream/authorizer.rs` for the expanded trait.
- Modify `crates/canopy-server/tests/domain.rs`: verify in-memory owner scoping and revocation parity.
- Modify `crates/canopy-server/tests/pg_integration.rs`: verify PostgreSQL owner-first lookup predicates and ownership-change revocation.
- Modify `README.md` and `docs/openapi.json`: document owner-aware gRPC issuance and the stream authorization policy without adding an HTTP playback API.

### Task 1: Principal Classification Boundary

**Files:**
- Create: `crates/canopy-server/src/principal.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Write principal classification tests**

Add tests using `InMemoryProfileStore` and `InMemoryInstanceSettingsStore` for these exact cases:

```rust
#[tokio::test]
async fn classifies_missing_identity_as_anonymous() {
    let service = service();
    assert_eq!(service.classify(None).await.unwrap(), PlaybackPrincipal::Anonymous);
}

#[tokio::test]
async fn classifies_valid_identity_without_profile_as_authenticated() {
    let service = service();
    let identity = UserIdentity { user_id: "known-token-user".into() };
    assert_eq!(service.classify(Some(&identity)).await.unwrap(), PlaybackPrincipal::Authenticated);
}

#[tokio::test]
async fn classifies_configured_profile_as_owner() {
    let (service, profiles, settings) = service_with_stores();
    let profile = profiles
        .upsert_profile("owner-user", Some("Owner"), true)
        .await
        .unwrap();
    settings.set_owner_profile_id(&profile.id).await.unwrap();
    let identity = UserIdentity { user_id: "owner-user".into() };
    assert_eq!(
        service.classify(Some(&identity)).await.unwrap(),
        PlaybackPrincipal::Owner { profile_id: profile.id }
    );
}
```

Also test that a profile which is not the configured owner is `Authenticated` and repository errors propagate unchanged.

- [ ] **Step 2: Run the tests and verify the expected compile failure**

Run: `cargo test -p canopy-server principal::tests --all-features`

Expected: FAIL because `principal` and `PrincipalService` do not exist.

- [ ] **Step 3: Implement the principal model and service**

Create the following public boundary in `principal.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackPrincipal {
    Anonymous,
    Authenticated,
    Owner { profile_id: String },
}

#[derive(Clone)]
pub struct PrincipalService {
    profiles: Arc<dyn ProfileRepository>,
    settings: Arc<dyn InstanceSettingsRepository>,
}

impl PrincipalService {
    pub fn new(
        profiles: Arc<dyn ProfileRepository>,
        settings: Arc<dyn InstanceSettingsRepository>,
    ) -> Self;

    pub async fn classify(
        &self,
        identity: Option<&UserIdentity>,
    ) -> CanopyResult<PlaybackPrincipal>;
}
```

`classify` returns `Anonymous` for no identity, looks up a supplied identity with `get_by_external_user_id`, returns `Authenticated` when no profile exists or the profile is not the configured owner, and returns `Owner { profile_id }` only when the profile ID equals `owner_profile_id()`.

Export the module from `lib.rs` with `pub mod principal;`.

- [ ] **Step 4: Run principal tests**

Run: `cargo test -p canopy-server principal::tests --all-features`

Expected: PASS for anonymous, authenticated, owner, non-owner, and error-propagation cases.

### Task 2: Owner-Scoped Playable Asset Repository Contract

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/playback.rs`
- Modify: `crates/canopy-server/src/stream/http.rs`
- Modify: `crates/canopy-server/src/stream/authorizer.rs`

- [ ] **Step 1: Add the failing contract usage to the playback fake**

Extend `FakePlayableAssets` with separate `personal_assets` and `public_assets` collections and add this expected trait method:

```rust
async fn assets_for_personal_playback(
    &self,
    owner_profile_id: &str,
    track_id: &str,
) -> CanopyResult<Vec<PlayableAsset>>;
```

Record calls so later resolver tests can assert personal lookup occurs before public lookup.

- [ ] **Step 2: Verify the contract does not compile yet**

Run: `cargo test -p canopy-server playback::tests --all-features`

Expected: FAIL because `PlayableAssetRepository` has no `assets_for_personal_playback` method.

- [ ] **Step 3: Add the repository method**

Add this method and documentation to `PlayableAssetRepository`:

```rust
/// Returns selectable assets for a ready personal track owned by the profile.
async fn assets_for_personal_playback(
    &self,
    owner_profile_id: &str,
    track_id: &str,
) -> CanopyResult<Vec<PlayableAsset>>;
```

Add explicit empty implementations to stream HTTP and authorizer test fakes. Do not provide a permissive default implementation on the trait.

- [ ] **Step 4: Verify all trait implementations are accounted for**

Run: `cargo check --workspace --all-targets --all-features`

Expected: FAIL only in the PostgreSQL and in-memory adapters until Task 3 implements the new method; no unidentified fake implementations remain.

### Task 3: PostgreSQL And In-Memory Policy Adapters

**Files:**
- Modify: `crates/canopy-server/src/jade_store/pg_stream.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/tests/domain.rs`
- Modify: `crates/canopy-server/tests/pg_integration.rs`

- [ ] **Step 1: Write failing PostgreSQL personal playback tests**

Extend the existing media policy integration test to assert:

```rust
let personal = stream_assets
    .assets_for_personal_playback(&owner_a.id, &personal_a_id)
    .await
    .unwrap();
assert_eq!(personal.len(), 1);
assert!(stream_assets
    .assets_for_personal_playback(&owner_b.id, &personal_a_id)
    .await
    .unwrap()
    .is_empty());
```

Set `instance_settings.owner_profile_id` to `owner_a`, authorize the personal asset successfully, change it to `owner_b`, and assert the same personal capability audience now returns `None`. Retain assertions that public assets cannot be authorized as personal and personal assets cannot be authorized as public.

- [ ] **Step 2: Write failing in-memory parity tests**

Build a personal-ready entry owned by `owner-a` and assert owner-scoped issuance succeeds only for `owner-a`. Configure the shared in-memory settings owner as `owner-a`, assert personal stream authorization succeeds, change the configured owner to `owner-b`, and assert authorization returns `None`.

- [ ] **Step 3: Run focused tests and verify failure**

Run: `cargo test -p canopy-server --test domain personal --all-features`

Expected: FAIL because the in-memory adapter lacks owner-scoped playable lookup/current-owner reauthorization.

Run through the project PostgreSQL harness: `bash scripts/test-pg.sh`

Expected: the expanded policy test FAILS before the SQL implementation.

- [ ] **Step 4: Implement PostgreSQL personal issuance**

Implement `assets_for_personal_playback` in `PgPlayableAssetRepository` with parsed UUID arguments and this policy predicate:

```sql
SELECT aa.id, aa.track_id, aa.codec, aa.content_type, aa.duration_ms
FROM audio_assets aa
JOIN tracks t ON t.id = aa.track_id
WHERE t.id = $1
  AND t.owner_profile_id = $2
  AND t.visibility = 'personal'
  AND t.ingest_status = 'ready'
ORDER BY aa.codec, aa.id
```

Bind `track_id` then `owner_profile_id`, map rows exactly like the public method, and propagate SQL errors as `CanopyError::Storage`.

- [ ] **Step 5: Tighten PostgreSQL personal stream authorization**

Replace the `owner_profile_id IS NOT NULL` predicate with current-owner enforcement:

```sql
AND t.owner_profile_id = (
    SELECT owner_profile_id
    FROM instance_settings
    WHERE singleton = TRUE
)
```

This makes an owner change revoke unexpired personal capabilities at stream authorization time.

- [ ] **Step 6: Implement in-memory parity**

Give `InMemoryAudioAssetStore` an optional shared `Arc<dyn InstanceSettingsRepository>` and add a constructor/builder used by wired non-PG mode and tests. Implement owner-scoped personal lookup using `visibility == Personal`, `ingest_status == Ready`, and exact owner equality. For `StreamAudience::Personal`, fetch the configured owner and authorize only when it equals the entry owner. When no settings repository or no owner is configured, deny personal authorization.

Do not hold a `std::sync::MutexGuard` across an `.await`; read settings before iterating entries.

- [ ] **Step 7: Run adapter tests**

Run: `cargo test -p canopy-server --test domain --all-features`

Expected: PASS.

Run: `bash scripts/test-pg.sh`

Expected: PASS, including owner-scoped issuance and ownership-change revocation.

### Task 4: Owner-First Resolver

**Files:**
- Modify: `crates/canopy-server/src/playback.rs`

- [ ] **Step 1: Write failing resolver policy tests**

Add tests for these behaviors:

```rust
#[tokio::test]
async fn owner_prefers_personal_asset_and_mints_personal_capability();

#[tokio::test]
async fn owner_falls_back_to_public_and_mints_public_capability();

#[tokio::test]
async fn anonymous_and_authenticated_principals_never_query_personal_assets();

#[tokio::test]
async fn personal_repository_error_does_not_fall_back_to_public();

#[tokio::test]
async fn inaccessible_and_missing_media_return_the_same_not_found_shape();
```

Decode generated URLs with `StreamTokenCodec::verify` and assert exact `StreamAudience`. Use a call log in the fake repository to assert owner order is `personal` then `public` only when personal is empty.

- [ ] **Step 2: Run resolver tests and verify failure**

Run: `cargo test -p canopy-server playback::tests --all-features`

Expected: FAIL because resolver methods do not accept `PlaybackPrincipal` and always mint public capabilities.

- [ ] **Step 3: Implement principal-aware resolution**

Change deterministic resolution to:

```rust
pub async fn resolve_at(
    &self,
    principal: &PlaybackPrincipal,
    track_id: &str,
    now_epoch_ms: u64,
) -> CanopyResult<PlaybackSource>;
```

For `Owner { profile_id }`, call `assets_for_personal_playback` first. If it returns a selectable asset, mint `Personal`. Only an empty successful result may fall back to `assets_for_public_playback`; any repository error propagates. For `Anonymous` and `Authenticated`, query public assets only and mint `Public`.

Extract a private helper that accepts the selected `PlayableAsset`, `StreamAudience`, and current time, then performs TTL calculation, token minting, and response construction once.

Change `resolve_for_session` to accept `&PlaybackPrincipal` and call the new `resolve_at`. Keep session loading after successful resolution so failed or unauthorized resolution cannot mutate session state.

- [ ] **Step 4: Run resolver tests**

Run: `cargo test -p canopy-server playback::tests --all-features`

Expected: PASS for codec preference, capability audiences, lookup order, error propagation, concealment, and session synchronization.

### Task 5: Optional gRPC Authentication And Principal Wiring

**Files:**
- Modify: `crates/canopy-server/src/api/grpc.rs`
- Modify: `crates/canopy-server/src/lib.rs`

- [ ] **Step 1: Write failing optional-auth extraction tests**

In the gRPC adapter test module, add exact cases for:

```rust
assert_eq!(extract_optional_metadata_identity(&MetadataMap::new(), &auth).unwrap(), None);
```

Also assert that a valid bearer token returns `Some(UserIdentity)`, an invalid bearer token returns `CanopyError::Unauthenticated`, and malformed `authorization` metadata is never treated as anonymous.

- [ ] **Step 2: Run gRPC tests and verify failure**

Run: `cargo test -p canopy-server api::grpc::tests --all-features`

Expected: FAIL because optional metadata extraction does not exist.

- [ ] **Step 3: Implement optional metadata extraction**

Add:

```rust
fn extract_optional_metadata_identity(
    metadata: &tonic::metadata::MetadataMap,
    auth: &AuthService,
) -> CanopyResult<Option<UserIdentity>>;
```

Return `Ok(None)` only when neither `authorization` nor `x-canopy-auth-token` is present. If either is present, delegate to strict metadata verification and return `Some(identity)` or the verification error. Do not read a body token for `ResolvePlayback`.

- [ ] **Step 4: Add principal service to the gRPC adapter**

Add `principal: PrincipalService` to `GrpcServices` and `GrpcApi`. In `resolve_playback`, preserve metadata before consuming the request, extract optional identity, call `principal.classify(identity.as_ref()).await`, and pass the result to `resolver.resolve_for_session`.

- [ ] **Step 5: Wire shared repositories in server startup**

Declare `instance_settings_repo: Arc<dyn InstanceSettingsRepository>` beside `profile_repo`.

In PostgreSQL mode construct `PgInstanceSettingsRepository` from the same pool. In non-PG mode construct one `Arc<InMemoryInstanceSettingsStore>` and share it with both `PrincipalService` and the in-memory playable store. Construct:

```rust
let principal = PrincipalService::new(profile_repo.clone(), instance_settings_repo);
```

Pass it through `GrpcServices`. Keep the proto and `ResolvePlayback` request shape unchanged.

- [ ] **Step 6: Run focused adapter and startup checks**

Run: `cargo test -p canopy-server api::grpc::tests --all-features`

Expected: PASS.

Run: `cargo check --workspace --all-targets --all-features`

Expected: PASS with every repository and service wired.

### Task 6: End-To-End Policy Verification And Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/openapi.json`
- Verify: `crates/canopy-proto/proto/canopy.proto`

- [ ] **Step 1: Update maintained documentation**

Document that `ResolvePlayback` accepts optional authentication metadata, anonymous/non-owner callers resolve public release-safe media, and the configured owner resolves personal media first with public fallback. Document that invalid supplied credentials return `Unauthenticated`, while inaccessible personal media is concealed as `NotFound`.

In `docs/openapi.json`, update only the private stream-authorization descriptions/security semantics affected by personal capability revalidation. Do not invent an HTTP playback endpoint; playback issuance remains gRPC.

- [ ] **Step 2: Confirm the protobuf contract remains stable**

Run: `git diff -- crates/canopy-proto/proto/canopy.proto`

Expected: no diff. Clients continue using the existing `ResolvePlayback(PlaybackRequest)` RPC.

- [ ] **Step 3: Format and run the complete Rust quality gate**

Run: `cargo fmt --all -- --check`

Expected: PASS. If it fails, run `cargo fmt --all`, inspect the formatting diff, then rerun the check.

Run: `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`

Expected: PASS with no warnings.

Run: `cargo test --workspace --all-features --locked`

Expected: PASS; the Nginx range/revocation test may remain ignored here because the streaming harness runs it explicitly.

- [ ] **Step 4: Run persistence and streaming harnesses**

Run: `bash scripts/test-pg.sh`

Expected: PASS, including owner-scoped lookup and ownership-change revocation.

Run: `bash scripts/test-streaming.sh`

Expected: PASS, including Nginx range handling and policy rechecks.

- [ ] **Step 5: Inspect the final unstaged diff**

Run: `git status --short`

Expected: only intended source, test, README, OpenAPI, design, and plan files are modified/untracked; nothing is staged.

Run: `git diff --check`

Expected: no whitespace errors.

## Deferred Follow-Up

Search and recommendations are intentionally excluded from this plan. Their next design phase will reuse `PlaybackPrincipal`/`PrincipalService` and establish owner-visible personal-plus-public result sets, personal-first ordering, privacy-preserving filtering, and deterministic deduplication for equivalent recordings.
