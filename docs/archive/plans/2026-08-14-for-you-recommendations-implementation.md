# For You and Recommendations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add canonical `GetForYouFeed` and `GetRecommendations` gRPC methods that initially delegate to discovery, and make PostgreSQL discovery fail closed when its materialized view is empty.

**Architecture:** `canopy-api` adds two additive RPCs with independent request and response messages, publishes new immutable Buf-generated Rust packages, and Canopy pins those exact packages. Canopy keeps one `DiscoveryService::feed` domain path, gives the gRPC adapter a focused constructor and shared response builder, and applies the materialized view's eligibility predicates to the base-table fallback.

**Tech Stack:** Protocol Buffers, Buf CLI/BSR, Rust 2024, Tonic 0.14.6, Prost, Tokio, SQLx, PostgreSQL, Cargo.

## Global Constraints

- The canonical schema lives in `/home/catalina/projects/canopy-api/proto/canopy/v1/canopy.proto`; Canopy's checked-in protobuf is a reference copy only.
- Existing `canopy.v1` RPCs and field numbers remain unchanged.
- `GetForYouFeed` and `GetRecommendations` use dedicated messages even though their initial fields match discovery.
- All three discovery-family RPCs are anonymous-capable in this phase.
- All three RPCs call the existing `DiscoveryService::feed`; no ranking or durable recommendation state is added.
- Eligible feed tracks are non-explicit, `release_safe`, and `ready`.
- Canopy must pin immutable generated SDK versions and must not consume a moving Buf label.
- Preserve unrelated working-tree changes in both repositories.
- Execute inline unless the user explicitly requests subagents.

---

## File Map

### `/home/catalina/projects/canopy-api`

- `proto/canopy/v1/canopy.proto`: canonical RPC and message definitions.
- `scripts/check-contract-boundary.sh`: CI-enforced contract surface assertion.
- `CHANGELOG.md`: release behavior.
- `docs/consumer-guide.md`: client-facing discovery-family usage and authorization.
- `.github/workflows/buf-ci.yml`: existing validation and publication workflow; inspect only unless publication reveals a workflow defect.

### `/home/catalina/projects/Canopy`

- `crates/canopy-proto/Cargo.toml`: immutable generated Prost/Tonic package pins.
- `Cargo.lock`: resolved generated SDK packages.
- `crates/canopy-proto/tests/v1_contract.rs`: generated message and client-method contract.
- `crates/canopy-proto/proto/canopy/v1/canopy.proto`: local canonical-schema reference copy.
- `crates/canopy-server/src/api/grpc/discovery.rs`: three transport methods sharing one feed implementation.
- `crates/canopy-server/src/lib.rs`: focused `DiscoveryGrpc` construction and server registration.
- `crates/canopy-server/tests/proto_contract.rs`: generated server-trait implementation check.
- `crates/canopy-server/src/jade_store/pg.rs`: fail-closed fallback SQL.
- `crates/canopy-server/tests/pg_integration.rs`: empty-materialized-view regression.
- `README.md`: implementation status and non-personalized initial semantics.

---

### Task 1: Add and validate the canonical API contract

**Files:**
- Modify: `/home/catalina/projects/canopy-api/scripts/check-contract-boundary.sh`
- Modify: `/home/catalina/projects/canopy-api/proto/canopy/v1/canopy.proto`
- Modify: `/home/catalina/projects/canopy-api/CHANGELOG.md`
- Modify: `/home/catalina/projects/canopy-api/docs/consumer-guide.md`

**Interfaces:**
- Consumes: existing `DiscoveryService.GetDiscoveryFeed`, `PageRequest`, `PageInfo`, and `TrackSummary`.
- Produces: `GetForYouFeed(GetForYouFeedRequest) -> GetForYouFeedResponse` and `GetRecommendations(GetRecommendationsRequest) -> GetRecommendationsResponse`.

- [ ] **Step 1: Extend the boundary test before changing the schema**

Append this exact invariant to `scripts/check-contract-boundary.sh`:

```bash
discovery_proto="proto/canopy/v1/canopy.proto"
required_discovery_rpcs=(
  GetDiscoveryFeed
  GetForYouFeed
  GetRecommendations
)

for rpc in "${required_discovery_rpcs[@]}"; do
  if ! grep -Eq "^[[:space:]]*rpc ${rpc}\\(" "$discovery_proto"; then
    echo "required DiscoveryService RPC is missing: ${rpc}" >&2
    exit 1
  fi
done
```

- [ ] **Step 2: Run the boundary test and verify RED**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
```

Expected: exit 1 with `required DiscoveryService RPC is missing: GetForYouFeed`.

- [ ] **Step 3: Add the RPCs and dedicated messages**

Add these methods after `GetDiscoveryFeed`:

```proto
  // GetForYouFeed returns the current anonymous-capable discovery feed through a stable endpoint reserved for future personalization.
  rpc GetForYouFeed(GetForYouFeedRequest) returns (GetForYouFeedResponse);
  // GetRecommendations returns the current anonymous-capable discovery feed through a stable endpoint reserved for future recommendation ranking.
  rpc GetRecommendations(GetRecommendationsRequest) returns (GetRecommendationsResponse);
```

Add these messages after `GetDiscoveryFeedResponse`:

```proto
// GetForYouFeedRequest supplies parameters for its corresponding RPC.
message GetForYouFeedRequest {
  // Opaque track identifiers to exclude from the feed on a best-effort basis.
  repeated string exclude_track_ids = 1;
  // Opaque pagination parameters; zero and empty values select service defaults.
  PageRequest page = 2;
}

// GetForYouFeedResponse contains results from its corresponding RPC.
message GetForYouFeedResponse {
  // Ordered renderable track resources in this result page.
  repeated TrackSummary tracks = 1;
  // Opaque pagination continuation information.
  PageInfo page_info = 2;
}

// GetRecommendationsRequest supplies parameters for its corresponding RPC.
message GetRecommendationsRequest {
  // Opaque track identifiers to exclude from the feed on a best-effort basis.
  repeated string exclude_track_ids = 1;
  // Opaque pagination parameters; zero and empty values select service defaults.
  PageRequest page = 2;
}

// GetRecommendationsResponse contains results from its corresponding RPC.
message GetRecommendationsResponse {
  // Ordered renderable track resources in this result page.
  repeated TrackSummary tracks = 1;
  // Opaque pagination continuation information.
  PageInfo page_info = 2;
}
```

Update the `DiscoveryService` comments to state that all three calls are anonymous-capable and that the two named feeds currently share discovery ordering.

- [ ] **Step 4: Verify GREEN and validate protobuf compatibility**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
buf format -w
buf format --diff --exit-code
buf lint
buf build
buf breaking --against buf.build/pandawave/canopy-api:v0.2.0
```

Expected: every command exits 0 and `buf format --diff` prints no diff.

- [ ] **Step 5: Document the additive contract**

Add a `CHANGELOG.md` entry that names both new RPCs, states that they initially mirror discovery, and confirms that no existing wire shape changed.

In `docs/consumer-guide.md`, document:

```markdown
### Discovery-family feeds

`GetDiscoveryFeed`, `GetForYouFeed`, and `GetRecommendations` are
anonymous-capable, use opaque pagination, and accept best-effort track
exclusions. The initial `For You` and recommendations responses intentionally
use the same ordering as discovery; consumers must not infer personalized
ranking until a later contract note says it is available.
```

- [ ] **Step 6: Re-run API checks and commit only scoped files**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
buf format --diff --exit-code
buf lint
buf build
buf breaking --against buf.build/pandawave/canopy-api:v0.2.0
git diff --check
git add proto/canopy/v1/canopy.proto scripts/check-contract-boundary.sh CHANGELOG.md docs/consumer-guide.md
git diff --cached --name-only
git commit -m "feat: add for you and recommendation feeds"
```

Expected: the staged-name check lists exactly those four files before the commit.

---

### Task 2: Publish and read back immutable generated SDK versions

**Files:**
- No source file is edited until publication succeeds.
- Read: `/home/catalina/projects/canopy-api/.github/workflows/buf-ci.yml`
- Later modify: `/home/catalina/projects/Canopy/crates/canopy-proto/Cargo.toml`
- Later modify: `/home/catalina/projects/Canopy/Cargo.lock`

**Interfaces:**
- Consumes: the validated canopy-api commit from Task 1.
- Produces: exact immutable Prost and Tonic Cargo versions that contain both new RPCs.

- [ ] **Step 1: Push the scoped API branch**

Run the push only after confirming the current branch and commit:

```bash
cd /home/catalina/projects/canopy-api
git status --short --branch
git log --oneline -2
git push -u origin codex/for-you-recommendations-api
```

Expected: the branch push succeeds and triggers the existing `Buf CI` workflow. If the current branch is not `codex/for-you-recommendations-api`, create or switch to that branch before Task 1's commit rather than pushing `master`.

- [ ] **Step 2: Wait for validation and publication**

Run:

```bash
api_run_id="$(gh run list --workflow "Buf CI" --branch codex/for-you-recommendations-api --limit 1 --json databaseId --jq '.[0].databaseId')"
test -n "$api_run_id"
gh run view "$api_run_id"
gh run watch "$api_run_id" --exit-status
```

Expected: `documentation-boundary`, `contract`, and `publish` succeed. Do not update Canopy if publication fails.

- [ ] **Step 3: Read back the generated Cargo versions**

Run:

```bash
cargo info --registry buf pandawave_canopy-api_community_neoeinstein-prost
cargo info --registry buf pandawave_canopy-api_community_neoeinstein-tonic
```

Record the literal latest immutable versions whose generated source contains `GetForYouFeed` and `GetRecommendations`. Verify the package source after download:

```bash
grep -RIn "get_for_you_feed\|get_recommendations" /home/catalina/.cargo/registry/src/buf.build-*/pandawave_canopy-api_community_neoeinstein-{prost,tonic}-*/src
```

Expected: the new request/response structs exist in Prost and both client/server methods exist in Tonic.

---

### Task 3: Pin and prove the generated Canopy contract

**Files:**
- Modify: `/home/catalina/projects/Canopy/crates/canopy-proto/tests/v1_contract.rs`
- Modify: `/home/catalina/projects/Canopy/crates/canopy-proto/Cargo.toml`
- Modify: `/home/catalina/projects/Canopy/Cargo.lock`
- Modify: `/home/catalina/projects/Canopy/crates/canopy-proto/proto/canopy/v1/canopy.proto`

**Interfaces:**
- Consumes: exact immutable package versions from Task 2.
- Produces: re-exported `GetForYouFeedRequest`, `GetForYouFeedResponse`, `GetRecommendationsRequest`, `GetRecommendationsResponse`, and generated discovery client/server methods.

- [ ] **Step 1: Write the compile-failing generated-contract test**

Extend `crates/canopy-proto/tests/v1_contract.rs` imports with:

```rust
GetForYouFeedRequest, GetForYouFeedResponse, GetRecommendationsRequest,
GetRecommendationsResponse,
```

Add:

```rust
#[test]
fn audited_v1_exposes_independent_discovery_feed_messages() {
    let page = Some(PageRequest {
        page_size: 25,
        page_token: String::new(),
    });

    let for_you = GetForYouFeedRequest {
        exclude_track_ids: vec!["track-1".into()],
        page: page.clone(),
    };
    let recommendations = GetRecommendationsRequest {
        exclude_track_ids: vec!["track-1".into()],
        page,
    };

    let for_you_response = GetForYouFeedResponse {
        tracks: Vec::new(),
        page_info: None,
    };
    let recommendations_response = GetRecommendationsResponse {
        tracks: Vec::new(),
        page_info: None,
    };

    assert_eq!(for_you.exclude_track_ids, recommendations.exclude_track_ids);
    assert!(for_you_response.tracks.is_empty());
    assert!(recommendations_response.tracks.is_empty());
}

#[allow(dead_code)]
fn generated_discovery_client_has_named_feed_methods(
    client: &mut DiscoveryServiceClient<tonic::transport::Channel>,
    for_you: GetForYouFeedRequest,
    recommendations: GetRecommendationsRequest,
) {
    std::mem::drop(client.get_for_you_feed(for_you));
    std::mem::drop(client.get_recommendations(recommendations));
}
```

- [ ] **Step 2: Run the contract test and verify RED**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo test -p canopy-proto --test v1_contract
```

Expected: compilation fails because the old pinned SDK has no new message types or client methods.

- [ ] **Step 3: Pin the two literal published versions**

Use the exact Prost and Tonic versions read back in Task 2 in `crates/canopy-proto/Cargo.toml`. Keep `registry = "buf"`, package names, aliases, and exact `=` constraints unchanged except for the version strings.

Then resolve only those packages:

```bash
cd /home/catalina/projects/Canopy
cargo update -p pandawave_canopy-api_community_neoeinstein-prost --precise "$PROST_VERSION"
cargo update -p pandawave_canopy-api_community_neoeinstein-tonic --precise "$TONIC_VERSION"
```

Before running these commands, assign `PROST_VERSION` and `TONIC_VERSION` to the literal values verified in Task 2 and reject empty values with `test -n "$PROST_VERSION"` and `test -n "$TONIC_VERSION"`.

- [ ] **Step 4: Synchronize the local reference protobuf**

Copy the canonical schema content from `/home/catalina/projects/canopy-api/proto/canopy/v1/canopy.proto` into `crates/canopy-proto/proto/canopy/v1/canopy.proto` through a reviewed patch. Do not copy repository-specific documentation files.

- [ ] **Step 5: Verify generated-client GREEN and expected server RED**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo test -p canopy-proto --test v1_contract
cargo test -p canopy-server --test proto_contract
```

Expected: `v1_contract` passes. `proto_contract` fails because `DiscoveryGrpc` does not yet implement `get_for_you_feed` and `get_recommendations`.

---

### Task 4: Implement three equivalent gRPC feed methods

**Files:**
- Modify: `/home/catalina/projects/Canopy/crates/canopy-server/src/api/grpc/discovery.rs`
- Modify: `/home/catalina/projects/Canopy/crates/canopy-server/src/lib.rs`
- Test: unit tests in `crates/canopy-server/src/api/grpc/discovery.rs`
- Test: `/home/catalina/projects/Canopy/crates/canopy-server/tests/proto_contract.rs`

**Interfaces:**
- Consumes: generated discovery messages and the existing `crate::discovery::DiscoveryService::feed(&[String], Page) -> CanopyResult<MediaPage>`.
- Produces: `DiscoveryGrpc::new(DiscoveryService, Arc<PageTokenCodec>)` and implementations of all three generated server methods.

- [ ] **Step 1: Write the endpoint-equivalence test before implementation**

Add a test module to `api/grpc/discovery.rs` that builds a focused adapter over two in-memory tracks:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use canopy_core::{MediaItem, PageTokenCodec};
    use canopy_proto::PageRequest;
    use crate::jade_store::InMemoryCatalog;

    fn adapter() -> DiscoveryGrpc {
        let catalog = InMemoryCatalog::with_items(vec![
            MediaItem {
                id: "track-1".into(),
                title: "First".into(),
                artist: "Artist A".into(),
                ..MediaItem::default()
            },
            MediaItem {
                id: "track-2".into(),
                title: "Second".into(),
                artist: "Artist B".into(),
                ..MediaItem::default()
            },
        ]);
        DiscoveryGrpc::new(
            crate::discovery::DiscoveryService::new(Arc::new(catalog)),
            Arc::new(
                PageTokenCodec::new(b"0123456789abcdef0123456789abcdef").unwrap(),
            ),
        )
    }

    #[tokio::test]
    async fn named_feeds_match_discovery_for_equivalent_requests() {
        let grpc = adapter();
        let page = Some(PageRequest {
            page_size: 1,
            page_token: String::new(),
        });
        let excluded = vec!["track-2".to_string()];

        let discovery = grpc
            .get_discovery_feed(Request::new(GetDiscoveryFeedRequest {
                exclude_track_ids: excluded.clone(),
                page: page.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        let for_you = grpc
            .get_for_you_feed(Request::new(GetForYouFeedRequest {
                exclude_track_ids: excluded.clone(),
                page: page.clone(),
            }))
            .await
            .unwrap()
            .into_inner();
        let recommendations = grpc
            .get_recommendations(Request::new(GetRecommendationsRequest {
                exclude_track_ids: excluded,
                page,
            }))
            .await
            .unwrap()
            .into_inner();

        let discovery_ids: Vec<_> = discovery.tracks.iter().map(|track| &track.id).collect();
        let for_you_ids: Vec<_> = for_you.tracks.iter().map(|track| &track.id).collect();
        let recommendation_ids: Vec<_> =
            recommendations.tracks.iter().map(|track| &track.id).collect();

        assert_eq!(for_you_ids, discovery_ids);
        assert_eq!(recommendation_ids, discovery_ids);
        assert_eq!(for_you.page_info, discovery.page_info);
        assert_eq!(recommendations.page_info, discovery.page_info);
    }
}
```

- [ ] **Step 2: Run the adapter test and verify RED**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo test -p canopy-server named_feeds_match_discovery_for_equivalent_requests
```

Expected: compilation fails because `DiscoveryGrpc::new` and the two new trait methods are absent.

- [ ] **Step 3: Give the adapter focused dependencies and one shared feed payload**

Change the adapter to:

```rust
pub struct DiscoveryGrpc {
    discovery: crate::discovery::DiscoveryService,
    page_tokens: Arc<canopy_core::PageTokenCodec>,
}

struct FeedPayload {
    tracks: Vec<canopy_proto::TrackSummary>,
    page_info: Option<canopy_proto::PageInfo>,
}

impl DiscoveryGrpc {
    pub fn new(
        discovery: crate::discovery::DiscoveryService,
        page_tokens: Arc<canopy_core::PageTokenCodec>,
    ) -> Self {
        Self {
            discovery,
            page_tokens,
        }
    }

    async fn feed(
        &self,
        exclude_track_ids: Vec<String>,
        page_request: Option<canopy_proto::PageRequest>,
    ) -> Result<FeedPayload, Status> {
        let page = page_from_request(page_request, &self.page_tokens).map_err(to_status)?;
        let result = self
            .discovery
            .feed(&exclude_track_ids, page)
            .await
            .map_err(to_status)?;
        let page_info = page_info(
            page,
            result.items.len(),
            result.has_more,
            &self.page_tokens,
        )
        .map_err(to_status)?;

        Ok(FeedPayload {
            tracks: result.items.into_iter().map(to_track_summary).collect(),
            page_info: Some(page_info),
        })
    }
}
```

Import the four new message types and alias the generated trait if needed to avoid confusing it with the domain service.

- [ ] **Step 4: Implement each RPC as a thin mapping**

Keep `get_discovery_feed` and add `get_for_you_feed` and `get_recommendations`. Each method must:

1. call `request.into_inner()`;
2. pass `exclude_track_ids` and `page` to `self.feed`;
3. move `tracks` and `page_info` into its dedicated response.

Do not duplicate pagination, repository, filtering, or mapping logic in the RPC methods.

- [ ] **Step 5: Update server construction**

In `crates/canopy-server/src/lib.rs`, replace tuple construction with:

```rust
.add_service(DiscoveryServiceServer::new(DiscoveryGrpc::new(
    services.discovery.clone(),
    services.page_tokens.clone(),
)))
```

Preserve the remaining service registration order and clones.

- [ ] **Step 6: Verify GREEN and commit the Canopy contract/adapter slice**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo fmt --all
cargo test -p canopy-proto --test v1_contract
cargo test -p canopy-server named_feeds_match_discovery_for_equivalent_requests
cargo test -p canopy-server --test proto_contract
git diff --check
git add Cargo.lock crates/canopy-proto/Cargo.toml crates/canopy-proto/proto/canopy/v1/canopy.proto crates/canopy-proto/tests/v1_contract.rs crates/canopy-server/src/api/grpc/discovery.rs crates/canopy-server/src/lib.rs crates/canopy-server/tests/proto_contract.rs
git diff --cached --name-only
git commit -m "feat: expose for you and recommendation feeds"
```

Expected: all three tests pass and only the listed contract/adapter files are staged.

---

### Task 5: Reproduce and fix the PostgreSQL discovery fallback leak

**Files:**
- Modify: `/home/catalina/projects/Canopy/crates/canopy-server/tests/pg_integration.rs`
- Modify: `/home/catalina/projects/Canopy/crates/canopy-server/src/jade_store/pg.rs`

**Interfaces:**
- Consumes: `DiscoveryRepository::shuffle_pool() -> CanopyResult<Vec<MediaItem>>`.
- Produces: fallback results constrained to `is_explicit = FALSE`, `visibility = 'release_safe'`, and `ingest_status = 'ready'`.

- [ ] **Step 1: Add a failing empty-view regression**

Import `DiscoveryRepository` in `pg_integration.rs`. Add a serial-safe test that:

1. runs all migrations;
2. ingests a uniquely named quarantined provider track with `provider_track`;
3. records all non-explicit release-safe/ready track IDs;
4. temporarily quarantines those rows;
5. refreshes `mv_discovery_pool` so it is empty;
6. restores the recorded rows to release-safe/ready without refreshing the view;
7. marks the first restored row explicit;
8. calls `PgCatalogRepository::shuffle_pool`;
9. restores the explicit flag, deletes the provider fixture, and refreshes the view before asserting;
10. asserts that the quarantined fixture and explicit row are absent while at least one remaining eligible row is present.

Use unique IDs and perform cleanup before assertions so a behavioral failure does not leave the shared disposable database altered.

- [ ] **Step 2: Run the PostgreSQL harness and verify RED**

Run:

```bash
cd /home/catalina/projects/Canopy
bash scripts/test-pg.sh
```

Expected: the new regression fails because the fallback currently returns the quarantined fixture and explicit row. The harness must still remove its isolated container and volume.

- [ ] **Step 3: Add the missing predicates**

In the fallback query in `PgCatalogRepository::shuffle_pool`, add before `ORDER BY`:

```sql
WHERE t.is_explicit = FALSE
  AND t.visibility = 'release_safe'
  AND t.ingest_status = 'ready'
```

Do not alter the primary materialized-view query or add a second random sort.

- [ ] **Step 4: Verify GREEN and commit the fix**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo fmt --all
bash scripts/test-pg.sh
git diff --check
git add crates/canopy-server/src/jade_store/pg.rs crates/canopy-server/tests/pg_integration.rs
git diff --cached --name-only
git commit -m "fix: constrain discovery fallback visibility"
```

Expected: the complete PostgreSQL harness passes and only the two fallback files are staged.

---

### Task 6: Align backend documentation and run final verification

**Files:**
- Modify: `/home/catalina/projects/Canopy/README.md`
- Test: complete Canopy workspace and PostgreSQL harness.
- Inspect: both repository diffs and statuses.

**Interfaces:**
- Consumes: completed API publication, generated SDK pin, adapter, and fallback fix.
- Produces: accurate implementation status and final verification evidence.

- [ ] **Step 1: Update implementation documentation**

In the README status table:

- keep search labeled as a prototype;
- mark discovery implemented after the fallback regression is green;
- add or expand a recommendations row stating that `GetForYouFeed` and `GetRecommendations` are implemented aliases of discovery while personalized ranking remains planned.

In the Discovery Service section, replace the future-only wording with:

```markdown
`GetDiscoveryFeed`, `GetForYouFeed`, and `GetRecommendations` currently
share the same public shuffle implementation, including exclusions,
diversification, and opaque pagination. The dedicated endpoint contracts allow
future ranking to evolve independently; no personalized ranking is performed
yet.
```

Correct any nearby search description that claims a streaming response or a materialized full-text-search path when the actual v1 RPC remains unary and PostgreSQL uses direct `pg_trgm` similarity.

- [ ] **Step 2: Run complete Canopy verification**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
bash scripts/test-pg.sh
```

Expected: every command exits 0. Record intentionally ignored tests separately; do not describe them as executed.

- [ ] **Step 3: Re-run canonical API verification**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
buf format --diff --exit-code
buf lint
buf build
buf breaking --against buf.build/pandawave/canopy-api:v0.2.0
```

Expected: every command exits 0.

- [ ] **Step 4: Commit documentation and inspect final scope**

Run:

```bash
cd /home/catalina/projects/Canopy
git add README.md
git diff --cached --name-only
git commit -m "docs: describe discovery feed aliases"
git status --short --branch
git log --oneline -5

cd /home/catalina/projects/canopy-api
git status --short --branch
git log --oneline -3
```

Expected: only `README.md` is staged for the documentation commit, pre-existing unrelated changes remain untouched, and both repositories show the intended commits.

- [ ] **Step 5: Prepare handoff**

Report:

- the canopy-api commit and successful Buf publication run;
- the exact Prost and Tonic versions pinned by Canopy;
- the Canopy commits for adapter, fallback, and documentation;
- targeted RED/GREEN evidence;
- complete verification output;
- ignored full-stack tests, if any;
- any remaining unrelated dirty files without modifying or staging them.

