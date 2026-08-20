# Authentication Email Delivery Design

**Status:** Approved on 2026-07-11.

## Goal

Deliver committed authentication verification and password-reset outbox rows through SMTP without exposing recipient addresses, tokens, decrypted payloads, rendered messages, or provider responses in logs or persistent error details.

## Scope

This design adds an in-process SMTP outbox worker, PostgreSQL leasing and retry state, runtime configuration, readiness reporting, message templates, and tests. It does not change the protobuf contract, authentication challenge policy, or downstream client behavior.

## Startup Policy

PostgreSQL mode requires complete SMTP configuration and an explicit valid `CANOPY_AUTH_OUTBOX_SEALING_KEY`. Startup fails when either is absent or invalid unless `CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true` is explicitly configured for development. The escape hatch disables the worker, leaves committed messages queued, and reports email delivery as disabled/degraded without logging payload details.

Production deployments must not set the escape hatch. There is no implicit development detection and no SMTP credential default.

## Components

### `EmailSender`

An async trait accepts a fully rendered internal message and returns a sanitized delivery result. `SmtpEmailSender` implements it with `lettre` over TLS. The trait keeps worker tests deterministic and permits future delivery adapters without changing outbox policy.

### `AuthOutboxRepository`

The PostgreSQL adapter atomically claims committed rows, completes successful rows, and reschedules or exhausts failed rows. Claims use `FOR UPDATE SKIP LOCKED` and a lease token so multiple Canopy processes can poll concurrently.

### `AuthOutboxWorker`

A supervised Tokio task polls in bounded batches, decrypts only claimed payloads, renders the matching template, invokes `EmailSender`, and records a sanitized outcome. The worker observes coordinated shutdown and does not spawn unbounded delivery tasks.

### Delivery Readiness

Shared readiness state records whether delivery is configured, the worker is alive, and SMTP connectivity is currently healthy. Liveness remains independent so a temporary SMTP outage does not restart the server or interrupt unrelated API traffic.

## PostgreSQL Model

An additive migration extends `auth_outbox` with:

- `lease_token UUID`
- `lease_expires_at TIMESTAMPTZ`
- `failed_at TIMESTAMPTZ`
- `last_error_kind VARCHAR(64)` containing only a fixed safe category

The migration makes `encrypted_payload` nullable and replaces its constraint with an invariant:

- pending, leased, retryable, and exhausted rows retain authenticated ciphertext of at least 16 bytes;
- successfully delivered rows have `delivered_at IS NOT NULL` and `encrypted_payload IS NULL`.

The existing pending index is updated to exclude delivered and exhausted rows and support `available_at`/lease scans.

## Delivery Flow

1. Poll for rows whose `available_at` has passed, whose lease is absent or expired, and which are neither delivered nor exhausted.
2. In a short transaction, lock rows with `FOR UPDATE SKIP LOCKED`, assign a random lease token and expiry, increment `attempts`, and return ciphertext plus non-sensitive identifiers.
3. Open the authenticated-encrypted payload in worker memory and validate its template and expiry fields.
4. Render the verification or password-reset message using configured public links.
5. Send through SMTP with a deterministic `Message-ID` derived from the outbox row ID.
6. On success, conditionally update by row ID and lease token, set `delivered_at`, clear `encrypted_payload`, and clear lease/error fields.
7. On retryable failure, conditionally clear the lease and move `available_at` using bounded exponential backoff with jitter.
8. On permanent failure or attempt exhaustion, set `failed_at`, clear the lease, and retain only the sealed payload plus a safe error category for operator inspection.

Lease-token conditions make stale workers unable to complete or reschedule a row after another worker has reclaimed it.

## Delivery Semantics

SMTP does not provide an atomic transaction with PostgreSQL. A crash after SMTP accepts a message but before Canopy records success can produce a duplicate. Delivery is therefore at least once, not exactly once.

Canopy reduces duplicate impact with deterministic message IDs and idempotent database transitions. Verification and reset tokens remain single-use, so receiving a duplicate message does not make a consumed challenge reusable.

## Retry Policy

Retry delays use bounded exponential backoff with jitter. Defaults are configurable but validated:

- poll interval: 2 seconds;
- lease duration: 60 seconds;
- batch size: 20;
- maximum attempts: 8;
- initial retry delay: 5 seconds;
- maximum retry delay: 15 minutes.

Configuration rejects zero values, an initial delay greater than the maximum, and lease durations too short for the SMTP timeout.

## SMTP And Message Configuration

Required production settings cover SMTP host, port, TLS mode, username, password, from address, from display name, connection timeout, and the public application base URL used in verification/reset links. Credentials use secret environment variables and are never included in debug output.

TLS is required outside loopback development. Templates are code-owned, plain-text plus HTML alternatives, and selected from the sealed payload's fixed template identifier. User-controlled values are escaped before HTML rendering.

## Error Handling And Observability

Logs and metrics may include outbox row ID, template kind, attempt count, latency, and a fixed outcome category. They must not include recipient addresses, tokens, ciphertext, decrypted fields, rendered subjects/bodies, SMTP response text, credentials, or message headers containing recipient data.

Safe failure categories are limited to values such as `configuration`, `connection`, `timeout`, `authentication`, `rejected`, `payload`, and `internal`. Raw errors remain in memory only long enough to classify them.

Missing required production configuration fails startup. Runtime SMTP failures mark email readiness unhealthy and trigger retries while liveness and unrelated RPCs remain available. A worker panic is supervised, marks readiness unhealthy, and initiates coordinated server shutdown rather than silently abandoning delivery.

## Testing

Unit tests cover configuration validation, backoff bounds, deterministic message IDs, template rendering/escaping, payload redaction, retry classification, and cancellation.

PostgreSQL integration tests cover concurrent claims, lease expiry/recovery, stale lease-token rejection, successful ciphertext clearing, retry scheduling, attempt exhaustion, and exclusion of delivered/exhausted rows.

Worker tests use a fake `EmailSender` to prove successful delivery, retry, permanent failure, bounded batches, graceful shutdown, and that observable fields never contain email addresses or token/template values.

Runtime construction tests prove PostgreSQL mode fails without SMTP by default and starts with the explicit undelivered-email development escape hatch.

## Rollout

1. Apply the additive outbox migration.
2. Configure the outbox sealing key, SMTP credentials, from identity, and public application base URL.
3. Deploy one instance and verify readiness plus delivery metrics.
4. Scale to multiple instances and confirm lease contention remains bounded.
5. Verify the development escape hatch is absent from production deployment manifests and startup environments.

## Non-Goals

- Exactly-once SMTP delivery.
- A separate worker binary or queue service.
- Marketing or non-authentication email.
- User-editable templates.
- Protobuf or OpenAPI changes.
