//! T5 scope.rs mock 单测（design §Testing Strategy「scope mock（活性/老化/混排 +
//! owner 分组 + 取腿表全组合）」的取腿前置面——腿装配矩阵已由 spec.rs T2 单测钉住，
//! 本面钉：fixture wire 解析（新/旧 server 两形）、房间/设备定向、空集报因、
//! server_url 三级链与 https 报因。fixture = sfu.rs stream_stat_row 真实 wire 形。

use std::path::Path;

use mediaservo_weaknet::scope::{
    StreamInfo, parse_streams_body, pick_server_url, targeting_for_devices, targeting_for_rooms,
    targeting_media,
};
use mediaservo_weaknet::spec::ScopeSel;

const LIVE: &str = include_str!("fixtures/stats_live_owner.json");
const LEGACY: &str = include_str!("fixtures/stats_legacy_no_owner.json");

fn rows(body: &str) -> Vec<StreamInfo> {
    parse_streams_body(body).expect("fixture 必可解析")
}

// ---------- wire 解析 ----------

#[test]
fn live_fixture_owner_and_liveness_shape() {
    let rs = rows(LIVE);
    assert_eq!(rs.len(), 6);
    let p = &rs[0];
    assert_eq!(p.room, "vehicle_test1");
    assert_eq!(p.owner.as_deref(), Some("dev-a"), "小刀 C owner 字段读出");
    assert_eq!(p.role, "producer");
    assert!(p.live, "transport_id 存在 = tuple 衍生活行");
    assert_eq!(p.local_port, Some(20000));
    assert_eq!(p.remote_ports, vec![41001]);
    assert!(rs.iter().any(|r| r.room == "ms-car2" && r.owner.is_none()), "无主房 owner=null → None");
    let zombie = rs.iter().find(|r| r.room == "zombie-room").unwrap();
    assert!(!zombie.live, "无 transport 观测字段（非 tuple 形）= 不活");
}

#[test]
fn legacy_fixture_without_owner_key_parses_as_none() {
    let rs = rows(LEGACY);
    assert_eq!(rs.len(), 4);
    assert!(rs.iter().all(|r| r.owner.is_none()), "小刀 C 前 server 无 owner 键 → None");
    assert!(rs.iter().all(|r| r.live));
}

#[test]
fn parse_rejects_non_json_body_with_cause() {
    let e = parse_streams_body("<html>502</html>").unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("stats JSON 解析失败"), "{}", e.msg);
}

// ---------- --stream 定向 ----------

#[test]
fn stream_pairs_are_ordered_local_remote_sorted_dedup() {
    // 一 room 多流（producer+consumer 各自 remote_port）→ (20000,41001)+(20000,42001)
    let t = targeting_for_rooms(&rows(LIVE), "vehicle_test1").unwrap();
    assert_eq!(t.scope, ScopeSel::Stream);
    assert_eq!(t.pairs, vec![(20000, 41001), (20000, 42001)]);
    assert!(t.ports.is_empty());
}

#[test]
fn stream_multi_room_comma_union() {
    let t = targeting_for_rooms(&rows(LIVE), "vehicle_test1,audio-vehicle_test1").unwrap();
    assert_eq!(t.pairs, vec![(20000, 41001), (20000, 41002), (20000, 42001)]);
}

#[test]
fn stream_dead_only_room_exit2_lists_live_rooms() {
    let e = targeting_for_rooms(&rows(LIVE), "zombie-room").unwrap_err();
    assert_eq!(e.code, 2, "空集=环境侧未连/名账，非参数形");
    assert!(e.msg.contains("无活性 transport"), "{}", e.msg);
    assert!(e.msg.contains("可用房间: audio-vehicle_test1,cam-noport,ms-car2,vehicle_test1"), "{}", e.msg);
}

#[test]
fn stream_tupleless_row_excluded_from_pairs() {
    // cam-noport 有 tuple 观测但 remote_port=null → 不可定向（WARN 通路），单独点名 = 空集报因
    let e = targeting_for_rooms(&rows(LIVE), "cam-noport").unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("流 [cam-noport] 无活性 transport"), "{}", e.msg);
}

#[test]
fn stream_empty_selector_is_bad_param() {
    let e = targeting_for_rooms(&rows(LIVE), " , ").unwrap_err();
    assert_eq!(e.code, 4);
}

// ---------- --device 定向（owner 分组 + peer_id 兜底） ----------

#[test]
fn device_owner_union_spans_owned_rooms_only() {
    // dev-a 拥有 vehicle_test1 / audio-vehicle_test1 / cam-noport；ms-car2(owner=null) 不沾
    let t = targeting_for_devices(&rows(LIVE), "dev-a").unwrap();
    assert_eq!(t.scope, ScopeSel::Device);
    assert_eq!(t.pairs, vec![(20000, 41001), (20000, 41002), (20000, 42001)]);
}

#[test]
fn device_legacy_fallback_matches_producer_peer_id_and_groups_room() {
    // 旧 server（无 owner）：car-01 命中 cam0-stream/cam1-stream 的 producer peer_id，
    // 房间分组连带该房 consumer 腿（41101/42101/41102）；car-09 的 other-room 排除。
    let t = targeting_for_devices(&rows(LEGACY), "car-01").unwrap();
    assert_eq!(t.pairs, vec![(20000, 41101), (20000, 41102), (20000, 42101)]);
}

#[test]
fn device_unknown_exit2_with_live_rooms() {
    let e = targeting_for_devices(&rows(LIVE), "ghost-dev").unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("无活性流"), "{}", e.msg);
    assert!(e.msg.contains("可用房间:"), "{}", e.msg);
}

// ---------- 段级媒体口（bash media_ports 等价） ----------

#[test]
fn media_targeting_unions_live_local_ports() {
    let t = targeting_media(&rows(LIVE)).unwrap();
    assert_eq!(t.scope, ScopeSel::Media);
    assert_eq!(t.ports, vec![20000]);
}

#[test]
fn media_targeting_empty_exit2_bash_wording() {
    let dead = vec![rows(LIVE).into_iter().find(|r| !r.live).unwrap()];
    let e = targeting_media(&dead).unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("媒体口观测为空"), "{}", e.msg);
    assert!(e.msg.contains("--rtp-port"), "报因须指逃生门: {}", e.msg);
}

// ---------- server_url 解析链 ----------

#[test]
fn server_url_flag_beats_env_beats_yaml() {
    let dir = std::env::temp_dir().join(format!("wnet-url-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = dir.join("server.yaml");
    std::fs::write(&yaml, "listen:\n  host: 0.0.0.0\n  port: 9800\n").unwrap();

    let by_flag = pick_server_url(Some("http://10.0.0.2:9999"), Some("http://env:1"), Some(&yaml)).unwrap();
    assert_eq!(by_flag, "http://10.0.0.2:9999");
    let by_env = pick_server_url(None, Some("http://env-host:9801/"), Some(&yaml)).unwrap();
    assert_eq!(by_env, "http://env-host:9801", "尾斜杠归一");
    let by_yaml = pick_server_url(None, None, Some(&yaml)).unwrap();
    assert_eq!(by_yaml, "http://127.0.0.1:9800", "探测形=回环 host + listen.port");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn server_url_all_sources_empty_names_three_tried() {
    let e = pick_server_url(None, None, None).unwrap_err();
    assert_eq!(e.code, 2);
    for src in ["--server-url", "WEAKNET_SERVER_URL", "server.yaml"] {
        assert!(e.msg.contains(src), "报因须点名三源，缺 {src}: {}", e.msg);
    }
}

#[test]
fn server_url_https_rejected_with_cause() {
    let e = pick_server_url(Some("https://safer.example"), None, None).unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("明文 http"), "https 报因 Out 范围: {}", e.msg);
}

#[test]
fn server_url_yaml_without_listen_port_reports_file() {
    let dir = std::env::temp_dir().join(format!("wnet-badyaml-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = dir.join("server.yaml");
    std::fs::write(&yaml, "listen:\n  host: 0.0.0.0\n").unwrap();
    let e = pick_server_url(None, None, Some(Path::new(&yaml))).unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("listen.port"), "{}", e.msg);
    std::fs::remove_dir_all(&dir).ok();
}
