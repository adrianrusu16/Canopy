//! Canopy server library.
//!
//! The crate is organized around domain modules that mirror the architecture
//! in the project README:
//!
//! * [`api`] - the gRPC client adapter.
//! * [`auth`], [`catalog`], [`search`], [`discovery`], [`playback`],
//!   [`providers`], [`health`] - domain services and application boundaries.
//! * [`stream`] - private HTTP authorization for Nginx media delivery.
//! * [`jade_store`] - in-memory and PostgreSQL repository adapters.
//! * [`media`], [`config`], [`observability`] - managed media and process wiring.
//!
//! Domain services depend on the ports defined in `canopy-core`, never on a
//! concrete backend, so storage implementations are interchangeable.

use std::sync::Arc;

#[cfg(not(feature = "pg"))]
use canopy_core::{AudioAsset, MediaItem};
#[cfg(feature = "pg")]
use canopy_proto::auth_service_server::AuthServiceServer;
use canopy_proto::catalog_service_server::CatalogServiceServer;
use canopy_proto::discovery_service_server::DiscoveryServiceServer;
use canopy_proto::history_service_server::HistoryServiceServer;
use canopy_proto::library_service_server::LibraryServiceServer;
use canopy_proto::playback_service_server::PlaybackServiceServer;
use canopy_proto::playlist_service_server::PlaylistServiceServer;
use canopy_proto::profile_service_server::ProfileServiceServer;
use canopy_proto::system_service_server::SystemServiceServer;
use tonic::transport::Server;
use tracing::info;

pub mod admin;
pub mod api;
pub mod auth;
pub mod catalog;
pub mod config;
pub mod discovery;
pub mod health;
pub mod history;
pub mod identity;
pub mod jade_store;
pub mod library;
pub mod likes;
pub mod media;
pub mod observability;
pub mod owner;
pub mod playback;
pub mod playlists;
pub mod preferences;
pub mod principal;
pub mod profile;
pub mod providers;
pub mod search;
pub mod stream;

pub use config::{Config, GoogleOidcConfig, IdentityTokenConfig, StreamConfig};

#[cfg(feature = "pg")]
use api::grpc::AuthGrpc;
use api::grpc::{
    CatalogGrpc, DiscoveryGrpc, GrpcServices, HistoryGrpc, LibraryGrpc, PlaybackGrpc, PlaylistGrpc,
    ProfileGrpc, SystemGrpc,
};
use catalog::CatalogService;
use discovery::DiscoveryService;
use health::HealthService;
use history::HistoryService;
#[cfg(not(feature = "pg"))]
use jade_store::{
    InMemoryAudioAssetStore, InMemoryCatalog, InMemoryInstanceSettingsStore, InMemoryLibraryStore,
    InMemoryLikeStore, InMemoryPlaybackHistoryStore, InMemoryPlaylistStore,
    InMemoryPreferencesStore,
};
use library::LibraryService;
use likes::LikeService;
use playback::{ResolverConfig, ResolverService};
use playlists::PlaylistService;
use preferences::PreferencesService;
use principal::PrincipalService;
use profile::ProfileService;
use search::SearchService;

/// Builds the demo catalog used by the prototype.
#[cfg(not(feature = "pg"))]
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
#[cfg(not(feature = "pg"))]
fn demo_assets() -> InMemoryAudioAssetStore {
    InMemoryAudioAssetStore::with_assets(vec![AudioAsset {
        track_id: "demo-1".into(),
        codec: "mp4".into(),
        content_type: "audio/mp4".into(),
        storage_key: "audio/tracks/demo-1.m4a".into(),
        size_bytes: 9_600_000,
        checksum_sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        duration_ms: 240_000,
    }])
}

fn shutdown_channel() -> (
    tokio::sync::watch::Sender<bool>,
    tokio::sync::watch::Receiver<bool>,
    tokio::sync::watch::Receiver<bool>,
    tokio::sync::watch::Receiver<bool>,
) {
    let (sender, receiver) = tokio::sync::watch::channel(false);
    (sender, receiver.clone(), receiver.clone(), receiver)
}

async fn wait_for_shutdown(mut receiver: tokio::sync::watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}
fn listener_result<E>(
    name: &str,
    shutting_down: &std::sync::atomic::AtomicBool,
    result: Result<(), E>,
) -> std::io::Result<()>
where
    E: std::fmt::Display,
{
    result.map_err(|error| std::io::Error::other(format!("{name} listener failed: {error}")))?;
    if shutting_down.load(std::sync::atomic::Ordering::Acquire) {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{name} listener exited unexpectedly"
        )))
    }
}

async fn wait_for_termination_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

/// Wires the domain services together and runs the gRPC server until shutdown.
pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // PostgreSQL mode is production mode and fails closed on connection errors.
    // Non-PG builds retain isolated in-memory stores for local tests and demos.
    let catalog_repo: Arc<dyn canopy_core::CatalogRepository>;
    let discovery_repo: Arc<dyn canopy_core::DiscoveryRepository>;
    let playable_asset_repo: Arc<dyn canopy_core::PlayableAssetRepository>;
    let profile_repo: Arc<dyn canopy_core::ProfileRepository>;
    let instance_settings_repo: Arc<dyn canopy_core::InstanceSettingsRepository>;
    let history_repo: Arc<dyn canopy_core::PlaybackHistoryRepository>;
    let library_repo: Arc<dyn canopy_core::LibraryRepository>;
    let like_repo: Arc<dyn canopy_core::LikeRepository>;
    let preferences_repo: Arc<dyn canopy_core::PreferencesRepository>;
    let playlist_repo: Arc<dyn canopy_core::PlaylistRepository>;
    #[cfg(feature = "pg")]
    let identity_service: Arc<identity::IdentityService>;
    #[cfg(feature = "pg")]
    let email_worker: Option<identity::AuthEmailWorker>;
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
                catalog_repo = Arc::new(pg_catalog.clone());
                discovery_repo = Arc::new(pg_catalog);
                playable_asset_repo =
                    Arc::new(jade_store::PgPlayableAssetRepository::new((*pool).clone()));
                profile_repo = Arc::new(jade_store::PgProfileRepository::new((*pool).clone()));
                instance_settings_repo = Arc::new(jade_store::PgInstanceSettingsRepository::new(
                    (*pool).clone(),
                ));
                history_repo = Arc::new(jade_store::PgPlaybackHistoryRepository::new(
                    (*pool).clone(),
                ));
                library_repo = Arc::new(jade_store::PgLibraryRepository::new((*pool).clone()));
                like_repo = Arc::new(jade_store::PgLikeRepository::new((*pool).clone()));
                preferences_repo =
                    Arc::new(jade_store::PgPreferencesRepository::new((*pool).clone()));
                playlist_repo = Arc::new(jade_store::PgPlaylistRepository::new((*pool).clone()));
                let access_token_config = config.identity_tokens.access_token_config();
                let access_tokens = if let Some(signing_key) =
                    config.identity_tokens.signing_key_base64.as_deref()
                {
                    identity::Ed25519AccessTokenIssuer::from_signing_key_base64(
                        access_token_config,
                        signing_key,
                    )?
                } else if config.identity_tokens.allow_ephemeral_dev_key {
                    identity::Ed25519AccessTokenIssuer::generate(access_token_config)
                } else {
                    return Err(Box::new(canopy_core::CanopyError::InvalidArgument(
                        "identity access-token signing key is required".into(),
                    )));
                };
                let mut service = identity::IdentityService::new(
                    Arc::new(jade_store::PgIdentityRepository::new((*pool).clone())),
                    Arc::new(identity::Argon2PasswordHasher::default()),
                    access_tokens,
                    Arc::new(identity::SystemClock),
                );
                if let Some(google_oidc) = config.google_oidc.clone() {
                    service = service.with_oidc_verifier(Arc::new(
                        identity::GoogleTokenInfoOidcVerifier::new(
                            google_oidc.client_ids,
                            google_oidc.tokeninfo_url,
                        )?,
                    ));
                }
                identity_service = Arc::new(service);
                let (email_readiness, configured_email_worker) =
                    if let Some(smtp) = config.auth_email.smtp.as_ref() {
                        let readiness = identity::EmailDeliveryReadiness::configured();
                        let renderer = identity::EmailRenderer::new(
                            smtp.public_base_url.clone(),
                            &smtp.from_address,
                        )?;
                        let sender = Arc::new(identity::SmtpEmailSender::new(smtp)?);
                        let repository =
                            Arc::new(jade_store::PgAuthOutboxRepository::new((*pool).clone()));
                        let worker = identity::AuthEmailWorker::new(
                            repository,
                            sender,
                            renderer,
                            config.auth_email.worker.clone(),
                            readiness.clone(),
                        );
                        (readiness, Some(worker))
                    } else {
                        (identity::EmailDeliveryReadiness::disabled(), None)
                    };
                email_worker = configured_email_worker;
                health = HealthService::with_db(pool)
                    .with_media_root(config.media_root.clone())
                    .with_email_delivery(email_readiness);
            }
            Err(error) => return Err(Box::new(error)),
        }
    }

    #[cfg(not(feature = "pg"))]
    {
        let catalog = demo_catalog();
        catalog_repo = Arc::new(catalog.clone());
        discovery_repo = Arc::new(catalog);
        let settings = Arc::new(InMemoryInstanceSettingsStore::default());
        playable_asset_repo = Arc::new(demo_assets().with_instance_settings(settings.clone()));
        profile_repo = Arc::new(jade_store::InMemoryProfileStore::default());
        instance_settings_repo = settings;
        history_repo = Arc::new(InMemoryPlaybackHistoryStore::new(catalog_repo.clone()));
        library_repo = Arc::new(InMemoryLibraryStore::new(catalog_repo.clone()));
        like_repo = Arc::new(InMemoryLikeStore::new(catalog_repo.clone()));
        preferences_repo = Arc::new(InMemoryPreferencesStore::default());
        playlist_repo = Arc::new(InMemoryPlaylistStore::new(catalog_repo.clone()));
        health = HealthService::new()
            .with_media_root(config.media_root.clone())
            .with_email_delivery(identity::EmailDeliveryReadiness::disabled());
    }

    // Domain services over the ports.
    let catalog = CatalogService::new(catalog_repo.clone());
    let search = SearchService::new(catalog_repo.clone());
    let principal = PrincipalService::new(profile_repo.clone(), instance_settings_repo.clone());
    let profile = ProfileService::new(profile_repo.clone(), history_repo.clone())
        .with_deletion_policy(instance_settings_repo, catalog_repo);
    let history = HistoryService::new(profile_repo.clone(), history_repo, principal.clone());
    let library = LibraryService::new(profile_repo.clone(), library_repo, principal.clone());
    let likes = LikeService::new(profile_repo.clone(), like_repo, principal.clone());
    let preferences = PreferencesService::new(profile_repo.clone(), preferences_repo);
    let playlists = PlaylistService::new(profile_repo, playlist_repo, principal.clone());
    let discovery = DiscoveryService::new(discovery_repo);

    // Public playback resolves only Canopy-managed assets into opaque capabilities.
    let tokens = Arc::new(stream::StreamTokenCodec::new(&config.stream.token_secret)?);
    let resolver = ResolverService::new(
        playable_asset_repo.clone(),
        tokens.clone(),
        ResolverConfig {
            public_base_url: config.stream.public_base_url.clone(),
            token_ttl: config.stream.token_ttl,
            ..ResolverConfig::default()
        },
    );
    let authorizer = Arc::new(stream::StreamAuthorizer::new(tokens, playable_asset_repo));
    let stream_router = stream::stream_auth_router(authorizer);
    let stream_listener = tokio::net::TcpListener::bind(config.stream.auth_addr).await?;

    // Health service is already created above (with or without DB pool).

    // gRPC adapter.
    let page_tokens = Arc::new(canopy_core::PageTokenCodec::new(
        &config.stream.token_secret,
    )?);
    let services = Arc::new(GrpcServices {
        catalog,
        search,
        profile,
        history,
        library,
        likes,
        preferences,
        playlists,
        health,
        resolver,
        discovery,
        identity: {
            #[cfg(feature = "pg")]
            {
                Some(identity_service.clone())
            }
            #[cfg(not(feature = "pg"))]
            {
                None
            }
        },
        principal,
        page_tokens,
    });

    info!(grpc_addr = %config.grpc_addr, "Public gRPC listener ready");
    info!(stream_auth_addr = %config.stream.auth_addr, "Private stream authorization listener ready");

    let (shutdown_sender, grpc_shutdown, http_shutdown, email_shutdown) = shutdown_channel();
    let shutting_down = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal_state = shutting_down.clone();
    tokio::spawn(async move {
        if let Err(error) = wait_for_termination_signal().await {
            tracing::error!(%error, "Failed to monitor process termination signal");
        }
        signal_state.store(true, std::sync::atomic::Ordering::Release);
        let _ = shutdown_sender.send(true);
    });

    let grpc_state = shutting_down.clone();
    let grpc = async move {
        let router = Server::builder()
            .concurrency_limit_per_connection(64)
            .max_concurrent_streams(64)
            .load_shed(true)
            .timeout(std::time::Duration::from_secs(15))
            .add_service(CatalogServiceServer::new(CatalogGrpc(services.clone())))
            .add_service(PlaybackServiceServer::new(PlaybackGrpc(services.clone())))
            .add_service(DiscoveryServiceServer::new(DiscoveryGrpc(services.clone())))
            .add_service(ProfileServiceServer::new(ProfileGrpc(services.clone())))
            .add_service(HistoryServiceServer::new(HistoryGrpc(services.clone())))
            .add_service(LibraryServiceServer::new(LibraryGrpc(services.clone())))
            .add_service(PlaylistServiceServer::new(PlaylistGrpc(services.clone())));
        #[cfg(feature = "pg")]
        let router = router.add_service(AuthServiceServer::new(AuthGrpc(identity_service)));
        let result = router
            .add_service(SystemServiceServer::new(SystemGrpc(services)))
            .serve_with_shutdown(config.grpc_addr, wait_for_shutdown(grpc_shutdown))
            .await;
        listener_result("gRPC", &grpc_state, result)
    };

    let http_state = shutting_down;
    let http = async move {
        let result = stream::serve_stream_auth(
            stream_listener,
            stream_router,
            wait_for_shutdown(http_shutdown),
        )
        .await;
        listener_result("stream authorization", &http_state, result)
    };

    #[cfg(feature = "pg")]
    let email = async move {
        if let Some(worker) = email_worker {
            match tokio::spawn(worker.run(email_shutdown)).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(std::io::Error::other("auth email worker failed")),
                Err(_) => Err(std::io::Error::other("auth email worker task failed")),
            }
        } else {
            wait_for_shutdown(email_shutdown).await;
            Ok(())
        }
    };

    #[cfg(not(feature = "pg"))]
    let email = async move {
        wait_for_shutdown(email_shutdown).await;
        Ok::<(), std::io::Error>(())
    };

    tokio::try_join!(grpc, http, email)?;
    Ok(())
}

#[cfg(test)]
mod wiring_tests {
    use std::sync::atomic::AtomicBool;

    use super::{listener_result, shutdown_channel, wait_for_shutdown};

    #[test]
    fn listener_exit_is_only_clean_during_shutdown() {
        let running = AtomicBool::new(false);
        assert!(listener_result("grpc", &running, Ok::<(), std::io::Error>(())).is_err());

        let stopping = AtomicBool::new(true);
        assert!(listener_result("grpc", &stopping, Ok::<(), std::io::Error>(())).is_ok());
    }

    #[tokio::test]
    async fn shutdown_notification_reaches_all_runtime_tasks() {
        let (sender, grpc, http, email) = shutdown_channel();
        sender.send(true).unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            tokio::join!(
                wait_for_shutdown(grpc),
                wait_for_shutdown(http),
                wait_for_shutdown(email)
            );
        })
        .await
        .unwrap();
    }
}
