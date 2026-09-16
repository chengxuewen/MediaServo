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
//! - 枚举源 = room_manager.list_rooms（与 admin 视图同源，禁第三份）；owner 权威源 =
//!   signaling.room_owner_of（RoomJoin 门读同一 map，同源=不变量成立的前提）。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Serialize;

use crate::admin::{AdminState, ErrorResponse, check_auth};
use crate::audit::{AuditEvent, log_event};
use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};

/// 列表项（wire 契约——单测钉字段形，多字段=回归失败）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoomEntry {
    pub room_id: String,
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomsResponse {
    pub rooms: Vec<RoomEntry>,
}

/// 房间 kind（C29 前缀约定，两变体封闭）。
pub fn room_kind(room_id: &str) -> &'static str {
    if room_id.starts_with("audio-") { "audio" } else { "video" }
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

async fn list_rooms(
    State(state): State<AdminState>,
    req: axum::extract::Request,
) -> Result<Json<RoomsResponse>, RoomsError> {
    let claims = check_auth(&req, &state)?; // token 提取/验签复用（同一发证链）
    // 身份门：只放行合法账号角色的 JWT。无 role claim（Legacy 等）/未知角色 → 401。
    // Device 身份不经 REST 发证，天然不在此面（显式拒=防未来 device token 混入）。
    let role = claims
        .role
        .as_deref()
        .and_then(CockpitRole::parse)
        .ok_or_else(|| {
            log_event(AuditEvent::AuthorizationDenied {
                action: "api_rooms".into(),
                peer_id: claims.sub.clone(),
                detail: "non-account or unknown role token".into(),
            });
            (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse { error: "account role required".into() }),
            )
        })?;
    // allowlist 以 accounts registry 现值为准（C33 热生效：改单即变）。特权角色
    // （admin/dispatcher）不依赖 allowlist（roles.rs 语义），账号表缺席不拦——否则
    // 内置 admin（不在 accounts.yaml 场景）也被误伤；viewer/operator token 而账号已删
    // （吊销窗口）→ 空列表（不 401——连接语义与 admin 面一致，UI 侧「需重新登录」兜底，F-S-7）。
    let account = state.accounts.list_accounts().into_iter().find(|a| a.username == claims.sub);
    let privileged = matches!(role, CockpitRole::Admin | CockpitRole::Dispatcher);
    let vehicles = match (&account, privileged) {
        (Some(a), _) => {
            if CockpitRole::parse(&a.role).as_ref() != Some(&role) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse { error: "account role mismatch".into() }),
                ));
            }
            a.vehicles.clone()
        }
        (None, true) => Vec::new(),
        (None, false) => Vec::new(),
    };
    let rooms = if !privileged && account.is_none() {
        Vec::new() // 吊销窗口：非特权且账号已删 → 空
    } else {
        state
            .signaling
            .room_manager
            .list_rooms()
            .into_iter()
            .filter_map(|r| {
                let owner = state.signaling.room_owner_of(&r.id);
                room_visible(&role, &vehicles, owner.as_deref()).then(|| {
                    let kind = room_kind(&r.id);
                    RoomEntry { room_id: r.id, kind }
                })
            })
            .collect()
    };
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
            assert!(!room_visible(&role, &[], Some("ms-car1")), "empty allowlist = nothing visible");
        }
    }

    /// 不变量正向：列表可见 ⇒ join 门必放行（同 owner 输入下比对 roles.rs 现成判定）。
    #[test]
    fn invariant_visible_implies_joinable() {
        for role in [CockpitRole::Admin, CockpitRole::Dispatcher, CockpitRole::Viewer, CockpitRole::Operator] {
            let allowed = allowed(&["ms-car1"]);
            let ident = SessionIdentity::Account(AccountIdentity { username: "u".into(), role: role.clone(), vehicles: allowed.clone() });
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
            let ident = SessionIdentity::Account(AccountIdentity { username: "u".into(), role: role.clone(), vehicles: allowed.clone() });
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
    async fn http_401_without_token() {
        let state = crate::admin::tests::make_state().await;
        let (code, _) = get_rooms(&state, None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
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
        state.signaling.room_manager.join_room("audio-ms-car1", "c-2", &mediaservo_common::protocol::PeerRole::Consumer).unwrap();
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
        assert_eq!(kind_of("vehicle_t1").as_deref(), Some("video"), "{body}");
        assert_eq!(kind_of("audio-ms-car1").as_deref(), Some("audio"), "{body}");
    }

    #[tokio::test]
    async fn http_viewer_allowlist_filter_hot_effect() {
        let state = crate::admin::tests::make_state().await;
        state.accounts.create_account("v1", "pw", "viewer", &["ms-car1".to_string()].to_vec()).unwrap();
        let tok = crate::admin::tests::token_of_role(&state, "v1", Some("viewer"));
        for (id, owner) in [("vehicle_t1", "ms-car1"), ("vehicle_t2", "ms-car2")] {
            state.signaling.room_manager.join_room(id, id, &mediaservo_common::protocol::PeerRole::Consumer).unwrap();
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
        let json = serde_json::to_value(RoomEntry { room_id: "vehicle_test1".into(), kind: "video" }).unwrap();
        assert_eq!(json, serde_json::json!({ "room_id": "vehicle_test1", "kind": "video" }));
        assert_eq!(room_kind("audio-v1"), "audio");
        assert_eq!(room_kind("vehicle_test1"), "video");
    }
}
