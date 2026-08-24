//! PSK admin API integration tests (psk-admin-management T4).
//!
//! Covers: auth gates (401/403), psk_hint masking in config, one-time view
//! + audit, rotation (hot-effective: old psk rejected 4003 / new accepted),
//! config file write-back, audit ring events.

use axum::body::Body;
use http::{Method, Request, StatusCode};
use mediaservo_common::auth::JwtClaims;
use mediaservo_server::accounts::AccountRegistry;
use mediaservo_server::admin::{AdminState, admin_router};
use mediaservo_server::audit::{self, AuditEvent};
use mediaservo_server::devices;
use mediaservo_server::signaling::SignalingServer;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower::util::ServiceExt;

fn temp_path(tag: &str, ext: &str) -> String {
    format!("/tmp/ms-psk-{tag}-{}.{ext}", uuid::Uuid::new_v4())
}

#[cfg(feature = "sfu-mediasoup")]
async fn make_state(config_path: String) -> (AdminState, String) {
    let sfu =
        std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
    let mut signaling = SignalingServer::new(sfu.clone(), 65536, None);
    let psk_state = Arc::new(std::sync::RwLock::new(Some("old-secret-123".to_string())));
    signaling.psk_state = Arc::clone(&psk_state);
    let (event_tx, _) = broadcast::channel(256);
    let devices_path = temp_path("dev", "yaml");
    let accounts_path = temp_path("acc", "yaml");
    let state = AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path,
        psk_state: Arc::clone(&psk_state),
        config_path,
        device_registry: Arc::new(devices::DeviceRegistry::empty()),
        devices_path,
        sfu_manager: sfu,
    };
    (state, devices_path)
}

#[cfg(not(feature = "sfu-mediasoup"))]
async fn make_state(config_path: String) -> (AdminState, String) {
    let mut signaling = SignalingServer::new(65536, None);
    let psk_state = Arc::new(std::sync::RwLock::new(Some("old-secret-123".to_string())));
    signaling.psk_state = Arc::clone(&psk_state);
    let (event_tx, _) = broadcast::channel(256);
    let devices_path = temp_path("dev", "yaml");
    let accounts_path = temp_path("acc", "yaml");
    let state = AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path,
        psk_state: Arc::clone(&psk_state),
        config_path,
        device_registry: Arc::new(devices::DeviceRegistry::empty()),
        devices_path: devices_path.clone(),
    };
    (state, devices_path)
}

fn role_token(state: &AdminState, role: &str) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
        as usize;
    let claims = JwtClaims {
        sub: role.into(),
        iat: now,
        exp: now + 3600,
        role: Some(role.into()),
        vehicles: None,
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(
            state.admin_jwt_secret.as_deref().unwrap().as_bytes(),
        ),
    )
    .unwrap()
}

fn admin_token(state: &AdminState) -> String {
    role_token(state, "admin")
}

fn dispatcher_token(state: &AdminState) -> String {
    role_token(state, "dispatcher")
}

fn auth_request(method: Method, uri: &str, token: Option<&str>, body: Body) -> Request<Body> {
    let is_post = method == Method::POST;
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    if is_post {
        b = b.header("content-type", "application/json");
    }
    b.body(body).unwrap()
}

async fn json_of(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 8192).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ── Auth gates ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn psk_endpoints_require_auth() {
    let (state, _) = make_state(temp_path("auth", "yaml")).await;
    let app = admin_router(state.clone());
    for (method, uri) in [(Method::GET, "/api/admin/psk"), (Method::POST, "/api/admin/psk")] {
        let res =
            app.clone().oneshot(auth_request(method, uri, None, Body::empty())).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn psk_dispatcher_denied() {
    let (state, _) = make_state(temp_path("disp", "yaml")).await;
    let app = admin_router(state.clone());
    // dispatcher 只读也不放行 — psk 为 admin 专属端点
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/psk",
            Some(&dispatcher_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_ne!(res.status(), StatusCode::OK, "dispatcher 禁止查看 psk");
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/psk",
            Some(&dispatcher_token(&state)),
            Body::from(r#"{"password":"new-secret-123"}"#),
        ))
        .await
        .unwrap();
    assert_ne!(res.status(), StatusCode::OK, "dispatcher 禁止轮换 psk");
}

// ── config hint ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn config_contains_masked_hint_not_plaintext() {
    let (state, _) = make_state(temp_path("hint", "yaml")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/config",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["psk_hint"], "ol··", "掩码 = 前两字符 + ··");
    let raw = serde_json::to_string(&body).unwrap();
    assert!(!raw.contains("old-secret-123"), "hint 响应不得含明文: {raw}");
}

// ── one-time view + audit ────────────────────────────────────────────────────

#[tokio::test]
async fn psk_view_returns_plaintext_once_and_audits() {
    let (state, _) = make_state(temp_path("view", "yaml")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/psk",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["psk"], "old-secret-123");
    assert_eq!(body["hint"], "ol··");
    // audit 落库
    let recent = audit::recent();
    assert!(
        recent.iter().any(|e| matches!(e, AuditEvent::PskViewed { actor } if actor == "admin")),
        "PskViewed 应入 audit 环: {recent:?}"
    );
}

#[tokio::test]
async fn psk_view_404_when_unset() {
    let (state, _) = make_state(temp_path("unset", "yaml")).await;
    *state.psk_state.write().unwrap_or_else(|e| e.into_inner()) = None;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/psk",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "未配置 psk → 404");
}

// ── rotation（热生效 + 写回 + audit）──────────────────────────────────────────

#[tokio::test]
async fn psk_rotate_hot_effective_and_persists() {
    let cfg_path = temp_path("rotate", "yaml");
    let (state, _) = make_state(cfg_path.clone()).await;
    // 预置 server.yaml（含 inline 注释）
    std::fs::write(&cfg_path, "listen:\n  host: \"0.0.0.0\"\npsk: \"old-secret-123\"   # PIT-49\n")
        .unwrap();

    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/psk",
            Some(&admin_token(&state)),
            Body::from(r#"{"password":"brand-new-secret-9"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["psk"], "brand-new-secret-9");

    // 热生效：内存共享态已更新
    assert_eq!(
        *state.psk_state.read().unwrap_or_else(|e| e.into_inner()),
        Some("brand-new-secret-9".to_string())
    );
    // 落盘 + 注释保留
    let on_disk = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(on_disk.contains("psk: \"brand-new-secret-9\"  # PIT-49"), "写回保留注释: {on_disk}");
    assert!(!on_disk.contains("old-secret-123"), "旧值清除: {on_disk}");
    // audit
    assert!(
        audit::recent()
            .iter()
            .any(|e| matches!(e, AuditEvent::PskRotated { actor } if actor == "admin")),
        "PskRotated 应入 audit 环"
    );
}

#[tokio::test]
async fn psk_rotate_invalid_value_400() {
    let (state, _) = make_state(temp_path("bad", "yaml")).await;
    let app = admin_router(state.clone());
    for bad in ["", "short", "has space", "has\"quote"] {
        let payload = format!(r#"{{"password": "{bad}"}}"#);
        let res = app
            .clone()
            .oneshot(auth_request(
                Method::POST,
                "/api/admin/psk",
                Some(&admin_token(&state)),
                Body::from(payload),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "bad psk {bad:?} → 400");
    }
    // 轮换失败不得改变现有 psk
    assert_eq!(
        *state.psk_state.read().unwrap_or_else(|e| e.into_inner()),
        Some("old-secret-123".to_string())
    );
}

#[tokio::test]
async fn psk_rotate_generates_when_absent() {
    let cfg_path = temp_path("gen", "yaml");
    let (state, _) = make_state(cfg_path.clone()).await;
    std::fs::write(&cfg_path, "listen:\n  host: \"x\"\n").unwrap();
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/psk",
            Some(&admin_token(&state)),
            Body::from("{}"),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    let generated = body["psk"].as_str().unwrap();
    assert_eq!(generated.len(), 32, "随机 32B hex（uuid v4 去横线）");
    assert!(!generated.contains('-'));
    let on_disk = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(on_disk.contains(&format!("psk: \"{generated}\"")), "追加写回: {on_disk}");
}
