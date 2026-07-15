//! Runtime configuration for the Canopy server.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use rustls_pki_types::pem::PemObject;

/// Configuration for capability issuance and the private authorization listener.
#[derive(Clone)]
pub struct StreamConfig {
    /// Public Nginx base URL returned to players.
    pub public_base_url: String,
    /// HMAC secret shared only inside the Canopy process.
    pub token_secret: Vec<u8>,
    /// Lifetime of an issued playback capability.
    pub token_ttl: std::time::Duration,
    /// Loopback/private address used by Nginx authorization subrequests.
    pub auth_addr: SocketAddr,
}

/// Configuration for native identity access-token signing and validation.
#[derive(Clone)]
pub struct IdentityTokenConfig {
    /// JWT issuer claim for native identity access tokens.
    pub issuer: String,
    /// JWT audience claim expected by first-party clients.
    pub audience: String,
    /// Key identifier embedded into access-token headers and payloads.
    pub key_id: String,
    /// Lifetime of an issued identity access token.
    pub ttl_seconds: u64,
    /// Base64-encoded 32-byte Ed25519 signing key seed.
    pub signing_key_base64: Option<String>,
    /// Explicit development escape hatch for local ephemeral signing keys.
    pub allow_ephemeral_dev_key: bool,
}

/// Configuration for Google OIDC ID-token verification.
#[derive(Clone)]
pub struct GoogleOidcConfig {
    /// Accepted Google OAuth client IDs from ID-token `aud` claims.
    pub client_ids: Vec<String>,
    /// Google tokeninfo endpoint used to validate ID tokens.
    pub tokeninfo_url: reqwest::Url,
}

impl GoogleOidcConfig {
    const DEFAULT_TOKENINFO_URL: &'static str = "https://oauth2.googleapis.com/tokeninfo";

    /// Builds optional Google OIDC configuration through an injected environment lookup.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> canopy_core::CanopyResult<Option<Self>> {
        use canopy_core::CanopyError;

        let client_ids: Vec<_> = lookup("CANOPY_GOOGLE_OIDC_CLIENT_IDS")
            .unwrap_or_default()
            .split(',')
            .map(|client_id| client_id.trim().to_owned())
            .filter(|client_id| !client_id.is_empty())
            .collect();
        if client_ids.is_empty() {
            return Ok(None);
        }

        let tokeninfo_url = lookup("CANOPY_GOOGLE_OIDC_TOKENINFO_URL")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Self::DEFAULT_TOKENINFO_URL.into());
        let tokeninfo_url = reqwest::Url::parse(&tokeninfo_url).map_err(|error| {
            CanopyError::InvalidArgument(format!(
                "invalid CANOPY_GOOGLE_OIDC_TOKENINFO_URL: {error}"
            ))
        })?;
        if tokeninfo_url.scheme() != "https" {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_GOOGLE_OIDC_TOKENINFO_URL must use HTTPS".into(),
            ));
        }

        Ok(Some(Self {
            client_ids,
            tokeninfo_url,
        }))
    }
}
/// TLS policy for the SMTP relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpTlsMode {
    /// SMTP over an implicit TLS connection.
    Implicit,
    /// Plain connection upgraded with required STARTTLS before authentication.
    StartTls,
}

/// Validated SMTP connection and message-origin configuration.
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
    pub ca_certificate_pem: Option<Vec<u8>>,
}

/// Polling, leasing, and retry limits for authentication email delivery.
#[derive(Clone, Debug)]
pub struct AuthEmailWorkerConfig {
    pub poll_interval: std::time::Duration,
    pub lease_duration: std::time::Duration,
    pub batch_size: u32,
    pub max_attempts: u32,
    pub initial_retry_delay: std::time::Duration,
    pub max_retry_delay: std::time::Duration,
}

/// Fail-closed authentication email delivery configuration.
#[derive(Clone)]
pub struct AuthEmailConfig {
    pub allow_undelivered_email: bool,
    pub outbox_sealing_key_base64: Option<String>,
    pub smtp: Option<SmtpConfig>,
    pub worker: AuthEmailWorkerConfig,
}

impl AuthEmailConfig {
    const DEFAULT_SMTP_TIMEOUT_SECS: u64 = 30;
    const DEFAULT_POLL_INTERVAL_SECS: u64 = 2;
    const DEFAULT_LEASE_SECS: u64 = 60;
    const DEFAULT_BATCH_SIZE: u32 = 20;
    const DEFAULT_MAX_ATTEMPTS: u32 = 8;
    const DEFAULT_INITIAL_RETRY_SECS: u64 = 5;
    const DEFAULT_MAX_RETRY_SECS: u64 = 15 * 60;
    #[cfg(not(feature = "pg"))]
    fn disabled_for_non_pg() -> Self {
        Self {
            allow_undelivered_email: true,
            outbox_sealing_key_base64: None,
            smtp: None,
            worker: AuthEmailWorkerConfig {
                poll_interval: std::time::Duration::from_secs(Self::DEFAULT_POLL_INTERVAL_SECS),
                lease_duration: std::time::Duration::from_secs(Self::DEFAULT_LEASE_SECS),
                batch_size: Self::DEFAULT_BATCH_SIZE,
                max_attempts: Self::DEFAULT_MAX_ATTEMPTS,
                initial_retry_delay: std::time::Duration::from_secs(
                    Self::DEFAULT_INITIAL_RETRY_SECS,
                ),
                max_retry_delay: std::time::Duration::from_secs(Self::DEFAULT_MAX_RETRY_SECS),
            },
        }
    }

    /// Builds fail-closed SMTP and outbox-worker configuration.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> canopy_core::CanopyResult<Self> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use canopy_core::CanopyError;

        let allow_undelivered_email = lookup("CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL")
            .map(|value| parse_bool("CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL", &value))
            .transpose()?
            .unwrap_or(false);

        let outbox_sealing_key_base64 = lookup("CANOPY_AUTH_OUTBOX_SEALING_KEY")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if let Some(encoded) = outbox_sealing_key_base64.as_deref() {
            let decoded = STANDARD.decode(encoded).map_err(|_| {
                CanopyError::InvalidArgument(
                    "CANOPY_AUTH_OUTBOX_SEALING_KEY must be valid base64".into(),
                )
            })?;
            if decoded.len() != 32 {
                return Err(CanopyError::InvalidArgument(
                    "CANOPY_AUTH_OUTBOX_SEALING_KEY must decode to 32 bytes".into(),
                ));
            }
        }

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

        let smtp_timeout_secs = lookup_positive_u64(
            &lookup,
            "CANOPY_SMTP_TIMEOUT_SECS",
            Self::DEFAULT_SMTP_TIMEOUT_SECS,
        )?;
        let worker = AuthEmailWorkerConfig {
            poll_interval: std::time::Duration::from_secs(lookup_positive_u64(
                &lookup,
                "CANOPY_AUTH_EMAIL_POLL_INTERVAL_SECS",
                Self::DEFAULT_POLL_INTERVAL_SECS,
            )?),
            lease_duration: std::time::Duration::from_secs(lookup_positive_u64(
                &lookup,
                "CANOPY_AUTH_EMAIL_LEASE_SECS",
                Self::DEFAULT_LEASE_SECS,
            )?),
            batch_size: lookup_positive_u32(
                &lookup,
                "CANOPY_AUTH_EMAIL_BATCH_SIZE",
                Self::DEFAULT_BATCH_SIZE,
            )?,
            max_attempts: lookup_positive_u32(
                &lookup,
                "CANOPY_AUTH_EMAIL_MAX_ATTEMPTS",
                Self::DEFAULT_MAX_ATTEMPTS,
            )?,
            initial_retry_delay: std::time::Duration::from_secs(lookup_positive_u64(
                &lookup,
                "CANOPY_AUTH_EMAIL_INITIAL_RETRY_SECS",
                Self::DEFAULT_INITIAL_RETRY_SECS,
            )?),
            max_retry_delay: std::time::Duration::from_secs(lookup_positive_u64(
                &lookup,
                "CANOPY_AUTH_EMAIL_MAX_RETRY_SECS",
                Self::DEFAULT_MAX_RETRY_SECS,
            )?),
        };
        if worker.lease_duration < std::time::Duration::from_secs(smtp_timeout_secs) {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_AUTH_EMAIL_LEASE_SECS must be at least CANOPY_SMTP_TIMEOUT_SECS".into(),
            ));
        }
        if worker.initial_retry_delay > worker.max_retry_delay {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_AUTH_EMAIL_INITIAL_RETRY_SECS must not exceed CANOPY_AUTH_EMAIL_MAX_RETRY_SECS".into(),
            ));
        }

        let smtp = if smtp_present {
            let host = required_lookup(&lookup, "CANOPY_SMTP_HOST")?;
            let username = required_lookup(&lookup, "CANOPY_SMTP_USERNAME")?;
            let password = required_lookup(&lookup, "CANOPY_SMTP_PASSWORD")?;
            let from_address = required_lookup(&lookup, "CANOPY_SMTP_FROM_ADDRESS")?;
            from_address.parse::<lettre::Address>().map_err(|_| {
                CanopyError::InvalidArgument(
                    "CANOPY_SMTP_FROM_ADDRESS must be a valid email address".into(),
                )
            })?;
            let from_name = required_lookup(&lookup, "CANOPY_SMTP_FROM_NAME")?;
            let public_base_url = required_lookup(&lookup, "CANOPY_AUTH_PUBLIC_BASE_URL")?;
            let public_base_url = reqwest::Url::parse(&public_base_url).map_err(|error| {
                CanopyError::InvalidArgument(format!(
                    "invalid CANOPY_AUTH_PUBLIC_BASE_URL: {error}"
                ))
            })?;
            let loopback_http = public_base_url.scheme() == "http"
                && matches!(public_base_url.host_str(), Some("localhost" | "127.0.0.1"));
            if public_base_url.scheme() != "https" && !loopback_http {
                return Err(CanopyError::InvalidArgument(
                    "CANOPY_AUTH_PUBLIC_BASE_URL must use HTTPS outside loopback".into(),
                ));
            }

            let tls_mode = match lookup("CANOPY_SMTP_TLS_MODE")
                .unwrap_or_else(|| "implicit".into())
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "implicit" => SmtpTlsMode::Implicit,
                "starttls" => SmtpTlsMode::StartTls,
                _ => {
                    return Err(CanopyError::InvalidArgument(
                        "CANOPY_SMTP_TLS_MODE must be implicit or starttls".into(),
                    ));
                }
            };
            let default_port = match tls_mode {
                SmtpTlsMode::Implicit => 465,
                SmtpTlsMode::StartTls => 587,
            };
            let port = lookup("CANOPY_SMTP_PORT")
                .unwrap_or_else(|| default_port.to_string())
                .parse::<u16>()
                .map_err(|_| {
                    CanopyError::InvalidArgument(
                        "CANOPY_SMTP_PORT must be a positive integer".into(),
                    )
                })?;
            if port == 0 {
                return Err(CanopyError::InvalidArgument(
                    "CANOPY_SMTP_PORT must be a positive integer".into(),
                ));
            }
            if outbox_sealing_key_base64.is_none() {
                return Err(CanopyError::InvalidArgument(
                    "CANOPY_AUTH_OUTBOX_SEALING_KEY is required when SMTP delivery is enabled"
                        .into(),
                ));
            }

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
            Some(SmtpConfig {
                host,
                port,
                tls_mode,
                username,
                password,
                from_address,
                from_name,
                public_base_url,
                timeout: std::time::Duration::from_secs(smtp_timeout_secs),
                ca_certificate_pem,
            })
        } else if allow_undelivered_email {
            None
        } else {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_SMTP_HOST is required unless CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL=true"
                    .into(),
            ));
        };

        Ok(Self {
            allow_undelivered_email,
            outbox_sealing_key_base64,
            smtp,
            worker,
        })
    }
}
impl IdentityTokenConfig {
    const DEFAULT_ISSUER: &'static str = "canopy";
    const DEFAULT_AUDIENCE: &'static str = "pandawave";
    const DEFAULT_KEY_ID: &'static str = "identity-access-v1";
    const DEFAULT_TTL_SECONDS: u64 = 900;

    /// Builds native identity token configuration through an injected environment lookup.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> canopy_core::CanopyResult<Self> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use canopy_core::CanopyError;

        let issuer = lookup("CANOPY_IDENTITY_ACCESS_TOKEN_ISSUER")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Self::DEFAULT_ISSUER.into());
        let audience = lookup("CANOPY_IDENTITY_ACCESS_TOKEN_AUDIENCE")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Self::DEFAULT_AUDIENCE.into());
        let key_id = lookup("CANOPY_IDENTITY_ACCESS_TOKEN_KEY_ID")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Self::DEFAULT_KEY_ID.into());
        let ttl_seconds = lookup("CANOPY_IDENTITY_ACCESS_TOKEN_TTL_SECS")
            .unwrap_or_else(|| Self::DEFAULT_TTL_SECONDS.to_string())
            .parse::<u64>()
            .map_err(|_| {
                CanopyError::InvalidArgument(
                    "CANOPY_IDENTITY_ACCESS_TOKEN_TTL_SECS must be a positive integer".into(),
                )
            })?;
        if ttl_seconds == 0 {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_IDENTITY_ACCESS_TOKEN_TTL_SECS must be a positive integer".into(),
            ));
        }

        let allow_ephemeral_dev_key = lookup("CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY")
            .map(|value| parse_bool("CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY", &value))
            .transpose()?
            .unwrap_or(false);
        let signing_key_base64 = lookup("CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        if let Some(signing_key) = signing_key_base64.as_deref() {
            let decoded = STANDARD.decode(signing_key).map_err(|_| {
                CanopyError::InvalidArgument(
                    "CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64 must be valid base64".into(),
                )
            })?;
            if decoded.len() != 32 {
                return Err(CanopyError::InvalidArgument(
                    "CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64 must decode to 32 bytes"
                        .into(),
                ));
            }
        } else if !allow_ephemeral_dev_key {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64 is required unless CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY=true".into(),
            ));
        }

        Ok(Self {
            issuer,
            audience,
            key_id,
            ttl_seconds,
            signing_key_base64,
            allow_ephemeral_dev_key,
        })
    }

    pub fn access_token_config(&self) -> crate::identity::AccessTokenConfig {
        crate::identity::AccessTokenConfig {
            issuer: self.issuer.clone(),
            audience: self.audience.clone(),
            key_id: self.key_id.clone(),
            ttl_seconds: self.ttl_seconds,
        }
    }
}

fn parse_bool(key: &str, value: &str) -> canopy_core::CanopyResult<bool> {
    use canopy_core::CanopyError;

    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(CanopyError::InvalidArgument(format!(
            "{key} must be true or false"
        ))),
    }
}

fn required_lookup(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
) -> canopy_core::CanopyResult<String> {
    lookup(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| canopy_core::CanopyError::InvalidArgument(format!("{key} is required")))
}

fn lookup_positive_u64(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: u64,
) -> canopy_core::CanopyResult<u64> {
    let value = lookup(key)
        .unwrap_or_else(|| default.to_string())
        .parse::<u64>()
        .map_err(|_| {
            canopy_core::CanopyError::InvalidArgument(format!("{key} must be a positive integer"))
        })?;
    if value == 0 {
        return Err(canopy_core::CanopyError::InvalidArgument(format!(
            "{key} must be a positive integer"
        )));
    }
    Ok(value)
}

fn lookup_positive_u32(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: u32,
) -> canopy_core::CanopyResult<u32> {
    let value = lookup(key)
        .unwrap_or_else(|| default.to_string())
        .parse::<u32>()
        .map_err(|_| {
            canopy_core::CanopyError::InvalidArgument(format!("{key} must be a positive integer"))
        })?;
    if value == 0 {
        return Err(canopy_core::CanopyError::InvalidArgument(format!(
            "{key} must be a positive integer"
        )));
    }
    Ok(value)
}

impl StreamConfig {
    /// Builds stream configuration through an injected environment lookup.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> canopy_core::CanopyResult<Self> {
        use canopy_core::CanopyError;

        let public_base_url = lookup("CANOPY_STREAM_PUBLIC_BASE_URL")
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                CanopyError::InvalidArgument("CANOPY_STREAM_PUBLIC_BASE_URL is required".into())
            })?;
        let parsed = reqwest::Url::parse(&public_base_url).map_err(|error| {
            CanopyError::InvalidArgument(format!("invalid CANOPY_STREAM_PUBLIC_BASE_URL: {error}"))
        })?;
        let is_loopback_http = parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"));
        if parsed.scheme() != "https" && !is_loopback_http {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_STREAM_PUBLIC_BASE_URL must use HTTPS outside loopback".into(),
            ));
        }

        let token_secret = lookup("CANOPY_STREAM_TOKEN_SECRET")
            .filter(|value| value.len() >= 32)
            .ok_or_else(|| {
                CanopyError::InvalidArgument(
                    "CANOPY_STREAM_TOKEN_SECRET must contain at least 32 bytes".into(),
                )
            })?
            .into_bytes();

        let token_ttl_secs = lookup("CANOPY_STREAM_TOKEN_TTL_SECS")
            .unwrap_or_else(|| "600".into())
            .parse::<u64>()
            .map_err(|_| {
                CanopyError::InvalidArgument(
                    "CANOPY_STREAM_TOKEN_TTL_SECS must be a positive integer".into(),
                )
            })?;
        if token_ttl_secs == 0 {
            return Err(CanopyError::InvalidArgument(
                "CANOPY_STREAM_TOKEN_TTL_SECS must be a positive integer".into(),
            ));
        }

        let auth_addr = lookup("CANOPY_STREAM_AUTH_ADDR")
            .unwrap_or_else(|| "127.0.0.1:8081".into())
            .parse::<SocketAddr>()
            .map_err(|error| {
                CanopyError::InvalidArgument(format!("invalid CANOPY_STREAM_AUTH_ADDR: {error}"))
            })?;

        Ok(Self {
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            token_secret,
            token_ttl: std::time::Duration::from_secs(token_ttl_secs),
            auth_addr,
        })
    }
}

/// Runtime configuration for the Canopy server.
///
/// Values are sourced from the environment with sensible defaults so the
/// server can still start in a local/demo setup without any configuration.
#[derive(Clone)]
pub struct Config {
    /// Address the gRPC server binds to.
    pub grpc_addr: SocketAddr,
    /// PostgreSQL connection string for the persistence layer.
    pub database_url: String,
    /// Maximum number of PostgreSQL connections in the pool.
    pub pg_max_connections: u32,
    /// Timeout (seconds) for acquiring a connection from the pool.
    pub pg_acquire_timeout_secs: u64,
    /// Shared secret used to verify login/profile tokens.
    pub auth_token_secret: String,
    /// Redis URL reserved for the future JadeCache layer.
    pub redis_url: String,
    /// Optional provider fixture JSON path to ingest on startup in PostgreSQL mode.
    pub provider_fixture_path: Option<String>,
    /// Public capability and private stream-authorization configuration.
    pub stream: StreamConfig,
    /// Native identity access-token signing and validation configuration.
    pub identity_tokens: IdentityTokenConfig,
    /// Optional Google OIDC ID-token verifier configuration.
    pub google_oidc: Option<GoogleOidcConfig>,
    /// Authentication email delivery and SMTP configuration.
    pub auth_email: AuthEmailConfig,
    /// Root containing Canopy's managed `library/` directory.
    pub media_root: PathBuf,
}

impl Config {
    /// Default gRPC bind address used when `CANOPY_GRPC_ADDR` is not set.
    const DEFAULT_GRPC_ADDR: &'static str = "[::1]:50051";
    /// Default database URL used when `CANOPY_DATABASE_URL` is not set.
    const DEFAULT_DATABASE_URL: &'static str = "postgres://canopy:canopy@localhost:5432/canopy";
    /// Default max connections in the PostgreSQL pool.
    const DEFAULT_PG_MAX_CONNECTIONS: u32 = 20;
    /// Default timeout (seconds) for acquiring a pool connection.
    const DEFAULT_PG_ACQUIRE_TIMEOUT_SECS: u64 = 5;
    /// Default auth token secret for local development.
    const DEFAULT_AUTH_TOKEN_SECRET: &'static str = "canopy-auth-secret";
    /// Default Redis URL for the cache layer.
    const DEFAULT_REDIS_URL: &'static str = "redis://localhost:6379";

    /// Loads configuration from environment variables.
    ///
    /// Recognized variables:
    /// * `CANOPY_GRPC_ADDR` — socket address the gRPC server binds to.
    /// * `CANOPY_DATABASE_URL` — PostgreSQL connection string.
    /// * `CANOPY_PG_MAX_CONNECTIONS` — max pool size (default 20).
    /// * `CANOPY_PG_ACQUIRE_TIMEOUT_SECS` — pool acquire timeout (default 5).
    /// * `CANOPY_REDIS_URL` — Redis connection string.
    /// * `CANOPY_PROVIDER_FIXTURE_PATH` — optional fixture JSON to ingest at startup in PostgreSQL mode.
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let grpc_addr = env::var("CANOPY_GRPC_ADDR")
            .unwrap_or_else(|_| Self::DEFAULT_GRPC_ADDR.to_string())
            .parse::<SocketAddr>()?;

        let database_url = env::var("CANOPY_DATABASE_URL")
            .unwrap_or_else(|_| Self::DEFAULT_DATABASE_URL.to_string());

        let pg_max_connections = env::var("CANOPY_PG_MAX_CONNECTIONS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(Self::DEFAULT_PG_MAX_CONNECTIONS);

        let pg_acquire_timeout_secs = env::var("CANOPY_PG_ACQUIRE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(Self::DEFAULT_PG_ACQUIRE_TIMEOUT_SECS);

        let auth_token_secret = env::var("CANOPY_AUTH_TOKEN_SECRET")
            .unwrap_or_else(|_| Self::DEFAULT_AUTH_TOKEN_SECRET.to_string());

        let redis_url =
            env::var("CANOPY_REDIS_URL").unwrap_or_else(|_| Self::DEFAULT_REDIS_URL.to_string());

        let provider_fixture_path = env::var("CANOPY_PROVIDER_FIXTURE_PATH")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let stream = StreamConfig::from_lookup(|key| env::var(key).ok())?;
        let identity_tokens = IdentityTokenConfig::from_lookup(|key| env::var(key).ok())?;
        let google_oidc = GoogleOidcConfig::from_lookup(|key| env::var(key).ok())?;
        #[cfg(feature = "pg")]
        let auth_email = AuthEmailConfig::from_lookup(|key| env::var(key).ok())?;
        #[cfg(not(feature = "pg"))]
        let auth_email = AuthEmailConfig::disabled_for_non_pg();
        let media_root = env::var("CANOPY_MEDIA_ROOT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .ok_or("CANOPY_MEDIA_ROOT is required")?;

        Ok(Self {
            grpc_addr,
            database_url,
            pg_max_connections,
            pg_acquire_timeout_secs,
            auth_token_secret,
            redis_url,
            provider_fixture_path,
            stream,
            identity_tokens,
            google_oidc,
            auth_email,
            media_root,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use canopy_core::CanopyError;

    use super::StreamConfig;

    fn parse(values: &[(&str, &str)]) -> canopy_core::CanopyResult<StreamConfig> {
        let values: HashMap<_, _> = values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        StreamConfig::from_lookup(|key| values.get(key).cloned())
    }

    fn required() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "CANOPY_STREAM_PUBLIC_BASE_URL",
                "https://media.example.test",
            ),
            (
                "CANOPY_STREAM_TOKEN_SECRET",
                "0123456789abcdef0123456789abcdef",
            ),
        ]
    }

    #[test]
    fn stream_config_requires_public_base_url() {
        assert!(matches!(
            parse(&[(
                "CANOPY_STREAM_TOKEN_SECRET",
                "0123456789abcdef0123456789abcdef",
            )]),
            Err(CanopyError::InvalidArgument(_))
        ));
    }

    #[test]
    fn stream_config_rejects_insecure_non_loopback_url() {
        let mut values = required();
        values[0].1 = "http://media.example.test";
        assert!(matches!(
            parse(&values),
            Err(CanopyError::InvalidArgument(_))
        ));
    }

    #[test]
    fn stream_config_accepts_http_loopback_urls() {
        for base_url in ["http://127.0.0.1:18080", "http://localhost:18080"] {
            let mut values = required();
            values[0].1 = base_url;
            assert!(parse(&values).is_ok());
        }
    }

    #[test]
    fn stream_config_requires_strong_secret() {
        let missing = parse(&[(
            "CANOPY_STREAM_PUBLIC_BASE_URL",
            "https://media.example.test",
        )]);
        assert!(matches!(missing, Err(CanopyError::InvalidArgument(_))));

        let mut short = required();
        short[1].1 = "too-short";
        assert!(matches!(
            parse(&short),
            Err(CanopyError::InvalidArgument(_))
        ));
    }

    #[test]
    fn stream_config_uses_safe_defaults_and_normalizes_base_url() {
        let mut values = required();
        values[0].1 = "https://media.example.test///";
        let config = parse(&values).unwrap();

        assert_eq!(config.public_base_url, "https://media.example.test");
        assert_eq!(config.token_ttl.as_secs(), 600);
        assert_eq!(config.auth_addr.to_string(), "127.0.0.1:8081");
    }

    #[test]
    fn stream_config_rejects_zero_and_malformed_ttl() {
        for ttl in ["0", "invalid"] {
            let mut values = required();
            values.push(("CANOPY_STREAM_TOKEN_TTL_SECS", ttl));
            assert!(matches!(
                parse(&values),
                Err(CanopyError::InvalidArgument(_))
            ));
        }
    }
    mod google_oidc {
        use std::collections::HashMap;

        use canopy_core::CanopyError;

        use super::super::GoogleOidcConfig;

        fn parse(values: &[(&str, &str)]) -> canopy_core::CanopyResult<Option<GoogleOidcConfig>> {
            let values: HashMap<_, _> = values
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            GoogleOidcConfig::from_lookup(|key| values.get(key).cloned())
        }

        #[test]
        fn remains_disabled_without_client_ids() {
            assert!(parse(&[]).unwrap().is_none());
            assert!(
                parse(&[("CANOPY_GOOGLE_OIDC_CLIENT_IDS", "  ,  ")])
                    .unwrap()
                    .is_none()
            );
        }

        #[test]
        fn accepts_comma_separated_client_ids_with_default_endpoint() {
            let config = parse(&[(
                "CANOPY_GOOGLE_OIDC_CLIENT_IDS",
                "client-1.apps.googleusercontent.com, client-2.apps.googleusercontent.com",
            )])
            .unwrap()
            .unwrap();

            assert_eq!(
                config.client_ids,
                vec![
                    "client-1.apps.googleusercontent.com".to_owned(),
                    "client-2.apps.googleusercontent.com".to_owned()
                ]
            );
            assert_eq!(
                config.tokeninfo_url.as_str(),
                "https://oauth2.googleapis.com/tokeninfo"
            );
        }

        #[test]
        fn rejects_non_https_tokeninfo_endpoint() {
            assert!(matches!(
                parse(&[
                    ("CANOPY_GOOGLE_OIDC_CLIENT_IDS", "client-1"),
                    (
                        "CANOPY_GOOGLE_OIDC_TOKENINFO_URL",
                        "http://example.test/tokeninfo"
                    ),
                ]),
                Err(CanopyError::InvalidArgument(_))
            ));
        }
    }
    mod identity_access_token {
        use std::collections::HashMap;

        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use canopy_core::CanopyError;

        use super::super::IdentityTokenConfig;

        fn parse(values: &[(&str, &str)]) -> canopy_core::CanopyResult<IdentityTokenConfig> {
            let values: HashMap<_, _> = values
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            IdentityTokenConfig::from_lookup(|key| values.get(key).cloned())
        }

        #[test]
        fn rejects_missing_signing_material_without_dev_flag() {
            assert!(matches!(
                parse(&[]),
                Err(CanopyError::InvalidArgument(message))
                    if message.contains("CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64")
            ));
        }

        #[test]
        fn accepts_configured_signing_material() {
            let signing_key = STANDARD.encode([7_u8; 32]);
            let config = parse(&[
                ("CANOPY_IDENTITY_ACCESS_TOKEN_ISSUER", "canopy.example"),
                ("CANOPY_IDENTITY_ACCESS_TOKEN_AUDIENCE", "pandawave"),
                (
                    "CANOPY_IDENTITY_ACCESS_TOKEN_KEY_ID",
                    "identity-key-2026-07",
                ),
                ("CANOPY_IDENTITY_ACCESS_TOKEN_TTL_SECS", "1200"),
                (
                    "CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64",
                    signing_key.as_str(),
                ),
            ])
            .unwrap();

            assert_eq!(config.issuer, "canopy.example");
            assert_eq!(config.audience, "pandawave");
            assert_eq!(config.key_id, "identity-key-2026-07");
            assert_eq!(config.ttl_seconds, 1200);
            assert_eq!(
                config.signing_key_base64.as_deref(),
                Some(signing_key.as_str())
            );
            assert!(!config.allow_ephemeral_dev_key);
        }

        #[test]
        fn permits_missing_signing_material_only_with_explicit_dev_flag() {
            let config = parse(&[("CANOPY_IDENTITY_ALLOW_EPHEMERAL_DEV_KEY", "true")]).unwrap();

            assert!(config.signing_key_base64.is_none());
            assert!(config.allow_ephemeral_dev_key);
        }

        #[test]
        fn rejects_malformed_signing_material() {
            assert!(matches!(
                parse(&[(
                    "CANOPY_IDENTITY_ACCESS_TOKEN_SIGNING_KEY_BASE64",
                    "not valid base64",
                )]),
                Err(CanopyError::InvalidArgument(_))
            ));
        }
    }
    mod auth_email {
        use std::collections::HashMap;

        use super::super::{AuthEmailConfig, SmtpTlsMode};

        const TEST_CA_PATH: &str = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/certs/local-integration-test-ca.pem"
        );

        fn parse(values: &[(&str, &str)]) -> canopy_core::CanopyResult<AuthEmailConfig> {
            let values: HashMap<_, _> = values
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect();
            AuthEmailConfig::from_lookup(|key| values.get(key).cloned())
        }

        fn required_smtp() -> Vec<(&'static str, &'static str)> {
            vec![
                ("CANOPY_SMTP_HOST", "smtp.example.test"),
                ("CANOPY_SMTP_TLS_MODE", "starttls"),
                ("CANOPY_SMTP_USERNAME", "canopy"),
                ("CANOPY_SMTP_PASSWORD", "test-password"),
                ("CANOPY_SMTP_FROM_ADDRESS", "auth@example.test"),
                ("CANOPY_SMTP_FROM_NAME", "Canopy"),
                ("CANOPY_AUTH_PUBLIC_BASE_URL", "https://app.example.test"),
                (
                    "CANOPY_AUTH_OUTBOX_SEALING_KEY",
                    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                ),
            ]
        }

        #[test]
        fn smtp_is_required_by_default() {
            let error = parse(&[]).err().expect("missing SMTP should fail");
            assert!(error.to_string().contains("CANOPY_SMTP_HOST"));
        }

        #[test]
        fn explicit_escape_hatch_allows_missing_smtp() {
            let config = parse(&[("CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL", "true")]).unwrap();

            assert!(config.allow_undelivered_email);
            assert!(config.smtp.is_none());
        }

        #[test]
        fn partial_smtp_configuration_is_rejected() {
            let error = parse(&[
                ("CANOPY_AUTH_ALLOW_UNDELIVERED_EMAIL", "true"),
                ("CANOPY_SMTP_HOST", "smtp.example.test"),
            ])
            .err()
            .expect("partial SMTP configuration should fail");

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
        fn valid_custom_smtp_ca_is_loaded() {
            let mut values = required_smtp();
            values.push(("CANOPY_SMTP_CA_CERT_PATH", TEST_CA_PATH));
            let smtp = parse(&values).unwrap().smtp.unwrap();

            assert_eq!(
                smtp.ca_certificate_pem.as_deref(),
                Some(
                    include_bytes!("../../../fixtures/certs/local-integration-test-ca.pem")
                        .as_slice()
                )
            );
        }

        #[test]
        fn empty_custom_smtp_ca_path_is_rejected() {
            let mut values = required_smtp();
            values.push(("CANOPY_SMTP_CA_CERT_PATH", " "));

            let error = parse(&values)
                .err()
                .expect("empty custom SMTP CA path should fail");

            assert!(
                error
                    .to_string()
                    .contains("CANOPY_SMTP_CA_CERT_PATH must not be empty")
            );
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
        #[test]
        fn configured_smtp_requires_explicit_outbox_sealing_key() {
            let values = required_smtp()
                .into_iter()
                .filter(|(key, _)| *key != "CANOPY_AUTH_OUTBOX_SEALING_KEY")
                .collect::<Vec<_>>();
            let error = parse(&values)
                .err()
                .expect("missing outbox sealing key should fail");

            assert!(error.to_string().contains("CANOPY_AUTH_OUTBOX_SEALING_KEY"));
        }

        #[test]
        fn remote_plaintext_smtp_mode_is_rejected() {
            let mut values = required_smtp();
            values[1].1 = "plaintext";

            assert!(parse(&values).is_err());
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
        #[cfg(not(feature = "pg"))]
        #[test]
        fn non_pg_runtime_disables_email_delivery_without_smtp() {
            let config = AuthEmailConfig::disabled_for_non_pg();

            assert!(config.allow_undelivered_email);
            assert!(config.smtp.is_none());
        }
    }
}
