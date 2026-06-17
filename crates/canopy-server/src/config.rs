//! Runtime configuration for the Canopy server.

use std::env;
use std::net::SocketAddr;

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
    /// RustFS (S3-compatible object storage) base URL.
    pub rustfs_url: String,
    /// RustFS bucket name for the media store.
    pub rustfs_bucket: String,
    /// RustFS access key for presigned URL generation.
    pub rustfs_access_key: String,
    /// RustFS secret key for presigned URL signing.
    pub rustfs_secret_key: String,
    /// Redis URL for the JadeCache layer.
    pub redis_url: String,
}

impl Config {
    /// Default gRPC bind address used when `CANOPY_GRPC_ADDR` is not set.
    const DEFAULT_GRPC_ADDR: &'static str = "[::1]:50051";
    /// Default database URL used when `CANOPY_DATABASE_URL` is not set.
    const DEFAULT_DATABASE_URL: &'static str =
        "postgres://canopy:canopy@localhost:5432/canopy";
    /// Default RustFS base URL.
    const DEFAULT_RUSTFS_URL: &'static str = "http://localhost:9000";
    /// Default RustFS media bucket name.
    const DEFAULT_RUSTFS_BUCKET: &'static str = "pandawave-media";
    /// Default RustFS access key.
    const DEFAULT_RUSTFS_ACCESS_KEY: &'static str = "canopy";
    /// Default RustFS secret key.
    const DEFAULT_RUSTFS_SECRET_KEY: &'static str = "canopy-secret";
    /// Default Redis URL for the cache layer.
    const DEFAULT_REDIS_URL: &'static str = "redis://localhost:6379";

    /// Loads configuration from environment variables.
    ///
    /// Recognized variables:
    /// * `CANOPY_GRPC_ADDR` — socket address the gRPC server binds to.
    /// * `CANOPY_DATABASE_URL` — PostgreSQL connection string.
    /// * `CANOPY_RUSTFS_URL` — RustFS base URL.
    /// * `CANOPY_RUSTFS_BUCKET` — RustFS media bucket name.
    /// * `CANOPY_RUSTFS_ACCESS_KEY` — RustFS access key.
    /// * `CANOPY_RUSTFS_SECRET_KEY` — RustFS secret key.
    /// * `CANOPY_REDIS_URL` — Redis connection string.
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let grpc_addr = env::var("CANOPY_GRPC_ADDR")
            .unwrap_or_else(|_| Self::DEFAULT_GRPC_ADDR.to_string())
            .parse::<SocketAddr>()?;

        let database_url = env::var("CANOPY_DATABASE_URL")
            .unwrap_or_else(|_| Self::DEFAULT_DATABASE_URL.to_string());

        let rustfs_url = env::var("CANOPY_RUSTFS_URL")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_URL.to_string());

        let rustfs_bucket = env::var("CANOPY_RUSTFS_BUCKET")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_BUCKET.to_string());

        let rustfs_access_key = env::var("CANOPY_RUSTFS_ACCESS_KEY")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_ACCESS_KEY.to_string());

        let rustfs_secret_key = env::var("CANOPY_RUSTFS_SECRET_KEY")
            .unwrap_or_else(|_| Self::DEFAULT_RUSTFS_SECRET_KEY.to_string());

        let redis_url = env::var("CANOPY_REDIS_URL")
            .unwrap_or_else(|_| Self::DEFAULT_REDIS_URL.to_string());

        Ok(Self {
            grpc_addr,
            database_url,
            rustfs_url,
            rustfs_bucket,
            rustfs_access_key,
            rustfs_secret_key,
            redis_url,
        })
    }
}
