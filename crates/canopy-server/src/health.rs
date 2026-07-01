//! Health service.
//!
//! Reports process liveness, build version, and dependency readiness.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(feature = "pg")]
use std::sync::Arc;

/// Health state for the service or one dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthState {
    /// Fully healthy.
    Healthy,
    /// Reachable but not fully healthy.
    Degraded,
    /// Not ready to serve traffic.
    Unhealthy,
}

impl HealthState {
    /// Wire-friendly lowercase status.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unhealthy => "unhealthy",
        }
    }
}

/// Outcome of a dependency health probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyStatus {
    /// Dependency name.
    pub name: String,
    /// Dependency status.
    pub status: HealthState,
    /// Short diagnostic message.
    pub message: String,
}

/// Outcome of a health probe.
#[derive(Clone, Debug)]
pub struct HealthStatus {
    /// Whether the service considers itself healthy.
    pub healthy: bool,
    /// Aggregate health status.
    pub status: HealthState,
    /// Build version of the running binary.
    pub version: String,
    /// Per-dependency status details.
    pub dependencies: Vec<DependencyStatus>,
}

/// Application service for health/readiness reporting.
#[derive(Clone, Default)]
pub struct HealthService {
    #[cfg(feature = "pg")]
    db_pool: Option<Arc<sqlx::PgPool>>,
    rustfs_endpoint: Option<String>,
    media_root: Option<PathBuf>,
}

impl HealthService {
    /// Creates a new health service without external dependencies checked.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "pg")]
            db_pool: None,
            rustfs_endpoint: None,
            media_root: None,
        }
    }

    /// Adds an optional RustFS endpoint probe.
    pub fn with_rustfs(mut self, endpoint: Option<String>) -> Self {
        self.rustfs_endpoint = endpoint;
        self
    }
    /// Adds the managed media root whose `library/` directory must be readable.
    pub fn with_media_root(mut self, media_root: PathBuf) -> Self {
        self.media_root = Some(media_root);
        self
    }

    #[cfg(feature = "pg")]
    /// Creates a new health service that checks PostgreSQL connectivity.
    pub fn with_db(pool: Arc<sqlx::PgPool>) -> Self {
        Self {
            db_pool: Some(pool),
            rustfs_endpoint: None,
            media_root: None,
        }
    }

    /// Returns the current health status.
    pub async fn check(&self) -> HealthStatus {
        let mut dependencies = Vec::new();

        #[cfg(feature = "pg")]
        if let Some(pool) = &self.db_pool {
            dependencies.push(check_postgres(pool).await);
        }

        if let Some(endpoint) = &self.rustfs_endpoint {
            dependencies.push(check_tcp_endpoint("rustfs", endpoint).await);
        }

        if let Some(media_root) = &self.media_root {
            dependencies.push(check_media_library(media_root).await);
        }

        let status = aggregate(&dependencies);

        HealthStatus {
            healthy: status == HealthState::Healthy,
            status,
            version: env!("CARGO_PKG_VERSION").to_string(),
            dependencies,
        }
    }
}

async fn check_media_library(media_root: &Path) -> DependencyStatus {
    let library = media_root.join("library");
    let result = async {
        let metadata = tokio::fs::metadata(&library).await?;
        if !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "managed library path is not a directory",
            ));
        }
        let _ = tokio::fs::read_dir(&library).await?;
        Ok::<(), std::io::Error>(())
    }
    .await;

    match result {
        Ok(()) => DependencyStatus {
            name: "media_library".into(),
            status: HealthState::Healthy,
            message: "managed library directory is readable".into(),
        },
        Err(error) => DependencyStatus {
            name: "media_library".into(),
            status: HealthState::Unhealthy,
            message: error.to_string(),
        },
    }
}

#[cfg(feature = "pg")]
async fn check_postgres(pool: &Arc<sqlx::PgPool>) -> DependencyStatus {
    match sqlx::query("SELECT 1").fetch_optional(pool.as_ref()).await {
        Ok(_) => DependencyStatus {
            name: "postgres".to_string(),
            status: HealthState::Healthy,
            message: "SELECT 1 succeeded".to_string(),
        },
        Err(err) => DependencyStatus {
            name: "postgres".to_string(),
            status: HealthState::Unhealthy,
            message: err.to_string(),
        },
    }
}

async fn check_tcp_endpoint(name: &str, endpoint: &str) -> DependencyStatus {
    let Some(addr) = endpoint_socket_addr(endpoint) else {
        return DependencyStatus {
            name: name.to_string(),
            status: HealthState::Unhealthy,
            message: format!("invalid endpoint: {endpoint}"),
        };
    };

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(&addr),
    )
    .await;
    match result {
        Ok(Ok(_)) => DependencyStatus {
            name: name.to_string(),
            status: HealthState::Healthy,
            message: format!("tcp connect succeeded: {addr}"),
        },
        Ok(Err(err)) => DependencyStatus {
            name: name.to_string(),
            status: HealthState::Unhealthy,
            message: err.to_string(),
        },
        Err(_) => DependencyStatus {
            name: name.to_string(),
            status: HealthState::Unhealthy,
            message: format!("tcp connect timed out: {addr}"),
        },
    }
}

fn endpoint_socket_addr(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    let authority = without_scheme.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    if authority.rsplit_once(':').is_some() {
        Some(authority.to_string())
    } else if endpoint.starts_with("https://") {
        Some(format!("{authority}:443"))
    } else {
        Some(format!("{authority}:80"))
    }
}

fn aggregate(dependencies: &[DependencyStatus]) -> HealthState {
    if dependencies
        .iter()
        .any(|dep| dep.status == HealthState::Unhealthy)
    {
        HealthState::Unhealthy
    } else if dependencies
        .iter()
        .any(|dep| dep.status == HealthState::Degraded)
    {
        HealthState::Degraded
    } else {
        HealthState::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn media_library_readiness_requires_a_readable_directory() {
        let root = tempfile::tempdir().unwrap();
        let health = HealthService::new().with_media_root(root.path().to_path_buf());

        let missing = health.check().await;
        assert_eq!(missing.status, HealthState::Unhealthy);
        assert_eq!(missing.dependencies[0].name, "media_library");

        tokio::fs::create_dir(root.path().join("library"))
            .await
            .unwrap();
        let ready = health.check().await;
        assert_eq!(ready.status, HealthState::Healthy);
    }

    #[test]
    fn endpoint_socket_addr_defaults_ports() {
        assert_eq!(
            endpoint_socket_addr("http://localhost:9000"),
            Some("localhost:9000".to_string())
        );
        assert_eq!(
            endpoint_socket_addr("https://rustfs.internal"),
            Some("rustfs.internal:443".to_string())
        );
        assert_eq!(
            endpoint_socket_addr("localhost"),
            Some("localhost:80".to_string())
        );
    }

    #[test]
    fn aggregate_marks_unhealthy_dependency_as_unhealthy() {
        let status = aggregate(&[DependencyStatus {
            name: "postgres".to_string(),
            status: HealthState::Unhealthy,
            message: "down".to_string(),
        }]);
        assert_eq!(status, HealthState::Unhealthy);
    }
}
