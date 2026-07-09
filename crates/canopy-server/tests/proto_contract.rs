use canopy_proto::{
    auth_service_server::AuthService, catalog_service_server::CatalogService,
    discovery_service_server::DiscoveryService, history_service_server::HistoryService,
    library_service_server::LibraryService, playback_service_server::PlaybackService,
    playlist_service_server::PlaylistService, profile_service_server::ProfileService,
    system_service_server::SystemService,
};
use canopy_server::api::grpc::{
    AuthGrpc, CatalogGrpc, DiscoveryGrpc, HistoryGrpc, LibraryGrpc, PlaybackGrpc, PlaylistGrpc,
    ProfileGrpc, SystemGrpc,
};

#[test]
fn audited_v1_services_are_generated() {
    fn assert_auth<T: AuthService>() {}
    fn assert_catalog<T: CatalogService>() {}
    fn assert_discovery<T: DiscoveryService>() {}
    fn assert_history<T: HistoryService>() {}
    fn assert_library<T: LibraryService>() {}
    fn assert_playback<T: PlaybackService>() {}
    fn assert_playlist<T: PlaylistService>() {}
    fn assert_profile<T: ProfileService>() {}
    fn assert_system<T: SystemService>() {}

    assert_auth::<AuthGrpc>();
    assert_catalog::<CatalogGrpc>();
    assert_playback::<PlaybackGrpc>();
    assert_discovery::<DiscoveryGrpc>();
    assert_profile::<ProfileGrpc>();
    assert_history::<HistoryGrpc>();
    assert_library::<LibraryGrpc>();
    assert_playlist::<PlaylistGrpc>();
    assert_system::<SystemGrpc>();
}
