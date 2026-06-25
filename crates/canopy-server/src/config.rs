//! Runtime configuration for the Canopy server.

use std::env;
use std::net::SocketAddr;

/// Backing source used to mint playback stream URLs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicSource {
    /// Local/S3-compatible object storage signed by Canopy.
    Rustfs,
    /// Supabase Storage signed URLs fetched by Canopy.
    Supabase,
}

impl MusicSource {
    fn from_env_value(value: &str) -> Self {
        if value.eq_ignore_ascii_case("supabase") {
            Self::Supabase
        } else {
            Self::Rustfs
        }
    }
}

/// Runtime configuration for the Canopy server.
///
/// Values are sourced from the environment with sensible defaults so the
/// server can still start in a local/demo setup without any configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Address the gRPC server binds to.
    pub grpc_addr: SocketAddr,
    /// PostgreSQL connection string for the persistence layer.
    pub database_url: String,
    /// Maximum number of PostgreSQL connections in the pool.
    pub pg_max_connections: u32,
    /// Timeout (seconds) for acquiring a connection from the pool.
    pub pg_acquire_timeout_secs: u64,
    /// Music source used for playback URL resolution.
    pub music_source: MusicSource,
    /// RustFS (S3-compatible object storage) base URL.
    pub rustfs_url: String,
    /// RustFS bucket name for the media store.
    pub rustfs_bucket: String,
    /// RustFS access key for presigned URL generation.
    pub rustfs_access_key: String,
    /// RustFS secret key for presigned URL signing.
    pub rustfs_secret_key: String,
    /// Supabase project URL used when Supabase is the music source.
    pub supabase_url: String,
    /// Supabase key used to request signed Storage URLs.
    pub supabase_key: String,
    /// Supabase Storage bucket containing music objects.
    pub supabase_storage_bucket: String,
    /// Supabase REST table/view containing music catalog rows.
    pub supabase_catalog_table: String,
    /// Whether startup should sync Supabase catalog rows into PostgreSQL.
    pub supabase_sync_on_start: bool,
    /// Signed URL lifetime when resolving Supabase playback.
    pub supabase_signed_url_ttl_secs: u64,
    /// Shared secret used to verify login/profile tokens.
    pub auth_token_secret: String,
    /// Redis URL for the JadeCache layer.
    pub redis_url: String,
    /// Whether health checks should probe RustFS reachability.
    pub health_check_rustfs: bool,
    /// Optional provider fixture JSON path to ingest on startup in PostgreSQL mode.
    pub provider_fixture_path: Option<String>,
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
    /// Default music source.
    const DEFAULT_MUSIC_SOURCE: &'static str = "rustfs";
    /// Default RustFS base URL.
    const DEFAULT_RUSTFS_URL: &'static str = "http://localhost:9000";
    /// Default RustFS media bucket name.
    const DEFAULT_RUSTFS_BUCKET: &'static str = "pandawave-media";
    /// Default RustFS access key.
    const DEFAULT_RUSTFS_ACCESS_KEY: &'static str = "canopy";
    /// Default RustFS secret key.
    const DEFAULT_RUSTFS_SECRET_KEY: &'static str = "canopy-secret";
    /// Default Supabase signed URL lifetime in seconds.
    const DEFAULT_SUPABASE_SIGNED_URL_TTL_SECS: u64 = 15 * 60;
    /// Default Supabase catalog table/view name.
    const DEFAULT_SUPABASE_CATALOG_TABLE: &'static str = "tracks";
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
    /// * `CANOPY_RUSTFS_URL` — RustFS base URL.
    /// * `CANOPY_RUSTFS_BUCKET` — RustFS media bucket name.
    /// * `CANOPY_RUSTFS_ACCESS_KEY` — RustFS access key.
    /// * `CANOPY_RUSTFS_SECRET_KEY` — RustFS secret key.
    /// * `CANOPY_REDIS_URL` — Redis connection string.
    /// * `CANOPY_HEALTH_CHECK_RUSTFS` — set `true` to include RustFS TCP reachability in health.
    /// * `CANOPY_SUPABASE_CATALOG_TABLE` — Supabase REST table/view to read catalog rows from.
    /// * `CANOPY_SUPABASE_SYNC_ON_START` — set `true` to ingest Supabase catalog rows at startup in PostgreSQL mode.
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

        let music_source = env::var("CANOPY_MUSIC_SOURCE")
            .map(|v| MusicSource::from_env_value(&v))
            .unwrap_or_else(|_| MusicSource::from_env_value(Self::DEFAULT_MUSIC_SOURCE));

        let rustfs_url =
            env::var("CANOPY_RUSTFS_URL").unwrap_or_else(|_| Self::DEFAULT_RUSTFS_URL.to_string());

        let rustfs_bucket = env::var("CANOPY_RUSTFS_BUCKET")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_BUCKET.to_string());

        let rustfs_access_key = env::var("CANOPY_RUSTFS_ACCESS_KEY")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_ACCESS_KEY.to_string());

        let rustfs_secret_key = env::var("CANOPY_RUSTFS_SECRET_KEY")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_SECRET_KEY.to_string());

        let supabase_url = env::var("CANOPY_SUPABASE_URL").unwrap_or_default();

        let supabase_key = env::var("CANOPY_SUPABASE_KEY").unwrap_or_default();

        let supabase_storage_bucket =
            env::var("CANOPY_SUPABASE_STORAGE_BUCKET").unwrap_or_else(|_| rustfs_bucket.clone());

        let supabase_signed_url_ttl_secs = env::var("CANOPY_SUPABASE_SIGNED_URL_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(Self::DEFAULT_SUPABASE_SIGNED_URL_TTL_SECS);

        let supabase_catalog_table = env::var("CANOPY_SUPABASE_CATALOG_TABLE")
            .unwrap_or_else(|_| Self::DEFAULT_SUPABASE_CATALOG_TABLE.to_string());

        let supabase_sync_on_start =
            env::var("CANOPY_SUPABASE_SYNC_ON_START")
                .ok()
                .is_some_and(|v| {
                    matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
                });

        let auth_token_secret = env::var("CANOPY_AUTH_TOKEN_SECRET")
            .unwrap_or_else(|_| Self::DEFAULT_AUTH_TOKEN_SECRET.to_string());

        let redis_url =
            env::var("CANOPY_REDIS_URL").unwrap_or_else(|_| Self::DEFAULT_REDIS_URL.to_string());

        let health_check_rustfs = env::var("CANOPY_HEALTH_CHECK_RUSTFS")
            .ok()
            .is_some_and(|v| {
                matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
            });

        let provider_fixture_path = env::var("CANOPY_PROVIDER_FIXTURE_PATH")
            .ok()
            .filter(|v| !v.trim().is_empty());

        Ok(Self {
            grpc_addr,
            database_url,
            pg_max_connections,
            pg_acquire_timeout_secs,
            music_source,
            rustfs_url,
            rustfs_bucket,
            rustfs_access_key,
            rustfs_secret_key,
            supabase_url,
            supabase_key,
            supabase_storage_bucket,
            supabase_catalog_table,
            supabase_sync_on_start,
            supabase_signed_url_ttl_secs,
            auth_token_secret,
            redis_url,
            health_check_rustfs,
            provider_fixture_path,
        })
    }
}
