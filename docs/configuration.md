# Configuration

Canopy reads process environment variables at startup. The Rust process does
not load .env files by itself: [.env.example](../.env.example) is a template
for Compose interpolation or for values exported by the caller.

Secrets in the template are development placeholders. Replace them before
exposing any listener beyond loopback.

## Loading Configuration

Invalid required stream, media, identity, Google, or SMTP configuration fails
startup before listeners serve traffic. PostgreSQL connection failure is fatal
when the pg feature is enabled.

Several defaults exist to support focused local work; they are not production
secret values. In particular, the database URL and legacy auth secret defaults
must not be relied on outside a local environment.

## Server and PostgreSQL

| Variable | Default | Purpose and validation |
| --- | --- | --- |
| CANOPY_GRPC_ADDR | [::1]:50051 | gRPC bind socket. A wildcard bind is a listen address, not a client endpoint. |
| CANOPY_DATABASE_URL | postgres://canopy:canopy@localhost:5432/canopy | PostgreSQL connection string. Required explicitly by canopy-admin. |
| CANOPY_PG_MAX_CONNECTIONS | 20 | Maximum SQLx pool size. |
| CANOPY_PG_ACQUIRE_TIMEOUT_SECS | 5 | Pool acquisition timeout in seconds. |
| CANOPY_AUTH_TOKEN_SECRET | canopy-auth-secret | Legacy shared-secret verifier used by compatibility authentication paths; replace outside local development. |
| CANOPY_PROVIDER_FIXTURE_PATH | unset | Optional deterministic catalog fixture ingested at PostgreSQL startup. |
| CANOPY_REDIS_URL | redis://localhost:6379 | Reserved for JadeCache; parsed but not connected by the current server. |

CANOPY_MEDIA_ROOT is also required by every server mode; it is described with
streaming settings below.

## Identity Tokens

| Variable | Default | Purpose and validation |
| --- | --- | --- |
| CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64 | required | Base64-encoded 32-byte Ed25519 seed for native access tokens. |
| CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY | false | Allows a generated process-local key only for explicit development. |
| CANOPY_IDENTITY_ACCESS_TOKEN_ISSUER | canopy | Access-token issuer claim. Non-empty values only. |
| CANOPY_IDENTITY_ACCESS_TOKEN_AUDIENCE | pandawave | Audience expected by first-party clients. |
| CANOPY_IDENTITY_ACCESS_TOKEN_KEY_ID | identity-access-v1 | Key identifier embedded in token headers and claims. |
| CANOPY_IDENTITY_ACCESS_TOKEN_TTL_SECS | 900 | Positive access-token lifetime in seconds. |

Exactly one signing strategy is required: stable signing material for a real
deployment, or the explicit ephemeral development escape hatch.

Google identity is optional:

| Variable | Default | Purpose and validation |
| --- | --- | --- |
| CANOPY_GOOGLE_OIDC_CLIENT_IDS | unset | Comma-separated accepted public OAuth client IDs. Empty disables Google login/linking. |
| CANOPY_GOOGLE_OIDC_TOKENINFO_URL | https://oauth2.googleapis.com/tokeninfo | ID-token validation endpoint; must use HTTPS. |

## Authentication Email

PostgreSQL builds require a complete SMTP configuration unless the development
escape hatch is enabled.

| Variable | Default | Purpose and validation |
| --- | --- | --- |
| CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL | false | Development-only mode that queues mail and reports degraded readiness. |
| CANOPY_AUTH_OUTBOX_SEALING_KEY | required with SMTP | Base64-encoded 32-byte AES-GCM key for persisted outbox payloads. |
| CANOPY_SMTP_HOST | required | SMTP relay host. |
| CANOPY_SMTP_TLS_MODE | implicit | implicit or starttls; plaintext SMTP is unsupported. |
| CANOPY_SMTP_PORT | 465 or 587 | Defaults from TLS mode; must be a positive u16. |
| CANOPY_SMTP_CA_CERT_PATH | unset | Optional readable PEM CA certificate; verification remains enabled. |
| CANOPY_SMTP_USERNAME | required | Relay authentication user. |
| CANOPY_SMTP_PASSWORD | required | Relay authentication password. |
| CANOPY_SMTP_FROM_ADDRESS | required | Valid sender email address. |
| CANOPY_SMTP_FROM_NAME | required | Sender display name. |
| CANOPY_AUTH_PUBLIC_BASE_URL | required | Base for verification/reset actions; HTTPS except localhost or 127.0.0.1 development. |
| CANOPY_SMTP_TIMEOUT_SECS | 30 | Positive connect/send timeout. |
| CANOPY_AUTH_EMAIL_POLL_INTERVAL_SECS | 2 | Positive worker poll interval. |
| CANOPY_AUTH_EMAIL_LEASE_SECS | 60 | Positive row lease; must be at least the SMTP timeout. |
| CANOPY_AUTH_EMAIL_BATCH_SIZE | 20 | Positive maximum rows claimed per pass. |
| CANOPY_AUTH_EMAIL_MAX_ATTEMPTS | 8 | Positive terminal attempt limit. |
| CANOPY_AUTH_EMAIL_INITIAL_RETRY_SECS | 5 | Positive initial retry delay. |
| CANOPY_AUTH_EMAIL_MAX_RETRY_SECS | 900 | Positive retry cap; must not be below the initial delay. |

Partial SMTP configuration is rejected. Custom CA material adds trust roots; it
does not disable certificate or hostname validation.

## Streaming and Managed Media

| Variable | Default | Purpose and validation |
| --- | --- | --- |
| CANOPY_MEDIA_ROOT | required | Managed root containing library/. Startup verifies the library path. |
| CANOPY_STREAM_PUBLIC_BASE_URL | required | Public Nginx origin. HTTPS outside localhost, 127.0.0.1, and Android-emulator development. |
| CANOPY_STREAM_TOKEN_SECRET | required | HMAC secret containing at least 32 bytes. |
| CANOPY_STREAM_TOKEN_TTL_SECS | 600 | Positive capability lifetime in seconds. |
| CANOPY_STREAM_AUTH_ADDR | 127.0.0.1:8081 | Private HTTP listener used by Nginx auth_request. Never publish it. |

The administrative importer reads two additional settings:

| Variable | Default | Purpose and validation |
| --- | --- | --- |
| CANOPY_MAX_AUDIO_BYTES | 2147483648 | Positive MP3 import-size limit (2 GiB). |
| CANOPY_MAX_ARTWORK_BYTES | 20971520 | Positive embedded/sidecar artwork limit (20 MiB). |

## Provider Fixtures

When CANOPY_PROVIDER_FIXTURE_PATH names a fixture in PostgreSQL mode, Canopy
ingests it before opening the listeners. Ingestion is idempotent by provider
and provider-track ID and remains quarantined until an explicit review/promotion
path changes policy.

Do not use a fixture path as a production provider integration. External
provider runtime adapters are not active.

## Reserved Compose Services

These variables configure containers in docker-compose.yml rather than the
Canopy runtime:

| Service | Variables |
| --- | --- |
| PostgreSQL | CANOPY_POSTGRES_USER, CANOPY_POSTGRES_PASSWORD, CANOPY_POSTGRES_DB, CANOPY_POSTGRES_PORT |
| Redis | CANOPY_REDIS_PORT |
| Adminer | CANOPY_ADMINER_PORT |
| RustFS | CANOPY_RUSTFS_BUCKET, CANOPY_RUSTFS_ACCESS_KEY, CANOPY_RUSTFS_SECRET_KEY, CANOPY_RUSTFS_PORT |

Redis is reserved for a future JadeCache adapter. RustFS is inactive storage
infrastructure. Adminer is an opt-in development profile. None is evidence that
Canopy currently reads from Redis or RustFS.

## Validation and Startup Failure

Before starting a shared environment, verify:

- the database is reachable and fully migrated;
- the media root and its library directory are readable;
- public stream and authentication URLs use TLS outside local development;
- capability, signing, outbox, SMTP, and database secrets are independent;
- the private stream address is reachable only from Nginx;
- SMTP lease duration is not shorter than the send timeout; and
- the client handoff contains no private configuration.

See [Deployment](deployment.md) for topology and
[Authentication](authentication.md) for identity behavior.
