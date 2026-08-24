//! translate.rs 翻译器测试（Task A2 + C1）：host.yaml → oxfile.toml 文本。
//!
//! C1 增补：camera_config(s) 富解析（source/fps 缺省）+ to_oxfile_in_dir 的
//! capturer 实例追加 --config/--token 绝对路径。
//!
//! 视频源模型定稿（2026-08）: cameras → sources（serde alias 兼容旧键）、
//! stream.camera → stream.source（alias 兼容）、Source mode 四类 + width/height。

#[test]
fn to_oxfile_emits_all_placeholder_apps() {
    let cfg = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\nstreams:\n  - id: \"cam0-stream\"\n    source: \"cam0\"\n    codec: \"h264\"\nrecord:\n  enabled: false\n";
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    for app in [
        "host-agent",
        "host-recorder",
        "host-controller",
        "host-emergency",
        "host-audio",
    ] {
        assert!(ox.contains(&format!("name = \"{app}\"")), "missing {app}");
    }
    assert!(ox.contains("name = \"host-capturer-cam0\""), "missing capturer 实例");
    assert!(ox.contains("name = \"host-streamer-cam0-stream\""), "missing streamer 实例");
    assert!(ox.contains("--camera cam0")); // 参数化实例（name 与 command 分行）
    assert!(ox.contains("--stream cam0-stream"));
    // [defaults] 固定字段（A2 审查 M4 补强）
    assert!(ox.contains("version = 1"));
    assert!(ox.contains("namespace = \"mediaservo-host\""));
    assert!(ox.contains("restart_policy = \"always\""));
}

#[test]
fn camera_config_defaults_and_explicit() {
    // 缺省: 无 mode → generator（原 stub 语义）、width/height 1280x720、fps 30
    let cfg = "sources:\n  - id: \"cam0\"\n";
    let cams = mediaservo_host::translate::camera_configs(cfg).unwrap();
    assert_eq!(cams.len(), 1);
    assert_eq!(cams[0].id, "cam0");
    assert_eq!(cams[0].mode, mediaservo_host::translate::SourceMode::Generator);
    assert_eq!(cams[0].width, 1280);
    assert_eq!(cams[0].height, 720);
    assert_eq!(cams[0].fps, 30);
    // 显式值
    let cfg = "sources:\n  - id: \"cam1\"\n    mode: \"camera\"\n    backend: \"v4l2\"\n    width: 1920\n    height: 1080\n    fps: 15\n";
    let cams = mediaservo_host::translate::camera_configs(cfg).unwrap();
    assert_eq!(cams[0].mode, mediaservo_host::translate::SourceMode::Camera);
    assert_eq!(cams[0].width, 1920);
    assert_eq!(cams[0].height, 1080);
    assert_eq!(cams[0].fps, 15);
    // 单个查找
    assert!(mediaservo_host::translate::camera_config(cfg, "cam1").unwrap().is_some());
    assert!(mediaservo_host::translate::camera_config(cfg, "nope").unwrap().is_none());
    // 坏配置 → Err
    assert!(mediaservo_host::translate::camera_configs("not yaml [[[").is_err());
}

/// 旧键兼容：cameras / camera 引用 / source="stub" 字段 → 新模式解析（alias 门）。
#[test]
fn legacy_cameras_and_camera_keys_still_parse() {
    let cfg = "cameras:\n  - id: \"cam0\"\n    source: \"stub\"\n    fps: 30\nstreams:\n  - id: \"s0\"\n    camera: \"cam0\"\n";
    let cams = mediaservo_host::translate::camera_configs(cfg).unwrap();
    assert_eq!(cams[0].mode, mediaservo_host::translate::SourceMode::Generator);
    assert_eq!(cams[0].fps, 30);
    let streams = mediaservo_host::translate::stream_configs(cfg).unwrap();
    assert_eq!(streams[0].source, "cam0", "旧 camera 键应经 alias 解析为 source");
    assert!(mediaservo_host::translate::to_oxfile(cfg).unwrap().contains("host-capturer-cam0"));
    // 旧 source≠stub → 明确拒绝（迁移提示）
    let old_v4l2 = "cameras:\n  - id: \"cam0\"\n    source: \"v4l2\"\n";
    assert!(mediaservo_host::translate::camera_configs(old_v4l2).unwrap_err().contains("未支持"));
}

#[test]
fn to_oxfile_in_dir_appends_config_and_token_paths() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n";
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, dir.path()).unwrap();
    // 绝对路径 + 每 source token 文件
    let abs = std::path::absolute(dir.path()).unwrap();
    assert!(ox.contains(&format!("--config {}/etc/host.yaml", abs.display())));
    assert!(ox.contains(&format!("--token {}/etc/link/cam0.token", abs.display())));
    // streamer 行追加 --gateway/—config/--token（D2 网关 + C2 同形）
    assert!(ox.contains("--stream s0 --gateway ws://127.0.0.1:17980/ws --config"));
    assert!(ox.contains(&format!("--token {}/etc/link/s0.token", abs.display())));
    // 无路径变体保持 A2 形态
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(ox.contains("--camera cam0"));
    assert!(!ox.contains("--config"));
}

#[test]
fn camera_config_rejects_zero_fps() {
    // C1 审查发现: fps=0 → generator.start 线程内 panic → 死线程 + 主线程永久阻塞
    // （C15 "failure as hang" 类）——必须在配置解析层拒绝。
    let cfg = "sources:\n  - id: \"cam0\"\n    fps: 0\n";
    let err = mediaservo_host::translate::camera_configs(cfg).unwrap_err();
    assert!(err.contains("fps"), "错误信息应指明 fps, got: {err}");
    assert!(err.contains("cam0"), "错误信息应含源 id, got: {err}");
    assert!(mediaservo_host::translate::camera_config(cfg, "cam0").is_err());
}

#[test]
fn stream_config_defaults_and_explicit() {
    // 缺省 source/codec → id/vp8
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n";
    let streams = mediaservo_host::translate::stream_configs(cfg).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].id, "s0");
    assert_eq!(streams[0].source, "s0");
    assert_eq!(streams[0].codec, "vp8");
    // 显式值（source 引用 sources[].id）
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s1\"\n    source: \"cam0\"\n    codec: \"h264\"\n";
    let streams = mediaservo_host::translate::stream_configs(cfg).unwrap();
    assert_eq!(streams[0].source, "cam0");
    assert_eq!(streams[0].codec, "h264");
    // 单个查找
    assert!(mediaservo_host::translate::stream_config(cfg, "s1").unwrap().is_some());
    assert!(mediaservo_host::translate::stream_config(cfg, "nope").unwrap().is_none());
    // 坏配置 → Err
    assert!(mediaservo_host::translate::stream_configs("not yaml [[[").is_err());
}

#[test]
fn record_config_defaults_and_explicit() {
    // 缺省: disabled + 默认输出目录（C3）
    let cfg = "sources:\n  - id: \"cam0\"\n";
    let rec = mediaservo_host::translate::record_config(cfg).unwrap();
    assert!(!rec.enabled, "缺省应 disabled");
    assert_eq!(rec.out_dir, std::path::PathBuf::from("/tmp/mediaservo-recordings"));
    // 显式值（YAML 嵌套 record 段）
    let cfg = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: true\n  out_dir: \"/var/rec\"\n";
    let rec = mediaservo_host::translate::record_config(cfg).unwrap();
    assert!(rec.enabled, "显式 enabled=true 应生效");
    assert_eq!(rec.out_dir, std::path::PathBuf::from("/var/rec"));
    // 缺 out_dir → 默认目录
    let cfg = "record:\n  enabled: true\n";
    let rec = mediaservo_host::translate::record_config(cfg).unwrap();
    assert!(rec.enabled);
    assert_eq!(rec.out_dir, std::path::PathBuf::from("/tmp/mediaservo-recordings"));
    // 坏配置 → Err
    assert!(mediaservo_host::translate::record_config("not yaml [[[").is_err());
}

#[test]
fn to_oxfile_in_dir_recorder_appends_config_and_token_paths() {
    // C3: recorder 固定 app 与 capturer/streamer 同形追加 --config/--token
    let dir = tempfile::tempdir().unwrap();
    let cfg = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: true\n";
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, dir.path()).unwrap();
    let abs = std::path::absolute(dir.path()).unwrap();
    assert!(ox.contains("host-recorder --config"));
    assert!(ox.contains(&format!("--config {}/etc/host.yaml", abs.display())));
    assert!(ox.contains(&format!("--token {}/etc/link/recorder.token", abs.display())));
    // 无路径变体保持 A2 形态（无参数）
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(ox.contains("name = \"host-recorder\""));
    assert!(!ox.contains("host-recorder --config"));
}

#[test]
fn signaling_local_port_passed_to_host_agent() {
    // [signaling] local_port 配置 → host-agent 命令追加 --port
    let cfg = "sources:\n  - id: \"cam0\"\nsignaling:\n  local_port: 17980\n";
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(ox.contains("host-agent --port 17980"), "agent 命令应带 --port, got:\n{ox}");
    // 缺省：不追加（agent 内置默认 17980）
    let ox = mediaservo_host::translate::to_oxfile("sources:\n  - id: \"cam0\"\n").unwrap();
    assert!(ox.contains("host-agent\"") && !ox.contains("host-agent --port"), "缺省不追加 --port");
    assert_eq!(mediaservo_host::translate::signaling_local_port(cfg).unwrap(), Some(17980));
    assert_eq!(mediaservo_host::translate::signaling_local_port("").unwrap(), None);
}

#[test]
fn signaling_gateway_url_resolution_and_streamer_arg() {
    // D2: [signaling] local_port → 子进程网关 URL；缺省 17980
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\nsignaling:\n  local_port: 18000\n";
    assert_eq!(
        mediaservo_host::translate::signaling_gateway_url(cfg).unwrap(),
        "ws://127.0.0.1:18000/ws"
    );
    // 缺省（无 [signaling] 段）→ 17980
    assert_eq!(
        mediaservo_host::translate::signaling_gateway_url("sources:\n  - id: \"cam0\"\n").unwrap(),
        "ws://127.0.0.1:17980/ws"
    );
    // streamer 命令追加 --gateway（with paths 与无 paths 变体一致）
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, std::path::Path::new("/tmp/x")).unwrap();
    assert!(
        ox.contains("--stream s0 --gateway ws://127.0.0.1:18000/ws --config"),
        "streamer 行应带 --gateway, got:\n{ox}"
    );
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(
        ox.contains("--stream s0 --gateway ws://127.0.0.1:18000/ws"),
        "无路径变体也应带 --gateway, got:\n{ox}"
    );
}

#[test]
fn to_oxfile_in_dir_passes_config_to_host_agent() {
    // E1: agent 拓扑监控期望态数据源 — 与 recorder 同形追加 --config
    let dir = tempfile::tempdir().unwrap();
    let cfg = "sources:\n  - id: \"cam0\"\n";
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, dir.path()).unwrap();
    let abs = std::path::absolute(dir.path()).unwrap();
    assert!(ox.contains("host-agent --config"), "agent 命令应带 --config, got:\n{ox}");
    assert!(ox.contains(&format!("--config {}/etc/host.yaml", abs.display())));
    // 无路径变体保持 A2 形态（无参数）
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(!ox.contains("host-agent --config"));
}