# API Documentation Ownership Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `canopy-api` a product-neutral, self-teaching gRPC contract repository while moving implementation, HTTP Swagger, deployment, and supported-version truth into Canopy.

**Architecture:** The canonical protobuf and its BSR documentation remain the gRPC source of truth. `canopy-api` gains a consumer guide, compatibility policy, complete protobuf comments, boundary checks, and contract-only publication CI. Canopy keeps exact generated SDK pins, implementation documentation, and an OpenAPI document containing only real HTTP routes; Nginx serves that document at `/openapi.json`.

**Tech Stack:** Protobuf, checksum-pinned Buf CLI and BSR, Rust 2024, Tonic, Axum, Nginx, OpenAPI 3.1 JSON, Bash, GitHub Actions.

## Global Constraints

- `canopy-api` must not mention PandaEngine workflows, Canopy runtime configuration, deployment endpoints, downstream secret names, or downstream CI/CD.
- Protobuf and BSR-generated documentation are authoritative for gRPC.
- OpenAPI must contain only real HTTP routes; it must not model gRPC methods as HTTP paths.
- `canopy-api` documents the latest stable released contract and preserves immutable release references; Canopy documents the exact release it implements.
- Consumers pin immutable generated SDK versions.
- Automation may publish schemas, SDKs, descriptors, documentation, and immutable version metadata; it must not copy prose between repositories.
- Keep the existing `canopy.v1` package and `FILE` breaking policy.
- Do not add HTTP transcoding annotations or a REST gateway.
- Do not modify or remove `/home/catalina/projects/canopy-api/.idea/`.
- The user manages Git. The executing agent must not stage, commit, push, switch branches, create tags, or modify GitHub secrets. Each task ends at a user-managed Git checkpoint.

## File Map

### `canopy-api`

- `README.md`: product-neutral entry point and current stable release.
- `docs/consumer-guide.md`: complete consumer handoff for transport, SDKs, auth, errors, pagination, and resource semantics.
- `docs/compatibility.md`: package-version, compatibility, deprecation, and release policy.
- `CONTRIBUTING.md`: maintainer-only ownership matrix and release procedure.
- `.github/CODEOWNERS`: contract and documentation ownership.
- `.github/workflows/buf-ci.yml`: contract checks and BSR publication for this repository only.
- `scripts/install-buf.sh`: installs the pinned Buf CLI only after verifying its release checksum.
- `scripts/check-contract-boundary.sh`: regression guard against product/runtime documentation drift.
- `proto/canopy/v1/canopy.proto`: canonical wire contract and generated BSR documentation.
- `buf.yaml`: `STANDARD` plus `COMMENTS` linting and existing `FILE` breaking policy.
- `CHANGELOG.md`: released `v0.1.0` and `v0.2.0` history plus current unreleased documentation work.
- Delete `docs/architecture.md`: mixed implementation/migration prose has other owners.
- Delete `openapi/openapi.json`: mixed pseudo-gRPC and implementation HTTP paths violate the approved boundary.

### Canopy

- `docs/openapi.json`: real Canopy/Nginx HTTP routes only.
- `crates/canopy-server/tests/http_openapi.rs`: executable OpenAPI ownership and path assertions.
- `deploy/nginx/canopy-stream.conf`: serve the Canopy-owned OpenAPI JSON.
- `docker-compose.streaming-test.yml`: mount the OpenAPI JSON read-only in the Nginx harness.
- `scripts/test-streaming.sh`: verify runtime OpenAPI exposure.
- `README.md`: current backend implementation and exact contract-support status.
- `docs/canopy-api-consumption.md`: Canopy-specific BSR facade, pins, upgrade, and verification procedure.
- `docs/canopy-api-bsr-design.md`: short supersession pointer preserving historical links.
- `crates/canopy-proto/Cargo.toml`: product-neutral facade description with immutable SDK pins unchanged.
- `Cargo.lock`: records the test-only direct Tonic dependency while preserving immutable SDK versions.
- `crates/canopy-proto/tests/v1_contract.rs`: all generated bounded-service client surfaces compile.
- `.github/workflows/ci.yml`: use lockfile-enforced Cargo commands.

---

### Task 1: Product-Neutral Consumer Documentation And Boundary Guard

**Files:**
- Create: `../canopy-api/scripts/check-contract-boundary.sh`
- Create: `../canopy-api/docs/consumer-guide.md`
- Create: `../canopy-api/docs/compatibility.md`
- Create: `../canopy-api/CONTRIBUTING.md`
- Create: `../canopy-api/.github/CODEOWNERS`
- Modify: `../canopy-api/README.md`
- Modify: `../canopy-api/CHANGELOG.md`
- Delete: `../canopy-api/docs/architecture.md`
- Delete: `../canopy-api/openapi/openapi.json`

**Interfaces:**
- Consumes: published module `buf.build/pandawave/canopy-api`, stable release `v0.2.0`, BSR commit `145678c1d73e45b7bbaebf7e16ee4d64`, Prost SDK `0.5.0-00000000000000-145678c1d73e.2`, and Tonic SDK `0.5.0-00000000000000-145678c1d73e.4`.
- Produces: `docs/consumer-guide.md`, `docs/compatibility.md`, and `scripts/check-contract-boundary.sh` for later CI and downstream links.

- [x] **Step 1: Add a failing contract-boundary check**

Create `../canopy-api/scripts/check-contract-boundary.sh` with executable mode:

```bash
#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

shopt -s nullglob
contract_docs=(README.md docs/*.md proto/canopy/v1/*.proto)

forbidden='PandaEngine|CANOPY_[A-Z0-9_]+|secrets\.[A-Z0-9_]+|/internal/stream/authorize|/nginx-health|PostgreSQL session recheck|SMTP delivery'

if grep -En "$forbidden" "${contract_docs[@]}"; then
  echo "product-specific or implementation-specific content found in contract documentation" >&2
  exit 1
fi

if [[ -e openapi/openapi.json ]]; then
  echo "mixed OpenAPI companion must not live in the contract repository" >&2
  exit 1
fi
```

- [x] **Step 2: Run the boundary check and verify the current repository fails**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
```

Expected: non-zero exit with matches in `README.md` or `docs/architecture.md`, followed by the mixed OpenAPI error.

- [x] **Step 3: Replace the README with a contract-owned entry point**

The new `../canopy-api/README.md` must contain these sections and facts:

````markdown
# Canopy API

Product-neutral protobuf contract published as `canopy.v1`.

## Contract

- Canonical source: `proto/canopy/v1/canopy.proto`
- Private BSR module: `buf.build/pandawave/canopy-api`
- Current stable release: `v0.2.0`
- Immutable stable commit: `145678c1d73e45b7bbaebf7e16ee4d64`
- Consumer setup: `docs/consumer-guide.md`
- Compatibility policy: `docs/compatibility.md`

Protobuf and BSR documentation are authoritative. Consumers pin immutable
generated SDK versions and treat server endpoints and deployment configuration
as implementation-provided settings.

## Validate

```bash
buf format --diff --exit-code
buf lint
buf build
bash scripts/check-contract-boundary.sh
```

## Releases

Backward-compatible changes remain in `canopy.v1`. Breaking redesigns use a
new protobuf package version. See `CHANGELOG.md` and `docs/compatibility.md`.
````

Do not include implementation status, backend configuration, consumer names, or downstream work.

- [x] **Step 4: Write the consumer guide**

Create `../canopy-api/docs/consumer-guide.md` with the following exact content model:

1. **Authority and discovery**
   - Protobuf is canonical.
   - BSR hosts versioned schema documentation and generated SDKs.
   - OpenAPI is not used for gRPC client generation.
2. **Rust SDK setup**
   - Show this registry configuration:

```toml
[registries.buf]
index = "sparse+https://buf.build/gen/cargo/"
credential-provider = "cargo:token"
```

   - Show `cargo login --registry buf "Bearer {token}"` as a local developer command.
   - Show these immutable dependencies:

```toml
[dependencies]
canopy-api-prost = { package = "pandawave_canopy-api_community_neoeinstein-prost", version = "=0.5.0-00000000000000-145678c1d73e.2", registry = "buf" }
canopy-api-tonic = { package = "pandawave_canopy-api_community_neoeinstein-tonic", version = "=0.5.0-00000000000000-145678c1d73e.4", registry = "buf" }
tonic = { version = "0.14.6", features = ["transport"] }
```

3. **Channel and metadata**
   - Implementations supply the endpoint and trust configuration.
   - Protected calls carry lowercase gRPC metadata key `authorization` and value `Bearer <access-token>`.
   - Absent metadata is anonymous only on anonymous-capable RPCs; malformed or invalid supplied metadata returns `UNAUTHENTICATED` and never downgrades to anonymous.
4. **Service authorization matrix**
   - Anonymous-capable: `CatalogService`, `DiscoveryService`, `PlaybackService`; a valid bearer may add identity context.
   - Public status: `SystemService.GetStatus`.
   - Auth bootstrap: `RegisterPassword`, `ResendVerification`, `VerifyEmail`, `LoginPassword`, `RequestPasswordReset`, `CompletePasswordReset`, `BeginGoogleLogin`, `CompleteGoogleLogin`, and `RefreshSession` do not require bearer metadata.
   - Protected auth: `ChangePassword`, `LinkGoogle`, `UnlinkGoogle`, `Logout`, `LogoutAll`, `ListSessions`, `RevokeSession`, `GetAccount`, and `DeleteAccount` require bearer metadata.
   - Durable user state: every `ProfileService`, `HistoryService`, `LibraryService`, and `PlaylistService` RPC requires bearer metadata and an existing profile.
5. **Session lifecycle**
   - `SessionEnvelope` is returned only by successful email verification, password login, Google login, and refresh.
   - Consumers store the access token, refresh token, both expiration fields, account, and session as one atomic replacement.
   - Only one refresh may be in flight per session.
   - Refresh rotates the token; concurrent or repeated use of an old refresh token can revoke the session family.
   - An ambiguous transport failure after sending refresh is not retried with the same token; the consumer requires reauthentication.
   - Password reset, password change, logout-all, and account deletion invalidate sessions; consumers clear local credentials after success.
6. **Google flow**
   - Call `BeginGoogleLogin`, bind the returned nonce to the provider request, and submit the resulting ID token plus challenge ID to `CompleteGoogleLogin`.
   - Handle the `GoogleLoginResponse` oneof exhaustively.
   - For `account_link_required`, authenticate the existing account and call protected `LinkGoogle` with the returned link challenge.
7. **Out-of-band challenges**
   - Verification and reset tokens arrive through an implementation-owned delivery channel.
   - Consumers pass the token unchanged to `VerifyEmail` or `CompletePasswordReset`.
   - The contract does not define email URL or application deep-link formats.
8. **Errors**
   - Branch on gRPC status codes, never status message text.
   - Document `INVALID_ARGUMENT`, `UNAUTHENTICATED`, `PERMISSION_DENIED`, `NOT_FOUND`, `ALREADY_EXISTS`, `FAILED_PRECONDITION`, `ABORTED`, `RESOURCE_EXHAUSTED`, `UNAVAILABLE`, and `INTERNAL` using canonical meanings.
   - State that `RESOURCE_EXHAUSTED` does not currently guarantee structured retry metadata; use bounded client backoff.
9. **Pagination and resources**
   - Page tokens are opaque and must be returned unchanged.
   - Page size zero selects the server default; consumers must tolerate server clamping.
   - `google.protobuf.Timestamp` is used for persisted instants; session envelope expiration fields are epoch milliseconds.
   - `PlaybackSource.stream_url` is opaque and used verbatim until `expires_at`.
   - Playlist reorder sends complete ordered membership and `expected_revision`; `ABORTED` means refetch and reconcile.
10. **Tonic request example**

```rust
use canopy_api_prost::canopy::v1::GetAccountRequest;
use canopy_api_tonic::canopy::v1::tonic::auth_service_client::AuthServiceClient;
use tonic::{Request, metadata::MetadataValue, transport::Channel};

async fn get_account(
    client: &mut AuthServiceClient<Channel>,
    access_token: &str,
) -> Result<(), tonic::Status> {
    let mut request = Request::new(GetAccountRequest {});
    let authorization = MetadataValue::try_from(format!("Bearer {access_token}"))
        .map_err(|_| tonic::Status::invalid_argument("invalid access token metadata"))?;
    request
        .metadata_mut()
        .insert("authorization", authorization);
    client.get_account(request).await?;
    Ok(())
}
```

- [x] **Step 5: Write the compatibility policy**

Create `../canopy-api/docs/compatibility.md` with these rules:

- `canopy.v1` is the current compatibility boundary.
- Additive fields and RPCs preserve proto3 default behavior for older consumers.
- Existing field numbers and names are never reused; removed fields are reserved by number and name.
- Incompatible redesigns use `canopy.v2` and coexist during migration.
- `FILE` breaking checks run against the pull-request base and released BSR history.
- BSR labels such as `main` and `v0.2.0` are discovery references; consumers pin immutable generated SDK versions.
- A contract release does not assert that a particular server deployment implements it.
- Deprecation requires a leading protobuf deprecation comment, a changelog entry, and at least one compatible release before removal in a new package version.
- Consumers tolerate unknown enum values, unknown fields, absent optional fields, and additional oneof variants by failing closed where security is involved.

- [x] **Step 6: Add maintainer governance**

Create `../canopy-api/.github/CODEOWNERS`:

```text
* @adrianrusu16
/proto/ @adrianrusu16
/docs/ @adrianrusu16
/buf.yaml @adrianrusu16
```

Create `../canopy-api/CONTRIBUTING.md` with this ownership matrix:

| Claim | Owner | Verification |
| --- | --- | --- |
| Wire shape and field semantics | `canopy-api` | `buf lint`, `buf build`, `buf breaking` |
| Contract auth/error/pagination behavior | `canopy-api` | protobuf comments and consumer guide review |
| Generated SDK publication | `canopy-api` | BSR push and generated SDK availability |
| Server support and deployment behavior | server implementation | server conformance and integration tests |
| Consumer storage, UI, and CI/CD | each consumer | consumer integration tests |

The release checklist must require: update protobuf comments, update consumer docs, update `CHANGELOG.md`, run all local contract checks, merge, let repository CI publish `main`, and create a Git tag only when intentionally applying a matching release label.

- [x] **Step 7: Record real release history and remove mixed documents**

Rewrite `../canopy-api/CHANGELOG.md`:

```markdown
# Changelog

## Unreleased

- Separate product-neutral contract guidance from implementation and consumer documentation.
- Add a complete consumer guide, compatibility policy, ownership guard, and generated-documentation linting.

## v0.2.0 - 2026-07-11

- Add password, Google identity, account lifecycle, and per-device session RPCs to `canopy.v1.AuthService`.
- Add canonical saved-track, liked-track, and playlist-track resource shapes.

## v0.1.0 - 2026-07-03

- Establish the audited bounded-service `canopy.v1` contract.
```

Delete `../canopy-api/docs/architecture.md` and `../canopy-api/openapi/openapi.json`. Git history retains the migration record; current documentation no longer owns product implementation or fake HTTP routes.

- [x] **Step 8: Run the boundary and baseline contract checks**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
buf format --diff --exit-code
buf lint
buf build
```

Expected: all commands exit zero.

- [x] **Step 9: User-managed Git checkpoint**

Report the exact changed/deleted files and stop for the user's commit decision. Do not stage or commit.

---

### Task 2: Complete Protobuf Documentation And Enforce Comment Coverage

**Files:**
- Modify: `../canopy-api/buf.yaml`
- Modify: `../canopy-api/proto/canopy/v1/canopy.proto`

**Interfaces:**
- Consumes: cross-cutting behavior in `../canopy-api/docs/consumer-guide.md`.
- Produces: BSR-generated API reference where every service, RPC, message, field, and oneof has a leading comment.

- [x] **Step 1: Enable the complete Buf comments category**

Change the lint block in `../canopy-api/buf.yaml` to:

```yaml
lint:
  use:
    - STANDARD
    - COMMENTS
  except:
    - RPC_REQUEST_RESPONSE_UNIQUE
    - RPC_RESPONSE_STANDARD_NAME
  disallow_comment_ignores: true
```

Keep `breaking.use: [FILE]` unchanged.

- [x] **Step 2: Run Buf lint and verify missing comments fail**

Run:

```bash
cd /home/catalina/projects/canopy-api
buf lint
```

Expected: failures including `COMMENT_RPC`, `COMMENT_MESSAGE`, `COMMENT_FIELD`, and `COMMENT_ONEOF`.

- [x] **Step 3: Document every service and RPC**

Add a leading comment to every RPC. The comments must encode these exact behavioral promises:

- `CatalogService`: anonymous browse/search/get; opaque pagination; `GetMedia` returns `NOT_FOUND` for unavailable identifiers.
- `PlaybackService.ResolvePlayback`: anonymous-capable; optional valid bearer context; opaque expiring stream URL; invalid supplied auth is `UNAUTHENTICATED`.
- `DiscoveryService.GetDiscoveryFeed`: anonymous-capable, batched, paginated, and excludes requested track IDs on a best-effort basis.
- `ProfileService`: bearer and profile required; update masks control mutation; delete may return `FAILED_PRECONDITION` for protected ownership state.
- `HistoryService`: bearer and profile required; disabling history purges history; recording returns `recorded=false` when disabled; deletes are idempotent.
- `LibraryService`: bearer and profile required; save/like and remove/unlike are idempotent; list methods return renderable resources plus relationship timestamps.
- `PlaylistService`: bearer and profile required; cross-owner identifiers are concealed as `NOT_FOUND`; add/remove are idempotent; reorder requires complete membership and expected revision, with stale revisions returning `ABORTED`.
- `AuthService`: use the public/protected matrix from the consumer guide; generic accepted responses resist account enumeration; verification/login/refresh session issuance; refresh rotation/reuse behavior; Google oneof/link proof; password reset/change session invalidation; session/account operations are idempotent where documented.
- `SystemService.GetStatus`: public status response; dependency details are implementation-defined and consumers tolerate unknown dependency names.

Do not mention Argon2, Ed25519, PostgreSQL, SMTP, Google tokeninfo endpoints, environment variables, or deployment topology.

- [x] **Step 4: Document every message, field, and oneof**

Use leading comments, not trailing comments. Apply these exact semantic rules throughout the file:

- Every `*_id` is an opaque stable identifier unless explicitly described as a challenge or session identifier.
- `PageRequest.page_token` and `PageInfo.next_page_token` are opaque and never parsed by consumers.
- `page_size = 0` selects the server default and larger values may be clamped.
- Optional message/scalar fields distinguish absence from a default value.
- Media durations and positions are milliseconds.
- `PlaybackSource.stream_url` is opaque and usable only until `expires_at`.
- `google.protobuf.Timestamp` fields are UTC instants.
- `google.protobuf.FieldMask` paths are lower_snake_case protobuf field names.
- `Preferences.values` is an application-defined object; consumers preserve unknown keys.
- `HistorySettings.enabled=false` means durable history recording is disabled.
- `completion_ratio` is in the inclusive range `0.0..=1.0`.
- Saved, liked, and playlist-track messages combine `TrackSummary` with relationship metadata.
- Playlist `revision` is the optimistic concurrency value used by reorder.
- Reorder `track_ids` is the complete ordered membership with no duplicates.
- `GenericAuthResponse.accepted` acknowledges request handling without proving account existence or state.
- Account status is an open string; consumers tolerate unknown future values.
- `SessionEnvelope` expiration integers are Unix epoch milliseconds and travel with the replacement token pair.
- Password fields are secrets and must not be logged; policy violations return `INVALID_ARGUMENT`.
- Challenge, verification, reset, refresh, link, nonce, and ID-token fields are opaque secrets and must not be logged.
- `GoogleLoginResponse.result` is an exclusive oneof; consumers handle both `session` and `account_link_required` and fail closed on an unknown future variant.
- `SessionSummary.current` identifies the session represented by the bearer token used for the list call.
- Dependency name/status/message fields are implementation-defined diagnostics, not an enum contract.

Comments on empty request messages must state the required bearer context or public nature of the call.

- [x] **Step 5: Verify formatting, comment coverage, and wire compatibility**

Run:

```bash
cd /home/catalina/projects/canopy-api
buf format -w
buf format --diff --exit-code
buf lint
buf build
buf breaking --against buf.build/pandawave/canopy-api:v0.2.0
bash scripts/check-contract-boundary.sh
```

Expected: all commands exit zero; the comment-only protobuf changes are `FILE` compatible.

- [x] **Step 6: User-managed Git checkpoint**

Report that `buf.yaml` and `proto/canopy/v1/canopy.proto` are ready for review. Do not stage or commit.

---

### Task 3: Contract-Only CI And BSR Publication

**Files:**
- Create: `../canopy-api/scripts/install-buf.sh`
- Create: `../canopy-api/.github/workflows/buf-ci.yml`
- Modify: `../canopy-api/CONTRIBUTING.md`

**Interfaces:**
- Consumes: `scripts/check-contract-boundary.sh`, `buf.yaml`, the named BSR module, official Buf `1.71.0` release checksum `d3de2838c68a5759ca276884254bc70df4e4ad185d6ed5f65f327b6ce6363eab`, and repository secret `BUF_TOKEN`.
- Produces: secret-free pull-request contract gates plus checksum-verified BSR publication and deleted-label archival for trusted repository events.

- [x] **Step 1: Add the checksum-pinned Buf installer**

Create `../canopy-api/scripts/install-buf.sh` with executable mode:

```bash
#!/usr/bin/env bash
set -euo pipefail

version="1.71.0"
sha256="d3de2838c68a5759ca276884254bc70df4e4ad185d6ed5f65f327b6ce6363eab"
asset="buf-Linux-x86_64"
install_root="${RUNNER_TEMP:?RUNNER_TEMP must be set}/buf-${version}"
binary="${install_root}/buf"

mkdir -p "$install_root"
curl \
  --fail \
  --location \
  --proto '=https' \
  --retry 3 \
  --show-error \
  --silent \
  --tlsv1.2 \
  --output "$binary" \
  "https://github.com/bufbuild/buf/releases/download/v${version}/${asset}"

printf '%s  %s\n' "$sha256" "$binary" | sha256sum --check --status
chmod 0755 "$binary"
printf '%s\n' "$install_root" >>"${GITHUB_PATH:?GITHUB_PATH must be set}"
```

- [x] **Step 2: Add the token-isolated Buf workflow**

Create `../canopy-api/.github/workflows/buf-ci.yml`:

```yaml
name: Buf CI

on:
  push:
  pull_request:
    types: [opened, synchronize, reopened]
  delete:

permissions:
  contents: read

jobs:
  documentation-boundary:
    if: github.event_name != 'delete'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: bash scripts/check-contract-boundary.sh

  contract:
    if: github.event_name != 'delete'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Install checksum-pinned Buf CLI
        run: bash scripts/install-buf.sh
      - name: Check format, lint, and build
        run: |
          buf format --diff --exit-code
          buf lint
          buf build
      - name: Check compatibility against the pull-request base
        if: github.event_name == 'pull_request'
        env:
          BASE_REF: ${{ github.base_ref }}
        run: buf breaking --against ".git#ref=origin/${BASE_REF}"

  publish:
    if: github.event_name == 'push'
    needs: [documentation-boundary, contract]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - name: Install checksum-pinned Buf CLI
        run: bash scripts/install-buf.sh
      - name: Check stable compatibility and publish named modules
        env:
          BUF_TOKEN: ${{ secrets.BUF_TOKEN }}
        run: |
          test -n "${BUF_TOKEN}"
          buf breaking --against buf.build/pandawave/canopy-api:v0.2.0
          buf push --git-metadata

  archive-deleted-label:
    if: github.event_name == 'delete'
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install checksum-pinned Buf CLI
        run: bash scripts/install-buf.sh
      - name: Archive the deleted Git ref label
        env:
          BUF_TOKEN: ${{ secrets.BUF_TOKEN }}
          DELETED_REF: ${{ github.event.ref }}
        run: |
          test -n "${BUF_TOKEN}"
          buf registry module label archive "buf.build/pandawave/canopy-api:${DELETED_REF}"
```

The workflow never passes `BUF_TOKEN` to a third-party action. Pull requests compare against their fetched Git base without a registry credential. Only the trusted push compatibility/publication step and delete archival step expose the secret to the checksum-verified Buf CLI.

- [x] **Step 3: Document the workflow's secret boundary**

In `CONTRIBUTING.md`, state:

- `BUF_TOKEN` is used only by the checksum-verified Buf CLI for stable-release compatibility, publication, and deleted-label archival.
- Pull requests from forks run every non-publication check without receiving the secret.
- Buf `1.71.0` and its official Linux x86_64 SHA-256 checksum are immutable workflow inputs.
- The secret is scoped to one publication or archival step and discarded with that step.
- No third-party GitHub Action receives `BUF_TOKEN`.
- The existing BSR repository must already exist because `buf push` never receives `--create`.

- [x] **Step 4: Verify the installer and local CI-equivalent checks**

Run:

```bash
cd /home/catalina/projects/canopy-api
RUNNER_TEMP=/tmp/canopy-api-buf-ci-test \
GITHUB_PATH=/tmp/canopy-api-buf-ci-test/github-path \
bash scripts/install-buf.sh
/tmp/canopy-api-buf-ci-test/buf-1.71.0/buf --version
bash -n scripts/install-buf.sh
bash scripts/check-contract-boundary.sh
buf format --diff --exit-code
buf lint
buf build
buf breaking --against buf.build/pandawave/canopy-api:v0.2.0
```

Expected: the downloaded binary verifies and reports `1.71.0`; all contract checks exit zero.

- [x] **Step 5: User-managed GitHub and Git checkpoint**

Ask the user to verify that `BUF_TOKEN` exists in the `adrianrusu16/canopy-api` repository settings before pushing. Do not read, print, create, stage, commit, or push the secret or workflow.

---

### Task 4: Canopy-Owned HTTP OpenAPI And Runtime Exposure

**Files:**
- Create: `crates/canopy-server/tests/http_openapi.rs`
- Modify: `docs/openapi.json`
- Modify: `deploy/nginx/canopy-stream.conf`
- Modify: `docker-compose.streaming-test.yml`
- Modify: `scripts/test-streaming.sh`

**Interfaces:**
- Consumes: real routes `/nginx-health`, `/stream/{capability}`, and private `/internal/stream/authorize`.
- Produces: Canopy-owned OpenAPI 3.1 JSON served by Nginx at `/openapi.json`; no gRPC-shaped HTTP paths.

- [x] **Step 1: Add a failing OpenAPI ownership test**

Create `crates/canopy-server/tests/http_openapi.rs`:

```rust
use std::collections::BTreeSet;

use serde_json::Value;

#[test]
fn openapi_documents_only_real_canopy_http_routes() {
    let document: Value = serde_json::from_str(include_str!("../../../docs/openapi.json"))
        .expect("docs/openapi.json must be valid JSON");
    assert_eq!(document["openapi"], "3.1.0");

    let paths = document["paths"]
        .as_object()
        .expect("OpenAPI paths must be an object");
    let actual = paths.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "/internal/stream/authorize",
        "/nginx-health",
        "/openapi.json",
        "/stream/{capability}",
    ]);

    assert_eq!(actual, expected);
    assert!(paths.keys().all(|path| !path.starts_with("/canopy.")));
    assert_eq!(
        paths["/internal/stream/authorize"]["get"]["x-internal"],
        true
    );
}
```

- [x] **Step 2: Run the focused test and verify the mixed document fails**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo test -p canopy-server --test http_openapi --locked
```

Expected: failure because the current document is OpenAPI 3.0.3 and contains pseudo-gRPC paths.

- [x] **Step 3: Replace the mixed OpenAPI document**

Rewrite `docs/openapi.json` as OpenAPI 3.1.0 with exactly these operations:

```json
{
  "openapi": "3.1.0",
  "info": {
    "title": "Canopy HTTP API",
    "version": "0.1.0",
    "description": "Real HTTP endpoints exposed by the Canopy streaming deployment. The gRPC contract is documented by protobuf and the Buf Schema Registry."
  },
  "paths": {
    "/openapi.json": {
      "get": {
        "operationId": "getOpenApiDocument",
        "summary": "Get the Canopy HTTP OpenAPI document",
        "responses": {
          "200": {
            "description": "The current Canopy HTTP OpenAPI document.",
            "content": { "application/json": { "schema": { "type": "object" } } }
          }
        }
      }
    },
    "/nginx-health": {
      "get": {
        "operationId": "getNginxHealth",
        "summary": "Check Nginx liveness",
        "responses": {
          "200": {
            "description": "Nginx is accepting requests.",
            "content": { "text/plain": { "schema": { "type": "string" } } }
          }
        }
      }
    },
    "/stream/{capability}": {
      "get": {
        "operationId": "streamMedia",
        "summary": "Stream media using an opaque capability",
        "parameters": [
          {
            "name": "capability",
            "in": "path",
            "required": true,
            "description": "Opaque capability returned by the gRPC playback resolver.",
            "schema": { "type": "string" }
          },
          {
            "name": "Range",
            "in": "header",
            "required": false,
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "200": { "description": "Complete media response." },
          "206": { "description": "Byte-range media response." },
          "403": { "description": "Capability is missing, invalid, expired, or no longer authorized." },
          "416": { "description": "Requested byte range is not satisfiable." },
          "default": { "description": "Streaming dependency failure." }
        }
      }
    },
    "/internal/stream/authorize": {
      "get": {
        "operationId": "authorizeStreamInternal",
        "summary": "Authorize an Nginx stream subrequest",
        "x-internal": true,
        "parameters": [
          {
            "name": "x-canopy-stream-token",
            "in": "header",
            "required": true,
            "schema": { "type": "string" }
          }
        ],
        "responses": {
          "204": {
            "description": "Authorized.",
            "headers": {
              "x-accel-redirect": { "schema": { "type": "string" } },
              "x-canopy-content-type": { "schema": { "type": "string" } }
            }
          },
          "403": { "description": "Denied without revealing policy details." },
          "503": { "description": "Authorization dependency unavailable." }
        }
      }
    }
  }
}
```

- [x] **Step 4: Serve the OpenAPI document through Nginx**

Add this exact location before the catch-all location in `deploy/nginx/canopy-stream.conf`:

```nginx
location = /openapi.json {
    alias /srv/canopy/docs/openapi.json;
    default_type application/json;
    add_header Cache-Control "no-cache" always;
    access_log off;
}
```

Add this read-only volume to the Nginx service in `docker-compose.streaming-test.yml`:

```yaml
- ./docs/openapi.json:/srv/canopy/docs/openapi.json:ro
```

- [x] **Step 5: Extend the streaming harness**

After Nginx becomes healthy in `scripts/test-streaming.sh`, add:

```bash
openapi_document="$(curl --fail --silent --show-error http://127.0.0.1:18080/openapi.json)"
grep -Fq '"openapi": "3.1.0"' <<<"$openapi_document"
grep -Fq '"/stream/{capability}"' <<<"$openapi_document"
if grep -Fq '"/canopy.' <<<"$openapi_document"; then
  echo "OpenAPI must not expose gRPC methods as HTTP paths" >&2
  exit 1
fi
```

- [x] **Step 6: Run focused and integration verification**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo test -p canopy-server --test http_openapi --locked
bash scripts/test-streaming.sh
```

Expected: the Rust test passes, `/openapi.json` returns the four-route document, and all existing range/auth/revocation tests remain green.

- [x] **Step 7: User-managed Git checkpoint**

Report the OpenAPI, Nginx, Compose, test, and harness changes. Do not stage or commit.

---

### Task 5: Canopy Contract Consumption And Owned Documentation

**Files:**
- Modify: `crates/canopy-proto/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/canopy-proto/tests/v1_contract.rs`
- Modify: `README.md`
- Create: `docs/canopy-api-consumption.md`
- Modify: `docs/canopy-api-bsr-design.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: stable `canopy-api` release `v0.2.0`, BSR commit `145678c1d73e45b7bbaebf7e16ee4d64`, and the product-neutral consumer guide.
- Produces: Canopy-owned support declaration, complete generated-client compile checks, and lockfile-enforced CI.

- [x] **Step 1: Extend the generated-client compile test**

Add these imports and test to `crates/canopy-proto/tests/v1_contract.rs`:

```rust
use canopy_proto::{
    auth_service_client::AuthServiceClient,
    catalog_service_client::CatalogServiceClient,
    discovery_service_client::DiscoveryServiceClient,
    history_service_client::HistoryServiceClient,
    library_service_client::LibraryServiceClient,
    playback_service_client::PlaybackServiceClient,
    playlist_service_client::PlaylistServiceClient,
    profile_service_client::ProfileServiceClient,
    system_service_client::SystemServiceClient,
};

#[test]
fn audited_v1_exposes_every_bounded_service_client() {
    fn accepts<T>(_client: Option<T>) {}

    accepts::<CatalogServiceClient<tonic::transport::Channel>>(None);
    accepts::<PlaybackServiceClient<tonic::transport::Channel>>(None);
    accepts::<DiscoveryServiceClient<tonic::transport::Channel>>(None);
    accepts::<ProfileServiceClient<tonic::transport::Channel>>(None);
    accepts::<HistoryServiceClient<tonic::transport::Channel>>(None);
    accepts::<LibraryServiceClient<tonic::transport::Channel>>(None);
    accepts::<PlaylistServiceClient<tonic::transport::Channel>>(None);
    accepts::<AuthServiceClient<tonic::transport::Channel>>(None);
    accepts::<SystemServiceClient<tonic::transport::Channel>>(None);
}

Add `tonic = { workspace = true }` under `[dev-dependencies]` in `crates/canopy-proto/Cargo.toml`, then refresh `Cargo.lock` offline before running the locked test.
```

Run:

```bash
cd /home/catalina/projects/Canopy
cargo test -p canopy-proto --test v1_contract --locked
```

Expected: pass against the pinned generated SDKs.

- [x] **Step 2: Remove consumer-specific facade wording**

Change `crates/canopy-proto/Cargo.toml` description to:

```toml
description = "Canopy facade over the published canopy.v1 gRPC contract."
```

Keep both existing exact dependency versions unchanged. They are the machine-verifiable primary support declaration.

- [x] **Step 3: Create the Canopy consumption document**

Create `docs/canopy-api-consumption.md` with:

- Module: `buf.build/pandawave/canopy-api`.
- Supported release: `v0.2.0`.
- Full BSR commit: `145678c1d73e45b7bbaebf7e16ee4d64`.
- Exact Prost and Tonic versions from `crates/canopy-proto/Cargo.toml`.
- `.cargo/config.toml` registry configuration and local `cargo login` command.
- Facade rule: re-export generated packages; never redefine protobuf messages locally.
- Upgrade procedure: review changelog and compatibility policy, update both exact SDK pins together, run facade/client compile tests, run all Canopy gates, then update the supported release statement.
- Canopy verification commands:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
bash scripts/test-pg.sh
bash scripts/test-streaming.sh
cargo clippy --workspace --all-features --tests --locked -- -D warnings
```

- Link to `https://github.com/adrianrusu16/canopy-api/blob/master/docs/consumer-guide.md` for contract semantics.
- Keep Canopy runtime/auth/deployment details in the Canopy README and implementation docs.

- [x] **Step 4: Preserve the old design URL without stale content**

Replace `docs/canopy-api-bsr-design.md` with:

```markdown
# Canopy API BSR Design

**Status:** Completed and superseded.

The BSR migration is complete. Current Canopy-specific dependency pins,
upgrade steps, and verification live in
[Canopy API Consumption](canopy-api-consumption.md).

Product-neutral contract semantics and compatibility policy live in the
canonical `canopy-api` repository.
```

Historical implementation plans may continue linking to this stable pointer.

- [x] **Step 5: Correct the Canopy README status**

Update only Canopy-owned claims:

- Rename the document heading and introduction so it describes the Canopy backend rather than claiming ownership of the whole ecosystem.
- API contract row: `v0.2.0`, full commit `145678c1d73e45b7bbaebf7e16ee4d64`, and the two immutable generated SDK pins.
- Auth/Profile row: implemented, including SMTP delivery and native durable-state authorization.
- CI row: Canopy runs lockfile-enforced Rust/PG/streaming gates; `canopy-api` owns Buf compatibility and publication gates.
- Replace the stale sentence saying breaking checks begin after `v0.1.0` with a link to `docs/canopy-api-consumption.md` and the contract repository.
- Do not describe a consumer's token storage, bridge, UI, or CI/CD.

- [x] **Step 6: Make Canopy CI honor immutable pins**

Add `--locked` to every Cargo command in `.github/workflows/ci.yml`:

```yaml
cargo check --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-features --tests --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
```

Keep the repository-owned `BUF_TOKEN` secret for Canopy dependency resolution, but scope `CARGO_REGISTRIES_BUF_TOKEN` to only the Cargo and test-script run steps. Checkout, toolchain, and cache actions must not receive the secret.

Run `bash scripts/test-streaming.sh` in the test job so the documented streaming/OpenAPI gate is enforced in CI. `cargo fmt` does not resolve dependencies and must not receive the registry token.

- [x] **Step 7: Verify Canopy-owned changes**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo fmt --all -- --check
cargo test -p canopy-proto --test v1_contract --locked
cargo test -p canopy-server --test http_openapi --locked
```

Expected: all commands exit zero.

- [x] **Step 8: User-managed Git checkpoint**

Report the exact Canopy-owned documentation, facade, test, and CI changes. Do not stage or commit.

---

### Task 6: Cross-Repository Verification And Release Handoff

**Files:**
- Verify all files changed by Tasks 1-5.
- Modify only files that fail the checks below.

**Interfaces:**
- Consumes: product-neutral contract docs, strict protobuf comments, Buf CI, Canopy HTTP OpenAPI, and Canopy's immutable SDK pins.
- Produces: a review-ready two-repository change set with no staged files and no stale active documentation.

- [x] **Step 1: Run the complete local `canopy-api` gate and bind the stable check to CI**

Run:

```bash
cd /home/catalina/projects/canopy-api
bash scripts/check-contract-boundary.sh
buf format --diff --exit-code
buf lint
buf build
buf breaking --against '.git#ref=HEAD'
git diff --check
```

Expected: all local commands exit zero. The trusted push job runs `buf breaking --against buf.build/pandawave/canopy-api:v0.2.0` immediately before publication; the remote comparison is intentionally deferred to CI so local verification does not export the working schema.

- [x] **Step 2: Run the complete Canopy gate**

Run:

```bash
cd /home/catalina/projects/Canopy
cargo fmt --all -- --check
cargo test --workspace --locked
bash scripts/test-pg.sh
bash scripts/test-streaming.sh
cargo clippy --workspace --all-features --tests --locked -- -D warnings
git diff --check
```

Expected: all commands exit zero. The streaming harness's intentionally ignored standalone test remains covered by `scripts/test-streaming.sh`.

- [x] **Step 3: Validate JSON and documentation boundaries**

Run:

```bash
cd /home/catalina/projects/Canopy
python3 -m json.tool docs/openapi.json >/dev/null
rg -n '"/canopy\.|production email delivery outside|v0\.1\.0 release label remain' \
  docs/openapi.json \
  docs/canopy-api-consumption.md \
  README.md
rg -n 'PandaEngine|CANOPY_[A-Z0-9_]+|/internal/stream/authorize|/nginx-health' \
  ../canopy-api/README.md \
  ../canopy-api/docs \
  ../canopy-api/proto/canopy/v1/canopy.proto
```

Expected: JSON validation succeeds and both `rg` commands return no matches. Historical completed plans are outside this active-document scan.

- [x] **Step 4: Review both repository diffs**

Run:

```bash
cd /home/catalina/projects/canopy-api
git status --short --branch
git diff --stat
git diff -- README.md CONTRIBUTING.md CHANGELOG.md docs proto buf.yaml .github scripts

cd /home/catalina/projects/Canopy
git status --short --branch
git diff --stat
git diff -- README.md docs/openapi.json docs/canopy-api-consumption.md \
  docs/canopy-api-bsr-design.md crates/canopy-proto crates/canopy-server/tests/http_openapi.rs \
  deploy/nginx/canopy-stream.conf docker-compose.streaming-test.yml scripts/test-streaming.sh \
  .github/workflows/ci.yml
```

Expected: only intended files appear; `.idea/`, unrelated owner-aware playback documents, and user changes remain untouched; no files are staged.

- [x] **Step 5: User-managed publication order**

Present this order without executing it:

1. The user reviews, commits, and pushes `canopy-api`.
2. The checksum-verified Buf CLI validates and publishes the named module with immutable Git metadata.
3. The current stable release remains `v0.2.0` because this change is documentation/comment compatible; a new version label is created only by an explicit release decision.
4. The user reviews, commits, and pushes Canopy.
5. Canopy CI proves the existing immutable `v0.2.0` generated SDK pins still compile and pass all integration gates.

- [x] **Step 6: Final user-managed Git checkpoint**

Summarize validation evidence and list both repository change sets. Do not stage, commit, tag, or push.

## Reference Material

- Buf CLI v1.71.0 release checksums: `https://github.com/bufbuild/buf/releases/tag/v1.71.0`
- Buf module publication and labels: `https://buf.build/docs/bsr/module/publish/`
- Buf `COMMENTS` lint category: `https://buf.build/docs/lint/rules/`
- BSR Cargo SDK consumption: `https://buf.build/docs/bsr/generated-sdks/cargo/`

## Self-Review Checklist

- Spec coverage: contract ownership, consumer onboarding, complete protobuf documentation, compatibility, CI/CD boundary, Swagger split, immutable Canopy pins, governance, migration, and verification each map to a task.
- Placeholder scan: no deferred implementation markers or unspecified file paths remain.
- Type consistency: generated client module names match `canopy.v1`; OpenAPI path assertions match Nginx/Axum routes; BSR module, release, commit, and SDK versions match current Canopy pins.
- Scope: no PandaEngine implementation, REST gateway, cross-repository prose synchronization, unrelated architecture refactor, Git operation, or secret mutation is included.
