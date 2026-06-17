//! Canopy gRPC server binary.

use canopy_server::{Config, observability, run};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    observability::init()?;
    info!("Starting Canopy gRPC server");

    let config = Config::from_env()?;
    run(config).await
}
