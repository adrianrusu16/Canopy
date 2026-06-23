//! Canopy server library.
//!
//! The crate is organized around domain modules that mirror the architecture
//! in the project README:
//!
//! * [`api`] — transport adapters (gRPC today, HTTP later).
//! * [`auth`], [`catalog`], [`search`], [`discovery`], [`playback`],
//!   [`providers`], [`health`] — domain services and their (planned) seams.
//! * [`jade_store`] — the persistence layer (in-memory today; PostgreSQL /
//!   RustFS planned).
//! * [`config`], [`observability`] — process wiring.
//!
//! Domain services depend on the ports defined in `canopy-core`, never on a
//! concrete backend, so storage implementations are interchangeable.

use std::{sync::Arc, time::Duration};

use canopy_core::{AudioAsset, MediaItem};
use canopy_proto::canopy_server::CanopyServer;
use tonic::transport::Server;
use tracing::info;

pub mod api;
pub mod auth;
pub mod catalog;
pub mod config;
pub mod discovery;
pub mod health;
pub mod jade_store;
pub mod observability;
pub mod playback;
pub mod providers;
pub mod search;
pub mod signing;

pub use config::Config;

use api::grpc::GrpcApi;
use catalog::CatalogService;
use discovery::DiscoveryService;
use health::HealthService;
use jade_store::{InMemoryAudioAssetStore, InMemoryCatalog, InMemorySessionStore};
use playback::{PlaybackService, ResolverConfig, ResolverService};
use search::SearchService;
use signing::HmacUrlSigner;

/// Builds the demo catalog used by the prototype.
fn demo_catalog() -> InMemoryCatalog {
    InMemoryCatalog::with_items(vec![MediaItem {
        id: "demo-1".into(),
        title: "Demo Track".into(),
        artist: "Demo Artist".into(),
        album: "Demo Album".into(),
        artwork_uri: "content://com.adrianrusu.mediaapp.audio/artwork/demo-1".into(),
        duration_ms: 240_000,
        bitrate_kbps: 320,
        mime_type: "audio/mp4".into(),
        is_explicit: false,
    }])
}

/// Builds the demo audio assets used by the prototype resolver.
fn demo_assets() -> InMemoryAudioAssetStore {
    InMemoryAudioAssetStore::with_assets(vec![AudioAsset {
        track_id: "demo-1".into(),
        codec: "mp4".into(),
        content_type: "audio/mp4".into(),
        object_key: "audio/tracks/demo-1.m4a".into(),
        size_bytes: 9_600_000,
        checksum_sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        duration_ms: 240_000,
    }])
}

/// Wires the domain services together and runs the gRPC server until shutdown.
pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // Try to connect to PostgreSQL when the `pg` feature is enabled.
    // If the connection succeeds, domain services run over the real schema.
    // If it fails (or the feature is off), we transparently fall back to the
    // in-memory prototype stores so the server can still start standalone.
    let catalog_repo: Arc<dyn canopy_core::CatalogRepository>;
    let discovery_repo: Arc<dyn canopy_core::DiscoveryRepository>;
    let asset_repo: Arc<dyn canopy_core::AudioAssetRepository>;
    let session_repo: Arc<dyn canopy_core::SessionRepository>;
    let health: HealthService;

    #[cfg(feature = "pg")]
    {
        let pool_result = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.pg_max_connections)
            .acquire_timeout(std::time::Duration::from_secs(
                config.pg_acquire_timeout_secs,
            ))
            .connect(&config.database_url)
            .await;

        match pool_result {
            Ok(pool) => {
                let pool = Arc::new(pool);
                info!(
                    database_url = %config.database_url,
                    max_connections = config.pg_max_connections,
                    "Connected to PostgreSQL; using persistent stores"
                );
                let pg_catalog = jade_store::PgCatalogRepository::new((*pool).clone());
                catalog_repo = Arc::new(pg_catalog.clone());
                discovery_repo = Arc::new(pg_catalog);
                asset_repo = Arc::new(jade_store::PgAudioAssetRepository::new((*pool).clone()));
                session_repo = Arc::new(jade_store::PgSessionRepository::new((*pool).clone()));
                health = HealthService::with_db(pool);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    database_url = %config.database_url,
                    "PostgreSQL connection failed; falling back to in-memory stores"
                );
                let catalog = demo_catalog();
                catalog_repo = Arc::new(catalog.clone());
                discovery_repo = Arc::new(catalog);
                asset_repo = Arc::new(demo_assets());
                session_repo = Arc::new(InMemorySessionStore::default());
                health = HealthService::new();
            }
        }
    }

    #[cfg(not(feature = "pg"))]
    {
        let catalog = demo_catalog();
        catalog_repo = Arc::new(catalog.clone());
        discovery_repo = Arc::new(catalog);
        asset_repo = Arc::new(demo_assets());
        session_repo = Arc::new(InMemorySessionStore::default());
        health = HealthService::new();
    }

    // Domain services over the ports.
    let catalog = CatalogService::new(catalog_repo.clone());
    let search = SearchService::new(catalog_repo);
    let playback = PlaybackService::new(session_repo);
    let discovery = DiscoveryService::new(discovery_repo);

    // Playback resolver: selects asset + mints presigned RustFS URLs from runtime config.
    let signer = Arc::new(HmacUrlSigner::new(config.rustfs_secret_key.clone()));
    let resolver = ResolverService::new(
        asset_repo,
        signer,
        ResolverConfig {
            base_url: config.rustfs_url.trim_end_matches('/').to_string(),
            bucket: config.rustfs_bucket.clone(),
            url_ttl: Duration::from_secs(15 * 60),
            ..ResolverConfig::default()
        },
    );

    // Health service is already created above (with or without DB pool).

    // gRPC adapter.
    let api = GrpcApi::new(catalog, search, playback, health, resolver, discovery);

    info!("Listening on {}", config.grpc_addr);
    Server::builder()
        .add_service(CanopyServer::new(api))
        .serve(config.grpc_addr)
        .await?;
    Ok(())
}
