//! api_matrix —— T6 安全栈 + 错误族矩阵（design §Testing「tower(0.4)::oneshot——401/403
//! (Origin 和 Host)/200/400(含 scenario ../ 与绝对路径)/409 + SSE：无 token 401、query
//! token 200、首帧含 streams+spec 字段」）。
//!
//! 零真实 bind、零引擎触达：Router 由注入 ServeConfig（固定 token + 临时 statedir +
//! caps_override）直构；apply/set 的 400/409 案都在解析/锁层终结（replay 不可达）。
//! 锁占用案用「本测试进程 pid+starttime 伪装载」——state::acquire_write_lock 的判活
//! 语义直接消费，钉住 REST×CLI 互斥面。

use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use mediaservo_weaknet::engine::Env;
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scope::Capabilities;
use mediaservo_weaknet::server::{build_router, ServeConfig};
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
            ifb_reason: "test 注入".into(),
        }),
        sse_frames_limit: None,
    }
}

fn req(method: Method, uri: &str, host: &str, bearer: Option<&str>, body: Body) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, host);
    if let Some(t) = bearer {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    b.body(body).unwrap()
}

fn get(uri: &str, bearer: Option<&str>) -> Request<Body> {
    req(Method::GET, uri, "127.0.0.1:19999", bearer, Body::empty())
}

fn post(uri: &str, json_body: &str) -> Request<Body> {
    req(
        Method::POST,
        uri,
        "127.0.0.1:19999",
        Some(TOKEN),
        Body::from(json_body.to_string()),
    )
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("响应非 JSON: {e}: {bytes:?}"))
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
    // 门序钉案：Host 门槛在 token 校验之前——rebinding 流量不泄露 token 对/错信号。
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
    // 错 Origin → 403
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
    // 对 Origin → 200
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
    // 无 Origin（curl 形）放行——design 裁决：Origin 只约束浏览器跨源（文档化）。
    let r = router
        .oneshot(req(Method::GET, "/v1/state", good_host, Some(TOKEN), Body::empty()))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

#[tokio::test]
async fn sse_query_token_exemption_and_first_frame() {
    let mut c = cfg("sse");
    c.sse_frames_limit = Some(1); // 发一帧即收口断 body（to_bytes 可完结）
    let router = build_router(c);
    // 无 token → 401
    let r = router.clone().oneshot(get("/v1/events", None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // 错 query token → 401
    let r = router
        .clone()
        .oneshot(get("/v1/events?token=wrong", None))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    // query token → 200 + 首帧含 spec/streams（interval 首拍即发，文档化「秒开」）
    let r = router
        .oneshot(get(&format!("/v1/events?token={TOKEN}"), None))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        r.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap(),
        "text/event-stream"
    );
    let raw = tokio::time::timeout(Duration::from_secs(10), to_bytes(r.into_body(), 1 << 20))
        .await
        .expect("首帧应在超时前送达")
        .unwrap();
    let text = String::from_utf8_lossy(&raw);
    let data = text
        .lines()
        .find_map(|l| l.strip_prefix("data:"))
        .expect("SSE data 行必在");
    let frame: serde_json::Value = serde_json::from_str(data.trim()).unwrap();
    for key in ["spec", "scope", "dir", "expires_at", "job", "tc", "streams"] {
        assert!(frame.get(key).is_some(), "状态帧缺字段 {key}: {frame}");
    }
    assert_eq!(frame["spec"], serde_json::Value::Null, "无 state = spec null（键恒在）");
    assert_eq!(frame["streams"], serde_json::json!([]));
    assert!(frame.get("ev").is_none(), "无新事件不带 ev（design §API）");
}

#[tokio::test]
async fn state_inactive_shape() {
    let r = build_router(cfg("state0"))
        .oneshot(get("/v1/state", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(body_json(r).await, serde_json::json!({"active": false}));
}

#[tokio::test]
async fn profiles_and_capabilities_shapes() {
    let router = build_router(cfg("prof"));
    let r = router.clone().oneshot(get("/v1/profiles", Some(TOKEN))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["profiles"].is_array());
    let r = router
        .clone()
        .oneshot(get("/v1/capabilities", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(
        body_json(r).await,
        serde_json::json!({"seed": true, "dir_lo": true, "ifb_ingress": false, "ifb_reason": "test 注入"})
    );
}

#[tokio::test]
async fn streams_graceful_empty_when_unconfigured() {
    let r = build_router(cfg("streams"))
        .oneshot(get("/v1/streams", Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v = body_json(r).await;
    assert_eq!(v["streams"], serde_json::json!([]), "server_url 不可得 = 200 + 空表 + note（禁 500）");
    assert!(v["note"].as_str().unwrap().contains("stats"), "{v}");
}

#[tokio::test]
async fn set_without_state_is_409_with_guide() {
    let r = build_router(cfg("set409"))
        .oneshot(post("/v1/set", r#"{"rtt_ms":20}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let v = body_json(r).await;
    assert_eq!(v["error"], "no active spec — 先 apply/开总闸", "UI 引导文案钉死（design §UI 写路径）");
}

#[tokio::test]
async fn apply_param_validation_400s() {
    let router = build_router(cfg("400"));
    let cases: &[&str] = &[
        "[1,2]",                                        // 非对象
        r#"{"duration":"abc"}"#,                        // 类型错
        r#"{"duration":1}"#,                            // <5（validate_duration 表）
        r#"{"rtt_ms":"80"}"#,                           // 数字键给字符串
        r#"{"bogus_key":1}"#,                           // 未知 spec 键（typos 显性化）
        r#"{"loss":"2","loss_mode":"bogus"}"#,          // loss_mode 域外
        r#"{"ports":"notarray"}"#,                      // ports 型错
        r#"{"stream_scope":{"rooms":[]}}"#,             // 空集
    ];
    for body in cases {
        let r = router
            .clone()
            .oneshot(post("/v1/apply", body))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "case {body:?} 应 400");
        assert!(body_json(r).await.get("error").is_some(), "错误体 {{error}} 形（{body:?}）");
    }
}

/// 活跃 job 态的 state.json（alive=true 用本进程 pid+starttime = 判活必真；false 用不可存在 pid）。
fn write_live_job(dirs: &Dirs, alive: bool) {
    use mediaservo_weaknet::state::{ChannelSer, JobRef, State, STATE_SCHEMA, Teardown};
    use mediaservo_weaknet::spec::{Dir, ImpairSpec, ScopeSel};
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
        job: Some(JobRef {
            name: "matrix-job".into(),
            pid,
            starttime: fuse::read_proc_starttime(std::process::id()).unwrap_or(0),
            done: 1,
            total: 3,
        }),
        teardown: Teardown {
            channel: ChannelSer::LocalRoot,
            sidecar: None,
            steps: vec![],
        },
    };
    st.write_to(&dirs.state_json()).unwrap();
}

#[tokio::test]
async fn scenario_run_stop_t8_semantics() {
    // T8 真实面（旧 501 桩已退）：400 门不变；合法 plan → start 相在 stats 定向处
    // 报因 500（闭口端口注入 = 零 tc/docker 触达的确定性）；stop = 200/幂等/陈旧自清。
    let mut c = cfg("scen");
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    c.server_url = Some(format!("http://127.0.0.1:{dead}"));
    let dirs = c.dirs.clone();
    let router = build_router(c);
    for bad in [r#"{"file":"../evil.yaml"}"#, r#"{"file":"/etc/passwd"}"#, r#"{"file":"sub/dir.yaml"}"#, r#"{"inline":1}"#, "{}"] {
        let r = router
            .clone()
            .oneshot(post("/v1/scenario/run", bad))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "case {bad:?} 应 400（穿越/绝对/型错/缺参——start 相前置真校验）");
    }
    let big = format!(r#"{{"inline":"{}"}}"#, "x".repeat(33 * 1024));
    let r = router.clone().oneshot(post("/v1/scenario/run", &big)).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "inline >32KB → 400");
    // 合法 inline（文法过）→ 定向 stats 不可达 = 诚实 500，且零副作用（未触锁/未写 state）
    let r = router
        .clone()
        .oneshot(post(
            "/v1/scenario/run",
            r#"{"inline":"scenario:\n  baseline_s: 1\n  steps:\n    - at_s: 5\n      set: {rtt_ms: 50}\n"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR, "合法 plan + stats 不可达 → 500（旧 501 桩退役）");
    assert!(body_json(r).await["error"].as_str().is_some(), "错误体 {{error}} 形");
    assert!(!dirs.state_json().exists() && !dirs.lock().exists(), "start 相前置失败 = 零落盘");
    // 测试注入 scenario 根（WEAKNET_ASSETS_DIR）：合法 file → 盘上解析命中 → 同一 500 报因
    let assets = std::env::temp_dir().join(format!("wnet-scen-{}", std::process::id()));
    std::fs::create_dir_all(assets.join("scenarios")).unwrap();
    std::fs::write(
        assets.join("scenarios").join("mini.yaml"),
        "scenario:\n  baseline_s: 1\n  steps:\n    - at_s: 5\n      set: {rtt_ms: 50}\n",
    )
    .unwrap();
    // SAFETY: 本 binary 仅此测试读 WEAKNET_ASSETS_DIR（/v1/scenarios 无并发案），请求串行发出
    unsafe { std::env::set_var("WEAKNET_ASSETS_DIR", &assets) };
    let r = router
        .clone()
        .oneshot(post("/v1/scenario/run", r#"{"file":"mini.yaml"}"#))
        .await
        .unwrap();
    // SAFETY: 请求已完成，回收窗口
    unsafe { std::env::remove_var("WEAKNET_ASSETS_DIR") };
    assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR, "file 解析命中后同一 start 相报因（非 400/404/501）");
    // stop：无活跃 job → 200 幂等
    let r = router.clone().oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["detail"].as_str().unwrap().contains("幂等"));
    // stop：属主存活（本进程 pid）→ 200 + cancel 旗标落盘
    write_live_job(&dirs, true);
    let r = router.clone().oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["detail"].as_str().unwrap().contains("已请求停止"));
    assert!(mediaservo_weaknet::scenario::cancel_path(&dirs).exists(), "存活属主 → 写 stop 旗标");
    std::fs::remove_file(mediaservo_weaknet::scenario::cancel_path(&dirs)).unwrap();
    // stop：死主（pid 不可存在）→ 200 + 盘上 job 旗标自清
    write_live_job(&dirs, false);
    let r = router.clone().oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(body_json(r).await["detail"].as_str().unwrap().contains("陈旧"));
    let st = mediaservo_weaknet::state::State::read_from(&dirs.state_json()).unwrap().unwrap();
    assert!(st.job.is_none(), "死主旗标落盘自清");
    std::fs::remove_dir_all(&dirs.statedir).ok();
    std::fs::remove_dir_all(&assets).ok();
}

#[tokio::test]
async fn live_scenario_job_gates_apply_set_409() {
    // rev-2.2 独占门 REST 面：job 存活期 apply/set 均在 replay 之前 409（零 tc 触达）。
    let c = cfg("job409");
    let dirs = c.dirs.clone();
    write_live_job(&dirs, true);
    let router = build_router(c);
    let r = router
        .clone()
        .oneshot(post("/v1/apply", r#"{"rtt_ms":80,"duration":40}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT, "job 活跃期 apply = 409");
    assert!(body_json(r).await["error"].as_str().unwrap().contains("scenario"));
    let r = router
        .clone()
        .oneshot(post("/v1/set", r#"{"rtt_ms":40}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT, "job 活跃期 set = 409（对称）");
    // 死主旗标不拦：apply 放行到下一层（stats 不可达 500，而非 409）
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
    assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR, "陈旧 job 判亡放行（gate 过后再报 env 因）");
    std::fs::remove_dir_all(&dirs.statedir).ok();
    std::fs::remove_dir_all(&dirs2.statedir).ok();
}

#[tokio::test]
async fn apply_conflicts_with_held_write_lock_409() {
    // REST×CLI 互斥面：锁文件伪装载（本进程 pid+starttime → holder_alive 判活为真）。
    // 合法 spec 但锁忙 = 409（engine::take_write_lock 在 resolve/replay 之前，零副作用）。
    let c = cfg("lock409");
    let dirs = c.dirs.clone();
    let pid = std::process::id();
    let st = fuse::read_proc_starttime(pid).expect("自身 starttime 必可读");
    std::fs::write(dirs.lock(), format!("{pid} {st}\n")).unwrap();
    let r = build_router(c)
        .oneshot(post("/v1/apply", r#"{"rtt_ms":80,"duration":40}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT, "锁占用 → 409（Fail::conflict 映射）");
    std::fs::remove_file(dirs.lock()).ok();
    std::fs::remove_dir_all(&dirs.statedir).ok();
}
