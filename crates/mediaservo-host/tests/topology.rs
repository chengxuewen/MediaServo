//! Task E1: topology monitor unit tests (monitor/topology.rs).
//!
//! Desired state (host.yaml declarations) vs actual state (oxmgr list processes + FrameBus publisher enumeration)
//! -> diff report + grace period suppressing the startup window. Pure unit tests, no dependency on the oxmgr daemon/
//! or iceoryx2 runtime (the discovery data source itself is covered by mediaservo-link tests/discovery.rs).

use mediaservo_host::monitor::topology::{
    Mismatch, OxProcess, TopologyMonitor, diff, parse_oxmgr_json,
};
use mediaservo_link::FrameTopic;

/// Typical host.yaml: 3 sources + 1 streamer + [record].
fn sample_cfg() -> &'static str {
    "sources:\n  - id: \"cam0\"\n  - id: \"cam1\"\n  - id: \"cam2\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n"
}

#[test]
fn expected_process_names_covers_fixed_and_instances() {
    let names = mediaservo_host::translate::expected_process_names(sample_cfg()).unwrap();
    // 5 fixed processes minus recorder ([record] defaults to disabled -> exits 0 by design without restart, E1 review)
    // + 3 capturer instances (multi-instance -> id suffix) + 1 streamer (single instance -> no suffix)
    for want in [
        "host-agent",
        "host-controller",
        "host-emergency",
        "host-audio",
        "host-capturer-cam0",
        "host-capturer-cam1",
        "host-capturer-cam2",
        "host-streamer-s0",
    ] {
        assert!(names.contains(&want.to_string()), "missing expected process {want}: {names:?}");
    }
    assert!(
        !names.contains(&"host-recorder".to_string()),
        "with record disabled the recorder must not be expected: {names:?}"
    );
    assert_eq!(names.len(), 8);
}

#[test]
fn expected_processes_include_recorder_only_when_record_enabled() {
    // E1 review: [record] enabled=false (default) -> host-recorder exits 0 by design and oxmgr
    // on_failure does not restart it -> excluded from expectations, else the default config reports a permanent ProcessMissing false alarm.
    let disabled = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: false\n";
    let names = mediaservo_host::translate::expected_process_names(disabled).unwrap();
    assert!(
        !names.contains(&"host-recorder".to_string()),
        "enabled=false means the recorder must not be expected: {names:?}"
    );

    let enabled = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: true\n  out_dir: \"/tmp/x\"\n";
    let names = mediaservo_host::translate::expected_process_names(enabled).unwrap();
    assert!(
        names.contains(&"host-recorder".to_string()),
        "enabled=true should expect the recorder: {names:?}"
    );
}

#[test]
fn diff_reports_missing_process_and_missing_publisher() {
    // Expect 3 capturers, actually only 2 running + 1 stopped -> reported missing
    let expected = mediaservo_host::translate::expected_process_names(sample_cfg()).unwrap();
    let actual = vec![
        OxProcess { name: "host-capturer-cam0".into(), status: "running".into() },
        OxProcess { name: "host-capturer-cam1".into(), status: "running".into() },
        OxProcess { name: "host-capturer-cam2".into(), status: "stopped".into() },
    ];
    let expected_topics = vec![
        FrameTopic::new("camera/cam0"),
        FrameTopic::new("camera/cam1"),
        FrameTopic::new("camera/cam2"),
    ];
    let actual_topics = vec![FrameTopic::new("camera/cam0")]; // only cam0 has an active publisher

    let m = diff(&expected, &expected_topics, &actual, &actual_topics);
    assert!(
        m.contains(&Mismatch::ProcessMissing { name: "host-capturer-cam2".into() }),
        "a stopped process should be reported missing: {m:?}"
    );
    assert!(
        m.contains(&Mismatch::PublisherMissing { topic: "camera/cam1".into() })
            && m.contains(&Mismatch::PublisherMissing { topic: "camera/cam2".into() }),
        "topics without an active publisher should be reported missing: {m:?}"
    );
    // no false alarms on present items
    assert!(!m.contains(&Mismatch::ProcessMissing { name: "host-capturer-cam0".into() }));
    assert!(
        !m.iter()
            .any(|x| matches!(x, Mismatch::PublisherMissing { topic } if topic == "camera/cam0"))
    );
}

#[test]
fn diff_empty_when_all_present() {
    let expected = mediaservo_host::translate::expected_process_names(sample_cfg()).unwrap();
    let actual: Vec<OxProcess> =
        expected.iter().map(|n| OxProcess { name: n.clone(), status: "running".into() }).collect();
    let expected_topics = vec![FrameTopic::new("camera/cam0")];
    let actual_topics = expected_topics.clone();
    assert!(diff(&expected, &expected_topics, &actual, &actual_topics).is_empty());
}

#[test]
fn parse_oxmgr_json_extracts_name_and_status() {
    // Measured oxmgr 0.5.0 list --json output (field superset, unknown fields ignored)
    let json = r#"[
      {"id":317,"name":"host-capturer-cam0","command":"sleep","args":["300"],"namespace":null,"restart_policy":"on_failure","pid":641299,"status":"running","desired_state":"running","last_exit_code":null},
      {"id":318,"name":"host-recorder","command":"sleep","args":["300"],"namespace":"host","restart_policy":"on_failure","pid":641300,"status":"stopped","desired_state":"running","last_exit_code":0}
    ]"#;
    let procs = parse_oxmgr_json(json).unwrap();
    assert_eq!(procs.len(), 2);
    assert_eq!(procs[0], OxProcess { name: "host-capturer-cam0".into(), status: "running".into() });
    assert_eq!(procs[1], OxProcess { name: "host-recorder".into(), status: "stopped".into() });
}

#[test]
fn parse_oxmgr_json_rejects_garbage() {
    assert!(parse_oxmgr_json("not json").is_err());
    assert!(parse_oxmgr_json("[]").unwrap().is_empty());
}

#[test]
fn grace_period_suppresses_startup_window() {
    // grace not elapsed -> grace_active=true (startup window)
    let m =
        TopologyMonitor::new_for_test(sample_cfg().to_string(), std::time::Duration::from_secs(60));
    assert!(m.grace_active(), "within the 60s grace it should be in the startup window");
    // grace=0 -> leave the startup window immediately
    let m = TopologyMonitor::new_for_test(sample_cfg().to_string(), std::time::Duration::ZERO);
    assert!(!m.grace_active(), "grace=0 should have no startup window");
}
