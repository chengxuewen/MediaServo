//! Task E1: 拓扑监控单元测试（monitor/topology.rs）。
//!
//! 期望态（host.yaml 声明）vs 实际态（oxmgr list 进程 + FrameBus 发布者枚举）
//! → diff 报告 + grace period 抑制启动窗口。纯单元测试，不依赖 oxmgr daemon/
//! iceoryx2 运行时（发现数据源本身在 mediaservo-link tests/discovery.rs 覆盖）。

use mediaservo_host::monitor::topology::{diff, parse_oxmgr_json, Mismatch, OxProcess, TopologyMonitor};
use mediaservo_link::FrameTopic;

/// 3 source + 1 streamer + [record] 的典型 host.yaml。
fn sample_cfg() -> &'static str {
    "sources:\n  - id: \"cam0\"\n  - id: \"cam1\"\n  - id: \"cam2\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n"
}

#[test]
fn expected_process_names_covers_fixed_and_instances() {
    let names = mediaservo_host::translate::expected_process_names(sample_cfg()).unwrap();
    // 5 固定进程 − recorder（[record] 缺省 disabled → 按设计 exit 0 不重启, E1 审查）
    // + 3 capturer 实例（多实例 → 带 id 后缀）+ 1 streamer（单实例 → 无后缀）
    for want in [
        "host-agent", "host-controller", "host-emergency", "host-audio",
        "host-capturer-cam0", "host-capturer-cam1", "host-capturer-cam2",
        "host-streamer-s0",
    ] {
        assert!(names.contains(&want.to_string()), "缺少期望进程 {want}: {names:?}");
    }
    assert!(!names.contains(&"host-recorder".to_string()), "record 未启用不应期望 recorder: {names:?}");
    assert_eq!(names.len(), 8);
}

#[test]
fn expected_processes_include_recorder_only_when_record_enabled() {
    // E1 审查: [record] enabled=false（缺省）→ host-recorder 按设计 exit 0 且 oxmgr
    // on_failure 不重启 → 不列入期望，否则默认配置永久 ProcessMissing 误报。
    let disabled = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: false\n";
    let names = mediaservo_host::translate::expected_process_names(disabled).unwrap();
    assert!(!names.contains(&"host-recorder".to_string()), "enabled=false 不应期望 recorder: {names:?}");

    let enabled = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: true\n  out_dir: \"/tmp/x\"\n";
    let names = mediaservo_host::translate::expected_process_names(enabled).unwrap();
    assert!(names.contains(&"host-recorder".to_string()), "enabled=true 应期望 recorder: {names:?}");
}

#[test]
fn diff_reports_missing_process_and_missing_publisher() {
    // 期望 3 capturer，实际只有 2 个 running + 1 个 stopped → 报告缺失
    let expected = mediaservo_host::translate::expected_process_names(sample_cfg()).unwrap();
    let actual = vec![
        OxProcess { name: "host-capturer-cam0".into(), status: "running".into() },
        OxProcess { name: "host-capturer-cam1".into(), status: "running".into() },
        OxProcess { name: "host-capturer-cam2".into(), status: "stopped".into() },
    ];
    let expected_topics = vec![
        FrameTopic::new("camera/cam0"), FrameTopic::new("camera/cam1"), FrameTopic::new("camera/cam2"),
    ];
    let actual_topics = vec![FrameTopic::new("camera/cam0")]; // 仅 cam0 有活跃发布者

    let m = diff(&expected, &expected_topics, &actual, &actual_topics);
    assert!(
        m.contains(&Mismatch::ProcessMissing { name: "host-capturer-cam2".into() }),
        "stopped 进程应报缺失: {m:?}"
    );
    assert!(
        m.contains(&Mismatch::PublisherMissing { topic: "camera/cam1".into() })
            && m.contains(&Mismatch::PublisherMissing { topic: "camera/cam2".into() }),
        "无活跃发布者的 topic 应报缺失: {m:?}"
    );
    // 不误报存在项
    assert!(!m.contains(&Mismatch::ProcessMissing { name: "host-capturer-cam0".into() }));
    assert!(!m.iter().any(|x| matches!(x, Mismatch::PublisherMissing { topic } if topic == "camera/cam0")));
}

#[test]
fn diff_empty_when_all_present() {
    let expected = mediaservo_host::translate::expected_process_names(sample_cfg()).unwrap();
    let actual: Vec<OxProcess> = expected.iter().map(|n| OxProcess { name: n.clone(), status: "running".into() }).collect();
    let expected_topics = vec![FrameTopic::new("camera/cam0")];
    let actual_topics = expected_topics.clone();
    assert!(diff(&expected, &expected_topics, &actual, &actual_topics).is_empty());
}

#[test]
fn parse_oxmgr_json_extracts_name_and_status() {
    // 实测 oxmgr 0.5.0 list --json 输出（字段超集, 未知字段忽略）
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
    // grace 未过 → grace_active=true（启动窗口）
    let m = TopologyMonitor::new_for_test(sample_cfg().to_string(), std::time::Duration::from_secs(60));
    assert!(m.grace_active(), "60s grace 内应处于启动窗口");
    // grace=0 → 立即退出启动窗口
    let m = TopologyMonitor::new_for_test(sample_cfg().to_string(), std::time::Duration::ZERO);
    assert!(!m.grace_active(), "grace=0 不应有启动窗口");
}
