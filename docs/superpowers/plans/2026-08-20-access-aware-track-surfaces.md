# Access-Aware Track Surfaces Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every Canopy backend surface return or reference every track accessible to the caller while hiding inaccessible personal, pending, and quarantined tracks.

**Architecture:** Add a transport-independent `TrackAccessScope` to `canopy-core`, derive it from current native identity and instance-owner state, and pass it into every track-facing repository query. PostgreSQL and in-memory adapters enforce the same predicate before counting or pagination; bounded optional-auth RPCs share native bearer/session verification, while durable services derive the scope for their verified profile.

**Tech Stack:** Rust 2024, Tokio, tonic gRPC, async-trait repository ports, SQLx, PostgreSQL 17, existing HMAC page/stream capability codecs.

**Spec:** `docs/superpowers/specs/2026-08-20-access-aware-track-surfaces-design.md`

## Global Constraints

- The HMI is a separate project; change only Canopy backend code and Canopy integration documentation.
- Preserve the existing `canopy.v1` protobuf messages, RPC names, and opaque page-token encoding.
- Anonymous callers and authenticated non-owners receive only `release_safe` + `ready` tracks.
- Only the currently configured instance owner receives public tracks plus their own `personal` + `ready` tracks.
- Invalid, malformed, expired, or revoked supplied credentials return `Unauthenticated`; never downgrade them to anonymous.
- `pending`, `quarantined`, and another owner's personal tracks are concealed as `NotFound` or omitted from collections.
- Apply access filtering before `COUNT`, offset, limit, and continuation-token calculation.
- Relationship removal, unlike, history deletion/clear, playlist-track removal, and playlist deletion remain possible after track access is lost.
- Existing inaccessible relationship rows remain stored but hidden and may become visible again if access is restored.
- Preserve unrelated working-tree edits; stage and commit only files named by the current task.

---

## File Structure

- `crates/canopy-core/src/access.rs`: owns the transport-independent track-access type and helpers.
- `crates/canopy-core/src/repository.rs`: defines scope-aware repository contracts.
- `crates/canopy-server/src/principal.rs`: maps verified identities/profiles and current owner settings to `TrackAccessScope`.
- `crates/canopy-server/src/api/grpc/mod.rs`: owns shared native bearer extraction and optional-auth scope derivation.
- `crates/canopy-server/src/{catalog,search,discovery,playback,library,likes,history,playlists}.rs`: passes access scope through application use cases.
- `crates/canopy-server/src/api/grpc/{catalog,discovery,playback}.rs`: applies optional native authentication to bounded public RPCs.
- `crates/canopy-server/src/jade_store/memory.rs`: enforces the access predicate in the in-memory adapters used by tests and standalone mode.
- `crates/canopy-server/src/jade_store/pg.rs`: enforces the predicate transactionally and before pagination in PostgreSQL.
- `crates/canopy-server/tests/{domain,pg_integration,local_integration}.rs`: proves policy parity, SQL behavior, native-session revocation, and pagination.
- `docs/client-integration.md` and `docs/playback.md`: hand off authentication, access, paging, and playback rules to HMI teams without prescribing their UI implementation.

---

### Task 1: Introduce the Shared Track Access Policy

**Files:**
- Create: `crates/canopy-core/src/access.rs`
- Modify: `crates/canopy-core/src/lib.rs`
- Modify: `crates/canopy-server/src/principal.rs`
- Test: `crates/canopy-server/src/principal.rs`

**Interfaces:**
- Consumes: `InstanceSettingsRepository::owner_profile_id()` and `ProfileRepository::get_by_external_user_id()`.
- Produces: `TrackAccessScope::{Public, Owner { profile_id }}`, `TrackAccessScope::owner_profile_id()`, `PrincipalService::track_scope_for_identity()`, and `PrincipalService::track_scope_for_profile()`.

- [ ] **Step 1: Write failing policy-classification tests**

Add tests proving anonymous, missing-profile, non-owner, configured-owner, and ownership-transfer outcomes:

```rust
#[tokio::test]
async fn configured_owner_receives_owner_scope_until_ownership_changes() {
    let (service, profiles, settings) = service_with_stores();
    let owner = profiles
        .upsert_profile("owner-user", Some("Owner"), true)
        .await
        .unwrap();
    let replacement = profiles
        .upsert_profile("replacement-user", Some("Replacement"), true)
        .await
        .unwrap();
    settings.set_owner_profile_id(&owner.id).await.unwrap();

    assert_eq!(
        service.track_scope_for_profile(&owner.id).await.unwrap(),
        TrackAccessScope::Owner { profile_id: owner.id.clone() }
    );

    settings.set_owner_profile_id(&replacement.id).await.unwrap();
    assert_eq!(
        service.track_scope_for_profile(&owner.id).await.unwrap(),
        TrackAccessScope::Public
    );
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run: `cargo test -p canopy-server principal::tests::configured_owner_receives_owner_scope_until_ownership_changes -- --exact`

Expected: FAIL because `TrackAccessScope` and `track_scope_for_profile` do not exist.

- [ ] **Step 3: Add the access type and principal mapping**

Create the core type:

```rust
//! Track visibility policy shared by every backend surface.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackAccessScope {
    Public,
    Owner { profile_id: String },
}

impl TrackAccessScope {
    pub fn owner_profile_id(&self) -> Option<&str> {
        match self {
            Self::Public => None,
            Self::Owner { profile_id } => Some(profile_id),
        }
    }
}
```

Export it from `canopy-core/src/lib.rs`, then add these methods to `PrincipalService` while retaining the existing playback classifier until Task 7:

```rust
pub async fn track_scope_for_identity(
    &self,
    identity: Option<&UserIdentity>,
) -> CanopyResult<TrackAccessScope> {
    let Some(identity) = identity else {
        return Ok(TrackAccessScope::Public);
    };
    let Some(profile) = self
        .profiles
        .get_by_external_user_id(&identity.user_id)
        .await?
    else {
        return Ok(TrackAccessScope::Public);
    };
    self.track_scope_for_profile(&profile.id).await
}

pub async fn track_scope_for_profile(
    &self,
    profile_id: &str,
) -> CanopyResult<TrackAccessScope> {
    if self.settings.owner_profile_id().await?.as_deref() == Some(profile_id) {
        Ok(TrackAccessScope::Owner { profile_id: profile_id.to_string() })
    } else {
        Ok(TrackAccessScope::Public)
    }
}
```

- [ ] **Step 4: Run the complete policy tests**

Run: `cargo test -p canopy-server principal::tests`

Expected: PASS for existing principal tests plus the new scope and ownership-transfer cases.

- [ ] **Step 5: Commit the shared policy**

```bash
git add crates/canopy-core/src/access.rs crates/canopy-core/src/lib.rs crates/canopy-server/src/principal.rs
git commit -m "feat: add shared track access scope"
```

---

### Task 2: Share Native Optional Authentication Across Bounded RPCs

**Files:**
- Modify: `crates/canopy-server/src/api/grpc/mod.rs`
- Test: `crates/canopy-server/src/api/grpc/mod.rs`

**Interfaces:**
- Consumes: `IdentityService::authenticate_access_token(&str)`, `PrincipalService::track_scope_for_identity()`, and `authorization: Bearer <access-token>` metadata.
- Produces: `optional_native_bearer_token(metadata) -> CanopyResult<Option<&str>>` and `extract_optional_track_scope(metadata, services) -> CanopyResult<TrackAccessScope>` for catalog, discovery, and playback.

- [ ] **Step 1: Write failing optional-auth boundary tests**

Add tests for the synchronous metadata boundary so no large service fixture is required:

```rust
#[test]
fn optional_native_bearer_is_absent_only_when_all_auth_metadata_is_absent() {
    let metadata = tonic::metadata::MetadataMap::new();
    assert_eq!(optional_native_bearer_token(&metadata).unwrap(), None);
}

#[test]
fn optional_native_bearer_rejects_legacy_header_on_bounded_service() {
    let mut metadata = tonic::metadata::MetadataMap::new();
    metadata.insert("x-canopy-auth-token", "legacy-token".parse().unwrap());

    assert!(matches!(
        optional_native_bearer_token(&metadata),
        Err(CanopyError::Unauthenticated(_))
    ));
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run: `cargo test -p canopy-server api::grpc::tests::optional_native_bearer -- --nocapture`

Expected: FAIL because `optional_native_bearer_token` does not exist.

- [ ] **Step 3: Implement strict native optional authentication**

Add the metadata precheck and async helper beside `extract_durable_principal`:

```rust
fn optional_native_bearer_token(
    metadata: &tonic::metadata::MetadataMap,
) -> CanopyResult<Option<&str>> {
    if metadata.get("authorization").is_some() {
        return extract_bearer_token(metadata).map(Some);
    }
    if metadata.get("x-canopy-auth-token").is_some() {
        return Err(CanopyError::unauthenticated(
            "bounded APIs require authorization Bearer metadata",
        ));
    }
    Ok(None)
}

pub(crate) async fn extract_optional_track_scope(
    metadata: &tonic::metadata::MetadataMap,
    services: &GrpcServices,
) -> CanopyResult<TrackAccessScope> {
    let Some(access_token) = optional_native_bearer_token(metadata)? else {
        return Ok(TrackAccessScope::Public);
    };
    let identity_service = services
        .identity
        .as_ref()
        .ok_or_else(|| CanopyError::unauthenticated("native identity is not configured"))?;
    let access = identity_service.authenticate_access_token(access_token).await?;
    let identity = UserIdentity { user_id: access.account_id };
    services
        .principal
        .track_scope_for_identity(Some(&identity))
        .await
}
```

Keep `extract_optional_metadata_identity` only for `legacy.rs` during migration. Add a comment that bounded adapters must call the async native helper.

- [ ] **Step 4: Run gRPC helper and identity tests**

Run: `cargo test -p canopy-server api::grpc::tests`
Run: `cargo test -p canopy-server identity::tests`

Expected: PASS; malformed bearer metadata remains `Unauthenticated`, and a present legacy-only header is not treated as anonymous.

- [ ] **Step 5: Commit the authentication boundary**

```bash
git add crates/canopy-server/src/api/grpc/mod.rs
git commit -m "feat: share native optional track authentication"
```

---

### Task 3: Make Catalog Browse, Search, and Metadata Access-Aware

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/catalog.rs`
- Modify: `crates/canopy-server/src/search.rs`
- Modify: `crates/canopy-server/src/api/grpc/catalog.rs`
- Modify: `crates/canopy-server/src/api/grpc/legacy.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Test: `crates/canopy-server/tests/domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `TrackAccessScope` and `extract_optional_track_scope()`.
- Produces: `CatalogRepository::{browse, search, get_media}` with `scope: &TrackAccessScope`; `CatalogService` and `SearchService` methods with the same scope argument.

- [ ] **Step 1: Write failing in-memory access and pagination tests**

Add a fixture containing public-ready, owner-personal-ready, other-owner-personal-ready, pending, and quarantined entries. Assert filtering precedes the offset:

```rust
fn access_entry(
    id: &str,
    artist: &str,
    visibility: MediaVisibility,
    ingest_status: IngestStatus,
    owner_profile_id: Option<&str>,
) -> InMemoryCatalogEntry {
    InMemoryCatalogEntry {
        item: MediaItem {
            id: id.into(),
            title: id.into(),
            artist: artist.into(),
            ..MediaItem::default()
        },
        visibility,
        ingest_status,
        owner_profile_id: owner_profile_id.map(str::to_owned),
    }
}

fn access_matrix_entries() -> Vec<InMemoryCatalogEntry> {
    vec![
        access_entry(
            "public-ready", "Artist Public",
            MediaVisibility::ReleaseSafe, IngestStatus::Ready, None,
        ),
        access_entry(
            "owner-ready", "Artist Owner",
            MediaVisibility::Personal, IngestStatus::Ready, Some("owner-a"),
        ),
        access_entry(
            "other-ready", "Artist Other",
            MediaVisibility::Personal, IngestStatus::Ready, Some("owner-b"),
        ),
        access_entry(
            "owner-pending", "Artist Pending",
            MediaVisibility::Personal, IngestStatus::Pending, Some("owner-a"),
        ),
        access_entry(
            "quarantined", "Artist Quarantined",
            MediaVisibility::Quarantined, IngestStatus::Quarantined, None,
        ),
    ]
}

#[tokio::test]
async fn owner_catalog_pages_over_public_and_owned_personal_tracks_only() {
    let catalog = InMemoryCatalog::from_entries(access_matrix_entries());
    let scope = TrackAccessScope::Owner { profile_id: "owner-a".into() };

    let first = catalog.browse(&scope, None, &[], page(1, 0)).await.unwrap();
    let second = catalog.browse(&scope, None, &[], page(1, 1)).await.unwrap();

    assert_eq!(first.total_count, 2);
    assert_eq!(second.total_count, 2);
    assert_eq!(
        vec![first.items[0].id.as_str(), second.items[0].id.as_str()],
        vec!["owner-ready", "public-ready"]
    );
    assert!(!second.has_more);
}

#[tokio::test]
async fn public_scope_conceals_every_non_public_partition() {
    let catalog = InMemoryCatalog::from_entries(access_matrix_entries());
    assert!(catalog
        .get_media(&TrackAccessScope::Public, "owner-ready")
        .await
        .unwrap()
        .is_none());
}
```

- [ ] **Step 2: Run the focused domain tests and verify they fail**

Run: `cargo test -p canopy-server --test domain owner_catalog_pages_over_public_and_owned_personal_tracks_only`
Run: `cargo test -p canopy-server --test domain public_scope_conceals_every_non_public_partition`

Expected: FAIL because the scope-aware repository methods do not exist.

- [ ] **Step 3: Replace public runtime catalog ports with scoped ports**

Change the trait signatures while retaining `list_personal(owner_profile_id, page)` only for profile-deletion policy:

```rust
async fn browse(
    &self,
    scope: &TrackAccessScope,
    parent_id: Option<&str>,
    genres: &[String],
    page: Page,
) -> CanopyResult<MediaPage>;

async fn search(
    &self,
    scope: &TrackAccessScope,
    query: &str,
    page: Page,
) -> CanopyResult<MediaPage>;

async fn get_media(
    &self,
    scope: &TrackAccessScope,
    media_id: &str,
) -> CanopyResult<Option<MediaItem>>;
```

Remove `browse_public`, `search_public`, `get_public_media`, `search_personal`, and `get_personal_media` after migrating their call sites and tests. Legacy browse/search/get calls pass `&TrackAccessScope::Public` explicitly.

- [ ] **Step 4: Implement the in-memory predicate before sort and pagination**

Replace separate public/personal vectors with one accessible-item helper:

```rust
fn accessible_items(&self, scope: &TrackAccessScope) -> Vec<MediaItem> {
    let mut items: Vec<_> = self.entries.iter()
        .filter(|entry| {
            entry.ingest_status == IngestStatus::Ready
                && (entry.visibility == MediaVisibility::ReleaseSafe
                    || matches!(
                        scope,
                        TrackAccessScope::Owner { profile_id }
                            if entry.visibility == MediaVisibility::Personal
                                && entry.owner_profile_id.as_deref() == Some(profile_id.as_str())
                    ))
        })
        .map(|entry| entry.item.clone())
        .collect();
    items.sort_by(|left, right| left.id.cmp(&right.id));
    items
}
```

Use this vector for browse, search, and get. Keep filtering and deterministic ordering before `page_items`.

- [ ] **Step 5: Implement one PostgreSQL access predicate for rows and counts**

Add a scope binder:

```rust
fn owner_scope_uuid(scope: &TrackAccessScope) -> CanopyResult<Option<uuid::Uuid>> {
    scope
        .owner_profile_id()
        .map(|id| parse_uuid_arg(id, "owner_profile_id"))
        .transpose()
}
```

Use the same condition in browse rows, browse count, search rows, search count, and get-by-id:

```sql
AND t.ingest_status = 'ready'
AND (
    t.visibility = 'release_safe'
    OR (
        $1::uuid IS NOT NULL
        AND t.visibility = 'personal'
        AND t.owner_profile_id = $1
    )
)
```

Bind the owner UUID as `$1`, then query/page values after it. Browse orders by `t.created_at, t.id`; search orders by `rank DESC, t.title, t.id`. Both count queries repeat the exact access and search predicates before offset/limit.

- [ ] **Step 6: Thread the scope through services and bounded gRPC**

Change application signatures and call them from metadata-aware adapters:

```rust
let metadata = request.metadata().clone();
let request = request.into_inner();
let scope = extract_optional_track_scope(&metadata, &self.0)
    .await
    .map_err(to_status)?;
let result = self.0.catalog
    .browse(&scope, request.parent_id.as_deref(), &request.genres, page)
    .await
    .map_err(to_status)?;
```

Apply the same extraction to `Search` and `GetMedia`. Keep inaccessible and unknown IDs on the same `Status::not_found` path.

- [ ] **Step 7: Add PostgreSQL matrix coverage and run it**

Add a test that inserts the five access-matrix rows, sets the current owner, and checks browse/search/get counts and ordering for public and owner scopes:

```rust
let public = catalog.browse(&TrackAccessScope::Public, None, &[], Page { limit: 10, offset: 0 }).await.unwrap();
let owner = catalog.browse(
    &TrackAccessScope::Owner { profile_id: owner.id.clone() },
    None,
    &[],
    Page { limit: 10, offset: 0 },
).await.unwrap();
assert_eq!(public.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(), vec![public_id.as_str()]);
assert_eq!(owner.total_count, 2);
assert!(catalog.get_media(&TrackAccessScope::Public, &owner_track_id).await.unwrap().is_none());
```

Run: `./scripts/test-pg.sh`

Expected: PASS against an isolated migrated PostgreSQL database.

- [ ] **Step 8: Run catalog regression tests and commit**

Run: `cargo test -p canopy-server --test domain`

Expected: PASS, including search validation and administrative `list_personal` deletion-policy coverage.

```bash
git add crates/canopy-core/src/repository.rs crates/canopy-server/src/catalog.rs crates/canopy-server/src/search.rs crates/canopy-server/src/api/grpc/catalog.rs crates/canopy-server/src/api/grpc/legacy.rs crates/canopy-server/src/jade_store/memory.rs crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/tests/domain.rs crates/canopy-server/tests/pg_integration.rs
git commit -m "feat: expose access-aware catalog"
```

---

### Task 4: Make Discovery, For You, and Recommendations Access-Aware

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/discovery.rs`
- Modify: `crates/canopy-server/src/api/grpc/discovery.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Test: `crates/canopy-server/src/api/grpc/discovery.rs`
- Test: `crates/canopy-server/tests/domain.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `TrackAccessScope`, `extract_optional_track_scope()`, and the scoped catalog predicate.
- Produces: `DiscoveryRepository::shuffle_pool(&TrackAccessScope)` and `DiscoveryService::feed(&TrackAccessScope, exclude_track_ids, page)`.

- [ ] **Step 1: Write failing discovery parity tests**

Add a domain test using the access-matrix fixture from Task 3:

```rust
#[tokio::test]
async fn owner_discovery_includes_public_and_owned_personal_tracks() {
    let discovery = DiscoveryService::new(Arc::new(
        InMemoryCatalog::from_entries(access_matrix_entries()),
    ));
    let feed = discovery
        .feed(
            &TrackAccessScope::Owner { profile_id: "owner-a".into() },
            &[],
            Page { limit: 10, offset: 0 },
        )
        .await
        .unwrap();
    assert_eq!(
        feed.items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>(),
        vec!["owner-ready", "public-ready"]
    );
}
```

- [ ] **Step 2: Run the discovery test and verify it fails**

Run: `cargo test -p canopy-server --test domain owner_discovery_includes_public_and_owned_personal_tracks`

Expected: FAIL because discovery does not accept a scope.

- [ ] **Step 3: Scope the discovery port and application service**

Change the port and service calls:

```rust
#[async_trait]
pub trait DiscoveryRepository: Send + Sync {
    async fn shuffle_pool(
        &self,
        scope: &TrackAccessScope,
    ) -> CanopyResult<Vec<MediaItem>>;
}

pub async fn feed(
    &self,
    scope: &TrackAccessScope,
    exclude_track_ids: &[String],
    page: Page,
) -> CanopyResult<MediaPage> {
    let limit = Self::clamp_limit(page.limit) as usize;
    let offset = page.offset as usize;
    let excluded: HashSet<&str> = exclude_track_ids.iter().map(String::as_str).collect();
    let candidates: Vec<MediaItem> = self.repo
        .shuffle_pool(scope)
        .await?
        .into_iter()
        .filter(|item| !excluded.contains(item.id.as_str()))
        .collect();
    let total = candidates.len();
    let items: Vec<_> = diversify(candidates, total)
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();
    Ok(MediaPage {
        total_count: total.min(i32::MAX as usize) as i32,
        has_more: total > offset.saturating_add(items.len()),
        items,
    })
}
```

- [ ] **Step 4: Preserve the public materialized view and add an owner pool**

For `Public`, keep `mv_discovery_pool` and its existing fail-closed public fallback. For `Owner`, select public plus owned personal-ready rows in one pool; do not append personal rows after a public list:

```sql
WITH accessible AS (
    SELECT t.*,
           md5(t.id::text || CURRENT_DATE::text) AS daily_rank
    FROM tracks t
    WHERE t.is_explicit = FALSE
      AND t.ingest_status = 'ready'
      AND (
          t.visibility = 'release_safe'
          OR (t.visibility = 'personal' AND t.owner_profile_id = $1)
      )
)
SELECT t.id AS track_id,
       t.title AS track_title,
       a.name AS artist_name,
       al.title AS album_title,
       t.duration_ms AS track_duration_ms,
       t.is_explicit AS track_explicit,
       COALESCE(t.artwork_storage_key, al.artwork_storage_key) AS artwork_storage_key,
       aa.content_type AS asset_content_type,
       aa.size_bytes AS asset_size_bytes
FROM accessible t
JOIN artists a ON t.artist_id = a.id
JOIN albums al ON t.album_id = al.id
LEFT JOIN LATERAL (
    SELECT codec, content_type, size_bytes
    FROM audio_assets
    WHERE track_id = t.id
    ORDER BY CASE codec
        WHEN 'opus' THEN 0
        WHEN 'mp4' THEN 1
        WHEN 'mp3' THEN 2
        WHEN 'flac' THEN 3
        ELSE 4
    END, codec
    LIMIT 1
) aa ON TRUE
ORDER BY t.daily_rank, t.id
```

The in-memory implementation calls `accessible_items(scope)` before returning the pool.

- [ ] **Step 5: Give the discovery adapter shared gRPC services**

Replace its separate service/token fields with the existing shared container:

```rust
pub struct DiscoveryGrpc(pub Arc<GrpcServices>);

async fn feed(
    &self,
    metadata: tonic::metadata::MetadataMap,
    exclude_track_ids: Vec<String>,
    page_request: Option<PageRequest>,
) -> Result<FeedPayload, Status> {
    let scope = extract_optional_track_scope(&metadata, &self.0).await.map_err(to_status)?;
    let page = page_from_request(page_request, &self.0.page_tokens).map_err(to_status)?;
    let result = self.0.discovery.feed(&scope, &exclude_track_ids, page).await.map_err(to_status)?;
    let page_info = page_info(
        page,
        result.items.len(),
        result.has_more,
        &self.0.page_tokens,
    )
    .map_err(to_status)?;
    Ok(FeedPayload {
        tracks: result.items.into_iter().map(to_track_summary).collect(),
        page_info: Some(page_info),
    })
}
```

Each RPC clones metadata before `into_inner()`. In `lib.rs`, register `DiscoveryGrpc(services.clone())`.

- [ ] **Step 6: Run discovery unit and PostgreSQL tests**

Run: `cargo test -p canopy-server discovery::tests`
Run: `cargo test -p canopy-server api::grpc::discovery::tests`

Run: `./scripts/test-pg.sh`

Expected: PASS; public excludes every personal row, owner receives one mixed accessible pool, exclusions apply before pagination, and all named feeds match.

- [ ] **Step 7: Commit discovery support**

```bash
git add crates/canopy-core/src/repository.rs crates/canopy-server/src/discovery.rs crates/canopy-server/src/api/grpc/discovery.rs crates/canopy-server/src/jade_store/memory.rs crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/src/lib.rs crates/canopy-server/tests/pg_integration.rs
git commit -m "feat: apply track access to discovery feeds"
```

---

### Task 5: Enforce Track Access for Saved Tracks, Likes, and History

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/library.rs`
- Modify: `crates/canopy-server/src/likes.rs`
- Modify: `crates/canopy-server/src/history.rs`
- Modify: `crates/canopy-server/src/profile.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Test: `crates/canopy-server/src/{library,likes,history}.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `PrincipalService::track_scope_for_profile()` and `CatalogRepository::get_media()`.
- Produces: scope-aware relationship creation, listing, and membership reads; scope-free cleanup methods.

- [ ] **Step 1: Write failing service tests for creation, hiding, and cleanup**

For each of saved tracks, likes, and history, configure an in-memory catalog and owner settings, then prove an inaccessible track cannot be added, a formerly accessible relation is hidden after ownership transfer, and cleanup still succeeds:

```rust
settings.set_owner_profile_id(&owner.id).await.unwrap();
service.save_track(&owner_identity, "owner-ready").await.unwrap();

settings.set_owner_profile_id(&replacement.id).await.unwrap();
let page = service.list_tracks(&owner_identity, Page { limit: 10, offset: 0 }).await.unwrap();
assert!(page.items.is_empty());
assert_eq!(page.total_count, 0);

service.remove_track(&owner_identity, "owner-ready").await.unwrap();
```

The like test uses `like_track`, `list_liked_tracks`, and `unlike_track`. The history test uses `record_playback`, `list_history`, and `delete_entry`/`clear_history`.

- [ ] **Step 2: Run focused service tests and verify they fail**

Run: `cargo test -p canopy-server library::tests`
Run: `cargo test -p canopy-server likes::tests`
Run: `cargo test -p canopy-server history::tests`

Expected: FAIL because repositories do not receive a current access scope.

- [ ] **Step 3: Add scope to relationship repository contracts**

Use these exact method shapes:

```rust
async fn save_track(&self, profile_id: &str, track_id: &str, scope: &TrackAccessScope) -> CanopyResult<LibraryItem>;
async fn list_tracks(&self, profile_id: &str, scope: &TrackAccessScope, page: Page) -> CanopyResult<SavedTrackPage>;
async fn is_saved(&self, profile_id: &str, track_id: &str, scope: &TrackAccessScope) -> CanopyResult<bool>;

async fn like_track(&self, profile_id: &str, track_id: &str, scope: &TrackAccessScope) -> CanopyResult<TrackLike>;
async fn list_liked_tracks(&self, profile_id: &str, scope: &TrackAccessScope, page: Page) -> CanopyResult<LikedTrackPage>;
async fn is_liked(&self, profile_id: &str, track_id: &str, scope: &TrackAccessScope) -> CanopyResult<bool>;

async fn record(&self, event: PlaybackHistoryEvent, scope: &TrackAccessScope) -> CanopyResult<bool>;
async fn list(&self, profile_id: &str, scope: &TrackAccessScope, page: Page) -> CanopyResult<PlaybackHistoryPage>;
```

Leave `remove_track`, `unlike_track`, `delete_entry`, and `clear` without a scope.

- [ ] **Step 4: Derive current scope inside durable application services**

Inject a clone of `PrincipalService` into the three constructors. After resolving the durable profile, derive and pass the scope:

```rust
let profile_id = self.profile_id(identity).await?;
let scope = self.principal.track_scope_for_profile(&profile_id).await?;
self.library.save_track(&profile_id, track_id, &scope).await
```

Use the same pattern for list and membership reads, likes, and history. Update production and test wiring in `lib.rs`, `profile.rs`, and each service test fixture.

- [ ] **Step 5: Give in-memory relationship stores a catalog dependency**

Replace access-blind `Default` construction in all call sites with constructors such as:

```rust
pub fn new(catalog: Arc<dyn CatalogRepository>) -> Self {
    Self { catalog, items: Mutex::new(HashMap::new()) }
}

async fn require_accessible(
    catalog: &dyn CatalogRepository,
    scope: &TrackAccessScope,
    track_id: &str,
) -> CanopyResult<MediaItem> {
    catalog.get_media(scope, track_id).await?
        .ok_or_else(|| CanopyError::not_found("track", track_id))
}
```

Creation calls `require_accessible` before mutation. Listing snapshots and sorts relation rows, resolves accessible media before slicing the vector, then derives `total_count` and `has_more` from that filtered vector. Membership reads return `false` when the relation exists but the track is inaccessible. Never hold a `MutexGuard` across `.await`.

- [ ] **Step 6: Make PostgreSQL writes validate access transactionally**

Use `INSERT ... SELECT` with the canonical predicate. For saved tracks:

```sql
INSERT INTO profile_library_items (profile_id, track_id)
SELECT $1, t.id
FROM tracks t
WHERE t.id = $2
  AND t.ingest_status = 'ready'
  AND (
      t.visibility = 'release_safe'
      OR ($3::uuid IS NOT NULL AND t.visibility = 'personal' AND t.owner_profile_id = $3)
  )
ON CONFLICT (profile_id, track_id) DO UPDATE SET updated_at = NOW()
RETURNING profile_id::text, track_id::text,
          (EXTRACT(EPOCH FROM added_at) * 1000)::bigint AS added_at_epoch_ms
```

If no row returns, emit `CanopyError::not_found("track", track_id)`. Apply the same `INSERT ... SELECT` rule to likes and history; history retains the existing profile-consent lock and completes validation plus insertion in its transaction.

- [ ] **Step 7: Filter PostgreSQL list/count/membership queries**

Join the relationship to `tracks` and repeat this predicate in both row and count queries:

```sql
AND t.ingest_status = 'ready'
AND (
    t.visibility = 'release_safe'
    OR ($2::uuid IS NOT NULL AND t.visibility = 'personal' AND t.owner_profile_id = $2)
)
```

Preserve relation ordering (`added_at DESC`, `liked_at DESC`, `played_at DESC, id DESC`). `is_saved` and `is_liked` use the same joined predicate. Deletion queries remain keyed only by relationship ownership and identifier.

- [ ] **Step 8: Add PostgreSQL revocation and paging tests**

Insert accessible and inaccessible relations directly to represent legacy/stale data. Assert owner-visible counts exclude hidden rows before a one-item page, transfer ownership, assert the rows disappear, then call cleanup and verify physical deletion:

```rust
let page = library.list_tracks(&owner.id, &owner_scope, Page { limit: 1, offset: 0 }).await.unwrap();
assert_eq!(page.total_count, 2);
assert!(page.has_more);

settings.set_owner_profile_id(&replacement.id).await.unwrap();
let public_scope = TrackAccessScope::Public;
let page = library.list_tracks(&owner.id, &public_scope, Page { limit: 10, offset: 0 }).await.unwrap();
assert_eq!(page.total_count, 1);
library.remove_track(&owner.id, &owner_track_id).await.unwrap();
```

Run: `./scripts/test-pg.sh`

Expected: PASS for saved, liked, and history subcases.

- [ ] **Step 9: Run service regressions and commit**

Run: `cargo test -p canopy-server library::tests`
Run: `cargo test -p canopy-server likes::tests`
Run: `cargo test -p canopy-server history::tests`
Run: `cargo test -p canopy-server profile::tests`

Expected: PASS with current-owner classification and profile lifecycle behavior intact.

```bash
git add crates/canopy-core/src/repository.rs crates/canopy-server/src/library.rs crates/canopy-server/src/likes.rs crates/canopy-server/src/history.rs crates/canopy-server/src/profile.rs crates/canopy-server/src/jade_store/memory.rs crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/src/lib.rs crates/canopy-server/tests/pg_integration.rs
git commit -m "feat: enforce access on track relationships"
```

---

### Task 6: Enforce Track Access for Playlist Membership

**Files:**
- Modify: `crates/canopy-core/src/repository.rs`
- Modify: `crates/canopy-server/src/playlists.rs`
- Modify: `crates/canopy-server/src/jade_store/memory.rs`
- Modify: `crates/canopy-server/src/jade_store/pg.rs`
- Modify: `crates/canopy-server/src/lib.rs`
- Test: `crates/canopy-server/src/playlists.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `PrincipalService::track_scope_for_profile()` and the canonical track predicate.
- Produces: scope-aware `PlaylistRepository::{add_track, reorder_tracks, list_tracks}` while preserving scope-free removal and playlist deletion.

- [ ] **Step 1: Write failing playlist access and hidden-membership tests**

Cover add rejection, visible listing, owner revocation, cleanup, and reorder semantics:

```rust
service.add_track(&owner_identity, &playlist.id, "owner-ready", None).await.unwrap();
settings.set_owner_profile_id(&replacement.id).await.unwrap();

let visible = service.list_tracks(
    &owner_identity,
    &playlist.id,
    Page { limit: 10, offset: 0 },
).await.unwrap();
assert!(visible.items.is_empty());
assert_eq!(visible.total_count, 0);

service.reorder_tracks(&owner_identity, &playlist.id, &[]).await.unwrap();
service.remove_track(&owner_identity, &playlist.id, "owner-ready").await.unwrap();
```

Add a mixed playlist case where the request reorders all and only visible tracks while one hidden legacy membership remains stored and untouched.

- [ ] **Step 2: Run playlist tests and verify they fail**

Run: `cargo test -p canopy-server playlists::tests::playlist_access_revocation_hides_but_does_not_block_cleanup`
Run: `cargo test -p canopy-server playlists::tests::reorder_requires_only_visible_membership`

Expected: FAIL because playlist repository methods do not accept access scope.

- [ ] **Step 3: Scope playlist repository and service methods**

Use these signatures:

```rust
async fn add_track(
    &self,
    profile_id: &str,
    playlist_id: &str,
    track_id: &str,
    position: Option<i32>,
    scope: &TrackAccessScope,
) -> CanopyResult<PlaylistTrackItem>;

async fn reorder_tracks(
    &self,
    profile_id: &str,
    playlist_id: &str,
    track_ids: &[String],
    scope: &TrackAccessScope,
) -> CanopyResult<Playlist>;

async fn list_tracks(
    &self,
    profile_id: &str,
    playlist_id: &str,
    scope: &TrackAccessScope,
    page: Page,
) -> CanopyResult<PlaylistTrackPage>;
```

Inject `PrincipalService` into `PlaylistService`, derive scope after resolving the profile, and pass it only to add/reorder/list. Keep `remove_track`, playlist metadata operations, and playlist deletion access-independent after playlist ownership validation.

- [ ] **Step 4: Implement in-memory visible-membership behavior**

Give `InMemoryPlaylistStore` the shared catalog dependency. On add, resolve the track under the scope before mutating. On list, resolve/filter memberships before pagination. On reorder:

```rust
async fn accessible_playlist_tracks(
    catalog: &dyn CatalogRepository,
    scope: &TrackAccessScope,
    current: &[PlaylistTrack],
) -> CanopyResult<Vec<PlaylistTrack>> {
    let mut visible = Vec::new();
    for track in current {
        if catalog.get_media(scope, &track.track_id).await?.is_some() {
            visible.push(track.clone());
        }
    }
    Ok(visible)
}

fn same_unique_ids(existing: &[PlaylistTrack], requested: &[String]) -> bool {
    let mut existing_ids: Vec<_> = existing
        .iter()
        .map(|track| track.track_id.as_str())
        .collect();
    existing_ids.sort_unstable();
    let mut requested_ids: Vec<_> = requested.iter().map(String::as_str).collect();
    requested_ids.sort_unstable();
    requested_ids.windows(2).all(|pair| pair[0] != pair[1])
        && existing_ids == requested_ids
}

let visible = accessible_playlist_tracks(&*self.catalog, scope, &current).await?;
if !same_unique_ids(&visible, track_ids) {
    return Err(CanopyError::InvalidArgument(
        "reorder must include exactly the accessible playlist track_ids".into(),
    ));
}
```

Update only rows whose IDs are in `track_ids`; retain hidden rows and their stored positions unchanged.

- [ ] **Step 5: Implement transactional PostgreSQL add and reorder**

Begin `add_track` with a transaction, lock/validate playlist ownership, and insert only from an accessible track selection:

```sql
INSERT INTO profile_playlist_tracks (playlist_id, track_id, position)
SELECT $1, t.id, $3
FROM tracks t
WHERE t.id = $2
  AND t.ingest_status = 'ready'
  AND (
      t.visibility = 'release_safe'
      OR ($4::uuid IS NOT NULL AND t.visibility = 'personal' AND t.owner_profile_id = $4)
  )
ON CONFLICT (playlist_id, track_id) DO UPDATE
SET position = EXCLUDED.position, updated_at = NOW()
RETURNING track_id
```

For reorder, query only accessible memberships inside the existing transaction, compare their sorted UUIDs to the requested UUIDs, and update only those rows. Do not require or update inaccessible memberships. Because the schema has no unique `(playlist_id, position)` constraint, hidden legacy positions may remain unchanged safely.

- [ ] **Step 6: Filter playlist list and count before pagination**

Add the canonical predicate to both the renderable-row query and `COUNT(*)` query. Keep order `ppt.position, ppt.added_at, t.title, t.id` so visible results are deterministic.

- [ ] **Step 7: Run unit and PostgreSQL playlist tests**

Run: `cargo test -p canopy-server playlists::tests`

Run: `./scripts/test-pg.sh`

Expected: PASS; inaccessible additions are `NotFound`, hidden memberships do not affect count/page tokens, reorder ignores but preserves them, and removal succeeds.

- [ ] **Step 8: Commit playlist policy**

```bash
git add crates/canopy-core/src/repository.rs crates/canopy-server/src/playlists.rs crates/canopy-server/src/jade_store/memory.rs crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/src/lib.rs crates/canopy-server/tests/pg_integration.rs
git commit -m "feat: enforce access on playlist tracks"
```

---

### Task 7: Align Playback with Shared Scope and Native Authentication

**Files:**
- Modify: `crates/canopy-server/src/principal.rs`
- Modify: `crates/canopy-server/src/playback.rs`
- Modify: `crates/canopy-server/src/api/grpc/playback.rs`
- Modify: `crates/canopy-server/src/api/grpc/mod.rs`
- Test: `crates/canopy-server/src/playback.rs`
- Test: `crates/canopy-server/src/api/grpc/playback.rs`
- Test: `crates/canopy-server/tests/pg_integration.rs`

**Interfaces:**
- Consumes: `TrackAccessScope` and `extract_optional_track_scope()`.
- Produces: `ResolverService::resolve_at(scope, track_id, now_epoch_ms)` with unchanged opaque `PlaybackSource` and stream capability behavior.

- [ ] **Step 1: Write failing playback scope tests**

Replace principal-oriented expectations with the shared scope and add revoked-policy coverage at stream authorization:

```rust
let source = resolver.resolve_at(
    &TrackAccessScope::Owner { profile_id: "owner-a".into() },
    "owner-track",
    1_000,
).await.unwrap();
assert_eq!(source.track_id, "owner-track");

let error = resolver.resolve_at(
    &TrackAccessScope::Public,
    "owner-track",
    1_000,
).await.unwrap_err();
assert!(matches!(error, CanopyError::NotFound { .. }));
```

- [ ] **Step 2: Run playback tests and verify they fail**

Run: `cargo test -p canopy-server playback::tests`

Expected: FAIL because `ResolverService` still accepts `PlaybackPrincipal`.

- [ ] **Step 3: Replace playback-only principal branching with track scope**

Change `resolve_at` to accept `&TrackAccessScope`:

```rust
let (assets, audience) = match scope {
    TrackAccessScope::Owner { profile_id } => {
        let personal = self.assets.assets_for_personal_playback(profile_id, track_id).await?;
        if personal.is_empty() {
            (self.assets.assets_for_public_playback(track_id).await?, StreamAudience::Public)
        } else {
            (personal, StreamAudience::Owner { profile_id: profile_id.clone() })
        }
    }
    TrackAccessScope::Public => (
        self.assets.assets_for_public_playback(track_id).await?,
        StreamAudience::Public,
    ),
};
```

Retain stream-token audience binding and `authorize_stream_asset` revalidation. Remove `PlaybackPrincipal` and `PrincipalService::classify` after confirming no call sites remain.

- [ ] **Step 4: Move bounded playback to native optional auth**

Replace the legacy stateless helper in `PlaybackGrpc`:

```rust
let metadata = request.metadata().clone();
let track_id = request.into_inner().track_id;
let scope = extract_optional_track_scope(&metadata, &self.0)
    .await
    .map_err(to_status)?;
let source = self.0.resolver
    .resolve_at(&scope, &track_id, current_epoch_ms().map_err(to_status)?)
    .await
    .map_err(to_status)?;
```

Remove bounded-service tests for legacy `AuthService` tokens from `grpc/mod.rs`; retain legacy verification tests in `legacy.rs`.

- [ ] **Step 5: Run playback and stream regression coverage**

Run: `cargo test -p canopy-server playback::tests`
Run: `cargo test -p canopy-server stream::tests`
Run: `cargo test -p canopy-server api::grpc::playback::tests`

Run: `./scripts/test-pg.sh`

Expected: PASS; owner personal-first/public-fallback behavior remains, public callers cannot resolve personal media, and a capability cannot bypass changed owner/visibility/status policy.

- [ ] **Step 6: Commit playback alignment**

```bash
git add crates/canopy-server/src/principal.rs crates/canopy-server/src/playback.rs crates/canopy-server/src/api/grpc/playback.rs crates/canopy-server/src/api/grpc/mod.rs crates/canopy-server/tests/pg_integration.rs
git commit -m "feat: align playback with shared track access"
```

---

### Task 8: Prove the End-to-End Contract and Publish HMI Integration Guidance

**Files:**
- Modify: `crates/canopy-server/tests/local_integration.rs`
- Modify: `docs/client-integration.md`
- Modify: `docs/playback.md`
- Modify: `docs/roadmap.md`

**Interfaces:**
- Consumes: all scoped repository/service/adapter behavior from Tasks 1-7.
- Produces: executable bounded-auth/session regression coverage and the integration contract for independent HMI teams.

- [ ] **Step 1: Add a failing native-session bounded-RPC test**

Extend the existing local integration flow. After login, call search and playback with the native access token, log out that session, then retry both calls with the same token:

```rust
let search = catalog.search(authorized(
    SearchRequest {
        query: "Moonlight".into(),
        page: Some(PageRequest { page_size: 1, page_token: String::new() }),
    },
    &session.access_token,
)).await.unwrap().into_inner();
assert!(!search.tracks.is_empty());

auth.logout(authorized(
    LogoutRequest {},
    &session.access_token,
)).await.unwrap();

let error = catalog.search(authorized(
    SearchRequest {
        query: "Moonlight".into(),
        page: Some(PageRequest { page_size: 1, page_token: String::new() }),
    },
    &session.access_token,
)).await.unwrap_err();
assert_eq!(error.code(), Code::Unauthenticated);
```

Repeat the revoked-token assertion for `ResolvePlayback`. Keep the anonymous search/playback smoke path successful for release-safe tracks.

- [ ] **Step 2: Run the local integration test and verify it fails before the final wiring is present**

Run: `./scripts/local-integration.sh test`

Expected before Tasks 2-7 are complete: FAIL because bounded search/playback use inconsistent authentication or do not recheck the native session.

- [ ] **Step 3: Document the caller access matrix and optional authentication**

Add this table to `docs/client-integration.md` while preserving its current unrelated edits:

```markdown
| Caller | Track results |
| --- | --- |
| No `authorization` metadata | `release_safe` + `ready` tracks |
| Valid non-owner native access token | `release_safe` + `ready` tracks |
| Valid token for the configured instance owner | Public tracks plus that profile's `personal` + `ready` tracks |
| Invalid, expired, malformed, or revoked supplied token | `UNAUTHENTICATED`; never anonymous fallback |
```

State that bounded optional-auth RPCs accept only `authorization: Bearer <access-token>`, that Canopy rechecks the native device session, and that `x-canopy-auth-token` is legacy-only.

- [ ] **Step 4: Document complete opaque pagination**

Add exact HMI integration instructions:

```markdown
For `Browse`, `Search`, discovery-family feeds, saved tracks, likes, history,
playlist lists, and playlist tracks:

1. Send `page_size`; `0` means 20 and values above 100 are clamped to 100.
2. Render the returned page.
3. If `page_info.next_page_token` is non-empty, pass it unchanged as the next
   request's `page_token`.
4. Stop only when `next_page_token` is empty.
5. Start again without a token when authentication, query, parent, genres,
   exclusions, playlist, or another result-shaping input changes.

Page tokens are opaque. Clients must not parse, alter, synthesize, persist as
durable offsets, or reuse them across users or changed request inputs.
```

Explain that access filtering happens before pagination, so callers must not attempt client-side personal/public merging.

- [ ] **Step 5: Document relationship and playback behavior**

In `docs/client-integration.md` and `docs/playback.md`, state:

```markdown
Canopy applies the same track-access rule to catalog, discovery, playback,
saved tracks, likes, history, and playlist tracks. Adding a relationship to an
inaccessible track returns `NOT_FOUND`. Existing relationships to a track that
becomes inaccessible are hidden but retained; removal, unlike, history cleanup,
playlist-track removal, and playlist deletion remain available.

Use `PlaybackService.ResolvePlayback` for every returned track and consume its
opaque URL verbatim. Canopy rechecks track access when resolving playback and
again when authorizing the stream capability.
```

Update `docs/roadmap.md` so catalog/discovery owner access is no longer described as future work; do not claim personalized recommendation ranking was added.

- [ ] **Step 6: Run formatting, unit, compile, PostgreSQL, and local integration verification**

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo test -p canopy-server --features pg --no-run
./scripts/test-pg.sh
./scripts/local-integration.sh test
```

Expected: every command exits 0. The PostgreSQL suite proves all access partitions and pre-pagination counts; local integration proves native token/session behavior plus anonymous and authenticated playback.

- [ ] **Step 7: Confirm the public contract did not change**

Run:

```bash
git diff --exit-code -- crates/canopy-proto/proto/canopy/v1/canopy.proto
git diff --check
```

Expected: the protobuf diff is empty and `git diff --check` reports no whitespace errors.

- [ ] **Step 8: Commit tests and integration documentation**

Before staging, inspect `git diff` for each documentation file and retain any pre-existing user edits. Then stage only the intended hunks and files:

```bash
git add crates/canopy-server/tests/local_integration.rs docs/client-integration.md docs/playback.md docs/roadmap.md
git commit -m "docs: publish access-aware track integration"
```
