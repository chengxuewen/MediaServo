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
        api_registry: Arc::new(mediaservo_server::apikeys::ApiKeyRegistry::empty()),
        api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
        api_token_ttl_secs: 3600,
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
        api_registry: Arc::new(mediaservo_server::apikeys::ApiKeyRegistry::empty()),
        api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
        api_token_ttl_secs: 3600,
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
        // the axum Json<T> extractor requires the content-type
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
    // GET allowed
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
    // POST (register) denied -- dispatcher is read-only
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
    assert_ne!(res.status(), StatusCode::OK, "dispatcher write operations must be denied");
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

    // hot-reload proof: within the same process (no restart), auth passes immediately after registering
    assert_eq!(
        devices::authenticate(&state.device_registry, Some("ms-hot-1"), Some(&secret),),
        Some(Ok(()))
    );

    // the list includes the new device
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
    assert_eq!(ids, vec!["ms-hot-1"], "the list should contain the newly registered device");
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
    assert_eq!(res2.status(), StatusCode::CONFLICT, "duplicate registration -> 409");
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

    // register
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

    // revoke
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

    // hot-reload proof: immediately 4010 (Unknown) after revocation
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

    // old key invalidated (BadSecret), new key effective (hot reload)
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
    assert_eq!(body["secret"], "admin-chosen-1", "the self-supplied secret returned as-is");
    assert_eq!(
        body["secret_hash"],
        serde_json::Value::String(format!("sha256:{}", {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(b"ms-prov-1:admin-chosen-1");
            h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
        }))
    );
    // hot-reload proof: after registering with a self-supplied secret, auth works immediately
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

// -- make_state_loaded: load the registry from devices.yaml (for persistence/reload closed-loop tests) --

#[cfg(feature = "sfu-mediasoup")]
async fn make_state_loaded(devices_path: String) -> AdminState {
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
        device_registry: Arc::new(devices::DeviceRegistry::load(&devices_path).unwrap()),
        devices_path,
        api_registry: Arc::new(mediaservo_server::apikeys::ApiKeyRegistry::empty()),
        api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
        api_token_ttl_secs: 3600,
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
        api_registry: Arc::new(mediaservo_server::apikeys::ApiKeyRegistry::empty()),
        api_keys_path: format!("/tmp/ms-apikeys-{}.yaml", uuid::Uuid::new_v4()),
        api_token_ttl_secs: 3600,
    }
}

// -- CRUD completion (secret semantics + list interplay + the persistence loop) --

#[tokio::test]
async fn devices_reset_secret_unknown_404() {
    // Arrange: empty registry
    let state = make_state(temp_devices_path("reset-unknown")).await;
    let app = admin_router(state.clone());

    // Act: reset-secret on a nonexistent device
    let res = app
        .oneshot(auth_request(
            Method::POST,
            "/api/admin/devices/ms-nope/reset-secret",
            Some(&admin_token(&state)),
            Body::empty(),
        ))
        .await
        .unwrap();

    // Assert: 404 (symmetric with revoke_unknown_404)
    assert_eq!(res.status(), StatusCode::NOT_FOUND, "reset-secret on an unknown device -> 404");
}

#[tokio::test]
async fn devices_list_never_leaks_secret() {
    // Arrange: register ms-leak-1 (generates a secret) -- the plaintext should only ever appear in the register response (the C33 one-time semantics)
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
    assert!(reg_body["secret"].is_string(), "the register response contains one-time plaintext");

    // Act: list query
    let res = app
        .oneshot(auth_request(Method::GET, "/api/admin/devices", Some(&token), Body::empty()))
        .await
        .unwrap();

    // Assert: list entries never contain secret/secret_hash (no later query can see the plaintext again)
    let body = json_of(res).await;
    let entries = body["devices"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry["device_id"], "ms-leak-1");
    assert!(
        entry.get("secret").is_none() && entry.get("secret_hash").is_none(),
        "the list must not leak secret/secret_hash: {entry}"
    );
    assert!(body.get("secret").is_none(), "the top level must not contain secret either");
}

#[tokio::test]
async fn devices_revoked_removed_from_list() {
    // Arrange: register ms-a + ms-b
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

    // Act: revoke ms-a
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

    // Assert: the list contains only the surviving ms-b
    let res = app
        .oneshot(auth_request(Method::GET, "/api/admin/devices", Some(&token), Body::empty()))
        .await
        .unwrap();
    let body = json_of(res).await;
    let ids: Vec<&str> = body["devices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["device_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["ms-b"], "after revocation the list should have dropped the device");
}

#[tokio::test]
async fn devices_register_persists_across_reload() {
    // Arrange: register ms-persist-1 (self-supplied secret) -> save to disk
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
    assert_eq!(res.status(), StatusCode::CREATED, "registration should be 201");

    // Act: rebuild state from the same devices.yaml (simulating a process restart -- loading from file)
    let state2 = make_state_loaded(path).await;

    // Assert: after reload auth passes + the list shows it (the save->load closed loop)
    assert_eq!(
        devices::authenticate(
            &state2.device_registry,
            Some("ms-persist-1"),
            Some("persist-secret-1")
        ),
        Some(Ok(())),
        "after reload the original secret should still authenticate"
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
    assert_eq!(
        ids,
        vec!["ms-persist-1"],
        "after reload the list should contain the persisted device"
    );
}

#[tokio::test]
async fn devices_register_generated_secret_is_uuid_v4() {
    // Arrange/Act: server-generated secret (no secret field provided)
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

    // Assert: uuid v4 shape (36 chars + 4 hyphens) -- copyable and usable by the provisioning side
    let body = json_of(res).await;
    let secret = body["secret"].as_str().unwrap();
    assert_eq!(secret.len(), 36, "the generated secret should be a uuid v4: {secret}");
    assert_eq!(secret.chars().filter(|c| *c == '-').count(), 4, "uuid hyphen count: {secret}");
}
