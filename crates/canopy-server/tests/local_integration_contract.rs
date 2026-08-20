use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use serde_json::Value;

const COMPOSE: &str = include_str!("../../../docker-compose.local-integration.yml");
const HANDOFF: &str = include_str!("../../../deploy/client-connection.example.json");
const SCRIPT: &str = include_str!("../../../scripts/local-integration.sh");
const SERVER_WIRING: &str = include_str!("../src/lib.rs");
const LOCAL_SEED_SQL: &str = include_str!("../../../fixtures/local-integration.sql");
const PRODUCTION_NGINX: &str = include_str!("../../../deploy/nginx/canopy-stream.conf");
const LOCAL_NGINX: &str =
    include_str!("../../../deploy/nginx/canopy-stream.local-integration.conf");

const README: &str = include_str!("../../../README.md");
const CLIENT_INTEGRATION: &str = include_str!("../../../docs/client-integration.md");
const ENV_EXAMPLE: &str = include_str!("../../../.env.example");

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
        "canopy-stream.local-integration.conf",
        "MP_SMTP_REQUIRE_STARTTLS",
        "MP_SMTP_TLS_CERT",
        "MP_SMTP_TLS_KEY",
        "MP_SMTP_AUTH",
        "restart: \"no\"",
        "no-new-privileges:true",
    ] {
        assert!(
            COMPOSE.contains(required),
            "local Compose is missing {required}"
        );
    }
    assert!(!COMPOSE.contains("container_name:"));
    assert!(!COMPOSE.contains("MP_SMTP_AUTH_ALLOW_INSECURE"));
    assert!(!COMPOSE.contains("host.docker.internal"));
}

#[test]
fn local_nginx_exposes_streaming_to_the_emulator_and_keeps_authorization_private() {
    assert!(LOCAL_NGINX.contains("listen 0.0.0.0:8080"));
    assert!(LOCAL_NGINX.contains("proxy_pass http://127.0.0.1:18081"));
}

#[test]
fn nginx_forwards_the_original_stream_uri_to_the_private_authorizer() {
    for config in [PRODUCTION_NGINX, LOCAL_NGINX] {
        assert!(config.contains("proxy_set_header X-Canopy-Original-URI $request_uri;"));
        assert!(!config.contains("map $request_uri"));
        assert!(!config.contains("X-Canopy-Stream-Token"));
    }
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
        "nohup",
        "built_canopy_binary",
    ] {
        assert!(
            SCRIPT.contains(required),
            "lifecycle script is missing {required}"
        );
    }
}

#[test]
fn lifecycle_refuses_to_replace_an_orphaned_postgres_volume() {
    for required in [
        "compose_volume_exists()",
        "orphaned PostgreSQL volume",
        "runtime state is missing",
    ] {
        assert!(
            SCRIPT.contains(required),
            "lifecycle script is missing orphaned-volume protection: {required}"
        );
    }
}

#[test]
fn generated_media_is_readable_by_unprivileged_nginx_workers() {
    assert!(SCRIPT.contains("chmod -R a+rX \"$state_root/media\""));
}

#[test]
fn local_integration_documentation_tracks_the_runtime() {
    for required in [
        "./scripts/local-integration.sh up",
        "./scripts/local-integration.sh test",
        "./scripts/local-integration.sh status",
        "./scripts/local-integration.sh down",
        "http://127.0.0.1:50051",
        "http://127.0.0.1:8080",
        "http://127.0.0.1:8080/openapi.json",
        "http://127.0.0.1:8025",
        "CANOPY_SMTP_CA_CERT_PATH",
    ] {
        assert!(README.contains(required), "README is missing {required}");
    }
    assert!(ENV_EXAMPLE.contains("CANOPY_SMTP_CA_CERT_PATH"));
    assert!(CLIENT_INTEGRATION.contains("## Local Reference Environment"));
    assert!(CLIENT_INTEGRATION.contains("./scripts/local-integration.sh up"));
    assert!(CLIENT_INTEGRATION.contains("http://127.0.0.1:50051"));
    assert!(CLIENT_INTEGRATION.contains("http://127.0.0.1:8080"));
    assert!(!CLIENT_INTEGRATION.contains("CANOPY_LOCAL_POSTGRES_PASSWORD"));
    assert!(!CLIENT_INTEGRATION.contains("CANOPY_LOCAL_SMTP_PASSWORD"));
}

#[test]
fn startup_diagnostics_do_not_log_the_database_url() {
    assert!(!SERVER_WIRING.contains("database_url = %config.database_url"));
}

#[test]
fn local_playback_seed_publishes_only_the_designated_fixture_track() {
    for required in [
        "provider_track_id = 'fixture-001'",
        "review_status = 'approved'",
        "ingest_status = 'ready'",
        "visibility = 'release_safe'",
    ] {
        assert!(LOCAL_SEED_SQL.contains(required));
    }
    assert!(!LOCAL_SEED_SQL.contains("fixture-002"));
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
