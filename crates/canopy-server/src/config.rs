//! Runtime configuration for the Canopy server.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

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
}
