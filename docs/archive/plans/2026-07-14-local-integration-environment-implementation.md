# Local Integration Environment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide one backend-owned command that runs Canopy with real PostgreSQL, authenticated STARTTLS email delivery, Nginx streaming, and a complete authentication/playback smoke test.

**Architecture:** Run the PostgreSQL-enabled `canopy` binary natively in WSL and run PostgreSQL, Mailpit, and Nginx in a dedicated `canopy-local-integration` Compose project. Keep generated state under `target/local-integration/`, verify SMTP with an ephemeral local CA, and use ignored Rust integration tests for all gRPC and HTTP assertions.

**Tech Stack:** Rust 2024, Tokio, Tonic/Prost BSR clients, Lettre 0.11.22 with rustls, rustls-pki-types 1.15.0, Reqwest, Docker Compose v2, PostgreSQL 18.4, Mailpit 1.30.4, Nginx stable-alpine, Bash, OpenSSL, and sqlx-cli.

## Global Constraints

- Implement `docs/superpowers/specs/2026-07-14-local-integration-environment-design.md`.
- Preserve the existing dirty worktree and all unrelated user changes.
- The user owns Git. Do not stage, commit, push, tag, switch branches, or create a worktree.
- Bind gRPC `50051`, private stream auth `18081`, PostgreSQL `55434`, SMTP `1025`, Mailpit UI `8025`, and Nginx `8080` to `127.0.0.1`.
- Use Compose project name `canopy-local-integration`.
- Use `postgres:18.4-alpine`, `axllent/mailpit:v1.30.4`, and `nginx:stable-alpine`.
- Keep ordinary workspace tests Docker-independent; real-boundary tests remain ignored.
- Keep SMTP hostname and certificate verification enabled. Never introduce `Tls::None`, opportunistic STARTTLS, or invalid-certificate/hostname acceptance.
- Never print tokens, message bodies, generated credentials, process environments, or private capability URLs.
- Do not change protobufs, migrations, OpenAPI routes, `canopy-api`, production Compose, or CI workflows.

## File Structure

- Create `fixtures/certs/local-integration-test-ca.pem`: public CA certificate used by deterministic tests.
- Modify `Cargo.toml`, `Cargo.lock`, and `crates/canopy-server/Cargo.toml`: add direct structured PEM validation with `rustls-pki-types`.
- Modify `crates/canopy-server/src/config.rs`: validate `CANOPY_SMTP_CA_CERT_PATH` and retain PEM bytes in `SmtpConfig`.
- Modify `crates/canopy-server/src/identity/email.rs`: add the custom CA to verified implicit TLS or required STARTTLS.
- Create `crates/canopy-server/tests/local_integration_contract.rs`: Docker-independent drift and shell-interface tests.
- Create `docker-compose.local-integration.yml`: disposable PostgreSQL, Mailpit, and Nginx dependencies.
- Create `crates/canopy-server/tests/local_integration.rs`: ignored readiness and authentication/playback tests.
- Create `scripts/local-integration.sh`: scoped `up`, `test`, `status`, and `down` lifecycle.
- Modify `.env.example`, `README.md`, and `docs/client-integration.md`: public configuration and local-reference guidance.

---

### Task 1: Verified Custom SMTP CA Support

**Files:**
- Create: `fixtures/certs/local-integration-test-ca.pem`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/canopy-server/Cargo.toml`
- Modify: `crates/canopy-server/src/config.rs:96`
- Modify: `crates/canopy-server/src/config.rs:156`
- Modify: `crates/canopy-server/src/config.rs:877`
- Modify: `crates/canopy-server/src/identity/email.rs:5`
- Modify: `crates/canopy-server/src/identity/email.rs:170`
- Modify: `crates/canopy-server/src/identity/email.rs:270`

**Interfaces:**
- Consumes: `CANOPY_SMTP_CA_CERT_PATH`, a path to one or more PEM certificates.
- Produces: `SmtpConfig::ca_certificate_pem: Option<Vec<u8>>`.
- Produces: private `custom_tls(config: &SmtpConfig) -> CanopyResult<Option<Tls>>`.
- Preserves: current system-root behavior when the variable is absent.

- [x] **Step 1: Create a deterministic public CA fixture**

Keep the private key in ignored state:

```bash
mkdir -p fixtures/certs target/local-integration-plan
openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 36500 \
  -subj "/CN=Canopy Local Integration Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -keyout target/local-integration-plan/test-ca.key \
  -out fixtures/certs/local-integration-test-ca.pem
openssl x509 -in fixtures/certs/local-integration-test-ca.pem -noout -subject
```

Expected: subject contains `Canopy Local Integration Test CA`; only the public certificate is created outside ignored `target/`.

- [x] **Step 2: Write failing configuration tests**

Add inside `config.rs` module `tests::auth_email`:

```rust
const TEST_CA_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../fixtures/certs/local-integration-test-ca.pem"
);

#[test]
fn valid_custom_smtp_ca_is_loaded() {
    let mut values = required_smtp();
    values.push(("CANOPY_SMTP_CA_CERT_PATH", TEST_CA_PATH));
    let smtp = parse(&values).unwrap().smtp.unwrap();
    assert_eq!(
        smtp.ca_certificate_pem.as_deref(),
        Some(include_bytes!("../../../fixtures/certs/local-integration-test-ca.pem").as_slice())
    );
}

#[test]
fn empty_custom_smtp_ca_path_is_rejected() {
    let mut values = required_smtp();
    values.push(("CANOPY_SMTP_CA_CERT_PATH", " "));
    let error = parse(&values)
        .err()
        .expect("empty custom SMTP CA path should fail");
    assert!(error.to_string().contains("CANOPY_SMTP_CA_CERT_PATH must not be empty"));
}

#[test]
fn unreadable_custom_smtp_ca_path_is_rejected() {
    let mut values = required_smtp();
    values.push((
        "CANOPY_SMTP_CA_CERT_PATH",
        "/definitely/missing/canopy-ca.pem",
    ));
    let error = parse(&values)
        .err()
        .expect("missing custom SMTP CA should fail");
    assert!(
        error
            .to_string()
            .contains("CANOPY_SMTP_CA_CERT_PATH must name a readable PEM certificate")
    );
}

#[test]
fn malformed_custom_smtp_ca_is_rejected() {
    let malformed = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(malformed.path(), b"not a certificate").unwrap();
    let path: &'static str = Box::leak(
        malformed
            .path()
            .to_string_lossy()
            .into_owned()
            .into_boxed_str(),
    );
    let mut values = required_smtp();
    values.push(("CANOPY_SMTP_CA_CERT_PATH", path));
    let error = parse(&values)
        .err()
        .expect("malformed custom SMTP CA should fail");
    assert!(
        error
            .to_string()
            .contains("CANOPY_SMTP_CA_CERT_PATH must contain valid PEM certificates")
    );
}
```

- [x] **Step 3: Verify the configuration tests fail**

Run:

```bash
cargo test -p canopy-server --features pg config::tests::auth_email --locked
```

Expected: compilation fails because `SmtpConfig` has no `ca_certificate_pem` field, or the new tests fail because the variable is ignored.

- [x] **Step 4: Add validated CA loading to `AuthEmailConfig`**

Add this `SmtpConfig` field:

```rust
pub ca_certificate_pem: Option<Vec<u8>>,
```

Capture the raw path before `smtp_present` and include its presence:

```rust
let smtp_ca_cert_path = lookup("CANOPY_SMTP_CA_CERT_PATH");
let smtp_present = smtp_ca_cert_path.is_some()
    || [
        "CANOPY_SMTP_HOST",
        "CANOPY_SMTP_PORT",
        "CANOPY_SMTP_TLS_MODE",
        "CANOPY_SMTP_USERNAME",
        "CANOPY_SMTP_PASSWORD",
        "CANOPY_SMTP_FROM_ADDRESS",
        "CANOPY_SMTP_FROM_NAME",
        "CANOPY_AUTH_PUBLIC_BASE_URL",
        "CANOPY_SMTP_TIMEOUT_SECS",
    ]
    .iter()
    .any(|key| lookup(key).is_some_and(|value| !value.trim().is_empty()));
```

Inside the complete SMTP branch, load and validate without exposing the path in errors:

```rust
let ca_certificate_pem = smtp_ca_cert_path
    .map(|path| {
        let path = path.trim();
        if path.is_empty() {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_SMTP_CA_CERT_PATH must not be empty".into(),
            ));
        }
        let pem = std::fs::read(path).map_err(|_| {
            CanopyError::InvalidArgument(
                "CANOPY_SMTP_CA_CERT_PATH must name a readable PEM certificate".into(),
            )
        })?;
        rustls_pki_types::CertificateDer::from_pem_slice(&pem).map_err(|_| {
            CanopyError::InvalidArgument(
                "CANOPY_SMTP_CA_CERT_PATH must contain valid PEM certificates".into(),
            )
        })?;
        lettre::transport::smtp::client::Certificate::from_pem(&pem).map_err(|_| {
            CanopyError::InvalidArgument(
                "CANOPY_SMTP_CA_CERT_PATH must contain valid PEM certificates".into(),
            )
        })?;
        Ok(pem)
    })
    .transpose()?;
```

Set `ca_certificate_pem` in the `SmtpConfig` literal.

The `rustls-pki-types` check is required because Lettre accepts a PEM payload with zero certificate blocks; Lettre then validates the complete non-empty bundle.

- [x] **Step 5: Verify configuration tests pass**

Run:

```bash
cargo test -p canopy-server --features pg config::tests::auth_email --locked
```

Expected: all authentication-email configuration tests pass.

- [x] **Step 6: Write failing SMTP TLS-policy tests**

In `identity/email.rs`, add:

```rust
fn smtp_config(tls_mode: SmtpTlsMode) -> SmtpConfig {
    SmtpConfig {
        host: "localhost".into(),
        port: 1025,
        tls_mode,
        username: "canopy".into(),
        password: "redacted-test-value".into(),
        from_address: "auth@example.test".into(),
        from_name: "Canopy".into(),
        public_base_url: Url::parse("http://127.0.0.1:3000/auth/").unwrap(),
        timeout: std::time::Duration::from_secs(5),
        ca_certificate_pem: Some(
            include_bytes!("../../../../fixtures/certs/local-integration-test-ca.pem").to_vec(),
        ),
    }
}

#[test]
fn custom_ca_keeps_starttls_required() {
    let tls = custom_tls(&smtp_config(SmtpTlsMode::StartTls)).unwrap();
    assert!(matches!(tls, Some(Tls::Required(_))));
}

#[test]
fn custom_ca_keeps_implicit_tls_wrapped() {
    let tls = custom_tls(&smtp_config(SmtpTlsMode::Implicit)).unwrap();
    assert!(matches!(tls, Some(Tls::Wrapper(_))));
}
```

- [x] **Step 7: Verify the email tests fail**

Run:

```bash
cargo test -p canopy-server --features pg identity::email::tests --locked
```

Expected: compilation fails because `custom_tls` and `Tls` do not exist.

- [x] **Step 8: Install the verified custom root in Lettre**

Extend the Lettre SMTP import:

```rust
transport::smtp::{
    Error as SmtpError,
    authentication::Credentials,
    client::{Certificate, Tls, TlsParametersBuilder},
},
```

Add:

```rust
fn custom_tls(config: &SmtpConfig) -> CanopyResult<Option<Tls>> {
    let Some(pem) = config.ca_certificate_pem.as_deref() else {
        return Ok(None);
    };
    let certificate = Certificate::from_pem(pem)
        .map_err(|_| CanopyError::InvalidArgument("invalid custom SMTP CA certificate".into()))?;
    let parameters = TlsParametersBuilder::new(config.host.clone())
        .add_root_certificate(certificate)
        .build()
        .map_err(|_| CanopyError::InvalidArgument("invalid SMTP TLS configuration".into()))?;

    Ok(Some(match config.tls_mode {
        SmtpTlsMode::Implicit => Tls::Wrapper(parameters),
        SmtpTlsMode::StartTls => Tls::Required(parameters),
    }))
}
```

Make the existing relay builder mutable and apply the custom policy before port and credentials:

```rust
let mut builder = match config.tls_mode {
    SmtpTlsMode::Implicit => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host),
    SmtpTlsMode::StartTls => {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
    }
}
.map_err(|_| CanopyError::InvalidArgument("invalid SMTP relay configuration".into()))?;

if let Some(tls) = custom_tls(config)? {
    builder = builder.tls(tls);
}
```

Do not call Lettre `dangerous_accept_*` methods and do not construct `Tls::None` or `Tls::Opportunistic`.

- [x] **Step 9: Run focused verification**

Run:

```bash
cargo test -p canopy-server --features pg config::tests::auth_email --locked
cargo test -p canopy-server --features pg identity::email::tests --locked
cargo clippy -p canopy-server --features pg --tests --locked -- -D warnings
git diff --check
```

Expected: all commands pass. Stop for a review checkpoint; do not stage or commit.

---

### Task 2: Compose Stack And Docker-Independent Contract

**Files:**
- Create: `crates/canopy-server/tests/local_integration_contract.rs`
- Create: `docker-compose.local-integration.yml`
- Create: `deploy/nginx/canopy-stream.local-integration.conf`

**Interfaces:**
- Consumes: generated `CANOPY_LOCAL_INTEGRATION_ROOT`, PostgreSQL password, and SMTP credentials.
- Produces: healthy Compose services `postgres`, `mailpit`, and `nginx`.
- Preserves: the client handoff's three public endpoints and keeps `55434`, `1025`, `8025`, and `18081` out of the machine-readable artifact.

- [x] **Step 1: Write the failing Compose contract test**

Create `local_integration_contract.rs`:

```rust
use serde_json::Value;

const COMPOSE: &str = include_str!("../../../docker-compose.local-integration.yml");
const HANDOFF: &str = include_str!("../../../deploy/client-connection.example.json");

#[test]
fn local_compose_is_scoped_tls_only_and_loopback_bound() {
    for required in [
        "postgres:18.4-alpine",
        "axllent/mailpit:v1.30.4",
        "nginx:stable-alpine",
        "127.0.0.1:55434:5432",
        "127.0.0.1:1025:1025",
        "127.0.0.1:8025:8025",
        "network_mode: host",
        "MP_SMTP_REQUIRE_STARTTLS",
        "MP_SMTP_TLS_CERT",
        "MP_SMTP_TLS_KEY",
        "MP_SMTP_AUTH",
        "canopy-stream.local-integration.conf",
        "restart: \"no\"",
        "no-new-privileges:true",
    ] {
        assert!(COMPOSE.contains(required), "local Compose is missing {required}");
    }
    assert!(!COMPOSE.contains("container_name:"));
    assert!(!COMPOSE.contains("MP_SMTP_AUTH_ALLOW_INSECURE"));
}

#[test]
fn operator_only_ports_do_not_enter_the_client_handoff() {
    let handoff: Value = serde_json::from_str(HANDOFF).unwrap();
    let serialized = serde_json::to_string(&handoff).unwrap();

    for operator_only in ["55434", "1025", "8025", "18081", "mailpit"] {
        assert!(
            !serialized.contains(operator_only),
            "client handoff exposes operator-only value {operator_only}"
        );
    }
    for public in [
        "http://127.0.0.1:50051",
        "http://127.0.0.1:8080",
        "http://127.0.0.1:8080/openapi.json",
    ] {
        assert!(serialized.contains(public));
    }
}
```

- [x] **Step 2: Verify the contract test fails**

Run:

```bash
cargo test -p canopy-server --test local_integration_contract --locked
```

Expected: compilation fails because `docker-compose.local-integration.yml` does not exist.

- [x] **Step 3: Create the local integration Compose model**

Create `docker-compose.local-integration.yml`:

```yaml
services:
  postgres:
    image: postgres:18.4-alpine
    restart: "no"
    environment:
      POSTGRES_USER: canopy_local
      POSTGRES_PASSWORD: ${CANOPY_LOCAL_POSTGRES_PASSWORD:?required}
      POSTGRES_DB: canopy_local
    ports:
      - "127.0.0.1:55434:5432"
    volumes:
      - postgres_data:/var/lib/postgresql
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U canopy_local -d canopy_local"]
      interval: 2s
      timeout: 3s
      retries: 30
      start_period: 3s
    security_opt:
      - no-new-privileges:true
    logging: &bounded-logging
      driver: json-file
      options:
        max-size: "5m"
        max-file: "2"
    networks: [local-integration]

  mailpit:
    image: axllent/mailpit:v1.30.4
    restart: "no"
    environment:
      MP_DISABLE_VERSION_CHECK: "true"
      MP_MAX_MESSAGES: "100"
      MP_SMTP_AUTH: ${CANOPY_LOCAL_SMTP_USERNAME:?required}:${CANOPY_LOCAL_SMTP_PASSWORD:?required}
      MP_SMTP_REQUIRE_STARTTLS: "true"
      MP_SMTP_TLS_CERT: /certs/server.crt
      MP_SMTP_TLS_KEY: /certs/server.key
    ports:
      - "127.0.0.1:1025:1025"
      - "127.0.0.1:8025:8025"
    volumes:
      - ${CANOPY_LOCAL_INTEGRATION_ROOT:?required}/certs:/certs:ro
    read_only: true
    tmpfs: [/tmp]
    healthcheck:
      test: ["CMD", "/mailpit", "readyz"]
      interval: 2s
      timeout: 3s
      retries: 30
      start_period: 3s
    security_opt:
      - no-new-privileges:true
    logging: *bounded-logging
    networks: [local-integration]

  nginx:
    image: nginx:stable-alpine
    restart: "no"
    network_mode: host
    volumes:
      - ./deploy/nginx/canopy-stream.local-integration.conf:/etc/nginx/conf.d/default.conf:ro
      - ./docs/openapi.json:/srv/canopy/docs/openapi.json:ro
      - ${CANOPY_LOCAL_INTEGRATION_ROOT:?required}/media/library:/srv/canopy/media/library:ro
    read_only: true
    tmpfs: [/var/cache/nginx, /var/run, /tmp]
    healthcheck:
      test: ["CMD", "wget", "--quiet", "--tries=1", "--spider", "http://127.0.0.1:8080/nginx-health"]
      interval: 2s
      timeout: 3s
      retries: 30
      start_period: 3s
    security_opt:
      - no-new-privileges:true
    logging: *bounded-logging

volumes:
  postgres_data:
    driver: local

networks:
  local-integration:
    driver: bridge
```

- [x] **Step 4: Validate Compose and run the contract**

Run:

```bash
CANOPY_LOCAL_INTEGRATION_ROOT="$PWD/target/local-integration" \
CANOPY_LOCAL_POSTGRES_PASSWORD=compose-validation-password \
CANOPY_LOCAL_SMTP_USERNAME=compose-validation-user \
CANOPY_LOCAL_SMTP_PASSWORD=compose-validation-password \
docker compose -p canopy-local-integration \
  -f docker-compose.local-integration.yml config --quiet
cargo test -p canopy-server --test local_integration_contract --locked
git diff --check
```

Expected: Compose validation and both contract tests pass. Stop for a review checkpoint; do not stage or commit.

---

### Task 3: Real-Boundary Rust Test Harness

**Files:**
- Create: `crates/canopy-server/tests/local_integration.rs`

**Interfaces:**
- Consumes: `CANOPY_LOCAL_GRPC_ENDPOINT`, `CANOPY_LOCAL_STREAM_ENDPOINT`, `CANOPY_LOCAL_OPENAPI_ENDPOINT`, and `CANOPY_LOCAL_MAILPIT_ENDPOINT`.
- Produces: ignored tests `local_environment_is_ready` and `local_environment_auth_and_playback_smoke`.
- Produces: sanitized helper errors that never include email bodies, tokens, or capability URLs.

- [x] **Step 1: Add parsing, channel, and authorization helpers**

Create `local_integration.rs`:

```rust
#![cfg(feature = "pg")]

use std::time::{Duration, Instant};

use canopy_proto::{
    GetStatusRequest, ListSessionsRequest, LoginPasswordRequest, LogoutRequest,
    RefreshSessionRequest, RegisterPasswordRequest, ResolvePlaybackRequest, SearchRequest,
    VerifyEmailRequest,
    auth_service_client::AuthServiceClient,
    catalog_service_client::CatalogServiceClient,
    playback_service_client::PlaybackServiceClient,
    system_service_client::SystemServiceClient,
};
use reqwest::{StatusCode, Url, header::RANGE};
use tokio::time::sleep;
use tonic::{
    Code, Request,
    metadata::MetadataValue,
    transport::{Channel, Endpoint},
};

const PASSWORD: &str = "Canopy-Local-Test-Password-42!";

fn endpoint(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

fn authorized<T>(message: T, access_token: &str) -> Request<T> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(format!("Bearer {access_token}"))
        .expect("access token must fit ASCII metadata");
    request.metadata_mut().insert("authorization", value);
    request
}

fn extract_verification_token(message: &str) -> Result<String, &'static str> {
    let action = message
        .lines()
        .filter_map(|line| Url::parse(line.trim()).ok())
        .find(|url| url.path().ends_with("/verify-email"))
        .ok_or("verification action URL is missing")?;
    action
        .query_pairs()
        .find_map(|(key, value)| (key == "token").then(|| value.into_owned()))
        .filter(|token| !token.is_empty())
        .ok_or("verification token is missing")
}

async fn channel() -> Result<Channel, &'static str> {
    let endpoint = endpoint("CANOPY_LOCAL_GRPC_ENDPOINT", "http://127.0.0.1:50051");
    Endpoint::from_shared(endpoint)
        .map_err(|_| "invalid local gRPC endpoint")?
        .connect()
        .await
        .map_err(|_| "local gRPC endpoint is unavailable")
}

#[test]
fn verification_token_parser_uses_the_action_url() {
    let body = "Verify your account\n\nhttp://127.0.0.1:3000/auth/verify-email?token=opaque%2Fvalue&expires_at=42\n";
    let token = extract_verification_token(body).unwrap();
    assert_eq!(token, "opaque/value");
}
```

- [x] **Step 2: Add bounded readiness and Mailpit polling**

Add:

```rust
async fn wait_for_ready() -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(channel) = channel().await {
            let mut system = SystemServiceClient::new(channel);
            if let Ok(response) = system.get_status(GetStatusRequest {}).await {
                let status = response.into_inner();
                let dependencies_ready = ["postgres", "media_library", "auth_email_delivery"]
                    .iter()
                    .all(|name| {
                        status
                            .dependencies
                            .iter()
                            .any(|dependency| {
                                dependency.name == *name && dependency.status == "healthy"
                            })
                    });
                if status.healthy && dependencies_ready {
                    let http = reqwest::Client::new();
                    let nginx = endpoint(
                        "CANOPY_LOCAL_STREAM_ENDPOINT",
                        "http://127.0.0.1:8080",
                    );
                    let openapi = endpoint(
                        "CANOPY_LOCAL_OPENAPI_ENDPOINT",
                        "http://127.0.0.1:8080/openapi.json",
                    );
                    let mailpit = endpoint(
                        "CANOPY_LOCAL_MAILPIT_ENDPOINT",
                        "http://127.0.0.1:8025",
                    );
                    let nginx_ready = http
                        .get(format!("{nginx}/nginx-health"))
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success());
                    let openapi_ready = http
                        .get(openapi)
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success());
                    let mailpit_ready = http
                        .get(format!("{mailpit}/readyz"))
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success());
                    if nginx_ready && openapi_ready && mailpit_ready {
                        return Ok(());
                    }
                }
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    Err("local integration environment did not become ready")
}

async fn wait_for_verification_token(email: &str) -> Result<String, &'static str> {
    let base = endpoint(
        "CANOPY_LOCAL_MAILPIT_ENDPOINT",
        "http://127.0.0.1:8025",
    );
    let mut url = Url::parse(&base)
        .map_err(|_| "invalid Mailpit endpoint")?
        .join("/view/latest.txt")
        .map_err(|_| "invalid Mailpit message endpoint")?;
    url.query_pairs_mut()
        .append_pair("query", &format!("to:{email}"));

    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(url.clone()).send().await
            && response.status().is_success()
            && let Ok(body) = response.text().await
            && let Ok(token) = extract_verification_token(&body)
        {
            return Ok(token);
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err("verification message did not arrive before timeout")
}
```

- [x] **Step 3: Add the ignored readiness test**

```rust
#[tokio::test]
#[ignore = "requires scripts/local-integration.sh"]
async fn local_environment_is_ready() {
    wait_for_ready()
        .await
        .expect("local integration readiness should succeed");
}
```

- [x] **Step 4: Add the complete authentication and playback smoke**

Add this test. Its assertions intentionally avoid formatting credentials or returned envelopes:

```rust
#[tokio::test]
#[ignore = "requires scripts/local-integration.sh"]
async fn local_environment_auth_and_playback_smoke() {
    wait_for_ready().await.expect("environment should be ready");
    let channel = channel().await.expect("gRPC should be reachable");
    let mut auth = AuthServiceClient::new(channel.clone());

    let email = format!("local-integration-{}@example.test", uuid::Uuid::new_v4());
    auth.register_password(RegisterPasswordRequest {
        email: email.clone(),
        password: PASSWORD.into(),
    })
    .await
    .expect("registration should be accepted");

    let verification_token = wait_for_verification_token(&email)
        .await
        .expect("verification delivery should succeed");
    let verified = auth
        .verify_email(VerifyEmailRequest {
            verification_token,
            device_label: "local-integration-verification".into(),
        })
        .await
        .expect("verification should succeed")
        .into_inner();
    assert!(
        !verified.access_token.is_empty() && !verified.refresh_token.is_empty(),
        "verification must return a complete session envelope"
    );

    auth.list_sessions(authorized(
        ListSessionsRequest { page: None },
        &verified.access_token,
    ))
    .await
    .expect("verified session should authorize protected calls");
    auth.logout(authorized(LogoutRequest {}, &verified.access_token))
        .await
        .expect("verification session logout should succeed");

    let logged_in = auth
        .login_password(LoginPasswordRequest {
            email,
            password: PASSWORD.into(),
            device_label: "local-integration-login".into(),
        })
        .await
        .expect("password login should succeed")
        .into_inner();
    let refreshed = auth
        .refresh_session(RefreshSessionRequest {
            refresh_token: logged_in.refresh_token.clone(),
        })
        .await
        .expect("single refresh should succeed")
        .into_inner();
    assert!(
        logged_in.refresh_token != refreshed.refresh_token,
        "refresh must rotate the opaque refresh token"
    );

    auth.logout(authorized(LogoutRequest {}, &refreshed.access_token))
        .await
        .expect("refreshed session logout should succeed");
    let rejected = auth
        .list_sessions(authorized(
            ListSessionsRequest { page: None },
            &refreshed.access_token,
        ))
        .await
        .expect_err("logged-out session must be rejected");
    assert_eq!(rejected.code(), Code::Unauthenticated);

    let mut catalog = CatalogServiceClient::new(channel.clone());
    let search = catalog
        .search(SearchRequest {
            query: "Moonlight Sonata".into(),
            page: None,
        })
        .await
        .expect("seeded catalog search should succeed")
        .into_inner();
    let track = search
        .tracks
        .into_iter()
        .find(|track| track.title == "Moonlight Sonata")
        .expect("seeded track should be discoverable");

    let mut playback = PlaybackServiceClient::new(channel);
    let source = playback
        .resolve_playback(ResolvePlaybackRequest { track_id: track.id })
        .await
        .expect("playback resolution should succeed")
        .into_inner();
    assert!(
        source.stream_url.starts_with("http://127.0.0.1:8080/stream/"),
        "playback must use the local Nginx stream origin"
    );

    let response = reqwest::Client::new()
        .get(source.stream_url)
        .header(RANGE, "bytes=0-15")
        .send()
        .await
        .map_err(|_| "stream range request failed")
        .expect("stream range request should succeed");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "stream body read failed")
        .expect("stream body should be readable");
    assert!(!bytes.is_empty(), "stream range response must contain bytes");
}
```

Do not add an assertion that formats either envelope with `Debug`, and do not replay `logged_in.refresh_token`.

- [x] **Step 5: Compile the harness and capture the red integration result**

Run:

```bash
cargo test -p canopy-server --features pg --test local_integration --locked
```

Expected: the parser test passes and the two real-boundary tests are ignored.

Run:

```bash
cargo test -p canopy-server --features pg --test local_integration \
  local_environment_is_ready --locked -- \
  --ignored --exact --test-threads=1
```

Expected: FAIL after the bounded timeout because no managed environment is running. This is the environment-level red phase.

Stop for a review checkpoint; do not stage or commit.

---

### Task 4: Scoped Local Lifecycle

**Files:**
- Modify: `crates/canopy-server/tests/local_integration_contract.rs`
- Create: `scripts/local-integration.sh`

**Interfaces:**
- Produces: `scripts/local-integration.sh up|test|status|down`.
- Produces: mode-`0600` `target/local-integration/runtime.env`, verified `canopy.pid`, `canopy.log`, generated certificates, and seeded media.
- Consumes: the Compose stack and both ignored Rust tests.

**Verified implementation outcome (2026-07-15):** The tracked script and
configuration files are canonical. Real-boundary testing required several
hardening changes beyond the initial snippets below: Cargo's bin directory is
added to `PATH`; generated state starts under `umask 077`; only generated
public media receives read/traverse permission; Canopy is copied to an
immutable state-root executable and started with `nohup`; dead recorded PIDs
are recovered while malformed or live mismatches fail closed; migrations use
`DATABASE_URL` without putting credentials in arguments; startup applies
`fixtures/local-integration.sql` after provider ingest; and diagnostics never
log the database URL. The snippets retain the TDD sequence, while
`scripts/local-integration.sh` defines final behavior.

- [x] **Step 1: Extend the contract with shell behavior**

Add:

```rust
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

const SCRIPT: &str = include_str!("../../../scripts/local-integration.sh");

#[test]
fn lifecycle_script_exposes_only_the_supported_commands() {
    for command in ["up", "test", "status", "down"] {
        assert!(
            SCRIPT.contains(&format!("{command})")),
            "lifecycle dispatch is missing {command}"
        );
    }
    for required in [
        "canopy-local-integration",
        "target/local-integration",
        "runtime.env",
        "canopy.pid",
        "CANOPY_SMTP_CA_CERT_PATH",
        "local_environment_is_ready",
        "local_environment_auth_and_playback_smoke",
    ] {
        assert!(SCRIPT.contains(required), "lifecycle script is missing {required}");
    }
}

#[test]
fn lifecycle_script_is_executable_and_rejects_unknown_commands_without_mutation() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts/local-integration.sh");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "lifecycle script must remain executable");

    let output = Command::new("bash")
        .arg(&path)
        .arg("unknown-command")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("up|test|status|down"));
}
```

- [x] **Step 2: Verify the shell contract fails**

Run:

```bash
cargo test -p canopy-server --test local_integration_contract --locked
```

Expected: compilation fails because `scripts/local-integration.sh` does not exist.

- [x] **Step 3: Implement paths, dispatch helpers, and ownership checks**

Create the executable script with this foundation:

```bash
#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_file="$repo_root/docker-compose.local-integration.yml"
compose_project="canopy-local-integration"
state_root="$repo_root/target/local-integration"
runtime_env="$state_root/runtime.env"
pid_file="$state_root/canopy.pid"
canopy_log="$state_root/canopy.log"
canopy_binary="$repo_root/target/debug/canopy"
cleanup_on_exit=0

grpc_endpoint="http://127.0.0.1:50051"
stream_endpoint="http://127.0.0.1:8080"
openapi_endpoint="http://127.0.0.1:8080/openapi.json"
mailpit_endpoint="http://127.0.0.1:8025"

usage() {
  echo "usage: ./scripts/local-integration.sh up|test|status|down" >&2
}

die() {
  echo "local integration: $1" >&2
  exit "${2:-1}"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

validate_prerequisites() {
  local command
  for command in docker openssl cargo curl sqlx ss awk readlink; do
    require_command "$command"
  done
  docker compose version >/dev/null
  openssl version >/dev/null
  cargo --version >/dev/null
  sqlx --version >/dev/null
}

compose_project_ids() {
  docker ps --all --filter "label=com.docker.compose.project=$compose_project" --quiet
}

read_recorded_pid() {
  [[ -f "$pid_file" ]] || return 1
  read -r recorded_pid recorded_start <"$pid_file"
  [[ "$recorded_pid" =~ ^[0-9]+$ && "$recorded_start" =~ ^[0-9]+$ ]]
}

recorded_pid_is_owned() {
  read_recorded_pid || return 1
  [[ -r "/proc/$recorded_pid/stat" ]] || return 1
  [[ "$(awk '{print $22}' "/proc/$recorded_pid/stat")" == "$recorded_start" ]] || return 1
  [[ "$(readlink -f "/proc/$recorded_pid/exe")" == "$canopy_binary" ]]
}

environment_active() {
  if [[ -f "$pid_file" ]]; then
    recorded_pid_is_owned || die "recorded Canopy PID is stale; refusing mutation"
    return 0
  fi
  [[ -n "$(compose_project_ids)" ]]
}

state_present() {
  [[ -e "$runtime_env" || -e "$pid_file" ]] || [[ -n "$(compose_project_ids)" ]]
}

assert_ports_available() {
  local port
  for port in 50051 18081 55434 1025 8025 8080; do
    if ss -H -ltn "sport = :$port" | grep -q .; then
      die "port $port is already in use"
    fi
  done
}
```

The top-level `case` must reject unknown or missing commands before
`validate_prerequisites` or any filesystem mutation.

- [x] **Step 4: Generate runtime state and verified TLS material**

Implement:

```bash
write_env() {
  printf '%s=%q\n' "$1" "$2"
}

prepare_state() {
  [[ "$state_root" == "$repo_root/target/local-integration" ]] \
    || die "refusing unsafe state path"
  rm -rf -- "$state_root"
  mkdir -p \
    "$state_root/certs" \
    "$state_root/media/library/audio/tracks/musopen"
  umask 077

  openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 2 \
    -subj "/CN=Canopy Local Integration CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -keyout "$state_root/certs/ca.key" \
    -out "$state_root/certs/ca.crt" >/dev/null 2>&1
  openssl req -new -newkey rsa:2048 -nodes -sha256 \
    -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
    -keyout "$state_root/certs/server.key" \
    -out "$state_root/certs/server.csr" >/dev/null 2>&1
  openssl x509 -req -sha256 -days 2 \
    -in "$state_root/certs/server.csr" \
    -CA "$state_root/certs/ca.crt" \
    -CAkey "$state_root/certs/ca.key" \
    -CAcreateserial -copy_extensions copy \
    -out "$state_root/certs/server.crt" >/dev/null 2>&1

  cp "$repo_root/fixtures/media/test-tone.mp3" \
    "$state_root/media/library/audio/tracks/musopen/beethoven-moonlight-sonata.mp3"

  local postgres_password smtp_password signing_key outbox_key stream_secret
  postgres_password="$(openssl rand -hex 24)"
  smtp_password="$(openssl rand -hex 24)"
  signing_key="$(openssl rand -base64 32 | tr -d '\n')"
  outbox_key="$(openssl rand -base64 32 | tr -d '\n')"
  stream_secret="$(openssl rand -hex 32)"

  {
    write_env CANOPY_LOCAL_INTEGRATION_ROOT "$state_root"
    write_env CANOPY_LOCAL_POSTGRES_PASSWORD "$postgres_password"
    write_env CANOPY_LOCAL_SMTP_USERNAME "canopy-local"
    write_env CANOPY_LOCAL_SMTP_PASSWORD "$smtp_password"
    write_env CANOPY_DATABASE_URL "postgres://canopy_local:$postgres_password@127.0.0.1:55434/canopy_local"
    write_env CANOPY_GRPC_ADDR "127.0.0.1:50051"
    write_env CANOPY_MEDIA_ROOT "$state_root/media"
    write_env CANOPY_PROVIDER_FIXTURE_PATH "$repo_root/fixtures/catalog.json"
    write_env CANOPY_STREAM_PUBLIC_BASE_URL "$stream_endpoint"
    write_env CANOPY_STREAM_AUTH_ADDR "127.0.0.1:18081"
    write_env CANOPY_STREAM_TOKEN_SECRET "$stream_secret"
    write_env CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64 "$signing_key"
    write_env CANOPY_AUTH_TOKEN_SECRET "$(openssl rand -hex 32)"
    write_env CANOPY_AUTH_OUTBOX_SEALING_KEY "$outbox_key"
    write_env CANOPY_SMTP_HOST "localhost"
    write_env CANOPY_SMTP_PORT "1025"
    write_env CANOPY_SMTP_TLS_MODE "starttls"
    write_env CANOPY_SMTP_USERNAME "canopy-local"
    write_env CANOPY_SMTP_PASSWORD "$smtp_password"
    write_env CANOPY_SMTP_FROM_ADDRESS "auth@canopy.local.test"
    write_env CANOPY_SMTP_FROM_NAME "Canopy"
    write_env CANOPY_AUTH_PUBLIC_BASE_URL "http://127.0.0.1:3000/auth/"
    write_env CANOPY_SMTP_TIMEOUT_SECS "10"
    write_env CANOPY_AUTH_EMAIL_POLL_INTERVAL_SECS "1"
    write_env CANOPY_AUTH_EMAIL_LEASE_SECS "15"
    write_env CANOPY_SMTP_CA_CERT_PATH "$state_root/certs/ca.crt"
    write_env CANOPY_LOCAL_GRPC_ENDPOINT "$grpc_endpoint"
    write_env CANOPY_LOCAL_STREAM_ENDPOINT "$stream_endpoint"
    write_env CANOPY_LOCAL_OPENAPI_ENDPOINT "$openapi_endpoint"
    write_env CANOPY_LOCAL_MAILPIT_ENDPOINT "$mailpit_endpoint"
  } >"$runtime_env"
  chmod 0600 "$runtime_env"
}

load_runtime() {
  [[ -f "$runtime_env" ]] || die "runtime state is missing"
  set -a
  # shellcheck disable=SC1090
  source "$runtime_env"
  set +a
}

compose() {
  docker compose --env-file "$runtime_env" \
    -p "$compose_project" -f "$compose_file" "$@"
}
```

All generated values are shell-escaped by `%q`; no command prints
`runtime.env` or starts Canopy with secrets in command-line arguments.

- [x] **Step 5: Implement process-safe stop and cleanup primitives**

Implement `stop_canopy`:

```bash
stop_canopy() {
  [[ -f "$pid_file" ]] || return 0
  recorded_pid_is_owned || {
    echo "local integration: recorded Canopy PID is stale; refusing mutation" >&2
    return 1
  }

  kill -TERM "$recorded_pid"
  local attempt state
  for attempt in {1..100}; do
    if [[ ! -r "/proc/$recorded_pid/stat" ]]; then
      wait "$recorded_pid" 2>/dev/null || true
      rm -f -- "$pid_file"
      return 0
    fi
    state="$(awk '{print $3}' "/proc/$recorded_pid/stat")"
    if [[ "$state" == "Z" ]]; then
      wait "$recorded_pid" 2>/dev/null || true
      rm -f -- "$pid_file"
      return 0
    fi
    sleep 0.1
  done

  recorded_pid_is_owned || return 1
  kill -KILL "$recorded_pid"
  wait "$recorded_pid" 2>/dev/null || true
  rm -f -- "$pid_file"
}

show_diagnostics() {
  [[ ! -f "$canopy_log" ]] || tail -n 100 "$canopy_log"
  if [[ -f "$runtime_env" ]]; then
    load_runtime
    compose ps
    compose logs --no-color postgres nginx
  fi
}

cleanup_managed() {
  local cleanup_status=0
  if [[ -f "$pid_file" ]]; then
    stop_canopy || cleanup_status=1
  fi
  if [[ -f "$runtime_env" ]]; then
    load_runtime
    compose down --volumes --remove-orphans || cleanup_status=1
  fi
  if [[ "$cleanup_status" == "0" ]]; then
    [[ "$state_root" == "$repo_root/target/local-integration" ]] \
      || die "refusing unsafe state cleanup"
    rm -rf -- "$state_root"
  fi
  return "$cleanup_status"
}

on_exit() {
  local status=$?
  local cleanup_status=0
  trap - EXIT
  set +e
  if [[ "$cleanup_on_exit" == "1" ]]; then
    [[ "$status" == "0" ]] || show_diagnostics
    cleanup_managed || cleanup_status=$?
    if [[ "$status" == "0" && "$cleanup_status" != "0" ]]; then
      status="$cleanup_status"
    fi
  fi
  exit "$status"
}
trap on_exit EXIT
```

This is the only place a force signal is allowed, and only after re-verifying
the recorded PID, start time, and executable path.
- [x] **Step 6: Start dependencies, migrate, build, and wait for readiness**

Implement:

```bash
start_environment() {
  environment_active && die "local integration environment is already active"
  assert_ports_available
  prepare_state
  load_runtime

  compose config --quiet
  compose up -d --wait postgres mailpit nginx
  sqlx migrate run --source "$repo_root/migrations" \
    --database-url "$CANOPY_DATABASE_URL"
  cargo build -p canopy-server --features pg --bin canopy --locked

  "$canopy_binary" >>"$canopy_log" 2>&1 &
  local pid=$!
  local start
  start="$(awk '{print $22}' "/proc/$pid/stat")"
  printf '%s %s\n' "$pid" "$start" >"$pid_file"

  cargo test -p canopy-server --features pg --test local_integration \
    local_environment_is_ready --locked -- \
    --ignored --exact --test-threads=1
}
```

`start_environment` must be called only after `validate_prerequisites`.
PostgreSQL migrations stay an explicit script action; Canopy startup is not
modified to run them.

- [x] **Step 7: Implement exact command behavior**

Dispatch only after validating the single command argument:

```bash
case "${1:-}" in
  up)
    validate_prerequisites
    cleanup_on_exit=1
    start_environment
    cleanup_on_exit=0
    printf '%s\n' \
      "gRPC: $grpc_endpoint" \
      "Streaming: $stream_endpoint" \
      "OpenAPI: $openapi_endpoint" \
      "Test inbox: $mailpit_endpoint"
    ;;
  test)
    validate_prerequisites
    environment_active && die "stop the interactive environment before test"
    cleanup_on_exit=1
    start_environment
    cargo test -p canopy-server --features pg --test local_integration \
      local_environment_auth_and_playback_smoke --locked -- \
      --ignored --exact --test-threads=1
    ;;
  status)
    require_command docker
    if ! state_present; then
      echo "local integration: stopped"
      exit 0
    fi
    if ! environment_active; then
      echo "local integration: stopped (stale state present)"
      exit 0
    fi
    [[ -f "$runtime_env" ]] || die "runtime state is incomplete"
    load_runtime
    echo "Canopy: running"
    compose ps
    printf '%s\n' \
      "gRPC: $grpc_endpoint" \
      "Streaming: $stream_endpoint" \
      "OpenAPI: $openapi_endpoint" \
      "Test inbox: $mailpit_endpoint"
    ;;
  down)
    require_command docker
    if ! state_present; then
      echo "local integration: already stopped"
      exit 0
    fi
    [[ -f "$runtime_env" ]] \
      || die "runtime state is incomplete; refusing mutation"
    [[ ! -f "$pid_file" ]] || recorded_pid_is_owned \
      || die "recorded Canopy PID is stale; refusing mutation"
    cleanup_managed
    ;;
  *)
    usage
    exit 2
    ;;
esac
```

`test` deliberately refuses an already-running interactive environment so it
always owns a clean database, inbox, process, and cleanup.

- [x] **Step 8: Set executable mode and run static checks**

Run:

```bash
chmod 0755 scripts/local-integration.sh
bash -n scripts/local-integration.sh
cargo test -p canopy-server --test local_integration_contract --locked
git diff --check
```

Expected: Bash syntax and contract tests pass, including executable-mode
verification.

- [x] **Step 9: Verify the interactive lifecycle**

Run:

```bash
./scripts/local-integration.sh up
./scripts/local-integration.sh status
./scripts/local-integration.sh down
./scripts/local-integration.sh status
```

Expected:
- `up` waits for healthy PostgreSQL, media, email, Nginx, OpenAPI, and Mailpit.
- first `status` reports Canopy and all three Compose services without secrets.
- `down` stops only the verified Canopy process and dedicated Compose project.
- final `status` reports `local integration: stopped`.

- [x] **Step 10: Verify the complete clean smoke run**

Run:

```bash
./scripts/local-integration.sh test
```

Expected: readiness, registration, delivered verification, verification,
protected access, logout, password login, refresh rotation, second logout,
post-logout rejection, catalog search, playback resolution, and the HTTP `206`
range request all pass. Cleanup runs on success and failure.

Run:

```bash
docker ps --filter label=com.docker.compose.project=canopy-local-integration --quiet
test ! -e target/local-integration
```

Expected: no container IDs and a zero exit status. Stop for a review checkpoint;
do not stage or commit.

---

### Task 5: Documentation And Final Verification

**Files:**
- Modify: `.env.example:21`
- Modify: `README.md:688`
- Modify: `README.md:740`
- Modify: `docs/client-integration.md`

**Interfaces:**
- Produces: one canonical backend-local command reference.
- Preserves: the client handoff as secret-free, client-neutral, and limited to public surfaces.

- [x] **Step 1: Write a failing documentation contract**

Extend `local_integration_contract.rs`:

```rust
#[test]
fn local_integration_documentation_tracks_the_runtime() {
    let readme = include_str!("../../../README.md");
    let handoff = include_str!("../../../docs/client-integration.md");
    let env_example = include_str!("../../../.env.example");

    for required in [
        "./scripts/local-integration.sh up",
        "./scripts/local-integration.sh test",
        "./scripts/local-integration.sh status",
        "./scripts/local-integration.sh down",
        "http://127.0.0.1:8025",
        "CANOPY_SMTP_CA_CERT_PATH",
    ] {
        assert!(readme.contains(required), "README is missing {required}");
    }
    assert!(env_example.contains("CANOPY_SMTP_CA_CERT_PATH"));
    assert!(handoff.contains("./scripts/local-integration.sh up"));
    assert!(handoff.contains("http://127.0.0.1:50051"));
    assert!(handoff.contains("http://127.0.0.1:8080"));
    assert!(!handoff.contains("CANOPY_LOCAL_POSTGRES_PASSWORD"));
    assert!(!handoff.contains("CANOPY_LOCAL_SMTP_PASSWORD"));
}
```

- [x] **Step 2: Verify the documentation contract fails**

Run:

```bash
cargo test -p canopy-server --test local_integration_contract --locked
```

Expected: `local_integration_documentation_tracks_the_runtime` fails.

- [x] **Step 3: Document the optional SMTP trust root**

Add beside the SMTP TLS variables in `.env.example`:

```dotenv
# Optional PEM CA bundle for a private or local SMTP relay. Verification remains enabled.
# CANOPY_SMTP_CA_CERT_PATH=/absolute/path/to/smtp-ca.pem
```

Add this README environment-table row:

```markdown
| `CANOPY_SMTP_CA_CERT_PATH` | unset | Optional readable PEM CA bundle added to verified SMTP TLS trust; it does not disable hostname or certificate validation |
```

- [x] **Step 4: Add the backend-local lifecycle guide**

Add this README section using the repository's existing running-server style:

````markdown
### Complete local integration environment

The backend-owned local reference runs Canopy in WSL and PostgreSQL, Mailpit,
and Nginx in the scoped `canopy-local-integration` Compose project. Install
Docker Compose v2, OpenSSL, the repository Rust toolchain, `curl`, and
`sqlx-cli`, then run:

```bash
./scripts/local-integration.sh up
./scripts/local-integration.sh status
./scripts/local-integration.sh down
```

The developer endpoints are gRPC at `http://127.0.0.1:50051`, streaming at
`http://127.0.0.1:8080`, OpenAPI at
`http://127.0.0.1:8080/openapi.json`, and the operator-only Mailpit inbox at
`http://127.0.0.1:8025`.

Run the clean end-to-end smoke flow with:

```bash
./scripts/local-integration.sh test
```

This environment is loopback-only and disposable. It is not a production
deployment and does not expose Mailpit, PostgreSQL, or the private stream
authorization listener to clients.
````

- [x] **Step 5: Connect the client handoff to the runnable reference**

Add a short `Local Reference Environment` section to
`docs/client-integration.md`. It must say:

- Backend developers start the documented public endpoints with
  `./scripts/local-integration.sh up`.
- `./scripts/local-integration.sh test` runs the complete backend smoke.
- Clients receive only the gRPC, streaming, and OpenAPI values already present
  in `deploy/client-connection.example.json`.
- Mailpit, PostgreSQL, SMTP credentials, generated secrets, and
  `127.0.0.1:18081` remain operator-only.

Do not add Mailpit or database fields to the JSON handoff.

- [x] **Step 6: Run focused documentation and runtime verification**

Run:

```bash
cargo test -p canopy-server --test local_integration_contract --locked
cargo test -p canopy-server --test client_handoff --locked
cargo test -p canopy-server --test http_openapi --locked
./scripts/local-integration.sh test
```

Expected: documentation, handoff, OpenAPI, and complete real-boundary checks pass.

- [x] **Step 7: Run repository-wide final gates**

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-features --tests --locked -- -D warnings
CANOPY_LOCAL_INTEGRATION_ROOT="$PWD/target/local-integration" \
CANOPY_LOCAL_POSTGRES_PASSWORD=compose-validation-password \
CANOPY_LOCAL_SMTP_USERNAME=compose-validation-user \
CANOPY_LOCAL_SMTP_PASSWORD=compose-validation-password \
docker compose -p canopy-local-integration \
  -f docker-compose.local-integration.yml config --quiet
git diff --check
```

Expected: every command passes with no warnings. Confirm
`scripts/local-integration.sh` remains mode `100755`, inspect the diff for
secret values, and leave all staging and commit decisions to the user.

Verified on 2026-07-15: formatting, the locked workspace test suite,
all-feature Clippy with warnings denied, resolved Compose validation, the
complete Docker-backed smoke flow, and `git diff --check` passed. The script
remained mode `100755`; teardown left no dedicated containers or
target/local-integration/` state; and the user retained all Git ownership.