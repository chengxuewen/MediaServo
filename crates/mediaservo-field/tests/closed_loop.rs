//! Field composition SDK tests: re-export completeness + error delegation + session stub behavior.

use mediaservo_field::{
    CameraSource, CaptureOptions, DeviceId, FieldError, FrameBus, MediaDeviceKind, MediaDevices,
    NodeAcl, Player, Recorder, Role, SignalClient,
};

#[test]
fn reexports_bring_entire_closed_loop() {
    // Link surface: signaling + frame bus types reachable
    let _: Option<SignalClient> = None;
    let _: Option<FrameBus> = None;

    // Deck surface: capture/record/playback types reachable
    let cams = MediaDevices::enumerate(MediaDeviceKind::Camera);
    assert_eq!(cams.len(), 1);
    let _ = CameraSource::open(DeviceId("stub:test-camera".into()), CaptureOptions::default());
    let _: Option<Player> = None;
    let _: Option<Recorder> = None;
}

#[test]
fn link_error_flows_into_field_error() {
    // FieldError: From<LinkError> delegation (a bad token triggers LinkError::Attach)
    let err: FieldError = mediaservo_link::LinkError::Attach("bad token".into()).into();
    assert!(matches!(err, FieldError::Link(_)));
}

#[tokio::test]
async fn session_connect_requires_cfg() {
    // connect now requires PushConfig/PullConfig (the signaling connection must know the server/room)
    use mediaservo_field::{PullConfig, PushConfig};

    let push_cfg = PushConfig::new("ws://127.0.0.1:9800/ws", "psk", "room");
    assert_eq!(push_cfg.width, 1280);
    assert_eq!(push_cfg.framerate, 30);

    let pull_cfg = PullConfig::default();
    assert!(pull_cfg.auto_subscribe);

    // Connection-failure path: LinkError when no server (not InvalidState, Phase 2)
    let err = mediaservo_field::PushSession::connect(push_cfg).await;
    assert!(matches!(err, Err(FieldError::Link(_))), "got {err:?}");
    let _ = pull_cfg;
}

#[test]
fn link_token_types_reexported() {
    // Capability token API reachable via link (composition SDK auth surface)
    let node = mediaservo_field::NodeId::new("field-test");
    let acl = NodeAcl::for_role(node.clone(), Role::Pusher);
    assert!(acl.can_subscribe(&mediaservo_field::FrameTopic::new("camera/*")));
    let _sk = mediaservo_field::Ed25519SigningKey::from_pem(
        b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIObCg8b+Le6kKOI/+pE+4+YhXUlr6X6h7q8p/MjvHmXT\n-----END PRIVATE KEY-----\n",
    );
    let _vk = mediaservo_field::Ed25519VerifyingKey::from_pem(
        b"-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAgXprEbnahCZoZtLpiUqR0ruqtzEfRXk/Gl/6F6PEm4o=\n-----END PUBLIC KEY-----\n",
    );
}
