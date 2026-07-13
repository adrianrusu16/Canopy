//! Health service.
//!
//! Reports process liveness, build version, and dependency readiness.

use std::path::{Path, PathBuf};

use crate::identity::{EmailDeliveryReadiness, EmailDeliveryState};

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
    media_root: Option<PathBuf>,
    email_delivery: Option<EmailDeliveryReadiness>,
}

impl HealthService {
    /// Creates a new health service without external dependencies checked.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "pg")]
            db_pool: None,
            media_root: None,
            email_delivery: None,
        }
    }

    /// Adds the managed media root whose library directory must be readable.
    pub fn with_media_root(mut self, media_root: PathBuf) -> Self {
        self.media_root = Some(media_root);
        self
    }

    /// Adds authentication email delivery readiness.
    pub fn with_email_delivery(mut self, readiness: EmailDeliveryReadiness) -> Self {
        self.email_delivery = Some(readiness);
        self
    }
    #[cfg(feature = "pg")]
    /// Creates a new health service that checks PostgreSQL connectivity.
    pub fn with_db(pool: Arc<sqlx::PgPool>) -> Self {
        Self {
            db_pool: Some(pool),
            media_root: None,
            email_delivery: None,
        }
    }

    /// Returns the current health status.
    pub async fn check(&self) -> HealthStatus {
        let mut dependencies = Vec::new();

        #[cfg(feature = "pg")]
        if let Some(pool) = &self.db_pool {
            dependencies.push(check_postgres(pool).await);
        }

        if let Some(media_root) = &self.media_root {
            dependencies.push(check_media_library(media_root).await);
        }

        if let Some(readiness) = &self.email_delivery {
            dependencies.push(check_email_delivery(readiness));
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

fn check_email_delivery(readiness: &EmailDeliveryReadiness) -> DependencyStatus {
    let snapshot = readiness.snapshot();
    let status = match snapshot.state {
        EmailDeliveryState::Healthy => HealthState::Healthy,
        EmailDeliveryState::Degraded => HealthState::Degraded,
        EmailDeliveryState::Unhealthy => HealthState::Unhealthy,
    };
    DependencyStatus {
        name: "auth_email_delivery".into(),
        status,
        message: snapshot.message.into(),
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
    fn aggregate_marks_unhealthy_dependency_as_unhealthy() {
        let status = aggregate(&[DependencyStatus {
            name: "postgres".to_string(),
            status: HealthState::Unhealthy,
            message: "down".to_string(),
        }]);
        assert_eq!(status, HealthState::Unhealthy);
    }

    #[tokio::test]
    async fn disabled_email_delivery_is_degraded() {
        let health = HealthService::new()
            .with_email_delivery(crate::identity::EmailDeliveryReadiness::disabled());
        let status = health.check().await;
        assert_eq!(status.status, HealthState::Degraded);
        assert_eq!(status.dependencies[0].name, "auth_email_delivery");
    }

    #[tokio::test]
    async fn smtp_failure_is_unhealthy() {
        let readiness = crate::identity::EmailDeliveryReadiness::configured();
        readiness.mark_unhealthy("SMTP connectivity failed");
        let status = HealthService::new().with_email_delivery(readiness);
        assert_eq!(status.check().await.status, HealthState::Unhealthy);
    }
}
