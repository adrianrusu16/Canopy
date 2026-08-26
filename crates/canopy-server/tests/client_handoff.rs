use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

const EXPECTED_BSR_MODULE: &str = "buf.build/pandawave/canopy-api";
const EXPECTED_RELEASE: &str = "v0.3.0";
const EXPECTED_COMMIT: &str = "ff8940d1a15b4034bb430fd47dd45cdc";
const EXPECTED_PROST_PACKAGE: &str = "pandawave_canopy-api_community_neoeinstein-prost";
const EXPECTED_PROST_VERSION: &str = "=0.5.0-00000000000000-ff8940d1a15b.2";
const EXPECTED_TONIC_PACKAGE: &str = "pandawave_canopy-api_community_neoeinstein-tonic";
const EXPECTED_TONIC_VERSION: &str = "=0.5.0-00000000000000-ff8940d1a15b.4";

const FORBIDDEN_KEYS: &[&str] = &[
    "database_url",
    "smtp_host",
    "smtp_port",
    "smtp_username",
    "smtp_password",
    "identity_access_token_signing_key",
    "auth_outbox_sealing_key",
    "stream_token_secret",
    "stream_auth_addr",
    "private_authorization_url",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientConnectionReference {
    schema_version: u32,
    environment: String,
    contract: ContractReference,
    transport: TransportReference,
    authentication: AuthenticationReference,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractReference {
    protobuf_package: String,
    bsr_module: String,
    release: String,
    commit: String,
    prost_package: String,
    prost_version: String,
    tonic_package: String,
    tonic_version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransportReference {
    grpc_endpoint: String,
    stream_base_url: String,
    openapi_url: String,
    tls_required_outside_loopback: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationReference {
    metadata_key: String,
    metadata_scheme: String,
    verification_action_relative_path: String,
    verification_token_query_parameter: String,
    password_reset_action_relative_path: String,
    password_reset_token_query_parameter: String,
    expiry_query_parameter: String,
    auth_service_requires_postgresql: bool,
    password_bootstrap_requires_email_delivery: bool,
}

fn load_reference() -> ClientConnectionReference {
    serde_json::from_str(include_str!(
        "../../../deploy/client-connection.example.json"
    ))
    .expect("client connection reference must be valid schema-versioned JSON")
}

fn assert_no_forbidden_keys(value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                assert!(
                    !FORBIDDEN_KEYS.contains(&key.as_str()),
                    "client connection reference exposes forbidden key {key}"
                );
                assert_no_forbidden_keys(child);
            }
        }
        Value::Array(array) => {
            for child in array {
                assert_no_forbidden_keys(child);
            }
        }
        _ => {}
    }
}

fn assert_public_url(url: &Url) {
    assert!(matches!(url.scheme(), "http" | "https"));
    if url.scheme() == "http" {
        assert!(
            matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
            "plaintext client URL must be loopback-only: {url}"
        );
    }
}

#[test]
fn client_connection_reference_is_public_versioned_and_consistent() {
    let raw = include_str!("../../../deploy/client-connection.example.json");
    let raw_value: Value = serde_json::from_str(raw).expect("reference JSON must parse");
    assert_no_forbidden_keys(&raw_value);

    let reference = load_reference();
    assert_eq!(reference.schema_version, 1);
    assert_eq!(reference.environment, "local-reference");

    assert_eq!(reference.contract.protobuf_package, "canopy.v1");
    assert_eq!(reference.contract.bsr_module, EXPECTED_BSR_MODULE);
    assert_eq!(reference.contract.release, EXPECTED_RELEASE);
    assert_eq!(reference.contract.commit, EXPECTED_COMMIT);
    assert_eq!(reference.contract.prost_package, EXPECTED_PROST_PACKAGE);
    assert_eq!(reference.contract.prost_version, EXPECTED_PROST_VERSION);
    assert_eq!(reference.contract.tonic_package, EXPECTED_TONIC_PACKAGE);
    assert_eq!(reference.contract.tonic_version, EXPECTED_TONIC_VERSION);

    let proto_manifest = include_str!("../../canopy-proto/Cargo.toml");
    assert!(proto_manifest.contains(&format!(
        "package = \"{}\", version = \"{}\"",
        EXPECTED_PROST_PACKAGE, EXPECTED_PROST_VERSION
    )));
    assert!(proto_manifest.contains(&format!(
        "package = \"{}\", version = \"{}\"",
        EXPECTED_TONIC_PACKAGE, EXPECTED_TONIC_VERSION
    )));

    let grpc = Url::parse(&reference.transport.grpc_endpoint)
        .expect("gRPC endpoint must be an absolute URL");
    let stream =
        Url::parse(&reference.transport.stream_base_url).expect("stream base URL must be absolute");
    let openapi =
        Url::parse(&reference.transport.openapi_url).expect("OpenAPI URL must be absolute");
    assert_public_url(&grpc);
    assert_public_url(&stream);
    assert_public_url(&openapi);
    assert!(reference.transport.tls_required_outside_loopback);
    assert_eq!(stream.scheme(), openapi.scheme());
    assert_eq!(stream.host_str(), openapi.host_str());
    assert_eq!(
        stream.port_or_known_default(),
        openapi.port_or_known_default()
    );
    assert_eq!(openapi.path(), "/openapi.json");

    let openapi_document: Value = serde_json::from_str(include_str!("../../../docs/openapi.json"))
        .expect("docs/openapi.json must remain valid JSON");
    assert!(openapi_document["paths"].get(openapi.path()).is_some());

    assert_eq!(reference.authentication.metadata_key, "authorization");
    assert_eq!(reference.authentication.metadata_scheme, "Bearer");
    assert_eq!(
        reference.authentication.verification_action_relative_path,
        "verify-email"
    );
    assert_eq!(
        reference.authentication.verification_token_query_parameter,
        "token"
    );
    assert_eq!(
        reference.authentication.password_reset_action_relative_path,
        "reset-password"
    );
    assert_eq!(
        reference
            .authentication
            .password_reset_token_query_parameter,
        "token"
    );
    assert_eq!(
        reference.authentication.expiry_query_parameter,
        "expires_at"
    );
    assert!(reference.authentication.auth_service_requires_postgresql);
    assert!(
        reference
            .authentication
            .password_bootstrap_requires_email_delivery
    );
}

#[test]
fn client_integration_docs_track_the_reference_contract() {
    let reference = load_reference();
    let handoff = include_str!("../../../docs/client-integration.md");
    let server_consumption = include_str!("../../../docs/api.md");

    for expected in [
        reference.contract.bsr_module.as_str(),
        reference.contract.release.as_str(),
        reference.contract.commit.as_str(),
        reference.contract.prost_version.as_str(),
        reference.contract.tonic_version.as_str(),
        reference.transport.grpc_endpoint.as_str(),
        reference.transport.stream_base_url.as_str(),
        reference.transport.openapi_url.as_str(),
    ] {
        assert!(
            handoff.contains(expected),
            "client integration handoff is missing {expected}"
        );
    }

    for required in [
        "CANOPY_GRPC_ADDR=127.0.0.1:50051",
        "CANOPY_AUTH_PUBLIC_BASE_URL",
        "verify-email",
        "reset-password",
        "SessionEnvelope",
        "authorization",
        "CANOPY_STREAM_AUTH_ADDR",
        "PostgreSQL",
        "SMTP",
    ] {
        assert!(
            handoff.contains(required),
            "client integration handoff is missing required guidance: {required}"
        );
    }

    assert!(server_consumption.contains("[Client Integration Handoff](client-integration.md)"));
}
#[test]
fn readme_links_the_client_integration_handoff_from_relevant_sections() {
    let readme = include_str!("../../../README.md");
    let link = "[Client Integration Handoff](docs/client-integration.md)";
    assert_eq!(
        readme.matches(link).count(),
        2,
        "README must link the handoff from API verification and server startup"
    );
}
