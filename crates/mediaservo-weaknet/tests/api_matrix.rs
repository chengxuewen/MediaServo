//! api_matrix -- T6 security stack + error-family matrix (design §Testing "tower(0.4)::oneshot -- 401/403
//! (Origin and Host)/200/400 (incl. scenario ../ and absolute paths)/409 + SSE: no token 401, query
//! token 200, the first frame carries the streams+spec fields").
//!
//! Zero real bind, zero engine touching: the Router is built directly from an injected ServeConfig (fixed token + temp statedir +
//! caps_override); the apply/set 400/409 cases all end at the parsing/lock layers (replay unreachable).
//! The lock-occupied case forges the holder with "this test process's pid+starttime" -- consuming state::acquire_write_lock's
//! liveness semantics directly, pinning the REST x CLI mutual-exclusion surface.

use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use mediaservo_weaknet::engine::Env;
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scope::Capabilities;
use mediaservo_weaknet::server::{ServeConfig, build_router};
use mediaservo_weaknet::state::Dirs;
use tower::ServiceExt;

const TOKEN: &str = "matrix-token-0123456789abcdef";

fn cfg(tag: &str) -> ServeConfig {
    let dir = std::env::temp_dir().join(format!("wnet-api-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    ServeConfig {
        token: TOKEN.to_string(),
        dirs: Dirs { statedir: dir },
        env: Env::default(),
        lan: false,
        listen_host: "127.0.0.1".to_string(),
        listen_port: 19999,
        server_url: None,
        caps_override: Some(Capabilities {
            seed: true,
            dir_lo: true,
            ifb_ingress: false,
            ifb_reason: "test injection".into(),
        }),
        sse_frames_limit: None,
    }
}

fn req(method: Method, uri: &str, host: &str, bearer: Option<&str>, body: Body) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, host);
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(body).unwrap()
}

fn get(uri: &str, bearer: Option<&str>) -> Request<Body> {
    req(Method::GET, uri, "127.0.0.1:19999", bearer, Body::empty())
}

fn post(uri: &str, json_body: &str) -> Request<Body> {
    req(Method::POST, uri, "127.0.0.1:19999", Some(TOKEN), Body::from(json_body.to_string()))
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("response not JSON: {e}: {bytes:?}"))
}

#[tokio::test]
async fn bearer_gate_401_missing_and_wrong() {
    let router = build_router(cfg("401"));
    let r = router.clone().oneshot(get("/v1/state", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(body_json(r).await["error"].as_str().unwrap().contains("token"));
    let r = router.oneshot(get("/v1/state", Some("nope"))).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn evil_host_403_precedes_auth() {
    // gate-order pinned case: the Host gate precedes token validation -- rebinding traffic leaks no token right/wrong signal.
    let router = build_router(cfg("host403"));
    let r = router
        .oneshot(req(Method::GET, "/v1/state", "evil.com", Some(TOKEN), Body::empty()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn lan_mode_origin_whitelist() {
    let mut c = cfg("lan");
    c.lan = true;
    c.listen_host = "10.1.2.3".into();
    let router = build_router(c);
    let good_host = "10.1.2.3:19999";
    // wrong Origin -> 403
    let r = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/state")
                .header(header::HOST, good_host)
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::ORIGIN, "http://evil.net:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // right Origin -> 200
    let r = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/state")
                .header(header::HOST, good_host)
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::ORIGIN, format!("http://{good_host}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // no Origin (the curl shape) passes -- design ruling: Origin only constrains browser cross-origin (documented).
    let r = router
        .oneshot(req(Method::GET, "/v1/state", good_host, Some(TOKEN), Body::empty()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn sse_query_token_exemption_and_first_frame() {
    let mut c = cfg("sse");
    c.sse_frames_limit = Some(1); // one frame then close and assert the body (to_bytes can finish)
    let router = build_router(c);
    // no token -> 401
    let r = router.clone().oneshot(get("/v1/events", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // wrong query token -> 401
    let r = router.clone().oneshot(get("/v1/events?token=wrong", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // query token -> 200 + the first frame carries spec/streams (interval fires on the first tick, documented "instant open")
    let r = router.oneshot(get(&format!("/v1/events?token={TOKEN}"), None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap(),
        "text/event-stream"
    );
    let raw = tokio::time::timeout(Duration::from_secs(10), to_bytes(r.into_body(), 1 << 20))
        .await
        .expect("the first frame should arrive before the timeout")
        .unwrap();
    let text = String::from_utf8_lossy(&raw);
    let data = text
        .lines()
        .find_map(|l| l.strip_prefix("data:"))
        .expect("an SSE data line must be present");
    let frame: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
    for key in ["spec", "scope", "dir", "expires_at", "job", "tc", "streams"] {
        assert!(frame.get(key).is_some(), "state frame missing field {key}: {frame}");
    }
    assert_eq!(frame["spec"], serde_json::Value::Null, "no state = spec null (key always present)");
    assert_eq!(frame["streams"], serde_json::json!([]));
    assert!(frame.get("ev").is_none(), "no new events = no ev (design §API)");
}

#[tokio::test]
async fn state_inactive_shape() {
    let r = build_router(cfg("state0")).oneshot(get("/v1/state", Some(TOKEN))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await, serde_json::json!({"active": false}));
}

#[tokio::test]
async fn profiles_and_capabilities_shapes() {
    let router = build_router(cfg("prof"));
    let r = router.clone().oneshot(get("/v1/profiles", Some(TOKEN))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["profiles"].is_array());
    let r = router.clone().oneshot(get("/v1/capabilities", Some(TOKEN))).await.unwrap();
    assert_eq!(
        body_json(r).await,
        serde_json::json!({"seed": true, "dir_lo": true, "ifb_ingress": false, "ifb_reason": "test injection"})
    );
}

#[tokio::test]
async fn streams_graceful_empty_when_unconfigured() {
    let r = build_router(cfg("streams")).oneshot(get("/v1/streams", Some(TOKEN))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v = body_json(r).await;
    assert_eq!(
        v["streams"],
        serde_json::json!([]),
        "server_url unobtainable = 200 + empty list + note (never 500)"
    );
    assert!(v["note"].as_str().unwrap().contains("stats"), "{v}");
}

#[tokio::test]
async fn set_without_state_is_409_with_guide() {
    let r = build_router(cfg("set409")).oneshot(post("/v1/set", r#"{"rtt_ms":20}"#)).await.unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let v = body_json(r).await;
    assert_eq!(
        v["error"], "no active spec -- apply first",
        "the UI guidance copy is pinned (design §UI write path)"
    );
}

#[tokio::test]
async fn apply_param_validation_400s() {
    let router = build_router(cfg("400"));
    let cases: &[&str] = &[
        "[1,2]",                               // not an object
        r#"{"duration":"abc"}"#,               // type error
        r#"{"duration":1}"#,                   // <5 (the validate_duration table)
        r#"{"rtt_ms":"80"}"#,                  // a numeric key given a string
        r#"{"bogus_key":1}"#,                  // unknown spec key (typos surface loudly)
        r#"{"loss":"2","loss_mode":"bogus"}"#, // loss_mode out of domain
        r#"{"ports":"notarray"}"#,             // ports type error
        r#"{"stream_scope":{"rooms":[]}}"#,    // empty set
    ];
    for body in cases {
        let r = router.clone().oneshot(post("/v1/apply", body)).await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "case {body:?} should be 400");
        assert!(body_json(r).await.get("error").is_some(), "error body {{error}} shape ({body:?})");
    }
}

/// A state.json with an active job (alive=true uses this process's pid+starttime = liveness certainly true; false uses an impossible pid).
fn write_live_job(dirs: &Dirs, alive: bool) {
    use mediaservo_weaknet::spec::{Dir, ImpairSpec, ScopeSel};
    use mediaservo_weaknet::state::{ChannelSer, JobRef, STATE_SCHEMA, State, Teardown};
    let pid = if alive { std::process::id() } else { u32::MAX };
    let st = State {
        schema: STATE_SCHEMA.to_string(),
        spec: ImpairSpec::default(),
        dir: Dir::Both,
        scope: ScopeSel::Media,
        iface: "lo".into(),
        ports: vec![40010],
        pairs: vec![],
        rooms: vec!["room-a".into()],
        devices: vec![],
        sig_port: None,
        expires_at_ms: fuse::now_epoch_ms() + 300_000,
        created_root: true,
        ifb_used: false,
        created_ifb: false,
        job: Some(JobRef {
            name: "matrix-job".into(),
            pid,
            starttime: fuse::read_proc_starttime(std::process::id()).unwrap_or(0),
            done: 1,
            total: 3,
        }),
        teardown: Teardown { channel: ChannelSer::LocalRoot, sidecar: None, steps: vec![] },
    };
    st.write_to(&dirs.state_json()).unwrap();
}

#[tokio::test]
async fn scenario_run_stop_t8_semantics() {
    // the T8 real surface (the old 501 stub retired): the 400 gate unchanged; a legal plan -> the start phase reports
    // cause 500 at stats targeting (a closed-port injection = deterministic zero tc/docker touching); stop = 200/idempotent/stale self-clear.
    let mut c = cfg("scen");
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    c.server_url = Some(format!("http://127.0.0.1:{dead}"));
    let dirs = c.dirs.clone();
    let router = build_router(c);
    for bad in [
        r#"{"file":"../evil.yaml"}"#,
        r#"{"file":"/etc/passwd"}"#,
        r#"{"file":"sub/dir.yaml"}"#,
        r#"{"inline":1}"#,
        "{}",
    ] {
        let r = router.clone().oneshot(post("/v1/scenario/run", bad)).await.unwrap();
        assert_eq!(
            r.status(),
            StatusCode::BAD_REQUEST,
            "case {bad:?} should be 400 (traversal/absolute/type/missing param -- the start phase's up-front real validation)"
        );
    }
    let big = format!(r#"{{"inline":"{}"}}"#, "x".repeat(33 * 1024));
    let r = router.clone().oneshot(post("/v1/scenario/run", &big)).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "inline >32KB → 400");
    // a legal inline (grammar passes) -> targeted stats unreachable = an honest 500, zero side effects (no lock touched/no state written)
    let r = router
        .clone()
        .oneshot(post(
            "/v1/scenario/run",
            r#"{"inline":"scenario:\n  baseline_s: 1\n  steps:\n    - at_s: 5\n      set: {rtt_ms: 50}\n"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "legal plan + stats unreachable -> 500 (the old 501 stub retired)"
    );
    assert!(body_json(r).await["error"].as_str().is_some(), "error body {{error}} shape");
    assert!(
        !dirs.state_json().exists() && !dirs.lock().exists(),
        "start-phase precondition failure = zero disk writes"
    );
    // a test-injected scenario root (WEAKNET_ASSETS_DIR): a legal file -> resolved on disk -> the same 500 cause
    let assets = std::env::temp_dir().join(format!("wnet-scen-{}", std::process::id()));
    std::fs::create_dir_all(assets.join("scenarios")).unwrap();
    std::fs::write(
        assets.join("scenarios").join("mini.yaml"),
        "scenario:\n  baseline_s: 1\n  steps:\n    - at_s: 5\n      set: {rtt_ms: 50}\n",
    )
    .unwrap();
    // SAFETY: only this test reads WEAKNET_ASSETS_DIR (no concurrent case on /v1/scenarios); requests sent serially
    unsafe { std::env::set_var("WEAKNET_ASSETS_DIR", &assets) };
    let r =
        router.clone().oneshot(post("/v1/scenario/run", r#"{"file":"mini.yaml"}"#)).await.unwrap();
    // SAFETY: the request has completed, reclamation window
    unsafe { std::env::remove_var("WEAKNET_ASSETS_DIR") };
    assert_eq!(
        r.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "after the file resolves, the same start-phase cause (not 400/404/501)"
    );
    // stop: no active job -> 200 idempotent
    let r = router.clone().oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["detail"].as_str().unwrap().contains("idempotent"));
    // stop: owner alive (this process's pid) -> 200 + the cancel flag on disk
    write_live_job(&dirs, true);
    let r = router.clone().oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["detail"].as_str().unwrap().contains("stop requested"));
    assert!(
        mediaservo_weaknet::scenario::cancel_path(&dirs).exists(),
        "live owner -> the stop flag written"
    );
    std::fs::remove_file(mediaservo_weaknet::scenario::cancel_path(&dirs)).unwrap();
    // stop: dead owner (an impossible pid) -> 200 + the job flag self-clears on disk
    write_live_job(&dirs, false);
    let r = router.clone().oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["detail"].as_str().unwrap().contains("stale"));
    let st = mediaservo_weaknet::state::State::read_from(&dirs.state_json()).unwrap().unwrap();
    assert!(st.job.is_none(), "a dead owner's flag self-clears on disk");
    std::fs::remove_dir_all(&dirs.statedir).ok();
    std::fs::remove_dir_all(&assets).ok();
}

#[tokio::test]
async fn live_scenario_job_gates_apply_set_409() {
    // the rev-2.2 exclusivity gate REST surface: while a job is alive, apply/set both 409 before replay (zero tc touching).
    let c = cfg("job409");
    let dirs = c.dirs.clone();
    write_live_job(&dirs, true);
    let router = build_router(c);
    let r =
        router.clone().oneshot(post("/v1/apply", r#"{"rtt_ms":80,"duration":40}"#)).await.unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT, "apply while a job is active = 409");
    assert!(body_json(r).await["error"].as_str().unwrap().contains("scenario"));
    let r = router.clone().oneshot(post("/v1/set", r#"{"rtt_ms":40}"#)).await.unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT, "set while a job is active = 409 (symmetric)");
    // a dead owner's flag doesn't block: apply passes to the next layer (stats unreachable 500, not 409)
    let mut c2 = cfg("job409b");
    let dead2 = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    c2.server_url = Some(format!("http://127.0.0.1:{dead2}"));
    let dirs2 = c2.dirs.clone();
    write_live_job(&dirs2, false);
    let r = build_router(c2)
        .oneshot(post("/v1/apply", r#"{"rtt_ms":80,"duration":40}"#))
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a stale job declared dead passes (the env cause reported after the gate)"
    );
    std::fs::remove_dir_all(&dirs.statedir).ok();
    std::fs::remove_dir_all(&dirs2.statedir).ok();
}

#[tokio::test]
async fn apply_conflicts_with_held_write_lock_409() {
    // the REST x CLI mutual-exclusion surface: a forged lock holder (this process's pid+starttime -> holder_alive liveness true).
    // a legal spec but a busy lock = 409 (engine::take_write_lock runs before resolve/replay, zero side effects).
    let c = cfg("lock409");
    let dirs = c.dirs.clone();
    let pid = std::process::id();
    let st = fuse::read_proc_starttime(pid).expect("own starttime must be readable");
    std::fs::write(dirs.lock(), format!("{pid} {st}\n")).unwrap();
    let r =
        build_router(c).oneshot(post("/v1/apply", r#"{"rtt_ms":80,"duration":40}"#)).await.unwrap();
    assert_eq!(
        r.status(),
        StatusCode::CONFLICT,
        "lock occupied -> 409 (the Fail::conflict mapping)"
    );
    std::fs::remove_file(dirs.lock()).ok();
    std::fs::remove_dir_all(&dirs.statedir).ok();
}
