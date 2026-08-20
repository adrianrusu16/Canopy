# Local Integration Environment Design

**Status:** Approved on 2026-07-14.

## Context

Canopy's PostgreSQL, authentication, email-delivery, catalog, playback, and
stream-authorization paths are implemented and have focused integration tests.
The repository does not yet provide one repeatable environment that connects
all of those paths across their real network boundaries.

The client integration handoff defines the local public gRPC, streaming, and
OpenAPI endpoints. This design makes those reference endpoints runnable by a
backend developer without introducing client-specific behavior or a production
deployment system.

## Goal

Provide a Canopy-owned local integration environment that starts disposable
PostgreSQL, TLS-capable SMTP inbox, and Nginx dependencies; runs Canopy as a
native WSL process; and proves registration through logout plus catalog through
HTTP range playback against the documented public surfaces.

The same environment must support both interactive development and a clean,
self-contained smoke-test run.

## Non-Goals

- Do not containerize Canopy or add a Canopy Dockerfile.
- Do not add Redis, RustFS, Adminer, a client application, or a REST-to-gRPC
  gateway.
- Do not change protobuf messages, RPC semantics, OpenAPI routes, or the
  machine-readable client handoff.
- Do not add production ingress, public TLS termination, orchestration, or
  deployment migrations.
- Do not add this environment to CI in this slice.
- Do not weaken SMTP certificate verification or add a plaintext SMTP mode.
- Do not expose generated credentials, token material, private authorization
  addresses, or database configuration to clients.

## Architecture

Canopy runs on the WSL host using the repository's normal Rust toolchain. A
dedicated Compose project named `canopy-local-integration` supplies only the
external dependencies:

| Component | Location | Public host binding | Purpose |
| --- | --- | --- | --- |
| Canopy | WSL host process | `127.0.0.1:50051` | Public `canopy.v1` gRPC API. |
| Canopy stream auth | WSL host process | `127.0.0.1:18081` | Private Nginx `auth_request` target. |
| PostgreSQL | Compose, `postgres:18.4-alpine` | `127.0.0.1:55434` | Disposable application database. |
| Mailpit SMTP | Compose, `axllent/mailpit:v1.30.4` | `127.0.0.1:1025` | Authenticated, required-STARTTLS test relay. |
| Mailpit UI/API | Compose, `axllent/mailpit:v1.30.4` | `127.0.0.1:8025` | Test inbox and message API. |
| Nginx | Compose host network, `nginx:stable-alpine` | `127.0.0.1:8080` | Streaming origin and OpenAPI document. |

The local Nginx container uses Linux host networking, binds only
`127.0.0.1:8080`, and reaches Canopy directly at `127.0.0.1:18081`. Its
dedicated `deploy/nginx/canopy-stream.local-integration.conf` forwards the
original opaque stream URI to the private authorizer and maps the generated
local media library read-only. PostgreSQL and Mailpit remain isolated on the
dedicated Compose bridge network. The Docker runtime must support Linux host
networking.

Canopy remains outside Compose so local runs reuse the normal Cargo build,
avoid injecting the private Buf token into a container build, and preserve the
repository's current dependency-only Compose pattern.

## Repository Artifacts

### Compose Environment

Add `docker-compose.local-integration.yml` with PostgreSQL, Mailpit, and Nginx
only. Every published port binds to loopback. Containers use the repository's
existing health-check and bounded-logging patterns, have no restart policy,
and are isolated under the dedicated Compose project name.

PostgreSQL uses a disposable named volume that is removed by the scoped `down`
operation. Mailpit uses in-memory message storage with a bounded message count.
Nginx receives only the public OpenAPI document, media library, and stream
configuration it needs.

The Compose file receives generated SMTP credentials and paths through the
invoking shell environment. It does not contain reusable credentials and does
not read a checked-in secret file.

### Lifecycle Script

Add `scripts/local-integration.sh` with four commands:

```text
./scripts/local-integration.sh up
./scripts/local-integration.sh test
./scripts/local-integration.sh status
./scripts/local-integration.sh down
```

The script owns only the `canopy-local-integration` Compose project and the
Canopy PID it records. It never stops containers from the repository's other
Compose projects and never kills a process discovered only by port number or
name.

All generated state lives below the ignored
`target/local-integration/` directory:

```text
target/local-integration/
  certs/
  media/library/
  runtime.env
  canopy.pid
  canopy.log
```

Generated signing, outbox-sealing, stream-token, and SMTP credential values
are written only to `runtime.env` with mode `0600` so later `status` and `down`
invocations can address the same scoped environment. The file is sourced by
the lifecycle script, never passed to Canopy as a command-line argument, and
never printed or copied to logs. Generated certificate and key files remain in
the ignored state directory. All generated runtime material is removed during
scoped cleanup and is never added to tracked configuration.

### SMTP Trust Configuration

Add optional `CANOPY_SMTP_CA_CERT_PATH` runtime configuration. When set, it
must name a readable PEM certificate bundle. `SmtpEmailSender` adds those
certificates to its TLS trust roots for both implicit TLS and STARTTLS without
disabling hostname or chain verification. Invalid paths or PEM data fail
startup with a configuration error that does not reveal credentials.

The local lifecycle script creates an ephemeral root CA and a server
certificate signed by it with `localhost` in the subject alternative name.
Mailpit serves that certificate, requires STARTTLS, and requires generated
SMTP credentials. Canopy connects to `localhost:1025`, trusts only the
generated CA in addition to the platform roots, and keeps verification fully
enabled.

This option supports local/private SMTP trust anchors generally, but it is not
an insecure development bypass.

### Seed Data

The script copies `fixtures/media/test-tone.mp3` into the storage-key location
for the existing Moonlight Sonata record in `fixtures/catalog.json`, grants
read/traverse permissions only to that generated public media subtree, and
sets `CANOPY_PROVIDER_FIXTURE_PATH` to the fixture. Canopy's existing
idempotent startup ingest creates the catalog record. After readiness, the
script applies `fixtures/local-integration.sql`, which fails if the designated
`fixture-001` record is missing and publishes only that record as approved,
ready, and release-safe. `fixture-002` remains quarantined.

## Lifecycle Behavior

### `up`

`up` performs these steps in order:

1. Resolve the repository root and validate Docker Compose, OpenSSL, Cargo,
   `curl`, and `sqlx-cli` availability.
2. Fail before mutation if any fixed public or private port is occupied, or if
   this integration environment is already active.
3. Remove only stale state owned by this environment, create the scoped state
   directories, generate TLS material and ephemeral secrets, and install the
   media fixture at its catalog storage key.
4. Validate the Compose model, start PostgreSQL, Mailpit, and Nginx, and wait
   for their health checks with a bounded timeout.
5. Apply the checked-in PostgreSQL migrations through the repository's
   existing `sqlx-cli` workflow. Canopy itself does not gain automatic runtime
   migrations.
6. Build the PostgreSQL-enabled Canopy binary, copy it to the scoped state
   directory as an immutable process-ownership target, start it with `nohup`,
   and record only its PID, start time, and sanitized log.
7. Wait with a bounded timeout until gRPC, stream authorization, email
   readiness, Nginx health, and OpenAPI are available.
8. Apply the checked-in local fixture policy SQL to publish only the
   designated playback record.
9. Print the four secret-free developer endpoints and leave the environment
   running.

The developer-facing endpoints are:

| Surface | Endpoint |
| --- | --- |
| gRPC | `http://127.0.0.1:50051` |
| Streaming | `http://127.0.0.1:8080` |
| OpenAPI | `http://127.0.0.1:8080/openapi.json` |
| Test inbox | `http://127.0.0.1:8025` |

The first three endpoints match the client handoff. The test inbox is an
operator-only diagnostic surface and is not added to the client handoff.

Verification and reset action URLs use the exact loopback client base
`http://127.0.0.1:3000/auth/`. No client server is started; the smoke test
extracts the opaque action token from the delivered message instead of fetching
the action URL.

### `test`

`test` refuses to run while an interactive local-integration environment is
active. It starts a clean environment through the same lifecycle functions,
runs the ignored real-boundary Rust smoke test, and always performs scoped
cleanup.

On failure it prints sanitized Canopy and Compose diagnostics before cleanup
and returns a nonzero status. It does not print process environments, message
bodies, tokens, SMTP credentials, or generated key material.

### `status`

`status` reports whether the recorded Canopy PID is alive, the health/state of
the three Compose services, and the four public developer endpoints. It is
read-only and never prints secret-bearing environment values.

### `down`

`down` sends a graceful termination signal only to the recorded Canopy PID,
waits for a bounded interval, stops the dedicated Compose project, removes its
disposable volumes, and removes `target/local-integration/`. It is idempotent
when the environment is already stopped. A PID is acted on only when the
recorded process can be verified as the environment-owned Canopy process;
otherwise cleanup stops and reports the stale PID safely.

## Smoke-Test Flow

Add an ignored Rust integration test that consumes the real gRPC API, Mailpit
HTTP API, Nginx, and PostgreSQL-backed server. The test receives only
non-secret endpoint values from the script and generates a unique account
address.

The smoke run proves:

1. `SystemService.GetStatus` reports PostgreSQL, media, and email delivery
   ready.
2. Nginx serves `/nginx-health` and the checked-in `/openapi.json` document.
3. `AuthService.RegisterPassword` accepts a unique email and strong password.
4. The test polls Mailpit with a bounded timeout until the verification email
   for that recipient arrives.
5. The test parses the plain-text action URL with a URL parser, extracts the
   opaque `token`, and calls `VerifyEmail` with a device label.
6. A protected session call succeeds with the returned access token.
7. The verification session is logged out, `LoginPassword` creates a new
   session, and `RefreshSession` replaces its complete `SessionEnvelope` once.
8. `Logout` succeeds with the refreshed access token, after which a protected
   call using that session is rejected.
9. `CatalogService.Search` discovers the seeded Moonlight Sonata track and
   `PlaybackService.ResolvePlayback` returns an opaque Nginx stream URL.
10. The backend-owned harness verifies the same capability directly against
    the loopback-only private authorizer without printing it.
11. An HTTP range request to the public Nginx URL returns
    `206 Partial Content` and non-empty media bytes.

The test does not replay the pre-rotation refresh token: reuse revocation is
already covered by focused authentication tests, and intentionally revoking
the family would obscure the end-to-end happy path. Tokens and message bodies
must not appear in assertion or failure text.

## Error Handling

- Missing tools, occupied ports, malformed generated certificates, migration
  failures, unhealthy dependencies, early Canopy exit, and readiness timeout
  fail fast with a concise operator-facing category.
- Every wait has a fixed deadline and reports which component failed.
- Startup failure triggers the same scoped cleanup as `down` after preserving
  sanitized diagnostics.
- Compose validation runs before containers start.
- A malformed or unreadable custom SMTP CA fails Canopy startup; it never
  falls back to unverified TLS.
- Mailpit polling is recipient-specific and bounded so stale messages cannot
  satisfy the smoke test.
- Cleanup failures are reported without replacing the original test failure.

## Testing Strategy

Implementation follows test-driven development with these layers:

1. Configuration unit tests reject empty, unreadable, and malformed custom CA
   inputs and accept a valid PEM bundle.
2. SMTP construction tests prove the custom trust root is used without an
   insecure verification mode.
3. Script-level checks validate unknown commands, prerequisite failures,
   active-environment refusal, PID ownership checks, and secret-free status
   output where those behaviors can be isolated safely.
4. `crates/canopy-server/tests/local_integration_contract.rs` verifies that
   script and Compose public endpoints match
   `deploy/client-connection.example.json` and that private ports or test-inbox
   details do not leak into the client artifact.
5. `docker compose -f docker-compose.local-integration.yml config --quiet`
   validates the resolved Compose model.
6. The ignored real-boundary smoke test proves the complete authentication and
   playback flow when invoked by `scripts/local-integration.sh test`.
7. Workspace formatting, tests, Clippy with all features, and diff checks
   remain final gates.

The ignored test is not part of the ordinary `cargo test --workspace` run
because it owns fixed ports and requires Docker. The lifecycle script is the
canonical way to run it.

## Documentation

- Add a concise local-integration section to `README.md` with prerequisites,
  lifecycle commands, endpoints, and the distinction between this environment
  and production deployment.
- Update `docs/client-integration.md` to point backend/client integrators to
  the runnable local reference environment without exposing Mailpit, database,
  or private stream-authorization values as client handoff fields.
- Add `CANOPY_SMTP_CA_CERT_PATH` to `.env.example` and the README environment
  reference.
- Keep `canopy-api` documentation unchanged because this environment is a
  Canopy implementation and deployment concern.

## File Map

- Create `docker-compose.local-integration.yml`.
- Create `scripts/local-integration.sh`.
- Create `crates/canopy-server/tests/local_integration.rs` for the ignored
  real-boundary smoke test.
- Create `crates/canopy-server/tests/local_integration_contract.rs` for
  endpoint, ownership, permission, and documentation drift.
- Create `deploy/nginx/canopy-stream.local-integration.conf` for the
  host-networked loopback reference.
- Create `fixtures/local-integration.sql` for the designated release-safe
  playback record.
- Modify `crates/canopy-server/src/config.rs` and
  `crates/canopy-server/src/identity/email.rs` for the optional SMTP CA path.
- Modify `crates/canopy-server/src/stream/http.rs` and
  `deploy/nginx/canopy-stream.conf` for explicit original-URI authorization.
- Modify `crates/canopy-server/src/lib.rs` so startup logs never emit the
  credential-bearing database URL.
- Modify focused configuration, email, stream, and integration tests before
  implementation code.
- Modify `.env.example`, `README.md`, and `docs/client-integration.md`.

No protobuf, migration, OpenAPI, client repository, `canopy-api`, production
Compose, or CI workflow file is changed.

## Security And Operational Boundaries

- All listeners bind to loopback; this is not a LAN-accessible environment.
- SMTP uses authenticated required STARTTLS with hostname and chain
  verification.
- Generated secret values live only for one run and are never documented or
  emitted.
- The private stream authorization listener remains absent from the public
  handoff.
- Playback capability URLs are treated as opaque and are never logged by the
  lifecycle script.
- Cleanup is scoped by both Compose project name and a verified recorded PID.
- Runtime schema migration remains an explicit operator/script action.
- Only the generated public media subtree receives world read/traverse
  permission; generated keys, credentials, and runtime state retain `umask 077`
  protection.

## Acceptance Criteria

- A backend developer can run one command to start the complete local
  reference environment and one command to stop it.
- A clean `test` run proves real SMTP delivery, email verification, password
  login, refresh rotation, logout enforcement, catalog discovery, playback
  authorization, and HTTP range serving.
- The public endpoints agree with the checked-in client connection handoff.
- SMTP certificate verification remains enabled while trusting the generated
  local CA.
- No generated credential, token, message body, private endpoint, or database
  value is written to tracked files or printed by normal lifecycle commands.
- Failure and cleanup are bounded, diagnostic, and limited to resources owned
  by this environment.
- Ordinary workspace tests remain Docker-independent.

## Verification Gates

The implementation plan must include, at minimum:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test local_integration_contract --locked
cargo test --workspace --locked
cargo clippy --workspace --all-features --tests --locked -- -D warnings
docker compose -f docker-compose.local-integration.yml config --quiet
./scripts/local-integration.sh test
git diff --check
```

The user retains control of staging, commits, tags, pushes, and branch changes.
