# Client Integration Handoff Design

**Status:** Approved for implementation planning.

## Context

`canopy-api` owns the product-neutral `canopy.v1` protobuf contract and
consumer semantics. Canopy owns the concrete server implementation,
deployment requirements, supported generated SDK versions, and public
connection surfaces. A future client repository owns channel construction,
credential storage, UI, and its own CI/CD.

The existing `docs/canopy-api-consumption.md` explains how Canopy consumes the
published contract. It does not provide the deployment-to-client handoff that
an external client team needs before it can connect to a Canopy instance.

## Goal

Provide a Canopy-owned, client-neutral connection and deployment handoff with
a secret-free machine-readable local reference configuration and an automated
conformance test that prevents the public handoff from drifting from Canopy's
contract pins and HTTP surfaces.

## Non-Goals

- Do not modify `canopy-api` or the protobuf wire contract.
- Do not implement PandaEngine or any other client.
- Do not add runtime service discovery or serve the reference JSON publicly.
- Do not add a REST gateway or describe gRPC methods as HTTP endpoints.
- Do not add the full PostgreSQL, Nginx, Canopy, and test-SMTP integration
  environment in this slice.
- Do not expose database, SMTP, token-signing, outbox-sealing, stream-token,
  or private authorization configuration.
- Do not prescribe client UI, secure-storage implementation, or CI/CD.

## Ownership Boundary

The handoff keeps four authorities distinct:

| Authority | Owns |
| --- | --- |
| `canopy-api` | Protobuf resources, RPC semantics, canonical gRPC errors, pagination, and compatibility. |
| Canopy | Supported contract release, public server surfaces, deployment prerequisites, auth-link rendering, and runtime availability. |
| Deployment operator | Concrete public endpoints, TLS certificates, DNS, SMTP relay, database, secrets, and rollout timing. |
| Client | Channel lifecycle, credential storage, UI, deep-link handling, retries, and client-specific tests. |

Canopy documentation may explain how any client connects to Canopy. It must
not depend on a named client repository or describe a client's internal
architecture.

## Artifacts

### Human-Readable Handoff

Create `docs/client-integration.md` as the backend-to-client handoff. It will
contain:

1. Authority and support boundaries.
2. Exact supported BSR module, package, release, commit, and generated SDK
   versions.
3. Public gRPC, streaming, and OpenAPI surfaces.
4. A same-host local reference configuration.
5. Requirements for clients outside the server network namespace.
6. Production TLS, DNS, certificate, and endpoint requirements.
7. PostgreSQL and SMTP prerequisites for functional authentication.
8. Email verification and password-reset action-link shapes.
9. Registration, verification, login, protected-call, refresh, and logout
   data flow.
10. Atomic `SessionEnvelope` replacement and serialized refresh requirements.
11. Canonical gRPC error handling by reference to the `canopy-api` consumer
    guide.
12. Playback URL opacity and expiry behavior.
13. A deployment-to-client handoff checklist and focused troubleshooting.
14. A list of internal addresses and secrets that must never be given to a
    client.

The Canopy README and `docs/canopy-api-consumption.md` will link to this
handoff rather than duplicating its content.

### Machine-Readable Local Reference

Create `deploy/client-connection.example.json` with this exact shape:

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

This file is a checked-in reference artifact, not a runtime endpoint. A
deployment operator may derive an environment-specific handoff from it, but
must replace loopback URLs with public TLS endpoints and must not add secrets.
Clients must reject unsupported `schema_version` values rather than guessing
at new fields.

## Public Connection Model

The local reference uses:

| Surface | URL | Purpose |
| --- | --- | --- |
| gRPC | `http://127.0.0.1:50051` | All `canopy.v1` RPCs. |
| Streaming | `http://127.0.0.1:8080` | Opaque playback capabilities returned by gRPC. |
| HTTP OpenAPI | `http://127.0.0.1:8080/openapi.json` | Documentation for real Canopy HTTP routes only. |

Canopy must be launched with
`CANOPY_GRPC_ADDR=127.0.0.1:50051` for the reference gRPC endpoint. Nginx must
serve the configured stream origin separately. The private
`CANOPY_STREAM_AUTH_ADDR` listener is an Nginx-to-Canopy implementation
boundary and is never client-facing.

A client outside the server network namespace requires an operator-provided
reachable host or DNS name. Binding Canopy to `0.0.0.0` may make the listener
reachable, but `0.0.0.0` is never a client endpoint. Outside loopback
development, gRPC, stream, OpenAPI, and auth-action URLs use TLS with a
certificate trusted by the client platform.

## Authentication Data Flow

`AuthService` is registered only in PostgreSQL builds. A usable password
bootstrap also requires working SMTP delivery; the development-only
undelivered-email escape hatch leaves challenges queued and cannot complete a
real external registration flow.

The handoff documents this sequence:

1. The client calls `RegisterPassword` and treats the generic accepted result
   as non-enumerating acknowledgement, not proof that an account was created.
2. Canopy joins `verify-email` to `CANOPY_AUTH_PUBLIC_BASE_URL` and delivers
   the resulting verification action URL through the configured email
   channel. The client-owned action handler extracts `token` and
   `expires_at` from that URL.
3. The client passes the token unchanged to `VerifyEmail` with a device label.
4. The client atomically stores the complete returned `SessionEnvelope`.
5. Protected calls send lowercase `authorization` metadata with
   `Bearer <access-token>`.
6. The client permits only one refresh operation in flight per session. A
   successful `RefreshSession` atomically replaces the entire envelope.
7. An ambiguous refresh transport result is not retried with the same refresh
   token because reuse can revoke the session family.
8. `Logout` clears the current local envelope after success. `LogoutAll`,
   password reset, and account deletion clear every locally known session for
   the account.

Password-reset links join `reset-password` to
`CANOPY_AUTH_PUBLIC_BASE_URL` and carry `token` and `expires_at` query
parameters. Tokens are opaque and are passed unchanged to the corresponding
RPC. The handoff links to the `canopy-api` consumer guide for the complete
public/protected method matrix and canonical status-code meanings.

## Playback Data Flow

Clients call `PlaybackService.ResolvePlayback` over gRPC and use the returned
`PlaybackSource.stream_url` verbatim until its expiry. They do not construct
stream paths, parse capabilities, send the capability to private authorization
routes, or derive storage paths. HTTP range behavior and capability expiry are
owned by the public streaming surface.

## Error Handling

- Connection and TLS failures are deployment failures, not gRPC application
  statuses.
- Clients branch on canonical gRPC status codes and never message text.
- Malformed supplied bearer metadata is an authentication failure and never
  downgrades a call to anonymous.
- Unknown handoff schema versions fail closed.
- The handoff contains no retry promise beyond the contract semantics.
- Refresh ambiguity requires reauthentication rather than token replay.

## Conformance Test

Create `crates/canopy-server/tests/client_handoff.rs` using existing
`serde`, `serde_json`, and `reqwest::Url` dependencies. The test defines local
`Deserialize` structs with `deny_unknown_fields` and verifies:

1. The JSON is valid UTF-8 JSON and exactly matches schema version 1.
2. Environment is `local-reference`.
3. Contract package, module, release, commit, generated package names, and
   exact generated versions match Canopy's supported contract.
4. The exact Prost and Tonic versions are present in
   `crates/canopy-proto/Cargo.toml`.
5. All three public URLs parse successfully.
6. Plaintext URLs are loopback-only and the file declares TLS mandatory
   outside loopback.
7. Streaming and OpenAPI use the same scheme, host, and port.
8. The OpenAPI URL path is `/openapi.json`, and `docs/openapi.json` contains
   that real route.
9. No JSON key exposes a forbidden deployment-secret or private-surface
   field: `database_url`, `smtp_host`, `smtp_port`, `smtp_username`,
   `smtp_password`, `identity_access_token_signing_key`,
   `auth_outbox_sealing_key`, `stream_token_secret`, `stream_auth_addr`, or
   `private_authorization_url`.
10. Authentication metadata and action-route names match the documented
    Canopy behavior.

The test validates the machine artifact, while human review remains
responsible for prose quality. CI runs the test through the existing locked
workspace test gate.

## Security And Privacy

The handoff may contain public endpoints, public contract identifiers, public
certificate requirements, and action-route shapes. It never contains raw
tokens, example bearer credentials, database addresses, SMTP credentials,
signing material, sealing keys, stream capability secrets, private listener
addresses, or account data.

Loopback plaintext examples are explicitly development-only. Production
operators terminate TLS before public traffic reaches Canopy and provide the
client with the public endpoint and trust requirements, not infrastructure
credentials.

## File Map

- Create `docs/client-integration.md`: human-readable deployment-to-client
  handoff.
- Create `deploy/client-connection.example.json`: secret-free local reference
  configuration.
- Create `crates/canopy-server/tests/client_handoff.rs`: reference artifact
  conformance test.
- Modify `README.md`: link to the client integration handoff from relevant
  API and running-server sections.
- Modify `docs/canopy-api-consumption.md`: distinguish server-side contract
  consumption from client connection guidance and link to the handoff.

No protobuf, database migration, runtime listener, or `canopy-api` file changes
are required.

## Verification

The implementation must pass:

```bash
cargo fmt --all -- --check
cargo test -p canopy-server --test client_handoff --locked
cargo test -p canopy-server --test http_openapi --locked
cargo test --workspace --locked
cargo clippy --workspace --all-features --tests --locked -- -D warnings
git diff --check
```

The user retains control of staging, commits, tags, and pushes.

## Acceptance Criteria

- A client team can identify every public endpoint and deployment-provided
  value it needs without receiving backend secrets.
- The handoff clearly separates local reference values from production
  operator-provided values.
- The handoff accurately describes password authentication, refresh rotation,
  action links, and playback URL usage.
- The machine-readable file is strict, versioned, secret-free, and verified in
  CI.
- Canopy's exact supported BSR release and SDK pins agree across the manifest,
  server-owned documentation, and reference handoff.
- No client-specific implementation or CI/CD detail is added to Canopy or
  `canopy-api`.

## Next Slice

After this handoff is complete, the backend team can design a separate local
integration environment that starts Canopy with PostgreSQL, Nginx, and a
TLS-capable test SMTP inbox and exercises registration through logout against
real network boundaries. PandaEngine consumes the handoff later in its own
repository.
