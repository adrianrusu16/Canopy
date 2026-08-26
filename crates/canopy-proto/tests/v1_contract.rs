use canopy_proto::{
    ArtworkRef, GetForYouFeedRequest, GetForYouFeedResponse, GetRecommendationsRequest,
    GetRecommendationsResponse, PageInfo, PageRequest, ResolvePlaybackRequest, TrackSummary,
    auth_service_client::AuthServiceClient, catalog_service_client::CatalogServiceClient,
    discovery_service_client::DiscoveryServiceClient, history_service_client::HistoryServiceClient,
    library_service_client::LibraryServiceClient, playback_service_client::PlaybackServiceClient,
    playlist_service_client::PlaylistServiceClient, profile_service_client::ProfileServiceClient,
    system_service_client::SystemServiceClient,
};

#[test]
fn audited_v1_common_messages_have_platform_neutral_shapes() {
    let page = PageRequest {
        page_size: 25,
        page_token: "opaque".into(),
    };
    let page_info = PageInfo {
        next_page_token: "next".into(),
    };
    let track = TrackSummary {
        id: "track-1".into(),
        title: "Track".into(),
        artist: None,
        album: None,
        duration_ms: 1_000,
        explicit: false,
        artwork: Some(ArtworkRef {
            id: "artwork-1".into(),
            content_hash: "a".repeat(64),
        }),
    };
    let playback = ResolvePlaybackRequest {
        track_id: track.id.clone(),
    };

    assert_eq!(page.page_size, 25);
    assert_eq!(page_info.next_page_token, "next");
    assert_eq!(playback.track_id, "track-1");
    assert_eq!(track.artwork.unwrap().id, "artwork-1");
}

#[test]
fn audited_v1_exposes_independent_discovery_feed_messages() {
    let page = Some(PageRequest {
        page_size: 25,
        page_token: String::new(),
    });

    let for_you = GetForYouFeedRequest {
        exclude_track_ids: vec!["track-1".into()],
        page: page.clone(),
    };
    let recommendations = GetRecommendationsRequest {
        exclude_track_ids: vec!["track-1".into()],
        page,
    };

    let for_you_response = GetForYouFeedResponse {
        tracks: Vec::new(),
        page_info: None,
    };
    let recommendations_response = GetRecommendationsResponse {
        tracks: Vec::new(),
        page_info: None,
    };

    assert_eq!(for_you.exclude_track_ids, vec!["track-1"]);
    assert_eq!(recommendations.exclude_track_ids, vec!["track-1"]);
    assert!(for_you_response.tracks.is_empty());
    assert!(recommendations_response.tracks.is_empty());
}

#[allow(dead_code)]
fn generated_discovery_client_has_named_feed_methods(
    client: &mut DiscoveryServiceClient<tonic::transport::Channel>,
    for_you: GetForYouFeedRequest,
    recommendations: GetRecommendationsRequest,
) {
    std::mem::drop(client.get_for_you_feed(for_you));
    std::mem::drop(client.get_recommendations(recommendations));
}

#[test]
fn audited_v1_exposes_every_bounded_service_client() {
    fn accepts<T>(_client: Option<T>) {}

    accepts::<CatalogServiceClient<tonic::transport::Channel>>(None);
    accepts::<PlaybackServiceClient<tonic::transport::Channel>>(None);
    accepts::<DiscoveryServiceClient<tonic::transport::Channel>>(None);
    accepts::<ProfileServiceClient<tonic::transport::Channel>>(None);
    accepts::<HistoryServiceClient<tonic::transport::Channel>>(None);
    accepts::<LibraryServiceClient<tonic::transport::Channel>>(None);
    accepts::<PlaylistServiceClient<tonic::transport::Channel>>(None);
    accepts::<AuthServiceClient<tonic::transport::Channel>>(None);
    accepts::<SystemServiceClient<tonic::transport::Channel>>(None);
}
