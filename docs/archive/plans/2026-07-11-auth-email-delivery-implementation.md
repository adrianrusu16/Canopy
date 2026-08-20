# Authentication Email Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Completed steps are checked; remaining steps use unchecked boxes.

**Goal:** Deliver committed authentication verification and password-reset outbox rows through supervised SMTP with safe retries, multi-instance leasing, secret clearing, and explicit readiness.

**Architecture:** Keep delivery inside the identity boundary while separating persistence, rendering/transport, and worker policy into focused ports and adapters. PostgreSQL provides short leased claims with FOR UPDATE SKIP LOCKED; an in-process Tokio worker performs at-least-once SMTP delivery through lettre, while shared readiness keeps liveness independent from outbound SMTP failures.

**Tech Stack:** Rust 2024, Tokio, SQLx/PostgreSQL, lettre 0.11.22 with Tokio/rustls SMTP, html-escape 0.2.13, existing AES-256-GCM sealed outbox payloads, tracing, and Tonic health responses.

## Global Constraints

- Do not commit, stage, push, or switch branches; the user handles Git.
- PostgreSQL mode requires valid SMTP configuration unless CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true is explicitly set.
- SMTP-enabled mode also requires CANOPY_AUTH_OUTBOX_SEALING_KEY to decode to exactly 32 bytes; the development fallback is never accepted with delivery enabled.
- The development escape hatch disables delivery, leaves rows queued, and reports degraded email readiness.
- Production SMTP supports implicit TLS or required STARTTLS only.
- Delivery is at least once. Never claim exactly-once SMTP semantics.
- Never log or persist recipient addresses, tokens, ciphertext, decrypted payloads, rendered messages, credentials, SMTP response text, or recipient-bearing headers.
- Logs may contain only outbox row ID, purpose, attempt number, latency, and a fixed safe outcome category.
- Successful delivery sets delivered_at and clears encrypted_payload.
- Retry, completion, and exhaustion updates require the active lease token.
- Protobuf and OpenAPI do not change because SMTP delivery is internal runtime behavior.
- Every implementation task follows red-green-refactor and ends with a clean diff check.

---

### Task 1: SMTP And Worker Configuration

**Files:**
- Modify: Cargo.toml
- Modify: crates/canopy-server/Cargo.toml
- Modify: crates/canopy-server/src/config.rs
- Test: crates/canopy-server/src/config.rs

**Interfaces:**
- Consumes: existing parse_bool and injected from_lookup configuration pattern.
- Produces: AuthEmailConfig, SmtpConfig, SmtpTlsMode, AuthEmailWorkerConfig, validated outbox sealing-key configuration, and Config::auth_email.

- [x] **Step 1: Write failing configuration tests**

Add config::tests::auth_email with these behaviors:

    #[test]
    fn smtp_is_required_by_default() {
        let error = AuthEmailConfig::from_lookup(|_| None).unwrap_err();
        assert!(error.to_string().contains("CANOPY_SMTP_HOST"));
    }

    #[test]
    fn explicit_escape_hatch_allows_missing_smtp() {
        let config = parse(&[
            ("CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL", "true"),
        ]).unwrap();
        assert!(config.allow_undelivered_email);
        assert!(config.smtp.is_none());
    }

    #[test]
    fn partial_smtp_configuration_is_rejected() {
        let error = parse(&[
            ("CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL", "true"),
            ("CANOPY_SMTP_HOST", "smtp.example.test"),
        ]).unwrap_err();
        assert!(error.to_string().contains("CANOPY_SMTP_USERNAME"));
    }

    #[test]
    fn complete_starttls_configuration_is_accepted() {
        let config = parse(&required_smtp()).unwrap();
        let smtp = config.smtp.unwrap();
        assert_eq!(smtp.tls_mode, SmtpTlsMode::StartTls);
        assert_eq!(smtp.port, 587);
    }

    #[test]
    fn configured_smtp_requires_explicit_outbox_sealing_key() {
        let values = required_smtp()
            .into_iter()
            .filter(|(key, _)| *key != "CANOPY_AUTH_OUTBOX_SEALING_KEY")
            .collect::<Vec<_>>();
        let error = parse(&values).unwrap_err();
        assert!(error.to_string().contains("CANOPY_AUTH_OUTBOX_SEALING_KEY"));
    }

    #[test]
    fn worker_rejects_lease_shorter_than_smtp_timeout() {
        let mut values = required_smtp();
        values.extend([
            ("CANOPY_AUTH_EMAIL_LEASE_SECS", "10"),
            ("CANOPY_SMTP_TIMEOUT_SECS", "30"),
        ]);
        assert!(parse(&values).is_err());
    }

- [x] **Step 2: Run the focused tests and verify RED**

Run:

    cargo test -p canopy-server config::tests::auth_email --locked

Expected: compilation fails because the email configuration types do not exist.

- [x] **Step 3: Implement validated configuration types**

Add:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SmtpTlsMode {
        Implicit,
        StartTls,
    }

    #[derive(Clone)]
    pub struct SmtpConfig {
        pub host: String,
        pub port: u16,
        pub tls_mode: SmtpTlsMode,
        pub username: String,
        pub password: String,
        pub from_address: String,
        pub from_name: String,
        pub public_base_url: reqwest::Url,
        pub timeout: std::time::Duration,
    }

    #[derive(Clone, Debug)]
    pub struct AuthEmailWorkerConfig {
        pub poll_interval: std::time::Duration,
        pub lease_duration: std::time::Duration,
        pub batch_size: u32,
        pub max_attempts: u32,
        pub initial_retry_delay: std::time::Duration,
        pub max_retry_delay: std::time::Duration,
    }

    #[derive(Clone)]
    pub struct AuthEmailConfig {
        pub allow_undelivered_email: bool,
        pub outbox_sealing_key_base64: Option<String>,
        pub smtp: Option<SmtpConfig>,
        pub worker: AuthEmailWorkerConfig,
    }

Implement AuthEmailConfig::from_lookup with these environment names:

    CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL
    CANOPY_AUTH_OUTBOX_SEALING_KEY
    CANOPY_SMTP_HOST
    CANOPY_SMTP_PORT
    CANOPY_SMTP_TLS_MODE
    CANOPY_SMTP_USERNAME
    CANOPY_SMTP_PASSWORD
    CANOPY_SMTP_FROM_ADDRESS
    CANOPY_SMTP_FROM_NAME
    CANOPY_AUTH_PUBLIC_BASE_URL
    CANOPY_SMTP_TIMEOUT_SECS
    CANOPY_AUTH_EMAIL_POLL_INTERVAL_SECS
    CANOPY_AUTH_EMAIL_LEASE_SECS
    CANOPY_AUTH_EMAIL_BATCH_SIZE
    CANOPY_AUTH_EMAIL_MAX_ATTEMPTS
    CANOPY_AUTH_EMAIL_INITIAL_RETRY_SECS
    CANOPY_AUTH_EMAIL_MAX_RETRY_SECS

Defaults: implicit port 465, STARTTLS port 587, SMTP timeout 30 seconds, poll 2 seconds, lease 60 seconds, batch 20, attempts 8, initial retry 5 seconds, maximum retry 900 seconds. Require HTTPS for the public base URL except localhost and 127.0.0.1. If no SMTP value is present and the escape hatch is true, return smtp: None. If any SMTP value is present, require every SMTP field. When smtp is Some, require CANOPY_AUTH_OUTBOX_SEALING_KEY to be valid base64 that decodes to exactly 32 bytes; required_smtp test fixtures include this key.

- [x] **Step 4: Add dependencies and wire Config**

Add workspace dependencies:

    lettre = { version = "0.11.22", default-features = false, features = ["builder", "hostname", "pool", "smtp-transport", "tokio1-rustls-tls"] }
    html-escape = "0.2.13"

Add both workspace dependencies to canopy-server. Add pub auth_email: AuthEmailConfig to Config and load it in Config::from_env.

- [x] **Step 5: Verify GREEN**

Run:

    cargo fmt --all
    cargo test -p canopy-server config::tests::auth_email --locked
    cargo clippy -p canopy-server --lib --all-features --locked -- -D warnings

Expected: focused tests pass with no warnings.

---

### Task 2: Additive Schema And Core Outbox Port

**Files:**
- Create: migrations/20260711000001_auth_outbox_delivery.sql
- Create: crates/canopy-core/src/auth_outbox_repository.rs
- Modify: crates/canopy-core/src/lib.rs
- Modify: crates/canopy-server/tests/identity_schema.rs

**Interfaces:**
- Produces: AuthOutboxFailureKind, ClaimAuthOutboxBatch, ClaimedAuthOutbox, MarkAuthOutboxFailed, and AuthOutboxRepository.

- [x] **Step 1: Write failing schema assertions**

Require the migration chain to contain lease_token, lease_expires_at, failed_at, last_error_kind, nullable delivered ciphertext, and a pending index that excludes exhausted rows. Task 3 integration tests prove the lease query behavior.

- [x] **Step 2: Run the schema test and verify RED**

    cargo test -p canopy-server --test identity_schema --locked

Expected: assertions fail because delivery schema and adapter do not exist.

- [x] **Step 3: Add the migration**

Create the additive migration:

    ALTER TABLE auth_outbox
        DROP CONSTRAINT auth_outbox_payload_ck,
        ALTER COLUMN encrypted_payload DROP NOT NULL,
        ADD COLUMN lease_token UUID,
        ADD COLUMN lease_expires_at TIMESTAMPTZ,
        ADD COLUMN failed_at TIMESTAMPTZ,
        ADD COLUMN last_error_kind VARCHAR(64);

    ALTER TABLE auth_outbox
        ADD CONSTRAINT auth_outbox_payload_lifecycle_ck CHECK (
            (
                delivered_at IS NULL
                AND encrypted_payload IS NOT NULL
                AND octet_length(encrypted_payload) >= 16
            )
            OR (
                delivered_at IS NOT NULL
                AND encrypted_payload IS NULL
            )
        ),
        ADD CONSTRAINT auth_outbox_lease_ck CHECK (
            (lease_token IS NULL) = (lease_expires_at IS NULL)
        ),
        ADD CONSTRAINT auth_outbox_terminal_ck CHECK (
            NOT (delivered_at IS NOT NULL AND failed_at IS NOT NULL)
        ),
        ADD CONSTRAINT auth_outbox_error_kind_ck CHECK (
            last_error_kind IS NULL OR last_error_kind IN (
                'configuration', 'connection', 'timeout',
                'authentication', 'rejected', 'payload', 'internal'
            )
        );

    DROP INDEX auth_outbox_pending_idx;

    CREATE INDEX auth_outbox_pending_idx
        ON auth_outbox(available_at, lease_expires_at, created_at)
        WHERE delivered_at IS NULL AND failed_at IS NULL;

- [x] **Step 4: Add the core persistence port**

Create these exact public types:

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum AuthOutboxFailureKind {
        Configuration,
        Connection,
        Timeout,
        Authentication,
        Rejected,
        Payload,
        Internal,
    }

    pub struct ClaimAuthOutboxBatch {
        pub now_epoch_ms: u64,
        pub lease_expires_at_epoch_ms: u64,
        pub batch_size: u32,
        pub lease_token: String,
    }

    pub struct ClaimedAuthOutbox {
        pub id: String,
        pub kind: String,
        pub encrypted_payload: Vec<u8>,
        pub key_id: String,
        pub attempts: u32,
        pub lease_token: String,
    }

    pub struct MarkAuthOutboxFailed {
        pub id: String,
        pub lease_token: String,
        pub error_kind: AuthOutboxFailureKind,
        pub available_at_epoch_ms: u64,
        pub failed_at_epoch_ms: Option<u64>,
    }

    #[async_trait]
    pub trait AuthOutboxRepository: Send + Sync {
        async fn claim_auth_outbox_batch(
            &self,
            command: ClaimAuthOutboxBatch,
        ) -> CanopyResult<Vec<ClaimedAuthOutbox>>;

        async fn mark_auth_outbox_delivered(
            &self,
            id: &str,
            lease_token: &str,
            delivered_at_epoch_ms: u64,
        ) -> CanopyResult<bool>;

        async fn mark_auth_outbox_failed(
            &self,
            command: MarkAuthOutboxFailed,
        ) -> CanopyResult<bool>;
    }

Implement AuthOutboxFailureKind::as_str and a manual redacted Debug for ClaimedAuthOutbox. Export all types from canopy-core/src/lib.rs.

- [x] **Step 5: Verify GREEN**

    cargo fmt --all
    cargo test -p canopy-server --test identity_schema --locked
    cargo test -p canopy-core --locked
    cargo clippy -p canopy-core --all-targets --locked -- -D warnings

---

### Task 3: PostgreSQL Lease And State Transitions

**Files:**
- Create: crates/canopy-server/src/jade_store/pg_auth_outbox.rs
- Modify: crates/canopy-server/src/jade_store/mod.rs
- Modify: crates/canopy-server/tests/pg_integration.rs

**Interfaces:**
- Consumes: Task 2 persistence port.
- Produces: PgAuthOutboxRepository::new(sqlx::PgPool).

- [x] **Step 1: Write failing PostgreSQL integration tests**

Add tests named:

    postgres_auth_outbox_claims_each_row_once_across_workers
    postgres_auth_outbox_expired_lease_can_be_reclaimed
    postgres_auth_outbox_stale_lease_cannot_complete_reclaimed_row
    postgres_auth_outbox_success_clears_ciphertext
    postgres_auth_outbox_retry_and_exhaustion_leave_safe_state

Use direct SQL only for fixture insertion and final assertions. Exercise transitions through AuthOutboxRepository. Set batch_size to 1 in the concurrent claim test so each repository instance must claim a different row.

- [x] **Step 2: Run focused tests and verify RED**

    bash scripts/test-pg.sh

Expected: compilation fails because PgAuthOutboxRepository does not exist.

- [x] **Step 3: Implement atomic claim**

Use one transaction and this query shape:

    WITH candidates AS (
        SELECT id
        FROM auth_outbox
        WHERE delivered_at IS NULL
          AND failed_at IS NULL
          AND available_at <= $1
          AND (lease_expires_at IS NULL OR lease_expires_at <= $1)
        ORDER BY available_at, created_at
        FOR UPDATE SKIP LOCKED
        LIMIT $2
    )
    UPDATE auth_outbox AS outbox
    SET lease_token = $3::uuid,
        lease_expires_at = $4,
        attempts = outbox.attempts + 1
    FROM candidates
    WHERE outbox.id = candidates.id
    RETURNING outbox.id::text, outbox.kind, outbox.encrypted_payload,
              outbox.key_id, outbox.attempts, outbox.lease_token::text

Validate positive batch size, lease expiry after now, parseable UUID lease token, and non-null returned ciphertext.

- [x] **Step 4: Implement conditional completion**

    UPDATE auth_outbox
    SET delivered_at = $3,
        encrypted_payload = NULL,
        lease_token = NULL,
        lease_expires_at = NULL,
        last_error_kind = NULL
    WHERE id = $1::uuid
      AND lease_token = $2::uuid
      AND delivered_at IS NULL
      AND failed_at IS NULL
    RETURNING TRUE

Return false when the lease no longer matches.

- [x] **Step 5: Implement conditional retry/exhaustion**

    UPDATE auth_outbox
    SET available_at = $4,
        failed_at = $5,
        last_error_kind = $3,
        lease_token = NULL,
        lease_expires_at = NULL
    WHERE id = $1::uuid
      AND lease_token = $2::uuid
      AND delivered_at IS NULL
      AND failed_at IS NULL
    RETURNING TRUE

Retry leaves failed_at null. Exhaustion sets failed_at. Both preserve sealed ciphertext.

- [x] **Step 6: Verify GREEN**

    cargo fmt --all
    bash scripts/test-pg.sh
    cargo clippy -p canopy-server --all-targets --features pg --locked -- -D warnings

---

### Task 4: Safe Rendering And SMTP Adapter

**Files:**
- Create: crates/canopy-server/src/identity/email.rs
- Modify: crates/canopy-server/src/identity/outbox.rs
- Modify: crates/canopy-server/src/identity/mod.rs

**Interfaces:**
- Consumes: EmailOutboxPayload, SmtpConfig, SmtpTlsMode, and AuthOutboxFailureKind.
- Produces: EmailOutboxPayload::open_with_key_id, RenderedEmail, EmailDeliveryError, EmailSender, EmailRenderer, and SmtpEmailSender.

- [x] **Step 1: Write failing renderer/redaction tests**

Cover encoded verification links, encoded reset links, deterministic message IDs, unknown purpose rejection, missing token rejection, unsupported sealing key IDs, HTML escaping, RenderedEmail debug redaction, and EmailDeliveryError safe debug output.

Example assertion:

    let rendered = renderer()
        .render("00000000-0000-0000-0000-000000000042", &payload)
        .unwrap();
    assert_eq!(
        rendered.message_id,
        "<canopy-auth-00000000-0000-0000-0000-000000000042@example.test>"
    );
    assert!(!format!("{rendered:?}").contains("ada@example.test"));

- [x] **Step 2: Run tests and verify RED**

    cargo test -p canopy-server identity::email::tests --locked

- [x] **Step 3: Implement transport types and renderer**

Define:

    pub struct RenderedEmail {
        pub message_id: String,
        pub to: String,
        pub subject: String,
        pub text_body: String,
        pub html_body: String,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct EmailDeliveryError {
        pub kind: AuthOutboxFailureKind,
        pub retryable: bool,
    }

    #[async_trait]
    pub trait EmailSender: Send + Sync {
        async fn send(&self, email: &RenderedEmail)
            -> Result<(), EmailDeliveryError>;
        async fn probe(&self) -> Result<(), EmailDeliveryError>;
    }

    #[derive(Clone)]
    pub struct EmailRenderer {
        public_base_url: reqwest::Url,
        from_domain: String,
    }

Add the persisted-key validation API in outbox.rs:

    pub fn open_with_key_id(key_id: &str, bytes: &[u8]) -> CanopyResult<Self> {
        if key_id != OUTBOX_KEY_ID {
            return Err(CanopyError::InvalidArgument(
                "unsupported auth outbox key id".into(),
            ));
        }
        Self::open(bytes)
    }

Use Url::query_pairs_mut for tokens and html_escape::encode_safe for HTML. Support only email_verification and password_reset. Implement manual redacted Debug for RenderedEmail.

- [x] **Step 4: Implement SmtpEmailSender**

Reuse one AsyncSmtpTransport<Tokio1Executor>. Build implicit TLS with relay and STARTTLS with starttls_relay. Set port, credentials, timeout, and pooling once.

Build messages with Message::builder, fixed from address, parsed recipient, fixed subject, deterministic message ID, and MultiPart::alternative_plain_html. Map SMTP failures to fixed categories without storing raw errors. probe uses test_connection.

- [x] **Step 5: Verify GREEN**

    cargo fmt --all
    cargo test -p canopy-server identity::email::tests --locked
    cargo clippy -p canopy-server --lib --all-features --locked -- -D warnings

---

### Task 5: Worker Policy And Cancellation

**Files:**
- Create: crates/canopy-server/src/identity/email_worker.rs
- Modify: crates/canopy-server/src/identity/mod.rs

**Interfaces:**
- Consumes: AuthOutboxRepository, EmailOutboxPayload::open_with_key_id, EmailRenderer, EmailSender, and AuthEmailWorkerConfig.
- Produces: EmailDeliveryReadiness, EmailDeliveryState, AuthEmailWorker::run_once, and AuthEmailWorker::run.

- [x] **Step 1: Write failing worker tests using fakes**

Add tests named:

    run_once_delivers_and_completes_claimed_row
    retryable_failure_reschedules_with_bounded_backoff
    permanent_failure_exhausts_immediately
    max_attempts_exhausts_retryable_failure
    stale_completion_is_treated_as_lease_loss
    run_stops_when_shutdown_watch_changes
    retry_delay_is_deterministic_jittered_and_bounded
    worker_debug_does_not_contain_payload_data

Fake repository rows must contain real sealed payloads.

- [x] **Step 2: Run tests and verify RED**

    cargo test -p canopy-server identity::email_worker::tests --locked

- [x] **Step 3: Implement shared readiness**

Define:

    pub enum EmailDeliveryState {
        Healthy,
        Degraded,
        Unhealthy,
    }

    pub struct EmailDeliverySnapshot {
        pub state: EmailDeliveryState,
        pub message: &'static str,
    }

    #[derive(Clone)]
    pub struct EmailDeliveryReadiness {
        inner: Arc<RwLock<EmailDeliverySnapshot>>,
    }

Provide configured(), disabled(), snapshot(), mark_healthy(), and mark_unhealthy(). Messages are fixed static strings.

- [x] **Step 4: Implement one bounded pass**

Define:

    pub struct AuthEmailWorker {
        repository: Arc<dyn AuthOutboxRepository>,
        sender: Arc<dyn EmailSender>,
        renderer: EmailRenderer,
        config: AuthEmailWorkerConfig,
        readiness: EmailDeliveryReadiness,
    }

    impl AuthEmailWorker {
        pub async fn run_once(&self, now_epoch_ms: u64)
            -> CanopyResult<usize>;

        pub async fn run(
            self,
            shutdown: watch::Receiver<bool>,
        ) -> CanopyResult<()>;
    }

run_once generates a UUID lease token, claims one batch, calls EmailOutboxPayload::open_with_key_id with the persisted key ID, validates payload expiry, renders, sends, and conditionally completes or fails. Expired or invalid payloads are permanent Payload failures. False transition results mean lease loss and are not retried by the stale worker.

Compute exponential retry plus deterministic SHA-256 jitter from outbox ID and attempt count, capped by max_retry_delay.

- [x] **Step 5: Implement polling and cancellation**

Probe SMTP before every claim pass. A failed probe marks readiness unhealthy, waits for the next poll, and skips claiming rows so an SMTP outage does not consume attempts. A successful probe marks readiness healthy before run_once. Use tokio::select between watch shutdown changes and sleep(poll_interval). Repository errors propagate to runtime supervision.

- [x] **Step 6: Verify GREEN**

    cargo fmt --all
    cargo test -p canopy-server identity::email_worker::tests --locked
    cargo clippy -p canopy-server --lib --all-features --locked -- -D warnings

---

### Task 6: Health And Runtime Supervision

**Files:**
- Modify: crates/canopy-server/src/health.rs
- Modify: crates/canopy-server/src/lib.rs

**Interfaces:**
- Consumes: Config::auth_email, PgAuthOutboxRepository, SmtpEmailSender, AuthEmailWorker, and EmailDeliveryReadiness.
- Produces: HealthService::with_email_delivery and a third supervised runtime future.

- [x] **Step 1: Write failing health/wiring tests**

Add:

    #[tokio::test]
    async fn disabled_email_delivery_is_degraded() {
        let health = HealthService::new()
            .with_email_delivery(EmailDeliveryReadiness::disabled());
        let status = health.check().await;
        assert_eq!(status.status, HealthState::Degraded);
        assert_eq!(status.dependencies[0].name, "auth_email_delivery");
    }

    #[tokio::test]
    async fn smtp_failure_is_unhealthy() {
        let readiness = EmailDeliveryReadiness::configured();
        readiness.mark_unhealthy("SMTP connectivity failed");
        let status = HealthService::new().with_email_delivery(readiness);
        assert_eq!(status.status, HealthState::Unhealthy);
    }

Extend wiring tests so a third receiver gets the same shutdown signal.

- [x] **Step 2: Run tests and verify RED**

    cargo test -p canopy-server health::tests --locked
    cargo test -p canopy-server wiring_tests --locked

- [x] **Step 3: Add email dependency health**

Add email_delivery: Option<EmailDeliveryReadiness> and:

    pub fn with_email_delivery(
        mut self,
        readiness: EmailDeliveryReadiness,
    ) -> Self {
        self.email_delivery = Some(readiness);
        self
    }

Map delivery state to HealthState under dependency name auth_email_delivery.

- [x] **Step 4: Construct runtime state**

With SMTP configured, construct PgAuthOutboxRepository, EmailRenderer, SmtpEmailSender, and AuthEmailWorker. Without SMTP under the explicit escape hatch, use EmailDeliveryReadiness::disabled and no worker. Attach readiness to HealthService in both cases.

- [x] **Step 5: Supervise with coordinated shutdown**

Clone a third watch receiver. Run worker.run in tokio::spawn, convert JoinError or worker error into a sanitized io::Error, and include the email future in tokio::try_join!(grpc, http, email). With no worker, the email future waits for shutdown and returns Ok.

- [x] **Step 6: Verify GREEN**

    cargo fmt --all
    cargo test -p canopy-server health::tests --locked
    cargo test -p canopy-server wiring_tests --locked
    cargo clippy -p canopy-server --all-targets --all-features --locked -- -D warnings

---

### Task 7: Documentation And Full Verification

**Files:**
- Modify: .env.example
- Modify: README.md
- Modify: docs/canopy-api-bsr-design.md
- Modify: docs/superpowers/plans/2026-07-09-authentication-completion.md
- Modify: this plan

- [x] **Step 1: Document configuration**

Add every Task 1 variable to .env.example using non-secret placeholders. Update the README configuration table with defaults, TLS restrictions, and the development-only escape hatch.

- [x] **Step 2: Document semantics/readiness**

Replace the current missing-worker note with text stating: delivery is supervised and at least once; deterministic message IDs reduce duplicates; challenges remain single-use; success clears ciphertext; SMTP outages affect readiness but not liveness or unrelated RPCs.

- [x] **Step 3: Run complete verification**

In WSL with Buf credentials available:

    cargo fmt --all -- --check
    cargo test --workspace --locked
    bash scripts/test-pg.sh
    cargo clippy --workspace --all-features --tests --locked -- -D warnings

Validate canonical OpenAPI JSON and scan for stale missing-worker status text.

- [x] **Step 4: Update status after verification**

Only after Step 3 passes, check Task 9 Steps 2-6 in the completion plan and mark this focused plan complete. Remove production email delivery from the remaining-work list in canopy-api-bsr-design.md. Do not change protobuf or OpenAPI.

- [x] **Step 5: Review without Git mutation**

Run:

    git diff --check
    git status --short
    git diff --stat

Confirm no raw token, recipient, rendered body, SMTP password, or real secret was added. Do not stage or commit.

---

## Self-Review

- Spec coverage: startup policy, TLS, leasing, retries, at-least-once semantics, deterministic message IDs, secret clearing, readiness, supervision, observability, and rollout documentation each map to a task.
- Placeholder review: every task names concrete files, interfaces, tests, commands, and expected red-green outcomes.
- Type consistency: Task 2 defines the persistence types, Task 3 implements them, Tasks 4-5 consume them, and Task 6 wires shared readiness.
- Scope: protobuf, OpenAPI, external queue services, separate worker binaries, and marketing email remain out of scope.
