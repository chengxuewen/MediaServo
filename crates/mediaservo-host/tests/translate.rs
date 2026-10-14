//! translate.rs renderer tests (Task A2 + C1): host.yaml -> oxfile.toml text.
//!
//! C1 additions: rich camera_config(s) parsing (source/fps defaults) + to_oxfile_in_dir's
//! capturer instances appending absolute --config/--token paths.
//!
//! Video source model finalized (2026-08): cameras -> sources (serde alias keeps the old key),
//! stream.camera -> stream.source (alias), Source mode in four kinds + width/height.

#[test]
fn to_oxfile_emits_all_placeholder_apps() {
    let cfg = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\nstreams:\n  - id: \"cam0-stream\"\n    source: \"cam0\"\n    codec: \"h264\"\nrecord:\n  enabled: false\n";
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    for app in ["host-agent", "host-recorder", "host-controller", "host-emergency", "host-audio"] {
        assert!(ox.contains(&format!("name = \"{app}\"")), "missing {app}");
    }
    assert!(ox.contains("name = \"host-capturer-cam0\""), "missing capturer instance");
    assert!(ox.contains("name = \"host-streamer-cam0-stream\""), "missing streamer instance");
    assert!(ox.contains("--camera cam0")); // parameterized instance (name and command on separate lines)
    assert!(ox.contains("--stream cam0-stream"));
    // [defaults] fixed fields (A2 review M4 hardening)
    assert!(ox.contains("version = 1"));
    assert!(ox.contains("namespace = \"mediaservo-host\""));
    assert!(ox.contains("restart_policy = \"always\""));
}

#[test]
fn camera_config_defaults_and_explicit() {
    // defaults: no mode -> generator (former stub semantics), width/height 1280x720, fps 30
    let cfg = "sources:\n  - id: \"cam0\"\n";
    let cams = mediaservo_host::translate::camera_configs(cfg).unwrap();
    assert_eq!(cams.len(), 1);
    assert_eq!(cams[0].id, "cam0");
    assert_eq!(cams[0].mode, mediaservo_host::translate::SourceMode::Generator);
    assert_eq!(cams[0].width, 1280);
    assert_eq!(cams[0].height, 720);
    assert_eq!(cams[0].fps, 30);
    // explicit value
    let cfg = "sources:\n  - id: \"cam1\"\n    mode: \"camera\"\n    backend: \"v4l2\"\n    width: 1920\n    height: 1080\n    fps: 15\n";
    let cams = mediaservo_host::translate::camera_configs(cfg).unwrap();
    assert_eq!(cams[0].mode, mediaservo_host::translate::SourceMode::Camera);
    assert_eq!(cams[0].width, 1920);
    assert_eq!(cams[0].height, 1080);
    assert_eq!(cams[0].fps, 15);
    // single lookup
    assert!(mediaservo_host::translate::camera_config(cfg, "cam1").unwrap().is_some());
    assert!(mediaservo_host::translate::camera_config(cfg, "nope").unwrap().is_none());
    // bad config -> Err
    assert!(mediaservo_host::translate::camera_configs("not yaml [[[").is_err());
}

/// Old-key compatibility: cameras / camera references / source="stub" fields -> parse in the new model (alias gate).
#[test]
fn legacy_cameras_and_camera_keys_still_parse() {
    let cfg = "cameras:\n  - id: \"cam0\"\n    source: \"stub\"\n    fps: 30\nstreams:\n  - id: \"s0\"\n    camera: \"cam0\"\n";
    let cams = mediaservo_host::translate::camera_configs(cfg).unwrap();
    assert_eq!(cams[0].mode, mediaservo_host::translate::SourceMode::Generator);
    assert_eq!(cams[0].fps, 30);
    let streams = mediaservo_host::translate::stream_configs(cfg).unwrap();
    assert_eq!(
        streams[0].source, "cam0",
        "the old camera key should resolve to source via the alias"
    );
    assert!(mediaservo_host::translate::to_oxfile(cfg).unwrap().contains("host-capturer-cam0"));
    // old source != stub -> explicit rejection (migration hint)
    let old_v4l2 = "cameras:\n  - id: \"cam0\"\n    source: \"v4l2\"\n";
    assert!(
        mediaservo_host::translate::camera_configs(old_v4l2).unwrap_err().contains("unsupported")
    );
}

#[test]
fn to_oxfile_in_dir_appends_config_and_token_paths() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n";
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, dir.path()).unwrap();
    // absolute paths + per-source token file
    let abs = std::path::absolute(dir.path()).unwrap();
    assert!(ox.contains(&format!("--config {}/etc/host.yaml", abs.display())));
    assert!(ox.contains(&format!("--token {}/etc/link/cam0.token", abs.display())));
    // streamer lines append --gateway/--config/--token (D2 gateway + C2 same shape). Between --gateway and
    // --config encoding flags may be inserted (D282 default h264 -> --encoder-backend software), hence separate pins.
    assert!(
        ox.contains("--stream s0 --gateway ws://127.0.0.1:17980/ws"),
        "gateway immediately after --stream"
    );
    assert!(
        ox.contains(&format!("--token {}/etc/link/s0-stream.token", abs.display())),
        "stream token name = <stream>-stream.token (C40)"
    );
    assert!(ox.contains("--config"), "the streamer line should carry --config");
    // the no-paths variant keeps the A2 shape
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(ox.contains("--camera cam0"));
    assert!(!ox.contains("--config"));
}

#[test]
fn camera_config_rejects_zero_fps() {
    // C1 review finding: fps=0 -> panic inside the generator.start thread -> dead thread + main thread blocked forever
    // (C15 "failure as hang" class) - must be rejected at the config-parsing layer.
    let cfg = "sources:\n  - id: \"cam0\"\n    fps: 0\n";
    let err = mediaservo_host::translate::camera_configs(cfg).unwrap_err();
    assert!(err.contains("fps"), "the error should mention fps, got: {err}");
    assert!(err.contains("cam0"), "the error should contain the source id, got: {err}");
    assert!(mediaservo_host::translate::camera_config(cfg, "cam0").is_err());
}

#[test]
fn stream_config_defaults_and_explicit() {
    // default source/codec -> id/h264 (D282)
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n";
    let streams = mediaservo_host::translate::stream_configs(cfg).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].id, "s0");
    assert_eq!(streams[0].source, "s0");
    assert_eq!(streams[0].codec, "h264");
    // explicit value (source references sources[].id)
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s1\"\n    source: \"cam0\"\n    codec: \"h264\"\n";
    let streams = mediaservo_host::translate::stream_configs(cfg).unwrap();
    assert_eq!(streams[0].source, "cam0");
    assert_eq!(streams[0].codec, "h264");
    // single lookup
    assert!(mediaservo_host::translate::stream_config(cfg, "s1").unwrap().is_some());
    assert!(mediaservo_host::translate::stream_config(cfg, "nope").unwrap().is_none());
    // bad config -> Err
    assert!(mediaservo_host::translate::stream_configs("not yaml [[[").is_err());
}

#[test]
fn record_config_defaults_and_explicit() {
    // default: disabled + the default output directory (C3)
    let cfg = "sources:\n  - id: \"cam0\"\n";
    let rec = mediaservo_host::translate::record_config(cfg).unwrap();
    assert!(!rec.enabled, "default should be disabled");
    assert_eq!(rec.out_dir, std::path::PathBuf::from("/tmp/mediaservo-recordings"));
    // explicit value (nested YAML record section)
    let cfg = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: true\n  out_dir: \"/var/rec\"\n";
    let rec = mediaservo_host::translate::record_config(cfg).unwrap();
    assert!(rec.enabled, "explicit enabled=true must take effect");
    assert_eq!(rec.out_dir, std::path::PathBuf::from("/var/rec"));
    // missing out_dir -> default directory
    let cfg = "record:\n  enabled: true\n";
    let rec = mediaservo_host::translate::record_config(cfg).unwrap();
    assert!(rec.enabled);
    assert_eq!(rec.out_dir, std::path::PathBuf::from("/tmp/mediaservo-recordings"));
    // bad config -> Err
    assert!(mediaservo_host::translate::record_config("not yaml [[[").is_err());
}

#[test]
fn to_oxfile_in_dir_recorder_appends_config_and_token_paths() {
    // C3: the recorder fixed app appends --config/--token in the same shape as capturer/streamer
    let dir = tempfile::tempdir().unwrap();
    let cfg = "sources:\n  - id: \"cam0\"\nrecord:\n  enabled: true\n";
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, dir.path()).unwrap();
    let abs = std::path::absolute(dir.path()).unwrap();
    assert!(ox.contains("host-recorder --config"));
    assert!(ox.contains(&format!("--config {}/etc/host.yaml", abs.display())));
    assert!(ox.contains(&format!("--token {}/etc/link/recorder.token", abs.display())));
    // the no-paths variant keeps the A2 shape(no args)
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(ox.contains("name = \"host-recorder\""));
    assert!(!ox.contains("host-recorder --config"));
}

#[test]
fn signaling_local_port_passed_to_host_agent() {
    // [signaling] local_port config -> the host-agent command appends --port
    let cfg = "sources:\n  - id: \"cam0\"\nsignaling:\n  local_port: 17980\n";
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(
        ox.contains("host-agent --port 17980"),
        "the agent command should carry --port, got:\n{ox}"
    );
    // default: not appended (agent has a built-in default 17980)
    let ox = mediaservo_host::translate::to_oxfile("sources:\n  - id: \"cam0\"\n").unwrap();
    assert!(
        ox.contains("host-agent\"") && !ox.contains("host-agent --port"),
        "default does not append --port"
    );
    assert_eq!(mediaservo_host::translate::signaling_local_port(cfg).unwrap(), Some(17980));
    assert_eq!(mediaservo_host::translate::signaling_local_port("").unwrap(), None);
}

#[test]
fn signaling_gateway_url_resolution_and_streamer_arg() {
    // D2: [signaling] local_port -> child-process gateway URL; default 17980
    let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\nsignaling:\n  local_port: 18000\n";
    assert_eq!(
        mediaservo_host::translate::signaling_gateway_url(cfg).unwrap(),
        "ws://127.0.0.1:18000/ws"
    );
    // default (no [signaling] section) -> 17980
    assert_eq!(
        mediaservo_host::translate::signaling_gateway_url("sources:\n  - id: \"cam0\"\n").unwrap(),
        "ws://127.0.0.1:17980/ws"
    );
    // the streamer command appends --gateway (consistent across with-paths and no-paths variants)
    let ox =
        mediaservo_host::translate::to_oxfile_in_dir(cfg, std::path::Path::new("/tmp/x")).unwrap();
    assert!(
        ox.contains("--stream s0 --gateway ws://127.0.0.1:18000/ws") && ox.contains("--config"),
        "the streamer line should carry --gateway, got:\n{ox}"
    );
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(
        ox.contains("--stream s0 --gateway ws://127.0.0.1:18000/ws"),
        "the no-paths variant should also carry --gateway, got:\n{ox}"
    );
}

#[test]
fn to_oxfile_in_dir_passes_config_to_host_agent() {
    // E1: agent topology-monitor desired-state data source - appends --config in the same shape as recorder
    let dir = tempfile::tempdir().unwrap();
    let cfg = "sources:\n  - id: \"cam0\"\n";
    let ox = mediaservo_host::translate::to_oxfile_in_dir(cfg, dir.path()).unwrap();
    let abs = std::path::absolute(dir.path()).unwrap();
    assert!(
        ox.contains("host-agent --config"),
        "the agent command should carry --config, got:\n{ox}"
    );
    assert!(ox.contains(&format!("--config {}/etc/host.yaml", abs.display())));
    // the no-paths variant keeps the A2 shape(no args)
    let ox = mediaservo_host::translate::to_oxfile(cfg).unwrap();
    assert!(!ox.contains("host-agent --config"));
}
