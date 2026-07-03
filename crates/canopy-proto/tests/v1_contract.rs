use canopy_proto::{ArtworkRef, PageInfo, PageRequest, ResolvePlaybackRequest, TrackSummary};

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
