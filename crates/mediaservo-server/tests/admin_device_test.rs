//! Device admin API integration tests (unified-device-admin T4).
//!
//! Exercises /api/admin/devices endpoints: auth gates, CRUD, hot-reload
//! (register → authenticate in same process, no restart), revoke, reset-secret.
//! Every test uses a unique devices.yaml path to avoid parallel-file races.

use axum::body::Body;
use http::{Method, Request, StatusCode};
use mediaservo_common::auth::JwtClaims;
use mediaservo_server::accounts::AccountRegistry;
use mediaservo_server::admin::{AdminState, admin_router};
use mediaservo_server::devices;
use mediaservo_server::signaling::SignalingServer;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower::util::ServiceExt;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn temp_devices_path(tag: &str) -> String {
    format!("/tmp/ms-admin-dev-{tag}-{}.yaml", uuid::Uuid::new_v4())
}

#[cfg(feature = "sfu-mediasoup")]
async fn make_state(devices_path: String) -> AdminState {
    let sfu =
        std::sync::Arc::new(
            mediaservo_server::sfu::SfuManager::new_with_port(
                mediaservo_server::sfu::random_udp_port(),
            )
            .await
            .unwrap(),
        );
    let signaling = SignalingServer::new(sfu.clone(), 65536, None);
    let (event_tx, _) = broadcast::channel(256);
    AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path: "/tmp/ms-admin-dev-accounts.yaml".into(),
        psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
        config_path: "/tmp/ms-admin-dev-server.yaml".into(),
        device_registry: Arc::new(devices::DeviceRegistry::empty()),
        devices_path,
        sfu_manager: sfu,
    }
}

#[cfg(not(feature = "sfu-mediasoup"))]
async fn make_state(devices_path: String) -> AdminState {
    let signaling = SignalingServer::new(65536, None);
    let (event_tx, _) = broadcast::channel(256);
    AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path: "/tmp/ms-admin-dev-accounts.yaml".into(),
        psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
        config_path: "/tmp/ms-admin-dev-server.yaml".into(),
        device_registry: Arc::new(devices::DeviceRegistry::empty()),
        devices_path,
    }
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
        // axum Json<T> extractor 要求 content-type
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
async fn devices_requires_auth() {
    let state = make_state(temp_devices_path("auth")).await;
    let app = admin_router(state.clone());
    let res = app
        .clone()
        .oneshot(auth_request(Method::GET, "/api/admin/devices", None, Body::empty()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn devices_dispatcher_read_only() {
    let state = make_state(temp_devices_path("dispatcher")).await;
    let app = admin_router(state.clone());
    // GET 允许
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices",
            Some(&dispatcher_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // POST（注册）拒绝 — dispatcher 只读
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&dispatcher_token(&state)),
            Body::from(r#"{"device_id": "ms-disp-1"}"#),
        ))
        .await
        .unwrap();
    assert_ne!(res.status(), StatusCode::OK, "dispatcher 写操作必须被拒");
}

// ── CRUD + hot-reload ────────────────────────────────────────────────────────

#[tokio::test]
async fn devices_list_empty() {
    let state = make_state(temp_devices_path("list-empty")).await;
    let app = admin_router(state.clone());
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert!(body["devices"].as_array().unwrap().is_empty(), "{body}");
    assert_eq!(body["count"], 0);
}

#[tokio::test]
async fn devices_register_and_hot_authenticate() {
    let state = make_state(temp_devices_path("register")).await;
    let app = admin_router(state.clone());
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&admin_token(&state)),
            Body::from(r#"{"device_id": "ms-hot-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "{:?}", res.status());
    let body = json_of(res).await;
    let secret = body["secret"].as_str().unwrap().to_string();
    assert_eq!(body["device_id"], "ms-hot-1");
    assert!(body["secret_hash"].as_str().unwrap().starts_with("sha256:"));

    // 热重载实证：同一进程内（未重启）注册后鉴权立即通过
    assert_eq!(
        devices::authenticate(&state.device_registry, Some("ms-hot-1"), Some(&secret),),
        Some(Ok(()))
    );

    // 列表包含新设备
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    let body = json_of(res).await;
    let ids: Vec<&str> = body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["device_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["ms-hot-1"], "列表应含新注册设备");
}

#[tokio::test]
async fn devices_register_duplicate_409() {
    let state = make_state(temp_devices_path("dup")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);

    let post = async |app: &axum::Router<()>, token: &str| {
        app.clone()
            .oneshot(auth_request(
                Method::POST,
                "/api/admin/devices",
                Some(token),
                Body::from(r#"{"device_id": "ms-dup-1"}"#),
            ))
            .await
            .unwrap()
    };
    let res1 = post(&app, &token).await;
    assert_eq!(res1.status(), StatusCode::CREATED);
    let res2 = post(&app, &token).await;
    assert_eq!(res2.status(), StatusCode::CONFLICT, "重复注册 → 409");
}

#[tokio::test]
async fn devices_invalid_id_400() {
    let state = make_state(temp_devices_path("badid")).await;
    let app = admin_router(state.clone());
    for bad in ["", "bad id with spaces!!", &"x".repeat(65)] {
        let payload = format!(r#"{{"device_id": "{bad}"}}"#);
        let res = app
            .clone()
            .clone()
            .oneshot(auth_request(
                Method::POST,
                "/api/admin/devices",
                Some(&admin_token(&state)),
                Body::from(payload),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "bad id {bad:?} → 400");
    }
}

#[tokio::test]
async fn devices_revoke_then_auth_fails() {
    let state = make_state(temp_devices_path("revoke")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);

    // 注册
    let res = app
        .clone()
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&token),
            Body::from(r#"{"device_id": "ms-rev-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = json_of(res).await;
    let secret = body["secret"].as_str().unwrap().to_string();

    // 吊销
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::DELETE,
            "/api/admin/devices/ms-rev-1",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 热重载实证：吊销后立即 4010（Unknown）
    assert_eq!(
        devices::authenticate(&state.device_registry, Some("ms-rev-1"), Some(&secret)),
        Some(Err(devices::DeviceAuthError::Unknown))
    );
}

#[tokio::test]
async fn devices_revoke_unknown_404() {
    let state = make_state(temp_devices_path("rev-unknown")).await;
    let app = admin_router(state.clone());
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::DELETE,
            "/api/admin/devices/ms-nope",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn devices_reset_secret_rotates() {
    let state = make_state(temp_devices_path("reset")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);

    let res = app
        .clone()
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&token),
            Body::from(r#"{"device_id": "ms-rot-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = json_of(res).await;
    let old_secret = body["secret"].as_str().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/ms-rot-1/reset-secret",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = json_of(res).await;
    let new_secret = body["secret"].as_str().unwrap().to_string();
    assert_ne!(old_secret, new_secret);

    // 旧密钥失效（BadSecret），新密钥生效（热重载）
    assert_eq!(
        devices::authenticate(&state.device_registry, Some("ms-rot-1"), Some(&old_secret)),
        Some(Err(devices::DeviceAuthError::BadSecret))
    );
    assert_eq!(
        devices::authenticate(&state.device_registry, Some("ms-rot-1"), Some(&new_secret)),
        Some(Ok(()))
    );
}

#[tokio::test]
async fn devices_register_with_provided_secret() {
    let state = make_state(temp_devices_path("prov-secret")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&admin_token(&state)),
            Body::from(r#"{"device_id": "ms-prov-1", "secret": "admin-chosen-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body = json_of(res).await;
    assert_eq!(body["secret"], "admin-chosen-1", "自带 secret 原样返回");
    assert_eq!(
        body["secret_hash"],
        serde_json::Value::String(format!("sha256:{}", {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(b"ms-prov-1:admin-chosen-1");
            h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
        }))
    );
    // 热重载实证：带自备 secret 注册后可立即鉴权
    assert_eq!(
        devices::authenticate(&state.device_registry, Some("ms-prov-1"), Some("admin-chosen-1")),
        Some(Ok(()))
    );
}

#[tokio::test]
async fn devices_register_invalid_provided_secret_400() {
    let state = make_state(temp_devices_path("bad-prov")).await;
    let app = admin_router(state.clone());
    for bad in ["", "short", "has space"] {
        let payload = format!(r#"{{"device_id": "ms-bad-1", "secret": "{bad}"}}"#);
        let res = app
            .clone()
            .oneshot(auth_request(
                Method::POST,
                "/api/admin/devices",
                Some(&admin_token(&state)),
                Body::from(payload),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "bad secret {bad:?} → 400");
    }
}

// ── make_state_loaded: 从 devices.yaml 加载注册表（持久化/重载闭环测试用）──

#[cfg(feature = "sfu-mediasoup")]
async fn make_state_loaded(devices_path: String) -> AdminState {
    let sfu = std::sync::Arc::new(
        mediaservo_server::sfu::SfuManager::new_with_port(
            mediaservo_server::sfu::random_udp_port(),
        )
        .await
        .unwrap(),
    );
    let signaling = SignalingServer::new(sfu.clone(), 65536, None);
    let (event_tx, _) = broadcast::channel(256);
    AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path: "/tmp/ms-admin-dev-accounts.yaml".into(),
        psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
        config_path: "/tmp/ms-admin-dev-server.yaml".into(),
        device_registry: Arc::new(devices::DeviceRegistry::load(&devices_path).unwrap()),
        devices_path,
        sfu_manager: sfu,
    }
}

#[cfg(not(feature = "sfu-mediasoup"))]
async fn make_state_loaded(devices_path: String) -> AdminState {
    let signaling = SignalingServer::new(65536, None);
    let (event_tx, _) = broadcast::channel(256);
    AdminState {
        signaling,
        event_tx,
        admin_jwt_secret: Some("test-secret-min-32-bytes!!!".into()),
        listen_host: "0.0.0.0".into(),
        listen_port: 9800,
        rate_limit: 100,
        room_capacity: 10,
        consumer_limit_per_stream: 50,
        accounts: Arc::new(AccountRegistry::empty()),
        accounts_path: "/tmp/ms-admin-dev-accounts.yaml".into(),
        psk_state: std::sync::Arc::new(std::sync::RwLock::new(None)),
        config_path: "/tmp/ms-admin-dev-server.yaml".into(),
        device_registry: Arc::new(devices::DeviceRegistry::load(&devices_path).unwrap()),
        devices_path,
    }
}

// ── 增删改查补齐（secret 语义 + 列表联动 + 持久化闭环）────────────────

#[tokio::test]
async fn devices_reset_secret_unknown_404() {
    // Arrange: 空注册表
    let state = make_state(temp_devices_path("reset-unknown")).await;
    let app = admin_router(state.clone());

    // Act: 对不存在设备 reset-secret
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/ms-nope/reset-secret",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();

    // Assert: 404（对称 revoke_unknown_404）
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "未知设备 reset-secret → 404");
}

#[tokio::test]
async fn devices_list_never_leaks_secret() {
    // Arrange: 注册 ms-leak-1（生成 secret）——明文只应出现在注册响应（C33 一次性语义）
    let state = make_state(temp_devices_path("no-leak")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&token),
            Body::from(r#"{"device_id": "ms-leak-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let reg_body = json_of(res).await;
    assert!(reg_body["secret"].is_string(), "注册响应含一次明文");

    // Act: 列表查询
    let res = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();

    // Assert: 列表条目绝不包含 secret/secret_hash（后续任何查询不可再见明文）
    let body = json_of(res).await;
    let entries = body["devices"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry["device_id"], "ms-leak-1");
    assert!(
        entry.get("secret").is_none() && entry.get("secret_hash").is_none(),
        "列表不得泄露 secret/secret_hash: {entry}"
    );
    assert!(body.get("secret").is_none(), "顶层也不得含 secret");
}

#[tokio::test]
async fn devices_revoked_removed_from_list() {
    // Arrange: 注册 ms-a + ms-b
    let state = make_state(temp_devices_path("revoke-list")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);
    for id in ["ms-a", "ms-b"] {
        let res = app
            .clone()
            .oneshot(auth_request(
                Method::POST,
                "/api/admin/devices",
                Some(&token),
                Body::from(format!(r#"{{"device_id": "{id}"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    // Act: 吊销 ms-a
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::DELETE,
            "/api/admin/devices/ms-a",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Assert: 列表只含存活的 ms-b
    let res = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    let body = json_of(res).await;
    let ids: Vec<&str> = body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["device_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["ms-b"], "吊销后列表应移除该设备");
}

#[tokio::test]
async fn devices_register_persists_across_reload() {
    // Arrange: 注册 ms-persist-1（自带 secret）→ save 落盘
    let path = temp_devices_path("persist");
    let state = make_state(path.clone()).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&admin_token(&state)),
            Body::from(r#"{"device_id": "ms-persist-1", "secret": "persist-secret-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "注册应 201");

    // Act: 用同一 devices.yaml 重建 state（模拟进程重启——从文件加载）
    let state2 = make_state_loaded(path).await;

    // Assert: 重载后鉴权通过 + 列表可见（save→load 闭环）
    assert_eq!(
        devices::authenticate(&state2.device_registry, Some("ms-persist-1"), Some("persist-secret-1")),
        Some(Ok(())),
        "重载后原 secret 应可鉴权"
    );
    let app2 = admin_router(state2.clone());
    let res = app2
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/devices",
            Some(&admin_token(&state2)),
            Body::empty(),
        ))
        .await
        .unwrap();
    let body = json_of(res).await;
    let ids: Vec<&str> = body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["device_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["ms-persist-1"], "重载后列表应含持久化设备");
}

#[tokio::test]
async fn devices_register_generated_secret_is_uuid_v4() {
    // Arrange/Act: 服务器生成 secret（不提供 secret 字段）
    let state = make_state(temp_devices_path("gen-secret")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices",
            Some(&admin_token(&state)),
            Body::from(r#"{"device_id": "ms-gen-1"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // Assert: uuid v4 形态（36 字符 + 4 连字符）——配发侧可复制可用
    let body = json_of(res).await;
    let secret = body["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 36, "生成 secret 应为 uuid v4: {secret}");
    assert_eq!(secret.chars().filter(|c| *c == '-').count(), 4, "uuid 连字符数: {secret}");
}
