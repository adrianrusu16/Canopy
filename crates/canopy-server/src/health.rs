//! Health service.
//!
//! Today this reports process liveness, the build version, and optionally
//! PostgreSQL connectivity. The dependency-aware readiness check (RustFS
//! reachability, degraded states) described in the architecture is a planned
//! addition.

use std::sync::Arc;

/// Outcome of a health probe.
#[derive(Clone, Debug)]
pub struct HealthStatus {
    /// Whether the service considers itself healthy.
    pub healthy: bool,
    /// Build version of the running binary.
    pub version: String,
}

/// Application service for health/readiness reporting.
#[derive(Clone, Default)]
pub struct HealthService {
    #[cfg(feature = "pg")]
    db_pool: Option<Arc<sqlx::PgPool>>,
}

impl HealthService {
    /// Creates a new health service without external dependencies checked.
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "pg")]
            db_pool: None,
        }
    }

    #[cfg(feature = "pg")]
    /// Creates a new health service that checks PostgreSQL connectivity.
    pub fn with_db(pool: Arc<sqlx::PgPool>) -> Self {
        Self { db_pool: Some(pool) }
    }

    /// Returns the current health status.
    ///
    /// When a PostgreSQL pool is attached, a lightweight `SELECT 1` probe is
    /// run; failure marks the status as unhealthy.
    pub async fn check(&self) -> HealthStatus {
        let mut healthy = true;

        #[cfg(feature = "pg")]
        if let Some(pool) = &self.db_pool {
            if sqlx::query("SELECT 1")
                .fetch_optional(pool.as_ref())
                .await
                .is_err()
            {
                healthy = false;
            }
        }

        HealthStatus {
            healthy,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}
