//! W2-A（p3-gui-viewer）：消费者面房间发现 `GET /api/rooms`。
//!
//! 契约 = `docs/plans/p3-gui-viewer/PLAN.md` §5-A（四席评审定稿）：
//! - **独立 router + 独立角色门**：admin 面 auth_middleware 对 viewer/operator 恒拒
//!   （admin.rs:831 实证），本端点必须共存但既不复用也不放宽那道门（F-S-1 BLOCKER 钉）。
//! - 权限 = D-H11 实证矩阵：admin/dispatcher 全量在线房；viewer/operator 仅 allowlist
//!   （`can_pull` 同一判定，roles.rs 现成，不新造第二实现）。
//! - **G16/F-S-2 裁决**：无主房（车未上线/离线）对一切角色隐藏——不变量
//!   「列表 ⊆ 可进；可进∖列表 仅允许 owner=None 一族」，单测双向钉。
//! - 响应 serde 钉死 `{room_id, kind}` 二字段（admin/rooms 富字段不下放，F-S-6/9）。
//!   W4d 增补（09-17）：**per-stream 流房派生**——媒体面 producer 活在
//!   `<整车房>_<stream_id>`（PIT-140 v2）且不经 RoomJoin 注册，只看注册列表的 SDK
//!   语义注记(2026-09-29): per-stream 房=底层隔离粒度，非产品语义——发现层表达
//!   归属(整车主条+流派生，去重钉在 psk_discover)；UI 按 base 分组。详见 pitfalls PIT-140 注记。
//!   消费者发现不到可播房（浏览器按流勾选天然免疫，web 未暴露）。派生源 =
//!   StatusReport streams[].connected；owner 继承 base 房（可见性判定同向）。
//!   kind 三面 = audio（C29 前缀）/ video（流房）/ control（整车房——无视频 producer）。
//!
//! - 枚举源 = room_manager.list_rooms（与 admin 视图同源，禁第三份）；owner 权威源 =
//!   signaling.room_owner_of（RoomJoin 门读同一 map，同源=不变量成立的前提）。

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;

use crate::admin::{AdminState, ErrorResponse, check_auth};
use crate::audit::{AuditEvent, log_event};
#[allow(unused_imports)]
// clippy --fix cfg 盲区（09-18 N8 实录）：本姿态-unused 的项是 stub 姿态/tests 分支在用
use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};

/// 列表项（wire 契约——单测钉字段形，多字段=回归失败）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoomEntry {
    pub room_id: String,
    pub kind: &'static str,
    /// room-grouping-semantics T1：归属整车房（流房反指；`Some` 仅派生流房/已注册
    /// 流房——发现即全量）。`None` = 平铺单位（整车/普通/audio 房）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// 整车房内嵌在线流派生（`generator1..N` 流 id 列表；仅 control 房携带）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streams: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomsResponse {
    pub rooms: Vec<RoomEntry>,
}

/// 房间 kind 三面（W4d）：audio（C29 前缀）恒先；stream_room = 派生集合成员
/// （`<整车房>_<流id>`，媒体面）；其余注册房 = control（整车房/控制面——其内
/// 无视频 producer，标 video 是误导舱端消费者）。
pub fn room_kind(room_id: &str, is_stream_room: bool) -> &'static str {
    if room_id.starts_with("audio-") {
        "audio"
    } else if is_stream_room {
        "video"
    } else {
        "control"
    }
}

/// StatusReport 里 connected 的流 id（派生集与 admin 视图同源数据，只换投影）。
fn online_stream_ids(status: &crate::status::StatusRegistry, room_id: &str) -> Vec<String> {
    use mediaservo_common::protocol::SignalingMessage;
    match status.get(room_id) {
        Some(SignalingMessage::StatusReport { streams, .. }) => {
            streams.iter().filter(|f| f.connected).map(|f| f.id.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// 可见性判定纯函数（无 IO，双向单测覆盖 owner 在/无 × 四角色矩阵）。
fn room_visible(role: &CockpitRole, allowed: &[String], owner: Option<&str>) -> bool {
    // G16 严格档：无主房（车离线/未注册）一律隐藏——join 门对 owner=None 放行属豁免族。
    let Some(owner) = owner else { return false };
    match role {
        CockpitRole::Admin | CockpitRole::Dispatcher => true,
        CockpitRole::Viewer | CockpitRole::Operator => role.can_pull(owner, allowed),
    }
}

type RoomsError = (StatusCode, Json<ErrorResponse>);

/// PSK 发现认证：`Authorization: Psk <secret>`（scheme 区分账号 Bearer）。
/// 命中 → Legacy 全量视角（所有在线房 + kind 派生，与 join 门能力对齐）；
/// 未配 PSK/密钥错/缺凭证 → None（落回 JWT 链）或 401（带了 Psk scheme 但密钥错——
/// 显式 401 防止静默滑进 JWT 链产生误导性错误语）。
fn psk_discover(
    req: &axum::extract::Request,
    state: &AdminState,
) -> Result<Option<Json<RoomsResponse>>, RoomsError> {
    let Some(raw) =
        req.headers().get(axum::http::header::AUTHORIZATION).and_then(|v| v.to_str().ok())
    else {
        return Ok(None);
    };
    let Some(secret) = raw.strip_prefix("Psk ") else { return Ok(None) };
    let configured = state.psk_state.read().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(configured) = configured else {
        return Err(unauthorized_rooms("psk authentication failed"));
    };
    // constant-time + dummy 垫（长度差也恒时——与 devices 同纪律）
    let a = configured.as_bytes();
    let b = secret.as_bytes();
    let lt = if a.len() < b.len() { a.len() } else { b.len() };
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..lt {
        diff |= a[i] ^ b[i];
    }
    if diff != 0 {
        return Err(unauthorized_rooms("psk authentication failed"));
    }
    // Legacy 全量视角（room-grouping-semantics T1）：先建派生集（base×在线流）——
    // 整车房 streams 内嵌；流房以**流房条目**（kind=video + parent 反指）出现，
    // **不再产生顶层 control 拷贝**（去重+归属一体——PIT-140 语义注记的 wire 落地）。
    let mut streams_of: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for r in state.signaling.room_manager.list_rooms() {
        if r.id.starts_with("audio-") || r.id.contains('_') {
            continue; // audio 前缀房与流房（含 `_`）不做派生源
        }
        for sid in online_stream_ids(&state.signaling.status_registry, &r.id) {
            streams_of.entry(r.id.clone()).or_default().push(sid);
        }
    }
    let mut rooms: Vec<RoomEntry> = Vec::new();
    let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in state.signaling.room_manager.list_rooms() {
        if r.id.starts_with("audio-") {
            rooms.push(RoomEntry { room_id: r.id, kind: "audio", parent: None, streams: None });
            continue;
        }
        if let Some((base, sid)) = r.id.rsplit_once('_')
            && let Some((parent, streams)) = streams_of.get_key_value(base)
            && streams.iter().any(|s| s == sid)
        {
            // 真注册流房（consumer join 过）：kind=video + parent 反指——去重+归属。
            rooms.push(RoomEntry {
                room_id: r.id.clone(),
                kind: "video",
                parent: Some(parent.clone()),
                streams: None,
            });
            emitted.insert(r.id);
            continue;
        }
        let streams = streams_of.remove(&r.id);
        rooms.push(RoomEntry {
            room_id: r.id.clone(),
            kind: "control",
            parent: None,
            streams: streams.clone(),
        });
        if let Some(ss) = &streams {
            for sid in ss {
                let id = format!("{}_{}", r.id, sid);
                if !emitted.contains(&id) {
                    // 未注册的派生流房：发现即全量（video + parent）
                    rooms.push(RoomEntry {
                        room_id: id,
                        kind: "video",
                        parent: Some(r.id.clone()),
                        streams: None,
                    });
                }
            }
        }
    }
    Ok(Some(Json(RoomsResponse { rooms })))
}

fn unauthorized_rooms(msg: &str) -> RoomsError {
    (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: msg.to_string() }))
}

async fn list_rooms(
    State(state): State<AdminState>,
    req: axum::extract::Request,
) -> Result<Json<RoomsResponse>, RoomsError> {
    // PSK 发现分支（psk-discover）：`Authorization: Psk <secret>` → Legacy 视角全量列表。
    // 权限论证：PSK 本就在 join 门全放行（roles.rs Legacy 直通）——"能进不能看"是意外
    // 产物非设计；打开发现不新增能力（能枚举≠能进入，进入已全通），只消除盲态。
    // 比对 constant-time（与 devices/psk 认证同纪律）；错 PSK 与缺凭证同消息（防枚举）。
    if let Some(psk_view) = psk_discover(&req, &state)? {
        return Ok(psk_view);
    }
    let claims = check_auth(&req, &state)?; // token 提取/验签复用（同一发证链）
    // 身份门：只放行合法账号角色的 JWT。无 role claim（Legacy 等）/未知角色 → 401。
    // Device 身份不经 REST 发证，天然不在此面（显式拒=防未来 device token 混入）。
    let role = claims.role.as_deref().and_then(CockpitRole::parse).ok_or_else(|| {
        log_event(AuditEvent::AuthorizationDenied {
            action: "api_rooms".into(),
            peer_id: claims.sub.clone(),
            detail: "non-account or unknown role token".into(),
        });
        (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "account role required".into() }))
    })?;
    // allowlist 以 accounts registry 现值为准（C33 热生效：改单即变）。特权角色
    // （admin/dispatcher）不依赖 allowlist（roles.rs 语义），账号表缺席不拦——否则
    // 内置 admin（不在 accounts.yaml 场景）也被误伤；viewer/operator token 而账号已删
    // （吊销窗口）→ 空列表（不 401——连接语义与 admin 面一致，UI 侧「需重新登录」兜底，F-S-7）。
    let account = state.accounts.list_accounts().into_iter().find(|a| a.username == claims.sub);
    let privileged = matches!(role, CockpitRole::Admin | CockpitRole::Dispatcher);
    // apikey 形（T1/T2 exchange 发证，sub="apikey:<id>"）无账号行=设计使然：授权
    // 完全内嵌 claims（与 RoomJoin 门同源；吊销=TTL 自然死 F10），vehicles 取 claims。
    let apikey_vehicles =
        claims.sub.starts_with("apikey:").then(|| claims.vehicles.clone().unwrap_or_default());
    let vehicles = match (&account, privileged, &apikey_vehicles) {
        (Some(a), _, _) => {
            if CockpitRole::parse(&a.role).as_ref() != Some(&role) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse { error: "account role mismatch".into() }),
                ));
            }
            a.vehicles.clone()
        }
        (None, _, Some(v)) => v.clone(),
        (None, _, None) => Vec::new(),
    };
    if !privileged && account.is_none() && apikey_vehicles.is_none() {
        return Ok(Json(RoomsResponse { rooms: Vec::new() })); // 吊销窗口：非特权且账号已删
    }
    // W4d 派生集：base 房（非 audio、有 owner）× 其 StatusReport connected 流。
    // 已注册的流房（consumer join 过）并入同源判定，不重复、kind 一致。
    let mut derived: Vec<(String, String, String)> = Vec::new(); // (stream_room, owner, sid)
    for r in state.signaling.room_manager.list_rooms() {
        if r.id.starts_with("audio-") {
            continue;
        }
        let Some(owner) = state.signaling.room_owner_of(&r.id) else { continue };
        for sid in online_stream_ids(&state.signaling.status_registry, &r.id) {
            derived.push((format!("{}_{}", r.id, sid), owner.clone(), sid.clone()));
        }
    }
    let stream_rooms: std::collections::HashSet<String> = derived
        .iter()
        .map(|(id, _, _)| id.clone())
        .chain(state.signaling.room_manager.list_rooms().into_iter().map(|r| r.id).filter(|id| {
            // 注册房命中派生命名 = 流房被 consumer join 过（同源再判一次）
            let Some((base, _)) = id.rsplit_once('_') else { return false };
            !id.starts_with("audio-")
                && !online_stream_ids(&state.signaling.status_registry, base).is_empty()
                && derived.iter().any(|(d, _, _)| d == id)
        }))
        .collect();
    let mut rooms: Vec<RoomEntry> = state
        .signaling
        .room_manager
        .list_rooms()
        .into_iter()
        .filter_map(|r| {
            let owner = state.signaling.room_owner_of(&r.id);
            // room-grouping-semantics T1：流房（在派生集）→ video + parent 反指；
            // 整车房 → control + streams 内嵌（成员级 owner 过滤后的可见流）。
            if let Some((base, sid)) = r.id.rsplit_once('_')
                && !r.id.starts_with("audio-")
                && let Some((_, owner_d, sid_d)) =
                    derived.iter().find(|(id, _, s)| *id == r.id && s == sid)
            {
                let _ = (owner_d, sid_d);
                return room_visible(&role, &vehicles, Some(owner_d)).then(|| RoomEntry {
                    room_id: r.id.clone(),
                    kind: "video",
                    parent: Some(base.to_string()),
                    streams: None,
                });
            }
            let streams: Vec<String> = derived
                .iter()
                .filter(|(id, owner_d, _)| {
                    id.rsplit_once('_').map(|(b, _)| *b == r.id).unwrap_or(false)
                        && room_visible(&role, &vehicles, Some(owner_d))
                })
                .map(|(_, _, sid)| sid.clone())
                .collect();
            let streams = (!streams.is_empty()).then_some(streams);
            room_visible(&role, &vehicles, owner.as_deref()).then(|| {
                let kind = room_kind(&r.id, stream_rooms.contains(&r.id));
                RoomEntry { room_id: r.id.clone(), kind, parent: None, streams }
            })
        })
        .collect();
    // 派生流房（多数未注册——producer 不 RoomJoin）：owner 继承 base。
    // T1：未注册流房**不再平铺**——已并入上方整车房的 streams 内嵌（可见性成员级过滤）。
    Ok(Json(RoomsResponse { rooms }))
}

/// 独立装配（main.rs merge；**不得**挂进 admin_router 的 layer）。
pub fn rooms_router(state: AdminState) -> Router {
    Router::new().route("/api/rooms", get(list_rooms)).with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn matrix_admin_dispatcher_see_owned_rooms() {
        for role in [CockpitRole::Admin, CockpitRole::Dispatcher] {
            assert!(room_visible(&role, &[], Some("ms-car1")), "{role:?}");
            assert!(!room_visible(&role, &[], None), "{role:?} ownerless hidden (G16)");
        }
    }

    #[test]
    fn matrix_viewer_operator_allowlist_only() {
        for role in [CockpitRole::Viewer, CockpitRole::Operator] {
            assert!(room_visible(&role, &allowed(&["ms-car1"]), Some("ms-car1")));
            assert!(!room_visible(&role, &allowed(&["ms-car1"]), Some("ms-car2")));
            assert!(!room_visible(&role, &allowed(&["ms-car1"]), None));
            assert!(
                !room_visible(&role, &[], Some("ms-car1")),
                "empty allowlist = nothing visible"
            );
        }
    }

    /// 不变量正向：列表可见 ⇒ join 门必放行（同 owner 输入下比对 roles.rs 现成判定）。
    #[test]
    fn invariant_visible_implies_joinable() {
        for role in [
            CockpitRole::Admin,
            CockpitRole::Dispatcher,
            CockpitRole::Viewer,
            CockpitRole::Operator,
        ] {
            let allowed = allowed(&["ms-car1"]);
            let ident = SessionIdentity::Account(AccountIdentity {
                username: "u".into(),
                role: role.clone(),
                vehicles: allowed.clone(),
            });
            for owner in [Some("ms-car1"), Some("ms-car2")] {
                if room_visible(&role, &allowed, owner) {
                    assert_eq!(
                        ident.join_vehicle_room(owner),
                        None,
                        "VISIBLE-BUT-NOT-JOINABLE: {role:?} owner={owner:?}"
                    );
                }
            }
        }
    }

    /// 不变量反向（豁免族显式化）：join 放行但列表隐藏的情形，**只允许** owner=None 一族。
    #[test]
    fn invariant_hidden_only_for_ownerless_or_denied() {
        let allowed = allowed(&["ms-car1"]);
        for role in [CockpitRole::Viewer, CockpitRole::Operator] {
            let ident = SessionIdentity::Account(AccountIdentity {
                username: "u".into(),
                role: role.clone(),
                vehicles: allowed.clone(),
            });
            // 有主但被 join 拒的房：列表同样拒（两向一致）
            assert!(ident.join_vehicle_room(Some("ms-car2")).is_some());
            assert!(!room_visible(&role, &allowed, Some("ms-car2")));
            // 豁免族：owner=None join 放行、列表隐藏——G16 裁决钉死在此。
            assert!(ident.join_vehicle_room(None).is_none());
            assert!(!room_visible(&role, &allowed, None));
        }
    }

    // ── axum 栈内集成（tower oneshot，admin.rs:1459 先例同法）──

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::util::ServiceExt;

    async fn get_rooms(state: &AdminState, auth: Option<&str>) -> (StatusCode, String) {
        let app = rooms_router(state.clone());
        let mut builder = Request::builder().method(Method::GET).uri("/api/rooms");
        if let Some(t) = auth {
            builder = builder.header("Authorization", format!("Bearer {t}"));
        }
        let resp = app.oneshot(builder.body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn psk_discover_no_duplicate_for_joined_stream_rooms() {
        // 2026-09-29 实盘钉：consumer join 过的流房（真注册房）不得以 control 与
        // 派生 video 重复出现（同名两遍→viewer 去重丢 video 项）。
        let state = crate::admin::tests::make_state().await;
        *state.psk_state.write().unwrap() = Some("psk".into());
        state
            .signaling
            .room_manager
            .join_room("vehicle", "c-1", &mediaservo_common::protocol::PeerRole::Host)
            .unwrap();
        state.signaling.set_room_owner_for_test("vehicle", "ms-car1");
        // 流房被 consumer join 过 = 真注册房
        state
            .signaling
            .room_manager
            .join_room("vehicle_gen1", "c-2", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        state.signaling.set_room_owner_for_test("vehicle_gen1", "ms-car1");
        let app = rooms_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/rooms")
                    .header("Authorization", "Psk psk")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids: Vec<&str> =
            v["rooms"].as_array().unwrap().iter().map(|r| r["room_id"].as_str().unwrap()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "同名房不得重复: {ids:?}");
        assert!(ids.contains(&"vehicle_gen1"), "{ids:?}");
    }

    #[tokio::test]
    async fn http_psk_discover_full_visibility_and_401_paths() {
        // psk-discover：PSK 持者=Legacy 全能（join 门全放行）→ 发现面同样全量。
        let state = crate::admin::tests::make_state().await;
        *state.psk_state.write().unwrap() = Some("sekrit-psk".into());
        state
            .signaling
            .room_manager
            .join_room("vehicle_x", "c-1", &mediaservo_common::protocol::PeerRole::Host)
            .unwrap();
        state.signaling.set_room_owner_for_test("vehicle_x", "ms-car1");
        state
            .signaling
            .room_manager
            .join_room("audio-ms-car1", "c-2", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        state.signaling.set_room_owner_for_test("audio-ms-car1", "ms-car1");

        // 命中 → 200 全量（含 audio 前缀三面 kind）
        let app = rooms_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/rooms")
                    .header("Authorization", "Psk sekrit-psk")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids: Vec<&str> =
            v["rooms"].as_array().unwrap().iter().map(|r| r["room_id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"vehicle_x"), "{ids:?}");
        assert!(ids.contains(&"audio-ms-car1"), "{ids:?}");

        // 错密钥 → 401（不滑进 JWT 链给误导语）
        let app = rooms_router(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/rooms")
                    .header("Authorization", "Psk wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let b = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        assert_eq!(
            String::from_utf8_lossy(&b),
            r#"{"error":"psk authentication failed"}"#,
            "错 PSK 语与未知 key 同形（防枚举）"
        );

        // 未配 PSK 的服务器：带 Psk scheme 也 401（不静默放行）
        let state2 = crate::admin::tests::make_state().await;
        let app = rooms_router(state2);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/rooms")
                    .header("Authorization", "Psk anything")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn http_401_without_token() {
        let state = crate::admin::tests::make_state().await;
        let (code, _) = get_rooms(&state, None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn http_apikey_token_visibility_uses_claims_vehicles() {
        // T5 实盘同形回归钉：apikey 形无账号行，可见性=claims.vehicles 直裁（非空列表旁路）。
        use mediaservo_common::auth::JwtClaims;
        let state = crate::admin::tests::make_state().await;
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                as usize;
        let mk = |vehicles: Vec<String>| {
            jsonwebtoken::encode(
                &jsonwebtoken::Header::default(),
                &JwtClaims {
                    sub: "apikey:ci-1".into(),
                    iat: now,
                    exp: now + 3600,
                    role: Some("viewer".into()),
                    vehicles: Some(vehicles),
                },
                &jsonwebtoken::EncodingKey::from_secret(
                    state.admin_jwt_secret.as_deref().unwrap().as_bytes(),
                ),
            )
            .unwrap()
        };
        state
            .signaling
            .room_manager
            .join_room("vehicle_demo", "c-1", &mediaservo_common::protocol::PeerRole::Host)
            .unwrap();
        state.signaling.set_room_owner_for_test("vehicle_demo", "ms-car1");
        // 白名单不含 owner → 空（非全放）。
        let (_, body) = get_rooms(&state, Some(&mk(vec!["ms-other".into()]))).await;
        assert_eq!(body, r#"{"rooms":[]}"#, "claims 白名单未命中必须空");
        // 命中 → 可见（含派生 kind）。
        let (_, body) = get_rooms(&state, Some(&mk(vec!["ms-car1".into()]))).await;
        assert!(body.contains(r#""room_id":"vehicle_demo""#), "{body}");
        assert!(body.contains(r#""kind":"control""#), "base 房无在线流=control kind");
    }

    #[tokio::test]
    async fn http_empty_rooms_shape_and_owned_visibility() {
        let state = crate::admin::tests::make_state().await;
        let tok = crate::admin::tests::token_of_role(&state, "admin", Some("admin"));
        // 无房 → 空列表 + wire 形状
        let (code, body) = get_rooms(&state, Some(&tok)).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(body, r#"{"rooms":[]}"#);
        // 建房但未登记 owner（车离线族）→ admin 也看不到（G16）
        state
            .signaling
            .room_manager
            .join_room("vehicle_t1", "c-1", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        assert_eq!(body, r#"{"rooms":[]}"#, "ownerless hidden even for admin (G16)");
        // 登记 owner → admin 全量可见，含 kind 派生
        state.signaling.set_room_owner_for_test("vehicle_t1", "ms-car1");
        state
            .signaling
            .room_manager
            .join_room("audio-ms-car1", "c-2", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        state.signaling.set_room_owner_for_test("audio-ms-car1", "ms-car1");
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // DashMap 迭代序无保证：按 room_id 检索，不断言数组序。
        let kind_of = |id: &str| {
            v["rooms"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["room_id"] == id)
                .map(|e| e["kind"].as_str().unwrap().to_string())
        };
        assert_eq!(kind_of("vehicle_t1").as_deref(), Some("control"), "{body}");
        assert_eq!(kind_of("audio-ms-car1").as_deref(), Some("audio"), "{body}");
    }

    /// W4d：per-stream 流房派生——producer 不 RoomJoin，注册列表看不见 = SDK
    /// 消费者发现不到可播房。流房由 base 房 StatusReport(connected) 合成，
    /// owner 继承 base；离线流不派生；已注册的同名流房不重复；audio 房不派生。
    #[tokio::test]
    async fn http_stream_rooms_derived_from_status_report() {
        use mediaservo_common::protocol::{SignalStatusJson, SignalingMessage, StreamFlowJson};
        let state = crate::admin::tests::make_state().await;
        let tok = crate::admin::tests::token_of_role(&state, "admin", Some("admin"));
        let report = |streams: Vec<StreamFlowJson>| SignalingMessage::StatusReport {
            room_id: "ms-car1".into(),
            topics: vec![],
            streams,
            processes: vec![],
            signal: SignalStatusJson {
                remote_connected: true,
                remote_since_secs: Some(1),
                remote_peer_id: "p".into(),
                children: vec![],
                agent_uptime_secs: 1,
            },
            ts: 1000,
            config_version: 0,
        };
        let flow = |id: &str, connected: bool| StreamFlowJson {
            id: id.into(),
            bytes_sent: 1,
            frames_encoded: 1,
            frame_width: 1280,
            frame_height: 720,
            connected,
        };
        // 整车房注册 + owner；流 test 在线 / 流 cam0 离线
        state
            .signaling
            .room_manager
            .join_room("ms-car1", "h-1", &mediaservo_common::protocol::PeerRole::Host)
            .unwrap();
        state.signaling.set_room_owner_for_test("ms-car1", "ms-car1");
        state
            .signaling
            .status_registry
            .store("ms-car1", report(vec![flow("test", true), flow("cam0-stream", false)]));
        // audio 房不参与派生
        state
            .signaling
            .room_manager
            .join_room("audio-ms-car1", "a-1", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        state.signaling.set_room_owner_for_test("audio-ms-car1", "ms-car1");

        let (_, body) = get_rooms(&state, Some(&tok)).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let kind_of = |id: &str| {
            v["rooms"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["room_id"] == id)
                .map(|e| e["kind"].as_str().unwrap().to_string())
        };
        // room-grouping-semantics T1：派生流房**不平铺**——经整车房 streams 内嵌表达；
        // 离线流不进 streams。整车房=control + streams 内嵌可见流。
        let streams_of = |id: &str| {
            v["rooms"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["room_id"] == id)
                .map(|e| e["streams"].as_array().map(|a| a.len()).unwrap_or(0))
        };
        assert_eq!(kind_of("ms-car1_test"), None, "{body} 派生流房不平铺");
        assert_eq!(kind_of("ms-car1_cam0-stream"), None, "{body} 离线流不出现");
        assert_eq!(kind_of("ms-car1").as_deref(), Some("control"), "{body} 整车房=控制面");
        assert_eq!(streams_of("ms-car1"), Some(1), "{body} streams 内嵌在线流（test 在线/cam0 离线）");
        assert_eq!(kind_of("audio-ms-car1").as_deref(), Some("audio"), "{body}");
        // G16 正向不变量（流房面）：列表可见 ⇒ join 放行——流房 owner 未登记
        // （producer 不 RoomJoin）→ join_vehicle_room(None) 豁免族放行，成立。
        let ident = SessionIdentity::Account(AccountIdentity {
            username: "v".into(),
            role: CockpitRole::Viewer,
            vehicles: vec!["ms-car1".into()],
        });
        assert!(ident.join_vehicle_room(None).is_none(), "stream-room join 豁免族 = 列表⊆可进");
        // viewer allowlist 命中 base owner → 流房同样可见（owner 继承同向）
        state
            .accounts
            .create_account("v2", "pw", "viewer", &["ms-car1".to_string()].to_vec())
            .unwrap();
        let vt = crate::admin::tests::token_of_role(&state, "v2", Some("viewer"));
        let (_, body) = get_rooms(&state, Some(&vt)).await;
        assert!(
            body.contains(r#""streams":["test"]"#) || body.contains(r#""streams": ["test"]"#),
            "{body} viewer 白名单内车的流经整车房 streams 内嵌"
        );
        // 不在 allowlist → 流房不可见
        state
            .accounts
            .create_account("v3", "pw", "viewer", &["ms-car2".to_string()].to_vec())
            .unwrap();
        let vt3 = crate::admin::tests::token_of_role(&state, "v3", Some("viewer"));
        let (_, body) = get_rooms(&state, Some(&vt3)).await;
        assert!(!body.contains("ms-car1_test"), "{body} 越权流房必须隐藏");
    }

    #[tokio::test]
    async fn http_viewer_allowlist_filter_hot_effect() {
        let state = crate::admin::tests::make_state().await;
        state
            .accounts
            .create_account("v1", "pw", "viewer", &["ms-car1".to_string()].to_vec())
            .unwrap();
        let tok = crate::admin::tests::token_of_role(&state, "v1", Some("viewer"));
        for (id, owner) in [("vehicle_t1", "ms-car1"), ("vehicle_t2", "ms-car2")] {
            state
                .signaling
                .room_manager
                .join_room(id, id, &mediaservo_common::protocol::PeerRole::Consumer)
                .unwrap();
            state.signaling.set_room_owner_for_test(id, owner);
        }
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        assert!(body.contains("vehicle_t1") && !body.contains("vehicle_t2"), "{body}");
        // C33 热生效：改 allowlist 即变可见集
        state.accounts.update_account("v1", None, Some(&["ms-car2".to_string()]), None).unwrap();
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        assert!(body.contains("vehicle_t2") && !body.contains("vehicle_t1"), "{body} (hot)");
        // 未知/缺失 role token 一律 401（Legacy 面）
        let bad = crate::admin::tests::token_of_role(&state, "v1", None);
        let (code, _) = get_rooms(&state, Some(&bad)).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    /// wire 契约：恰好 {room_id, kind} 二字段（多/少字段= serde 输出变化=本测先红）。
    #[test]
    fn wire_shape_exactly_two_fields() {
        let json =
            serde_json::to_value(RoomEntry { room_id: "vehicle_test1".into(), kind: "video", parent: None, streams: None })
                .unwrap();
        assert_eq!(json, serde_json::json!({ "room_id": "vehicle_test1", "kind": "video" }));
        assert_eq!(room_kind("audio-v1", false), "audio");
        assert_eq!(room_kind("vehicle_test1", true), "video");
        assert_eq!(room_kind("vehicle", false), "control");
    }
}
