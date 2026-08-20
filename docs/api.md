# API Consumption

This document records the exact released contract implemented by the Canopy
backend. Product-neutral RPC and message semantics live in the canonical
[consumer guide](https://github.com/adrianrusu16/canopy-api/blob/master/docs/consumer-guide.md).

Deployment-provided public endpoints, TLS requirements, authentication action
links, and client-ready environment checks live in the
[Client Integration Handoff](client-integration.md).

## Supported Contract

- BSR module: `buf.build/pandawave/canopy-api`
- Stable release: `v0.2.0`
- Immutable BSR commit: `af019e2d7fa245a2a7d9fc21a4dd9afa`
- Prost SDK: `=0.5.0-00000000000000-af019e2d7fa2.2`
- Tonic SDK: `=0.5.0-00000000000000-af019e2d7fa2.4`

The exact SDK versions in `crates/canopy-proto/Cargo.toml` and `Cargo.lock`
are the machine-verifiable support declaration. Release labels are useful for
discovery but are not dependency pins.

## Registry Setup

The workspace configures the private Buf Cargo registry in
`.cargo/config.toml`:

```toml
[registries.buf]
index = "sparse+https://buf.build/gen/cargo/"
credential-provider = "cargo:token"
```

Authenticate a local Cargo installation with a personal registry token:

```bash
cargo login --registry buf "Bearer {token}"
```

Credentials belong in the local Cargo credential store or repository secret
configuration. They must not be committed to `local.properties`, source files,
container build arguments, or documentation examples.

## Facade Rule

`crates/canopy-proto` re-exports the generated Prost resources and Tonic
service modules behind the stable local `canopy_proto` crate name. It must not
redefine protobuf messages, services, field numbers, or enum values locally.
Canopy runtime and deployment behavior remains documented in this repository;
the contract repository owns product-neutral protocol promises.

## Upgrade Procedure

1. Review the contract changelog and compatibility policy.
2. Select one immutable BSR commit and its matching Prost and Tonic packages.
3. Update both exact generated SDK pins together.
4. Refresh `Cargo.lock` with authenticated registry access.
5. Run the facade and generated-client compile tests.
6. Run every Canopy verification gate below.
7. Update the supported release and commit recorded in this document.

## Verification

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
bash scripts/test-pg.sh
bash scripts/test-streaming.sh
cargo clippy --workspace --all-features --tests --locked -- -D warnings
```

The HTTP OpenAPI document at `docs/openapi.json` covers only actual Canopy
HTTP routes. Protobuf and BSR-generated documentation remain authoritative for
the gRPC API.

See [Development](development.md) for the complete local and CI verification
workflows.
