//! offerer_prefs_test - core mechanism check of setCodecPreferences in offerer mode
//!
//! T5 empirical conclusions (set-codec-preferences plan):
//!   - offerer mode: setting preferences by track_id takes effect - create_offer codec order is reordered per preferences
//!     (all H264 before VP8, measured)
//!   - answerer mode (SFU server-offer): set succeeds but the answer is unaffected
//!     (libwebrtc intersects by offer order) -> pinning a codec needs reduceCodecs (mediasoup official pattern)
//!   - mid parameterization infeasible: transceivers have no mid before negotiation (core offerer case) -> locate by track_id

// Quick experiment: does setCodecPreferences in offerer mode affect the create_offer codec order
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn offerer_prefs_reorders_offer() {
    use mediaservo_webrtc::traits::PeerConnectionApi;
    use mediaservo_webrtc::{
        RTCConfiguration, RTCOfferOptions, RTCPeerConnectionFactory, TrackKind,
    };
    let pc = RTCPeerConnectionFactory::new()
        .create_peer_connection(RTCConfiguration::default())
        .await
        .expect("PC");
    let track_id = pc.add_track("video", TrackKind::Video).expect("add_track");
    let _ = &track_id;

    // prefer H264 first
    let caps = pc.get_sender_capabilities(TrackKind::Video).expect("caps").unwrap();
    let mut codecs = caps.codecs.clone();
    codecs.sort_by_key(|c| if c.mime_type.starts_with("video/H264") { 0 } else { 1 });
    codecs
        .retain(|c| c.mime_type.starts_with("video/H264") || c.mime_type.starts_with("video/VP8"));
    let r = pc.transceiver_set_codec_preferences(&track_id, codecs);
    println!("set_codec_preferences: {r:?}");
    assert!(r.is_ok(), "set failed: {r:?}");

    let offer = pc.create_offer(&RTCOfferOptions::default()).await.expect("offer");
    // print the offer's codec order
    for line in offer.sdp.lines() {
        if line.starts_with("a=rtpmap:") || line.starts_with("m=video") {
            println!("{line}");
        }
    }
    // assert H264 precedes VP8 in the offer
    let h264_pos = offer.sdp.find("H264").unwrap_or(usize::MAX);
    let vp8_pos = offer.sdp.find("VP8").unwrap_or(usize::MAX);
    assert!(h264_pos < vp8_pos, "offer should have H264 first: h264={h264_pos} vp8={vp8_pos}");
}
