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
