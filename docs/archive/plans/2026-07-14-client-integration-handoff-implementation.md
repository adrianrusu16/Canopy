# Client Integration Handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give any client team a Canopy-owned, secret-free deployment handoff whose local reference configuration is machine-readable and continuously checked against Canopy's public surfaces and exact `canopy.v1` SDK pins.

**Architecture:** `docs/client-integration.md` teaches the backend-to-client connection boundary, while `deploy/client-connection.example.json` carries the same local public values in a strict versioned shape. A Rust integration test deserializes the reference, validates URL and secret-boundary invariants, checks the real HTTP OpenAPI route, and cross-checks Canopy's exact generated SDK dependency declarations. Existing documents link to the handoff rather than duplicating it.

**Tech Stack:** Rust 2024, Serde, serde_json, reqwest::Url, Cargo integration tests, Markdown, JSON, existing `canopy.v1` BSR-generated Prost/Tonic SDKs.

## Global Constraints

- Follow `docs/superpowers/specs/2026-07-14-client-integration-handoff-design.md` exactly.
- Keep all work in the Canopy repository; do not modify `canopy-api` or any client repository.
- Do not change protobuf declarations, database migrations, runtime listeners, or authentication behavior.
- Keep `deploy/client-connection.example.json` secret-free and local-reference-only; it is not a runtime discovery endpoint.
- Plaintext public URLs are allowed only for loopback development; production handoff requirements use TLS.
- Do not expose `CANOPY_STREAM_AUTH_ADDR`, database addresses, SMTP settings, token-signing material, outbox-sealing material, or stream-token secrets to clients.
- Preserve the corrected BOM-free `docs/openapi.json`; do not relax JSON parsing to tolerate the obsolete document.
- Work with existing user changes. Do not revert `docs/canopy-api-bsr-design.md`, owner-aware playback documents, or other unrelated files.
- The user owns Git. Do not stage, commit, tag, push, switch branches, or modify secrets.
- Use ASCII for all new and modified files.

## File Map

- `deploy/client-connection.example.json`: strict versioned local public connection reference.
- `crates/canopy-server/tests/client_handoff.rs`: conformance tests for the JSON, contract pins, OpenAPI route, human handoff, and repository navigation.
- `docs/client-integration.md`: human-readable deployment-to-client handoff.
- `docs/canopy-api-consumption.md`: server-side contract consumption document that links to client connection guidance.
- `README.md`: repository navigation links from API verification and server startup sections.

---

### Task 1: Versioned Machine-Readable Connection Reference

**Files:**
- Create: `crates/canopy-server/tests/client_handoff.rs`
- Create: `deploy/client-connection.example.json`
- Verify: `crates/canopy-proto/Cargo.toml`
- Verify: `docs/openapi.json`

**Interfaces:**
- Consumes: exact Canopy SDK dependency declarations and the backend-owned HTTP OpenAPI document.
- Produces: `ClientConnectionReference` test schema and `deploy/client-connection.example.json` schema version 1 for later documentation tests.

- [x] **Step 1: Write the failing machine-reference conformance test**

Create `crates/canopy-server/tests/client_handoff.rs`:

```rust
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

const EXPECTED_BSR_MODULE: &str = "buf.build/pandawave/canopy-api";
const EXPECTED_RELEASE: &str = "v0.2.0";
const EXPECTED_COMMIT: &str = "145678c1d73e45b7bbaebf7e16ee4d64";
const EXPECTED_PROST_PACKAGE: &str = "pandawave_canopy-api_community_neoeinstein-prost";
const EXPECTED_PROST_VERSION: &str = "=0.5.0-00000000000000-145678c1d73e.2";
const EXPECTED_TONIC_PACKAGE: &str = "pandawave_canopy-api_community_neoeinstein-tonic";
const EXPECTED_TONIC_VERSION: &str = "=0.5.0-00000000000000-145678c1d73e.4";

const FORBIDDEN_KEYS: &[&str] = &[
    "database_url",
    "smtp_host",
    "smtp_port",
    "smtp_username",
    "smtp_password",
    "identity_access_token_signing_key",
    "auth_outbox_sealing_key",
    "stream_token_secret",
    "stream_auth_addr",
    "private_authorization_url",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientConnectionReference {
    schema_version: u32,
    environment: String,
    contract: ContractReference,
    transport: TransportReference,
    authentication: AuthenticationReference,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractReference {
    protobuf_package: String,
    bsr_module: String,
    release: String,
    commit: String,
    prost_package: String,
    prost_version: String,
    tonic_package: String,
    tonic_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransportReference {
    grpc_endpoint: String,
    stream_base_url: String,
    openapi_url: String,
    tls_required_outside_loopback: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationReference {
    metadata_key: String,
    metadata_scheme: String,
    verification_action_relative_path: String,
    verification_token_query_parameter: String,
    password_reset_action_relative_path: String,
    password_reset_token_query_parameter: String,
    expiry_query_parameter: String,
    auth_service_requires_postgresql: bool,
    password_bootstrap_requires_email_delivery: bool,
}

fn load_reference() -> ClientConnectionReference {
    serde_json::from_str(include_str!(
        "../../../deploy/client-connection.example.json"
    ))
    .expect("client connection reference must be valid schema-versioned JSON")
}

fn assert_no_forbidden_keys(value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                assert!(
                    !FORBIDDEN_KEYS.contains(&key.as_str()),
                    "client connection reference exposes forbidden key {key}"
                );
                assert_no_forbidden_keys(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                assert_no_forbidden_keys(child);
            }
        }
        _ => {}
    }
}

fn assert_public_url(url: &Url) {
    assert!(matches!(url.scheme(), "http" | "https"));
    if url.scheme() == "http" {
        assert!(
            matches!(
                url.host_str(),
                Some("127.0.0.1" | "localhost" | "::1")
            ),
            "plaintext client URL must be loopback-only: {url}"
        );
    }
}

#[test]
fn client_connection_reference_is_public_versioned_and_consistent() {
    let raw = include_str!("../../../deploy/client-connection.example.json");
    let raw_value: Value = serde_json::from_str(raw).expect("reference JSON must parse");
    assert_no_forbidden_keys(&raw_value);

    let reference = load_reference();
    assert_eq!(reference.schema_version, 1);
    assert_eq!(reference.environment, "local-reference");

    assert_eq!(reference.contract.protobuf_package, "canopy.v1");
    assert_eq!(reference.contract.bsr_module, EXPECTED_BSR_MODULE);
    assert_eq!(reference.contract.release, EXPECTED_RELEASE);
    assert_eq!(reference.contract.commit, EXPECTED_COMMIT);
    assert_eq!(reference.contract.prost_package, EXPECTED_PROST_PACKAGE);
    assert_eq!(reference.contract.prost_version, EXPECTED_PROST_VERSION);
    assert_eq!(reference.contract.tonic_package, EXPECTED_TONIC_PACKAGE);
    assert_eq!(reference.contract.tonic_version, EXPECTED_TONIC_VERSION);

    let proto_manifest = include_str!("../../canopy-proto/Cargo.toml");
    assert!(proto_manifest.contains(&format!(
        "package = \"{}\", version = \"{}\"",
        EXPECTED_PROST_PACKAGE, EXPECTED_PROST_VERSION
    )));
    assert!(proto_manifest.contains(&format!(
        "package = \"{}\", version = \"{}\"",
        EXPECTED_TONIC_PACKAGE, EXPECTED_TONIC_VERSION
    )));

    let grpc = Url::parse(&reference.transport.grpc_endpoint)
        .expect("gRPC endpoint must be an absolute URL");
    let stream = Url::parse(&reference.transport.stream_base_url)
        .expect("stream base URL must be absolute");
    let openapi =
        Url::parse(&reference.transport.openapi_url).expect("OpenAPI URL must be absolute");
    assert_public_url(&grpc);
    assert_public_url(&stream);
    assert_public_url(&openapi);
    assert!(reference.transport.tls_required_outside_loopback);
    assert_eq!(stream.scheme(), openapi.scheme());
    assert_eq!(stream.host_str(), openapi.host_str());
    assert_eq!(stream.port_or_known_default(), openapi.port_or_known_default());
    assert_eq!(openapi.path(), "/openapi.json");

    let openapi_document: Value = serde_json::from_str(include_str!(
        "../../../docs/openapi.json"
    ))
    .expect("docs/openapi.json must remain valid JSON");
    assert!(openapi_document["paths"].get(openapi.path()).is_some());

    assert_eq!(reference.authentication.metadata_key, "authorization");
    assert_eq!(reference.authentication.metadata_scheme, "Bearer");
    assert_eq!(
        reference.authentication.verification_action_relative_path,
        "verify-email"
    );
    assert_eq!(
        reference.authentication.verification_token_query_parameter,
        "token"
    );
    assert_eq!(
        reference.authentication.password_reset_action_relative_path,
        "reset-password"
    );
    assert_eq!(
        reference
            .authentication
            .password_reset_token_query_parameter,
        "token"
    );
    assert_eq!(
        reference.authentication.expiry_query_parameter,
        "expires_at"
    );
    assert!(reference.authentication.auth_service_requires_postgresql);
    assert!(
        reference
            .authentication
            .password_bootstrap_requires_email_delivery
    );
}
```

- [x] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p canopy-server --test client_handoff --locked
```

Expected: compilation fails because
`deploy/client-connection.example.json` does not exist. This proves the test
is coupled to the required machine artifact.

- [x] **Step 3: Add the exact local reference JSON**

Create `deploy/client-connection.example.json`:

```json
{
  "schema_version": 1,
  "environment": "local-reference",
  "contract": {
    "protobuf_package": "canopy.v1",
    "bsr_module": "buf.build/pandawave/canopy-api",
    "release": "v0.2.0",
    "commit": "145678c1d73e45b7bbaebf7e16ee4d64",
    "prost_package": "pandawave_canopy-api_community_neoeinstein-prost",
    "prost_version": "=0.5.0-00000000000000-145678c1d73e.2",
    "tonic_package": "pandawave_canopy-api_community_neoeinstein-tonic",
    "tonic_version": "=0.5.0-00000000000000-145678c1d73e.4"
  },
  "transport": {
    "grpc_endpoint": "http://127.0.0.1:50051",
    "stream_base_url": "http://127.0.0.1:8080",
    "openapi_url": "http://127.0.0.1:8080/openapi.json",
    "tls_required_outside_loopback": true
  },
  "authentication": {
    "metadata_key": "authorization",
    "metadata_scheme": "Bearer",
    "verification_action_relative_path": "verify-email",
    "verification_token_query_parameter": "token",
    "password_reset_action_relative_path": "reset-password",
    "password_reset_token_query_parameter": "token",
    "expiry_query_parameter": "expires_at",
    "auth_service_requires_postgresql": true,
    "password_bootstrap_requires_email_delivery": true
  }
}
```

- [x] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
cargo test -p canopy-server --test client_handoff --locked
```

Expected: `client_connection_reference_is_public_versioned_and_consistent`
passes.

- [x] **Step 5: Review the machine artifact without Git mutation**

Run:

```bash
git diff --check -- deploy/client-connection.example.json crates/canopy-server/tests/client_handoff.rs
git status --short -- deploy/client-connection.example.json crates/canopy-server/tests/client_handoff.rs
```

Expected: no whitespace errors; only the two intended new files appear. Do not
stage or commit them.

---

### Task 2: Human Deployment-To-Client Handoff

**Files:**
- Modify: `crates/canopy-server/tests/client_handoff.rs`
- Create: `docs/client-integration.md`
- Modify: `docs/canopy-api-consumption.md`

**Interfaces:**
- Consumes: `ClientConnectionReference`, `load_reference`, and the schema-1 JSON from Task 1.
- Produces: stable human guidance and a server-side documentation link for repository navigation.

- [x] **Step 1: Add the failing human-document conformance test**

Append to `crates/canopy-server/tests/client_handoff.rs`:

```rust
#[test]
fn client_integration_docs_track_the_reference_contract() {
    let reference = load_reference();
    let handoff = include_str!("../../../docs/client-integration.md");
    let server_consumption = include_str!("../../../docs/canopy-api-consumption.md");

    for expected in [
        reference.contract.bsr_module.as_str(),
        reference.contract.release.as_str(),
        reference.contract.commit.as_str(),
        reference.contract.prost_version.as_str(),
        reference.contract.tonic_version.as_str(),
        reference.transport.grpc_endpoint.as_str(),
        reference.transport.stream_base_url.as_str(),
        reference.transport.openapi_url.as_str(),
    ] {
        assert!(
            handoff.contains(expected),
            "client integration handoff is missing {expected}"
        );
    }

    for required in [
        "CANOPY_GRPC_ADDR=127.0.0.1:50051",
        "CANOPY_AUTH_PUBLIC_BASE_URL",
        "verify-email",
        "reset-password",
        "SessionEnvelope",
        "authorization",
        "CANOPY_STREAM_AUTH_ADDR",
        "PostgreSQL",
        "SMTP",
    ] {
        assert!(
            handoff.contains(required),
            "client integration handoff is missing required guidance: {required}"
        );
    }

    assert!(server_consumption.contains(
        "[Client Integration Handoff](client-integration.md)"
    ));
}
```

- [x] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p canopy-server --test client_handoff --locked
```

Expected: compilation fails because `docs/client-integration.md` does not
exist. After that file is present, the test must still fail until
`docs/canopy-api-consumption.md` contains the required link.

- [x] **Step 3: Create the complete client integration handoff**

Create `docs/client-integration.md` with this content:

````markdown
# Client Integration Handoff

This document tells any client team how to connect to a deployed Canopy
backend. The canonical protobuf resources, RPC behavior, authorization matrix,
pagination rules, and status-code meanings live in the
[canopy-api consumer guide](https://github.com/adrianrusu16/canopy-api/blob/master/docs/consumer-guide.md).
Canopy owns the concrete server support and deployment boundary documented
here.

## Supported Contract

- Protobuf package: `canopy.v1`
- BSR module: `buf.build/pandawave/canopy-api`
- Stable release: `v0.2.0`
- Immutable BSR commit: `145678c1d73e45b7bbaebf7e16ee4d64`
- Prost SDK: `=0.5.0-00000000000000-145678c1d73e.2`
- Tonic SDK: `=0.5.0-00000000000000-145678c1d73e.4`

Generated SDK versions are immutable dependency pins. Release labels are for
discovery and communication. See [Canopy API Consumption](canopy-api-consumption.md)
for the backend's dependency-upgrade procedure.

## Public Surfaces

The checked-in [local reference configuration](../deploy/client-connection.example.json)
uses these public values:

| Surface | Local reference | Purpose |
| --- | --- | --- |
| gRPC | `http://127.0.0.1:50051` | All `canopy.v1` RPCs. |
| Streaming | `http://127.0.0.1:8080` | Opaque playback URLs returned by gRPC. |
| HTTP OpenAPI | `http://127.0.0.1:8080/openapi.json` | Real Canopy HTTP routes only. |

The reference requires `CANOPY_GRPC_ADDR=127.0.0.1:50051`. Nginx serves the
streaming and OpenAPI origin separately. `CANOPY_STREAM_AUTH_ADDR` is a private
Nginx-to-Canopy listener and must never be called or disclosed as a client
endpoint.

The JSON file is a versioned reference artifact, not runtime discovery.
Clients must reject unknown `schema_version` values instead of guessing at a
new shape.

## Local And Remote Connectivity

Loopback URLs work only when the client shares the server's network namespace
or has an explicit local forwarding path. A client outside that namespace
needs an operator-provided reachable host or DNS name. The operator may bind
Canopy to `0.0.0.0:50051`, but `0.0.0.0` is a listen address and is never a
client endpoint.

Canopy authentication requires the PostgreSQL build:

```bash
cargo run -p canopy-server --features pg --bin canopy
```

The operator must satisfy all server configuration and migration requirements
in the README before sharing the public endpoints.

## Production Transport

Outside loopback development, the deployment provides TLS gRPC and HTTPS
streaming endpoints. The external proxy or ingress must support HTTP/2 gRPC,
present a certificate trusted by the client platform, preserve gRPC status and
metadata, and route streaming URLs without exposing private Nginx locations.

The client handoff includes public DNS names, ports when nonstandard, TLS
server names, and any private-CA installation requirement. It never includes
database configuration, SMTP credentials, token-signing keys, outbox-sealing
keys, stream-token secrets, or private authorization addresses.

## Password Authentication Bootstrap

`AuthService` is registered only when Canopy runs with PostgreSQL support. A
functional registration or password-recovery flow also requires SMTP delivery.
`CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true` leaves messages queued and is not a
usable external-client bootstrap.

The password flow is:

1. Call `RegisterPassword`. Treat the generic accepted response as
   non-enumerating acknowledgement, not proof that an account was created.
2. Canopy joins `verify-email` to `CANOPY_AUTH_PUBLIC_BASE_URL` and delivers
   an email URL containing `token` and `expires_at` query parameters.
3. The client-owned action handler passes `token` unchanged to `VerifyEmail`
   with a device label.
4. Atomically store the complete returned `SessionEnvelope`.
5. Send lowercase `authorization` metadata as `Bearer <access-token>` on
   protected calls.
6. Permit only one refresh in flight for a session. After `RefreshSession`,
   atomically replace the complete `SessionEnvelope`.
7. Do not retry an ambiguously completed refresh with the same refresh token;
   require authentication again because reuse can revoke the session family.
8. After successful `Logout`, clear the current local envelope. `LogoutAll`,
   password reset, and account deletion invalidate all account sessions.

Password reset joins `reset-password` to `CANOPY_AUTH_PUBLIC_BASE_URL` and
uses the same `token` and `expires_at` query parameters. Tokens and page tokens
are opaque and must be passed unchanged.

When Google login is enabled, the deployment operator also provides the
accepted public OAuth client IDs. Google client secrets and provider-validation
credentials are never client handoff values.

## Playback

Call `PlaybackService.ResolvePlayback` over gRPC and use the returned
`PlaybackSource.stream_url` verbatim until its expiry. Do not construct stream
paths, parse capabilities, derive storage paths, or call private authorization
routes. Streaming range behavior and capability revalidation belong to the
public streaming endpoint.

## Errors And Recovery

- Connection, DNS, and TLS failures are deployment failures, not gRPC
  application statuses.
- Branch on canonical gRPC status codes, never message text.
- Invalid supplied credentials return `UNAUTHENTICATED` and never downgrade a
  request to anonymous.
- An unknown handoff schema version fails closed.
- An ambiguous refresh requires reauthentication rather than refresh-token
  replay.
- Use the `canopy-api` consumer guide for operation idempotency and canonical
  status-code meanings.

## Deployment Handoff Checklist

Before declaring an environment client-ready, the backend operator provides
and verifies:

1. The schema-versioned, secret-free connection values.
2. A reachable TLS gRPC endpoint and certificate trust requirements.
3. A reachable HTTPS streaming origin and `/openapi.json` URL.
4. The exact supported BSR release, commit, and SDK versions.
5. A client-owned `CANOPY_AUTH_PUBLIC_BASE_URL` action origin.
6. Working PostgreSQL migrations and SMTP delivery for password bootstrap.
7. Accepted public Google OAuth client IDs when Google login is enabled.
8. A successful public status call, registration delivery, verification,
   login, refresh, logout, playback resolution, and ranged stream request.

The client never receives `CANOPY_STREAM_AUTH_ADDR`, database addresses, SMTP
credentials, signing or sealing keys, stream secrets, raw test tokens, or
private infrastructure routes.

## Focused Troubleshooting

| Symptom | Backend handoff check |
| --- | --- |
| Connection refused | Confirm the shared endpoint is reachable from the client's network namespace and is not a listen-only address. |
| TLS handshake failure | Confirm DNS, TLS server name, certificate chain, and client trust configuration. |
| `UNIMPLEMENTED` for AuthService | Confirm Canopy was built and started with PostgreSQL support. |
| Registration accepted but no email arrives | Confirm SMTP delivery and readiness; the undelivered-email escape hatch is not an external flow. |
| `UNAUTHENTICATED` after login | Confirm lowercase `authorization: Bearer <access-token>` metadata and current envelope expiry. |
| Refresh invalidates the session | Confirm the client serialized refresh and never replayed an old or ambiguously used token. |
| Playback URL returns `403` | Resolve playback again; the opaque capability may be expired or policy may have changed. |
````

- [x] **Step 4: Link server-side consumption guidance to the handoff**

Insert this paragraph after the opening paragraph of
`docs/canopy-api-consumption.md`:

```markdown
Deployment-provided public endpoints, TLS requirements, authentication action
links, and client-ready environment checks live in the
[Client Integration Handoff](client-integration.md).
```

- [x] **Step 5: Run the focused test and verify GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test client_handoff --locked
```

Expected: both client handoff tests pass and formatting is unchanged.

- [x] **Step 6: Review the human handoff without Git mutation**

Run:

```bash
rg -n 'PandaEngine|CANOPY_DATABASE_URL|CANOPY_SMTP_PASSWORD|CANOPY_STREAM_TOKEN_SECRET' \
  docs/client-integration.md deploy/client-connection.example.json
git diff --check -- docs/client-integration.md docs/canopy-api-consumption.md
```

Expected: the boundary scan returns no matches and the whitespace check exits
zero. Do not stage or commit.

---

### Task 3: Repository Navigation

**Files:**
- Modify: `crates/canopy-server/tests/client_handoff.rs`
- Modify: `README.md`

**Interfaces:**
- Consumes: `docs/client-integration.md` from Task 2.
- Produces: two discoverable README entry points for backend dependency and public client connection guidance.

- [x] **Step 1: Add the failing README navigation test**

Append to `crates/canopy-server/tests/client_handoff.rs`:

```rust
#[test]
fn readme_links_the_client_integration_handoff_from_relevant_sections() {
    let readme = include_str!("../../../README.md");
    let link = "[Client Integration Handoff](docs/client-integration.md)";
    assert_eq!(
        readme.matches(link).count(),
        2,
        "README must link the handoff from API verification and server startup"
    );
}
```

- [x] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p canopy-server --test client_handoff --locked
```

Expected: `readme_links_the_client_integration_handoff_from_relevant_sections`
fails because the README does not yet contain the link.

- [x] **Step 3: Add the API verification navigation link**

Replace the final sentence of the CI paragraph near the existing Canopy API
Consumption link with:

```markdown
The PostgreSQL harness starts an isolated PostgreSQL 18.4 Compose project, applies the migration chain, runs feature tests serially, and destroys the stack. `scripts/test-streaming.sh` adds a real Nginx container and synthetic MP3, verifies the served HTTP OpenAPI document, `206` range responses, denial behavior, and immediate policy revocation. Canopy uses lockfile-enforced Cargo commands; the canonical `canopy-api` repository owns Buf format, lint, compatibility, and publication gates. See [Canopy API Consumption](docs/canopy-api-consumption.md) for backend dependency pins and [Client Integration Handoff](docs/client-integration.md) for deployment-provided client connection values.
```

- [x] **Step 4: Add the running-server navigation link**

After the production-style PostgreSQL `cargo run` example in the
`Running the Server` section, insert:

```markdown
The checked-in local client reference uses
`CANOPY_GRPC_ADDR=127.0.0.1:50051` and a separately served Nginx origin at
`http://127.0.0.1:8080`. See the
[Client Integration Handoff](docs/client-integration.md) before sharing an
environment with a client team.
```

- [x] **Step 5: Run the focused test and verify GREEN**

Run:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test client_handoff --locked
```

Expected: all three client handoff tests pass.

- [x] **Step 6: Review README scope without Git mutation**

Run:

```bash
rg -n 'Client Integration Handoff|CANOPY_GRPC_ADDR=127.0.0.1:50051' README.md
git diff --check -- README.md
```

Expected: exactly two handoff links and one local reference address appear;
the whitespace check exits zero. Do not stage or commit.

---

### Task 4: Full Verification And Backend Handoff

**Files:**
- Verify all files changed by Tasks 1-3.
- Modify only files that fail the checks below.

**Interfaces:**
- Consumes: strict reference JSON, human handoff, navigation links, current OpenAPI, and exact generated SDK pins.
- Produces: a review-ready backend handoff with no staged files and an explicit next-slice boundary.

- [x] **Step 1: Run formatting and focused handoff tests**

Run:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test client_handoff --locked
cargo test -p canopy-server --test http_openapi --locked
```

Expected: formatting succeeds; three handoff tests and the OpenAPI ownership
test pass.

- [x] **Step 2: Run the complete locked workspace gate**

Run:

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-features --tests --locked -- -D warnings
```

Expected: all workspace tests pass and Clippy emits no warnings.

- [x] **Step 3: Validate artifact syntax and documentation boundaries**

Run:

```bash
python3 -m json.tool deploy/client-connection.example.json >/dev/null
python3 -m json.tool docs/openapi.json >/dev/null
rg -n 'PandaEngine|CANOPY_DATABASE_URL|CANOPY_SMTP_PASSWORD|CANOPY_STREAM_TOKEN_SECRET|CANOPY_AUTH_OUTBOX_SEALING_KEY' \
  docs/client-integration.md deploy/client-connection.example.json
rg -n 'TBD|TODO|implement later|fill in details' \
  docs/client-integration.md deploy/client-connection.example.json
git diff --check
```

Expected: both JSON files parse; both `rg` scans return no matches; Git reports
no whitespace errors.

- [x] **Step 4: Review the final diff and file modes**

Run through WSL Git:

```bash
git status --short --branch
git diff --stat
git diff -- README.md docs/client-integration.md docs/canopy-api-consumption.md \
  deploy/client-connection.example.json crates/canopy-server/tests/client_handoff.rs
stat -c '%a %n' scripts/test-pg.sh scripts/test-streaming.sh
git diff --cached --name-only
```

Expected: only intended handoff files and pre-existing user changes appear;
both scripts remain mode `755`; the cached-name command prints nothing.

- [x] **Step 5: Report the user-managed publication checkpoint**

Report:

1. The exact files added and modified.
2. Focused and full verification evidence.
3. That the reference file is local-only and secret-free.
4. That production endpoint generation and the full SMTP integration
   environment remain separate deployment work.
5. That no protobuf or `canopy-api` change was required.
6. That no file was staged, committed, tagged, or pushed.

Do not perform any Git mutation.

## Self-Review Checklist

- Spec coverage: ownership, machine schema, local and production transport,
  auth links, session lifecycle, playback, errors, security, verification, and
  next-slice boundary each map to an implementation task.
- Placeholder scan: no task contains deferred implementation markers or
  unspecified file paths.
- Type consistency: JSON keys exactly match Rust `Deserialize` fields and the
  approved design; test helper and struct names are stable across tasks.
- Pin consistency: module, release, commit, package names, and exact versions
  match `crates/canopy-proto/Cargo.toml` and
  `docs/canopy-api-consumption.md`.
- Scope: no client implementation, runtime discovery, protobuf change,
  migration, Git operation, or secret mutation is included.
