//! Observability wiring (tracing, and — in the future — metrics).
//!
//! For now this only initializes a `tracing` subscriber. Correlation-ID
//! propagation across the gRPC boundary and Prometheus metrics are planned
//! additions that will be configured from here.

use tracing::Level;
use tracing_subscriber::FmtSubscriber;

/// Installs the global `tracing` subscriber.
///
/// Safe to call once at process start-up.
pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}
