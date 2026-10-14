//! ros_bridge.yaml generation tests (Task B3): topic lists + token_path single source.

use mediaservo_link::bridge::ros_bridge;

#[test]
fn output_contains_all_camera_vision_stream_topics_and_token_path() {
    let yaml = ros_bridge(
        &["cam0".to_string(), "cam1".to_string()],
        &["cam0-stream".to_string()],
        "/opt/mediaservo-host/etc/link/ros-vision.token",
    );
    for topic in ["camera/cam0", "camera/cam1", "vision/cam0", "vision/cam1", "stream/cam0-stream"]
    {
        assert!(yaml.contains(topic), "missing topic {topic}:\n{yaml}");
    }
    assert!(
        yaml.contains("token_path: /opt/mediaservo-host/etc/link/ros-vision.token"),
        "missing token_path:\n{yaml}"
    );
}

#[test]
fn vision_topics_mirror_camera_ids_not_stream_ids() {
    let yaml =
        ros_bridge(&["cam0".to_string(), "cam1".to_string()], &["cam0-stream".to_string()], "tok");
    assert!(yaml.contains("vision/cam0") && yaml.contains("vision/cam1"));
    assert!(!yaml.contains("vision/cam0-stream"));
}

#[test]
fn empty_lists_emit_sections_without_items() {
    let yaml = ros_bridge(&[], &[], "tok");
    assert!(yaml.contains("camera:") && yaml.contains("vision:") && yaml.contains("stream:"));
    // no items means no topic lines are emitted
    let topic_lines = yaml.lines().filter(|l| l.trim_start().starts_with("- ")).count();
    assert_eq!(topic_lines, 0, "empty lists must yield no topic lines:\n{yaml}");
}
