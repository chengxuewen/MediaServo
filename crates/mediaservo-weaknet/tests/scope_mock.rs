//! T5 scope.rs mock unit tests (design §Testing Strategy "scope mock (liveness/aging/mixed +
//! owner grouping + the full leg-extraction table)") -- the leg-extraction precondition surface. The leg assembly matrix is already pinned by the spec.rs T2 tests;
//! this surface pins: fixture wire parsing (new/old server shapes), room/device targeting, empty-set cause,
//! the server_url three-level chain and the https cause. Fixture = the real wire shape of sfu.rs stream_stat_row.

use std::path::Path;

use mediaservo_weaknet::scope::{
    StreamInfo, parse_streams_body, pick_server_url, targeting_for_devices, targeting_for_rooms,
    targeting_media,
};
use mediaservo_weaknet::spec::ScopeSel;

const LIVE: &str = include_str!("fixtures/stats_live_owner.json");
const LEGACY: &str = include_str!("fixtures/stats_legacy_no_owner.json");

fn rows(body: &str) -> Vec<StreamInfo> {
    parse_streams_body(body).expect("the fixture must be parseable")
}

// ---------- wire parsing ----------

#[test]
fn live_fixture_owner_and_liveness_shape() {
    let rs = rows(LIVE);
    assert_eq!(rs.len(), 6);
    let p = &rs[0];
    assert_eq!(p.room, "vehicle_test1");
    assert_eq!(p.owner.as_deref(), Some("dev-a"), "knife-C owner field read back");
    assert_eq!(p.role, "producer");
    assert!(p.live, "transport_id present = tuple-derived live row");
    assert_eq!(p.local_port, Some(20000));
    assert_eq!(p.remote_ports, vec![41001]);
    assert!(
        rs.iter().any(|r| r.room == "ms-car2" && r.owner.is_none()),
        "ownerless room owner=null -> None"
    );
    let zombie = rs.iter().find(|r| r.room == "zombie-room").unwrap();
    assert!(!zombie.live, "no transport observation field (non-tuple shape) = not alive");
}

#[test]
fn legacy_fixture_without_owner_key_parses_as_none() {
    let rs = rows(LEGACY);
    assert_eq!(rs.len(), 4);
    assert!(rs.iter().all(|r| r.owner.is_none()), "pre-knife-C servers have no owner key -> None");
    assert!(rs.iter().all(|r| r.live));
}

#[test]
fn parse_rejects_non_json_body_with_cause() {
    let e = parse_streams_body("<html>502</html>").unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("stats JSON parse failed"), "{}", e.msg);
}

// ---------- --stream targeting ----------

#[test]
fn stream_pairs_are_ordered_local_remote_sorted_dedup() {
    // one room, multiple streams (producer+consumer each with their remote_port) -> (20000,41001)+(20000,42001)
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
    assert_eq!(
        e.code, 2,
        "empty set = environment-side not connected/wrong name, not a param error"
    );
    assert!(e.msg.contains("no live transport"), "{}", e.msg);
    assert!(
        e.msg.contains("available rooms: audio-vehicle_test1,cam-noport,ms-car2,vehicle_test1"),
        "{}",
        e.msg
    );
}

#[test]
fn stream_tupleless_row_excluded_from_pairs() {
    // cam-noport has tuple observations but remote_port=null -> not targetable (the WARN channel); naming it alone = empty-set cause
    let e = targeting_for_rooms(&rows(LIVE), "cam-noport").unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("streams [cam-noport] have no live transport"), "{}", e.msg);
}

#[test]
fn stream_empty_selector_is_bad_param() {
    let e = targeting_for_rooms(&rows(LIVE), " , ").unwrap_err();
    assert_eq!(e.code, 4);
}

// ---------- --device targeting (owner grouping + peer_id fallback) ----------

#[test]
fn device_owner_union_spans_owned_rooms_only() {
    // dev-a owns vehicle_test1 / audio-vehicle_test1 / cam-noport; ms-car2 (owner=null) is untouched
    let t = targeting_for_devices(&rows(LIVE), "dev-a").unwrap();
    assert_eq!(t.scope, ScopeSel::Device);
    assert_eq!(t.pairs, vec![(20000, 41001), (20000, 41002), (20000, 42001)]);
}

#[test]
fn device_legacy_fallback_matches_producer_peer_id_and_groups_room() {
    // old server (no owner): car-01 hits the producer peer_id of cam0-stream/cam1-stream,
    // room grouping pulls in that room's consumer legs (41101/42101/41102); car-09's other-room excluded.
    let t = targeting_for_devices(&rows(LEGACY), "car-01").unwrap();
    assert_eq!(t.pairs, vec![(20000, 41101), (20000, 41102), (20000, 42101)]);
}

#[test]
fn device_unknown_exit2_with_live_rooms() {
    let e = targeting_for_devices(&rows(LIVE), "ghost-dev").unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("no live streams"), "{}", e.msg);
    assert!(e.msg.contains("available rooms:"), "{}", e.msg);
}

// ---------- room-level media ports (bash media_ports equivalent) ----------

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
    assert!(e.msg.contains("media port observation empty"), "{}", e.msg);
    assert!(e.msg.contains("--rtp-port"), "the cause must point at the escape hatch: {}", e.msg);
}

// ---------- server_url resolution chain ----------

#[test]
fn server_url_flag_beats_env_beats_yaml() {
    let dir = std::env::temp_dir().join(format!("wnet-url-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = dir.join("server.yaml");
    std::fs::write(&yaml, "listen:\n  host: 0.0.0.0\n  port: 9800\n").unwrap();

    let by_flag =
        pick_server_url(Some("http://10.0.0.2:9999"), Some("http://env:1"), Some(&yaml)).unwrap();
    assert_eq!(by_flag, "http://10.0.0.2:9999");
    let by_env = pick_server_url(None, Some("http://env-host:9801/"), Some(&yaml)).unwrap();
    assert_eq!(by_env, "http://env-host:9801", "trailing slash normalized");
    let by_yaml = pick_server_url(None, None, Some(&yaml)).unwrap();
    assert_eq!(by_yaml, "http://127.0.0.1:9800", "probe shape = loopback host + listen.port");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn server_url_all_sources_empty_names_three_tried() {
    let e = pick_server_url(None, None, None).unwrap_err();
    assert_eq!(e.code, 2);
    for src in ["--server-url", "WEAKNET_SERVER_URL", "server.yaml"] {
        assert!(
            e.msg.contains(src),
            "the cause must name all three sources, missing {src}: {}",
            e.msg
        );
    }
}

#[test]
fn server_url_https_rejected_with_cause() {
    let e = pick_server_url(Some("https://safer.example"), None, None).unwrap_err();
    assert_eq!(e.code, 2);
    assert!(e.msg.contains("plaintext http"), "the https cause is out of scope: {}", e.msg);
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
