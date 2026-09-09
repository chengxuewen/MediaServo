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

#[tokio::test]
async fn scenario_basename_gate_400_before_honest_501() {
    let router = build_router(cfg("scen"));
    for bad in [r#"{"file":"../evil.yaml"}"#, r#"{"file":"/etc/passwd"}"#, r#"{"file":"sub/dir.yaml"}"#, r#"{"inline":1}"#, "{}"] {
        let r = router
            .clone()
            .oneshot(post("/v1/scenario/run", bad))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "case {bad:?} 应 400（穿越/绝对/型错/缺参——501 前真校验）");
    }
    let big = format!(r#"{{"inline":"{}"}}"#, "x".repeat(33 * 1024));
    let r = router.clone().oneshot(post("/v1/scenario/run", &big)).await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "inline >32KB → 400");
    // 合法 basename → 501（诚实报 T8，不 404）
    let r = router
        .clone()
        .oneshot(post("/v1/scenario/run", r#"{"file":"cell-edge.yaml"}"#))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_IMPLEMENTED);
    assert!(body_json(r).await["error"].as_str().unwrap().contains("T8"));
    let r = router.oneshot(post("/v1/scenario/stop", "{}")).await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_IMPLEMENTED);
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
