//! Task 2: static ACL permission-matrix tests (D237 + E2 extension).

use mediaservo_link::{FrameTopic, NodeAcl, NodeId, Role};

fn acl(role: Role) -> NodeAcl {
    NodeAcl::for_role(NodeId::new("n0"), role)
}

#[test]
fn capture_pub_camera_not_control_no_sub() {
    let a = acl(Role::Capture);
    assert!(a.can_publish(&FrameTopic::new("camera/front/raw")));
    assert!(!a.can_publish(&FrameTopic::new("control/cmd")));
    assert!(!a.can_subscribe(&FrameTopic::new("camera/front/raw")), "capture must not subscribe");
}

#[test]
fn processor_sub_camera_pub_video() {
    let a = acl(Role::Processor);
    assert!(a.can_subscribe(&FrameTopic::new("camera/front/raw")));
    assert!(a.can_publish(&FrameTopic::new("video/stitched")), "derived topic (D239)");
    assert!(!a.can_publish(&FrameTopic::new("control/cmd")));
}

#[test]
fn pusher_sub_not_pub() {
    let a = acl(Role::Pusher);
    assert!(a.can_subscribe(&FrameTopic::new("camera/front/raw")));
    assert!(a.can_subscribe(&FrameTopic::new("video/stitched")));
    // F3: pusher node subscribes vision results (vision/<camera-id>, D-H8 chain)
    assert!(a.can_subscribe(&FrameTopic::new("vision/cam0")));
    // stats/* publish permission (push-status reporting, E2 design)
    assert!(a.can_publish(&FrameTopic::new("stats/stream-s0")));
    assert!(a.can_publish(&FrameTopic::new("stats/x")));
    // no camera/video/vision publish rights
    assert!(!a.can_publish(&FrameTopic::new("camera/front/raw")));
    assert!(!a.can_publish(&FrameTopic::new("video/x")));
    // one-way contract (host.rs build_acl passes explicit --topic for subscriber roles)
    assert!(!a.can_subscribe(&FrameTopic::new("control/cmd")));
}

#[test]
fn recorder_sub_not_pub() {
    let a = acl(Role::Recorder);
    assert!(a.can_subscribe(&FrameTopic::new("camera/front/raw")));
    assert!(a.can_subscribe(&FrameTopic::new("video/stitched")));
    // F3: streamer token defaults to Recorder (C2 legacy) -> subscribes vision results (D-H8)
    assert!(a.can_subscribe(&FrameTopic::new("vision/cam0")));
    assert!(!a.can_publish(&FrameTopic::new("camera/front/raw")));
    // E2 push-status reporting (streamer token defaults to Recorder, C2 legacy)
    assert!(a.can_publish(&FrameTopic::new("stats/stream-s0")));
}

#[test]
fn control_pub_cmd_sub_telemetry_status() {
    let a = acl(Role::Control);
    assert!(a.can_publish(&FrameTopic::new("control/cmd")));
    assert!(!a.can_publish(&FrameTopic::new("camera/front/raw")));
    assert!(a.can_subscribe(&FrameTopic::new("control/telemetry")));
    assert!(a.can_subscribe(&FrameTopic::new("status/x")));
    assert!(!a.can_subscribe(&FrameTopic::new("camera/front/raw")));
}

#[test]
fn perception_pub_perception_sub_camera() {
    let a = acl(Role::Perception);
    assert!(a.can_publish(&FrameTopic::new("perception/objects")));
    // F3: ROS vision node publishes vision/<camera-id> (D-H7/D-H8, single-source bridge config)
    assert!(a.can_publish(&FrameTopic::new("vision/cam0")));
    assert!(a.can_subscribe(&FrameTopic::new("camera/front/raw")));
    assert!(!a.can_publish(&FrameTopic::new("video/x")));
}

#[test]
fn puller_no_perm() {
    let a = acl(Role::Puller);
    assert!(!a.can_publish(&FrameTopic::new("camera/x")));
    assert!(!a.can_subscribe(&FrameTopic::new("camera/x")));
}

#[test]
fn monitor_sub_frames_and_stats_no_pub() {
    let a = acl(Role::Monitor);
    assert!(a.can_subscribe(&FrameTopic::new("camera/front/raw")));
    assert!(a.can_subscribe(&FrameTopic::new("stats/stream-s0")));
    assert!(!a.can_subscribe(&FrameTopic::new("control/cmd")));
    assert!(!a.can_publish(&FrameTopic::new("camera/front/raw")));
    assert!(!a.can_publish(&FrameTopic::new("stats/stream-s0")));
}
