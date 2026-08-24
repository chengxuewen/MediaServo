//! Account admin API integration tests (unified-device-admin T7).
//!
//! Exercises /api/admin/accounts endpoints: auth gates, CRUD, hot-reload
//! (create → /api/auth/login succeeds in same process, no restart), role
//! rotation, self-delete guard, list excludes password_hash.
//! Unique temp yaml paths per test to avoid parallel-file races.

use axum::body::Body;
use http::{Method, Request, StatusCode};
use mediaservo_common::auth::JwtClaims;
use mediaservo_server::accounts::AccountRegistry;
use mediaservo_server::admin::{AdminState, admin_router, login_router};
use mediaservo_server::devices;
use mediaservo_server::signaling::SignalingServer;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower::util::ServiceExt;

fn temp_path(tag: &str, ext: &str) -> String {
    format!("/tmp/ms-admin-acct-{tag}-{}.{ext}", uuid::Uuid::new_v4())
}

#[cfg(feature = "sfu-mediasoup")]
async fn make_state(devices_path: String, accounts_path: String) -> AdminState {
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
        accounts_path,
        device_registry: Arc::new(devices::DeviceRegistry::empty()),
        devices_path,
        sfu_manager: sfu,
    }
}

#[cfg(not(feature = "sfu-mediasoup"))]
async fn make_state(devices_path: String, accounts_path: String) -> AdminState {
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
        accounts_path,
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
    let is_post_or_put = method == Method::POST || method == Method::PUT;
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {t}"));
    }
    if is_post_or_put {
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
async fn accounts_requires_auth() {
    let state = make_state(temp_path("auth", "yaml"), temp_path("auth", "yaml")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(Method::GET, "/api/admin/accounts", None, Body::empty()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn accounts_dispatcher_read_only() {
    let state = make_state(temp_path("disp", "yaml"), temp_path("disp", "yaml")).await;
    let app = admin_router(state.clone());
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/accounts",
            Some(&dispatcher_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/accounts",
            Some(&dispatcher_token(&state)),
            Body::from(r#"{"username":"eve","password":"pw","role":"viewer","vehicles":[]}"#),
        ))
        .await
        .unwrap();
    assert_ne!(res.status(), StatusCode::OK, "dispatcher 写操作必须被拒");
}

// ── CRUD + hot-reload ────────────────────────────────────────────────────────

#[tokio::test]
async fn accounts_create_then_login_hot() {
    let state = make_state(temp_path("create", "yaml"), temp_path("create", "yaml")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/accounts",
            Some(&admin_token(&state)),
            Body::from(
                r#"{"username":"bob","password":"pw123","role":"operator","vehicles":["ms-car9"]}"#,
            ),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "{:?}", res.status());

    // 热重载实证：建号后不经重启，/api/auth/login 立即成功
    let login_app = login_router(state.clone());
    let res = login_app
        .oneshot(auth_request(
            Method::POST,
            "/api/auth/login",
            None,
            Body::from(r#"{"username":"bob","password":"pw123"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    assert_eq!(body["role"], "operator");
    assert!(!body["token"].as_str().unwrap().is_empty(), "应签发 JWT");
}

#[tokio::test]
async fn accounts_list_excludes_password_hash() {
    let state = make_state(temp_path("list", "yaml"), temp_path("list", "yaml")).await;
    let app = admin_router(state.clone());
    // 创建两个账号
    for u in ["carol", "dave"] {
        let payload =
            format!(r#"{{"username":"{u}","password":"pw","role":"viewer","vehicles":[]}}"#);
        let res = app
            .clone()
            .oneshot(auth_request(
                Method::POST,
                "/api/admin/accounts",
                Some(&admin_token(&state)),
                Body::from(payload),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
    let res = app
        .oneshot(auth_request(
            Method::GET,
            "/api/admin/accounts",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = json_of(res).await;
    // 无哈希外泄
    let raw = serde_json::to_string(&body).unwrap();
    assert!(!raw.contains("password_hash"), "password_hash 绝不外泄: {raw}");
    let accounts = body["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(body["count"], 2);
}

#[tokio::test]
async fn accounts_create_duplicate_409() {
    let state = make_state(temp_path("dup", "yaml"), temp_path("dup", "yaml")).await;
    let app = admin_router(state.clone());
    let post = |app: &axum::Router<()>| {
        app.clone().oneshot(auth_request(
            Method::POST,
            "/api/admin/accounts",
            Some(&admin_token(&state)),
            Body::from(r#"{"username":"eve","password":"pw","role":"viewer","vehicles":[]}"#),
        ))
    };
    let res1 = post(&app).await.unwrap();
    assert_eq!(res1.status(), StatusCode::CREATED);
    let res2 = post(&app).await.unwrap();
    assert_eq!(res2.status(), StatusCode::CONFLICT, "重复创建 → 409");
}

#[tokio::test]
async fn accounts_invalid_role_400() {
    let state = make_state(temp_path("badrole", "yaml"), temp_path("badrole", "yaml")).await;
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/accounts",
            Some(&admin_token(&state)),
            Body::from(r#"{"username":"eve","password":"pw","role":"superuser","vehicles":[]}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accounts_update_role_hot() {
    let state = make_state(temp_path("upd", "yaml"), temp_path("upd", "yaml")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);

    let res = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/accounts",
            Some(&token),
            Body::from(r#"{"username":"carol","password":"pw","role":"viewer","vehicles":[]}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // 提升角色
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::PUT,
            "/api/admin/accounts/carol",
            Some(&token),
            Body::from(r#"{"role":"dispatcher"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 热重载实证：登录后角色已变（新登录拿 dispatcher 角色）
    let login_app = login_router(state.clone());
    let res = login_app
        .oneshot(auth_request(
            Method::POST,
            "/api/auth/login",
            None,
            Body::from(r#"{"username":"carol","password":"pw"}"#),
        ))
        .await
        .unwrap();
    let body = json_of(res).await;
    assert_eq!(body["role"], "dispatcher", "角色更新应立即生效");
}

#[tokio::test]
async fn accounts_delete_makes_login_fail() {
    let state = make_state(temp_path("del", "yaml"), temp_path("del", "yaml")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);

    let res = app
        .clone()
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/accounts",
            Some(&token),
            Body::from(r#"{"username":"eve","password":"pw","role":"viewer","vehicles":[]}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    let res = app
        .clone()
        .oneshot(auth_request(
            Method::DELETE,
            "/api/admin/accounts/eve",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 热重载实证：删除后立即登录 401
    let login_app = login_router(state.clone());
    let res = login_app
        .oneshot(auth_request(
            Method::POST,
            "/api/auth/login",
            None,
            Body::from(r#"{"username":"eve","password":"pw"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn accounts_self_delete_denied() {
    let state = make_state(temp_path("self", "yaml"), temp_path("self", "yaml")).await;
    let token = admin_token(&state); // sub = "admin"
    let app = admin_router(state.clone());
    let res = app
        .oneshot(auth_request(
            Method::DELETE,
            "/api/admin/accounts/admin",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST, "不能删除登录中的自己");
}

#[tokio::test]
async fn accounts_unknown_update_delete_404() {
    let state = make_state(temp_path("miss", "yaml"), temp_path("miss", "yaml")).await;
    let app = admin_router(state.clone());
    let token = admin_token(&state);
    let res = app
        .clone()
        .oneshot(auth_request(
            Method::PUT,
            "/api/admin/accounts/nobody",
            Some(&token),
            Body::from(r#"{"role":"admin"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .oneshot(auth_request(
            Method::DELETE,
            "/api/admin/accounts/nobody",
            Some(&token),
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
