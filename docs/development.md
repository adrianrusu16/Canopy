# Development

This guide covers local setup, server modes, the disposable integration
environment, and verification. Deployment requirements are documented
separately in [Deployment](deployment.md).

## Prerequisites

Install:

- the repository's stable Rust toolchain;
- Docker with Compose v2;
- sqlx-cli with PostgreSQL support;
- Bash, curl, and OpenSSL;
- for the full WSL integration environment: ss, awk, readlink, and nohup.

Authenticate Cargo to the private Buf registry before a clean dependency
resolution. See [API Consumption](api.md).

## Quick Start

The most complete local environment is the checked-in lifecycle script:

```bash
./scripts/local-integration.sh up
./scripts/local-integration.sh status
```

It runs Canopy natively and uses the scoped canopy-local-integration Compose
project for PostgreSQL, Mailpit, and Nginx. It generates local-only credentials
and certificates beneath the ignored target/local-integration/ directory.

Public development endpoints are:

| Surface | Host | Android emulator |
| --- | --- | --- |
| gRPC | http://127.0.0.1:50051 | http://10.0.2.2:50051 |
| Streaming | http://127.0.0.1:8080 | http://10.0.2.2:8080 |
| OpenAPI | http://127.0.0.1:8080/openapi.json | http://10.0.2.2:8080/openapi.json |

Mailpit is operator-only at http://127.0.0.1:8025. PostgreSQL, SMTP, and the
private stream authorizer are not client handoff surfaces.

## Run Canopy

### In-memory mode

The non-PostgreSQL build is useful for focused domain work. It still requires
the media, stream, and identity-token configuration parsed at startup:

```bash
mkdir -p /tmp/canopy-media/library
export CANOPY_MEDIA_ROOT=/tmp/canopy-media
export CANOPY_STREAM_PUBLIC_BASE_URL=http://127.0.0.1:8080
export CANOPY_STREAM_TOKEN_SECRET=0123456789abcdef0123456789abcdef
export CANOPY_STREAM_AUTH_ADDR=127.0.0.1:8081
export CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY=true
cargo run -p canopy-server --bin canopy
```

This mode does not register AuthService and does not provide durable state.

### PostgreSQL mode

The root docker-compose.yml provisions development dependencies; it does not
run Canopy or Nginx. Start PostgreSQL, migrate it, export the required server
configuration, and then run:

```bash
docker compose up -d postgres
sqlx migrate run

export CANOPY_DATABASE_URL=postgres://canopy:canopy@localhost:5432/canopy
export CANOPY_MEDIA_ROOT=/tmp/canopy-media
export CANOPY_STREAM_PUBLIC_BASE_URL=http://127.0.0.1:8080
export CANOPY_STREAM_TOKEN_SECRET=0123456789abcdef0123456789abcdef
export CANOPY_STREAM_AUTH_ADDR=127.0.0.1:8081
export CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY=true
export CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true

cargo run -p canopy-server --features pg --bin canopy
```

The undelivered-email and ephemeral-key settings are development escapes. Use
real signing and SMTP configuration for a usable client authentication flow.
See [Configuration](configuration.md).

## Local Integration Environment

The lifecycle commands are:

```bash
./scripts/local-integration.sh up
./scripts/local-integration.sh test
./scripts/local-integration.sh status
./scripts/local-integration.sh down
./scripts/local-integration.sh reset
```

- up creates or resumes the scoped environment.
- test creates a clean environment, proves registration, email verification,
  session refresh/logout, playback resolution, and ranged streaming, then
  removes it.
- status reports the native process and dependency state.
- down stops the scoped environment while retaining its PostgreSQL volume.
- reset removes the scoped environment and its generated state, then recreates
  it. Use it only when discarding local integration data is intended.

The script refuses unsafe state paths, checks required ports, keeps generated
secrets out of logs, and binds operator-only services to loopback.

## Database Integration Tests

Run the isolated PostgreSQL suite with:

```bash
bash scripts/test-pg.sh
```

The harness starts PostgreSQL on a scoped local port, applies the complete
migration chain, runs feature tests serially, and removes its containers and
network on exit.

Advanced callers may provide a deliberately managed database:

```bash
CANOPY_TEST_DATABASE_URL=postgres://user:password@localhost:5432/canopy_test \
  bash scripts/test-pg.sh
```

Tests never fall back from CANOPY_TEST_DATABASE_URL to the normal application
database URL.

## Streaming Integration Tests

```bash
bash scripts/test-streaming.sh
```

The streaming harness creates disposable PostgreSQL and Nginx services, mounts
a synthetic read-only media tree, starts the real Canopy authorizer, and checks
OpenAPI serving, direct-path denial, opaque-capability denial, 206 byte ranges,
and immediate policy revocation.

## CI Verification

The GitHub Actions gate runs the equivalent of:

```bash
cargo check --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-features --tests --locked -- -D warnings
cargo fmt --all -- --check
bash scripts/test-pg.sh
bash scripts/test-streaming.sh
cargo build --workspace --release --locked
```

Canopy owns runtime and integration verification. The canonical canopy-api
repository owns protobuf format, lint, compatibility, and publication gates.

## SQLx Query Checking

Current repositories use runtime-checked sqlx::query calls, so ordinary builds
do not require a live database or checked-in offline metadata. If a future
change adopts query! or query_as!, prepare metadata against a migrated
development database:

```bash
docker compose up -d postgres
sqlx migrate run
cargo sqlx prepare --workspace
```

Review and commit generated .sqlx/ files only as part of that explicit
compile-time-query change.
