//! Canopy server library.
//!
//! The crate is organized around domain modules that mirror the architecture
//! in the project README:
//!
//! * [`api`] - transport adapters (gRPC today, HTTP later).
//! * [`auth`], [`catalog`], [`search`], [`discovery`], [`playback`],
//!   [`providers`], [`health`] - domain services and their (planned) seams.
//! * [`jade_store`] - the persistence layer (in-memory today; PostgreSQL /
//!   RustFS planned).
//! * [`config`], [`observability`] - process wiring.
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
pub mod history;
pub mod jade_store;
pub mod library;
pub mod likes;
pub mod observability;
pub mod playback;
pub mod preferences;
pub mod profile;
pub mod providers;
pub mod search;
pub mod signing;
pub mod supabase;

pub use config::{Config, MusicSource};

use api::grpc::{GrpcApi, GrpcServices};
use auth::AuthService;
use catalog::CatalogService;
use discovery::DiscoveryService;
use health::HealthService;
use history::HistoryService;
use jade_store::{
    InMemoryAudioAssetStore, InMemoryCatalog, InMemoryLibraryStore, InMemoryLikeStore,
    InMemoryPlaybackHistoryStore, InMemoryPreferencesStore, InMemorySessionStore,
};
use library::LibraryService;
use likes::LikeService;
use playback::{PlaybackService, ResolverConfig, ResolverService};
use preferences::PreferencesService;
use profile::ProfileService;
use search::SearchService;
use signing::HmacUrlSigner;
use supabase::{SupabaseConfig, SupabaseStorageUrlProvider};

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
    let profile_repo: Arc<dyn canopy_core::ProfileRepository>;
    let history_repo: Arc<dyn canopy_core::PlaybackHistoryRepository>;
    let library_repo: Arc<dyn canopy_core::LibraryRepository>;
    let like_repo: Arc<dyn canopy_core::LikeRepository>;
    let preferences_repo: Arc<dyn canopy_core::PreferencesRepository>;
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
                if let Some(path) = config.provider_fixture_path.as_deref() {
                    let fixture = providers::TestFixtureProvider::new(path)?;
                    let ingestion = providers::IngestionService::new(Arc::new(pg_catalog.clone()));
                    let result = ingestion.ingest_from_provider(&fixture).await?;
                    info!(
                        fixture_path = %path,
                        succeeded = result.succeeded,
                        failed = result.failed,
                        "Ingested provider fixture into PostgreSQL catalog"
                    );
                    if !result.failures.is_empty() {
                        tracing::warn!(failures = ?result.failures, "Provider fixture ingest had failures");
                    }
                }
                if config.music_source == MusicSource::Supabase && config.supabase_sync_on_start {
                    let provider =
                        providers::SupabaseCatalogProvider::new(providers::SupabaseCatalogConfig {
                            project_url: config.supabase_url.clone(),
                            api_key: config.supabase_key.clone(),
                            table: config.supabase_catalog_table.clone(),
                        });
                    let ingestion = providers::IngestionService::new(Arc::new(pg_catalog.clone()));
                    let result = ingestion.ingest_from_provider(&provider).await?;
                    info!(
                        table = %config.supabase_catalog_table,
                        succeeded = result.succeeded,
                        failed = result.failed,
                        "Ingested Supabase catalog into PostgreSQL catalog"
                    );
                    if !result.failures.is_empty() {
                        tracing::warn!(failures = ?result.failures, "Supabase catalog ingest had failures");
                    }
                }
                catalog_repo = Arc::new(pg_catalog.clone());
                discovery_repo = Arc::new(pg_catalog);
                asset_repo = Arc::new(jade_store::PgAudioAssetRepository::new((*pool).clone()));
                session_repo = Arc::new(jade_store::PgSessionRepository::new((*pool).clone()));
                profile_repo = Arc::new(jade_store::PgProfileRepository::new((*pool).clone()));
                history_repo = Arc::new(jade_store::PgPlaybackHistoryRepository::new(
                    (*pool).clone(),
                ));
                library_repo = Arc::new(jade_store::PgLibraryRepository::new((*pool).clone()));
                like_repo = Arc::new(jade_store::PgLikeRepository::new((*pool).clone()));
                preferences_repo =
                    Arc::new(jade_store::PgPreferencesRepository::new((*pool).clone()));
                health = HealthService::with_db(pool).with_rustfs(
                    config
                        .health_check_rustfs
                        .then(|| config.rustfs_url.clone()),
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    database_url = %config.database_url,
                    "PostgreSQL connection failed; falling back to in-memory stores"
                );
                if config.provider_fixture_path.is_some() {
                    tracing::warn!(
                        "Skipping provider fixture ingest because PostgreSQL is unavailable"
                    );
                }
                let catalog = demo_catalog();
                catalog_repo = Arc::new(catalog.clone());
                discovery_repo = Arc::new(catalog);
                asset_repo = Arc::new(demo_assets());
                session_repo = Arc::new(InMemorySessionStore::default());
                profile_repo = Arc::new(jade_store::InMemoryProfileStore::default());
                history_repo = Arc::new(InMemoryPlaybackHistoryStore::default());
                library_repo = Arc::new(InMemoryLibraryStore::default());
                like_repo = Arc::new(InMemoryLikeStore::default());
                preferences_repo = Arc::new(InMemoryPreferencesStore::default());
                health = HealthService::new().with_rustfs(
                    config
                        .health_check_rustfs
                        .then(|| config.rustfs_url.clone()),
                );
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
        profile_repo = Arc::new(jade_store::InMemoryProfileStore::default());
        history_repo = Arc::new(InMemoryPlaybackHistoryStore::default());
        library_repo = Arc::new(InMemoryLibraryStore::default());
        like_repo = Arc::new(InMemoryLikeStore::default());
        preferences_repo = Arc::new(InMemoryPreferencesStore::default());
        health = HealthService::new().with_rustfs(
            config
                .health_check_rustfs
                .then(|| config.rustfs_url.clone()),
        );
    }

    // Domain services over the ports.
    let catalog = CatalogService::new(catalog_repo.clone());
    let search = SearchService::new(catalog_repo);
    let playback = PlaybackService::new(session_repo);
    let auth = AuthService::new(config.auth_token_secret.clone());
    let profile = ProfileService::new(profile_repo.clone());
    let history = HistoryService::new(profile_repo.clone(), history_repo);
    let library = LibraryService::new(profile_repo.clone(), library_repo);
    let likes = LikeService::new(profile_repo.clone(), like_repo);
    let preferences = PreferencesService::new(profile_repo, preferences_repo);
    let discovery = DiscoveryService::new(discovery_repo);

    // Playback resolver: selects an asset and mints a short-lived stream URL.
    let resolver_config = ResolverConfig {
        base_url: config.rustfs_url.trim_end_matches('/').to_string(),
        bucket: config.rustfs_bucket.clone(),
        url_ttl: Duration::from_secs(match config.music_source {
            MusicSource::Rustfs => 15 * 60,
            MusicSource::Supabase => config.supabase_signed_url_ttl_secs,
        }),
        ..ResolverConfig::default()
    };
    let resolver = match config.music_source {
        MusicSource::Rustfs => {
            let signer = Arc::new(HmacUrlSigner::new(config.rustfs_secret_key.clone()));
            ResolverService::new(asset_repo, signer, resolver_config)
        }
        MusicSource::Supabase => ResolverService::with_url_provider(
            asset_repo,
            Arc::new(SupabaseStorageUrlProvider::new(SupabaseConfig {
                project_url: config.supabase_url.clone(),
                api_key: config.supabase_key.clone(),
                bucket: config.supabase_storage_bucket.clone(),
                signed_url_ttl_secs: config.supabase_signed_url_ttl_secs,
            })),
            resolver_config,
        ),
    };

    // Health service is already created above (with or without DB pool).

    // gRPC adapter.
    let api = GrpcApi::new(GrpcServices {
        catalog,
        search,
        playback,
        profile,
        history,
        library,
        likes,
        preferences,
        health,
        resolver,
        discovery,
        auth,
    });

    info!("Listening on {}", config.grpc_addr);
    Server::builder()
        .add_service(CanopyServer::new(api))
        .serve(config.grpc_addr)
        .await?;
    Ok(())
}
