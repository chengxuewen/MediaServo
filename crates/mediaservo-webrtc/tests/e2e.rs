//! E2E test: RTCPeerConnection + RTCDataChannel creation and SDP exchange.
//! Run with: cargo test -p mediaservo-webrtc --features backend-webrtc-rs

#[cfg(feature = "backend-webrtc-rs")]
mod tests {
    use mediaservo_webrtc::data_channel::{
        RTCDataChannel, RTCDataChannelInit, RTCDataChannelState,
    };
    use mediaservo_webrtc::traits::PeerConnectionApi;
    use mediaservo_webrtc::*;

    #[tokio::test]
    async fn e2e_create_and_exchange_sdp() {
        let factory = RTCPeerConnectionFactory::new();

        let pc_a =
            factory.create_peer_connection(RTCConfiguration::default()).await.expect("create pc_a");
        let pc_b =
            factory.create_peer_connection(RTCConfiguration::default()).await.expect("create pc_b");

        // Verify initial states
        assert!(matches!(pc_a.connection_state(), RTCPeerConnectionState::New));
        assert!(matches!(pc_b.connection_state(), RTCPeerConnectionState::New));

        // A creates data channel
        let mut dc_a = pc_a
            .create_data_channel("test", RTCDataChannelInit::default())
            .await
            .expect("create data channel");

        assert_eq!(dc_a.label(), "test");
        assert!(matches!(dc_a.state(), RTCDataChannelState::Connecting));

        // SDP exchange
        let offer = pc_a.create_offer(&RTCOfferOptions::default()).await.expect("create offer");
        assert!(!offer.sdp.is_empty());
        pc_a.set_local_description(&offer).await.expect("set local");
        pc_b.set_remote_description(&offer).await.expect("set remote");

        let answer = pc_b.create_answer(&RTCAnswerOptions::default()).await.expect("create answer");
        assert!(!answer.sdp.is_empty());
        pc_b.set_local_description(&answer).await.expect("set local answer");
        pc_a.set_remote_description(&answer).await.expect("set remote answer");

        // Wait for ICE gathering
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Verify PC states — just ensure no failures
        assert!(!matches!(pc_a.connection_state(), RTCPeerConnectionState::Failed));
        assert!(!matches!(pc_b.connection_state(), RTCPeerConnectionState::Failed));

        // Clean up
        dc_a.close().await;
        pc_a.close().await;
        pc_b.close().await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
