# Deployment

Canopy's checked-in Compose files are development and test references, not a
production topology. A production deployment must supply durable dependencies,
TLS termination, network isolation, backups, and secret management.

## Required Components

An active PostgreSQL deployment consists of:

- the Canopy server built with the pg feature;
- PostgreSQL with the complete migration chain applied;
- a durable managed-media volume shared with the streaming tier;
- Nginx using the checked-in authorization pattern;
- TLS-capable ingress or reverse proxies for public gRPC and HTTPS streaming;
- a TLS-only authenticated SMTP relay for usable password registration and
  recovery.

Redis, RustFS, and Adminer are not required Canopy runtime components.

## Public and Private Surfaces

Public surfaces:

| Surface | Purpose |
| --- | --- |
| gRPC endpoint | canopy.v1 control-plane services |
| HTTPS /stream/{capability} | authorized audio delivery |
| HTTPS /openapi.json | actual Canopy HTTP support routes |
| /nginx-health | token-free Nginx orchestration health |

Private surfaces:

- CANOPY_STREAM_AUTH_ADDR and /internal/stream/authorize;
- Nginx /_canopy_auth and /_canopy_media/ locations;
- PostgreSQL, SMTP credentials, signing/sealing keys, and managed storage
  paths.

A bind such as 0.0.0.0:50051 is not a client connection value. Give clients a
reachable DNS name, port, TLS server name, and certificate trust requirements.

## Database Migrations

Apply every migration before starting a new Canopy version:

```bash
export CANOPY_DATABASE_URL=postgres://...
sqlx migrate run
```

PostgreSQL-mode startup fails if the database cannot be reached. Migration,
application, administrative, and backup credentials should follow the
deployment's least-privilege policy; the local defaults are not production
credentials.

Back up PostgreSQL and the managed-media volume as one policy-consistent system.
The repository does not yet provide a complete backup/restore or disaster
recovery runbook.

## Production Transport

Outside loopback and Android-emulator development, the deployment provides:

- TLS gRPC over HTTP/2;
- HTTPS streaming;
- certificates trusted by the client platform;
- preserved gRPC metadata and status codes; and
- streaming routes that do not expose private Nginx locations.

The bundled Nginx files contain no TLS directives. Place them behind the
deployment's HTTPS proxy or add deployment-owned TLS configuration without
changing the internal authorization boundary.

Canopy limits each gRPC connection to 64 concurrent requests and HTTP/2
streams, applies a 15-second server deadline, and sheds excess load. Public
DDoS controls, connection/request rate limits, request-size limits, and
source/account fairness belong at the ingress and deployment layers.

## Nginx Streaming Boundary

The checked-in configuration:

1. accepts only a single-segment /stream/{capability} route;
2. calls Canopy with auth_request and the original URI;
3. receives an internal X-Accel destination after current-policy validation;
4. serves the managed library from an internal, read-only location; and
5. advertises byte-range support.

Mount docs/openapi.json read-only at /srv/canopy/docs/openapi.json and the
managed library read-only at /srv/canopy/media/library. Keep auto-indexing
disabled. The private authorization listener must not be exposed through an
ingress, service, or client connection artifact.

See [Playback and Streaming](playback.md) for capability semantics.

## Health and Readiness

Canopy's System service reports version, aggregate status, and dependency
details. Readiness evaluates:

- PostgreSQL connectivity in pg mode;
- existence and readability of the managed library directory; and
- authentication email delivery state.

Configured SMTP failure makes readiness unhealthy. Deliberately disabled
development delivery is degraded. Nginx reports its own process health at
/nginx-health; that endpoint does not establish Canopy or database readiness.

Configure orchestration so an unhealthy dependency does not silently leave a
client-ready endpoint in rotation.

## Authentication Delivery

Production password bootstrap requires:

- stable Ed25519 access-token signing material;
- a stable 32-byte outbox sealing key;
- complete authenticated SMTP configuration;
- implicit TLS or required STARTTLS;
- a public HTTPS action base owned by the client experience; and
- worker lease/retry values that satisfy configuration validation.

CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL and
CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY are not production modes.

## Client Handoff

Before declaring an environment client-ready:

1. provide the versioned, secret-free connection artifact;
2. prove the public TLS gRPC and HTTPS streaming endpoints from the client's
   network;
3. publish certificate trust requirements;
4. record the exact supported BSR release and generated SDK pins;
5. verify registration email delivery and the client-owned action URL;
6. exercise login, refresh, logout, playback resolution, and a ranged stream;
7. include accepted public Google OAuth client IDs when enabled; and
8. exclude private listeners, database/SMTP credentials, and token secrets.

Use the [Client Integration Handoff](client-integration.md) as the shared
backend/client checklist.

## Operational Gaps

Before production, the deployment team must additionally define and rehearse:

- PostgreSQL and media-volume backup/restore;
- signing, sealing, stream, SMTP, and database secret rotation;
- capacity and resource limits;
- log retention and redaction review;
- correlation and metrics export;
- reconciliation for incomplete media imports; and
- incident response for leaked stream capabilities or identity sessions.

Track current work in [Project Status and Roadmap](roadmap.md).
