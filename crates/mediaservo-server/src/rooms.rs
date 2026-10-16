//! W2-A (p3-gui-viewer): consumer-facing room discovery `GET /api/rooms`.
//!
//! Contract = `docs/plans/p3-gui-viewer/PLAN.md` S5-A (four-seat review final):
//! - **Separate router + separate role gate**: the admin-plane auth_middleware always rejects viewer/operator
//!   (proven at admin.rs:831); this endpoint must coexist but neither reuses nor relaxes that gate (F-S-1 BLOCKER pin).
//! - Permissions = the D-H11 proven matrix: admin/dispatcher see all online rooms; viewer/operator only allowlist
//!   (same check as `can_pull`, roles.rs as-is -- no second implementation).
//! - **G16/F-S-2 ruling**: ownerless rooms (vehicle offline/unregistered) are hidden from every role -- the invariant
//!   "listed subset-of joinable; joinable minus listed only allows the owner=None family", pinned by two-way unit tests.
//! - Response serde pinned to the two fields `{room_id, kind}` (admin/rooms rich fields not handed down, F-S-6/9).
//!   W4d addition (09-17): **per-stream room derivation** -- media-plane producers live in
//!   `<vehicle-room>_<stream_id>` (PIT-140 v2) without RoomJoin registration, so SDK consumers reading only the registered list
//!   Semantics note (2026-09-29): a per-stream room is the underlying isolation granularity, not product semantics -- the discovery layer
//!   expresses ownership (vehicle main entry + derived streams; dedup pinned in psk_discover); the UI groups by base. See PIT-140 note.
//!   cannot discover playable rooms (browsers pick by stream so they are naturally immune; web does not expose it). Derivation source =
//!   StatusReport streams[].connected; owner inherits the base room (visibility checks in the same direction).
//!   The three kinds = audio (C29 prefix) / video (stream room) / control (vehicle room -- no video producer inside).
//!
//! - Enumeration source = room_manager.list_rooms (same source as the admin view; a third copy is forbidden); owner authority =
//!   signaling.room_owner_of (the RoomJoin gate reads the same map; shared source is the premise of the invariant).

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;

use crate::admin::{AdminState, ErrorResponse, check_auth};
use crate::audit::{AuditEvent, log_event};
#[allow(unused_imports)]
// clippy --fix cfg blind spot (09-18 N8 record): items unused in this posture are used by the stub posture/tests branches
use crate::roles::{AccountIdentity, CockpitRole, SessionIdentity};

/// List item (wire contract -- unit test pins the field shape; extra fields = regression failure).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoomEntry {
    pub room_id: String,
    pub kind: &'static str,
    /// room-grouping-semantics T1: the owning vehicle room (back-reference for stream rooms; `Some` only for derived/registered
    /// stream rooms -- discovery is full). `None` = flat unit (vehicle/ordinary/audio rooms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Online streams embedded in the vehicle room (`generator1..N` stream id list; carried by control rooms only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streams: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomsResponse {
    pub rooms: Vec<RoomEntry>,
}

/// The three room kinds (W4d): audio (C29 prefix) always first; stream_room = a member of the derived set
/// (`<vehicle-room>_<stream-id>`, media plane); remaining registered rooms = control (vehicle room/control plane -- no
/// video producer inside; labeling video would mislead cockpit consumers).
pub fn room_kind(room_id: &str, is_stream_room: bool) -> &'static str {
    if room_id.starts_with("audio-") {
        "audio"
    } else if is_stream_room {
        "video"
    } else {
        "control"
    }
}

/// Stream ids with connected=true in StatusReport (same source data as the admin view, only a different projection).
fn online_stream_ids(status: &crate::status::StatusRegistry, room_id: &str) -> Vec<String> {
    use mediaservo_common::protocol::SignalingMessage;
    match status.get(room_id) {
        Some(SignalingMessage::StatusReport { streams, .. }) => {
            streams.iter().filter(|f| f.connected).map(|f| f.id.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// Visibility pure function (no IO; two-way unit tests cover owner present/absent x the four-role matrix).
fn room_visible(role: &CockpitRole, allowed: &[String], owner: Option<&str>) -> bool {
    // G16 strict mode: ownerless rooms (vehicle offline/unregistered) are always hidden -- the join gate allowing owner=None is the exempt family.
    let Some(owner) = owner else { return false };
    match role {
        CockpitRole::Admin | CockpitRole::Dispatcher => true,
        CockpitRole::Viewer | CockpitRole::Operator => role.can_pull(owner, allowed),
    }
}

type RoomsError = (StatusCode, Json<ErrorResponse>);

/// PSK discovery auth: `Authorization: Psk <secret>` (the scheme distinguishes account Bearer).
/// Hit -> Legacy full view (all online rooms + kind derivation, aligned with join-gate capability);
/// no PSK configured/wrong key/no credential -> None (falls back to the JWT chain) or 401 (Psk scheme present but wrong key --
/// an explicit 401 prevents silently sliding into the JWT chain with a misleading error).
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
    // constant-time + dummy pad (length differences are also constant-time -- same discipline as devices)
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
    // Legacy full view (room-grouping-semantics T1): build the derived set first (base x online streams) --
    // vehicle rooms embed streams; stream rooms appear as **stream-room entries** (kind=video + parent back-reference),
    // **no top-level control copies anymore** (dedup + ownership unified -- the wire form of the PIT-140 semantics note).
    let mut streams_of: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for r in state.signaling.room_manager.list_rooms() {
        if r.id.starts_with("audio-") || r.id.contains('_') {
            continue; // audio-prefix rooms and stream rooms (containing `_`) are not derivation sources
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
            // Truly registered stream room (a consumer joined): kind=video + parent back-reference -- dedup + ownership.
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
                    // Unregistered derived stream room: discovery is full (video + parent)
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
    // PSK discovery branch (psk-discover): `Authorization: Psk <secret>` -> Legacy full view list.
    // Permission argument: PSK already passes the join gate fully (roles.rs Legacy passthrough) -- "can join but not see" was an
    // accident, not design; opening discovery adds no capability (enumeration != entry; entry is already fully allowed), it only removes the blind spot.
    // Constant-time comparison (same discipline as devices/psk auth); wrong PSK and missing credential share a message (anti-enumeration).
    if let Some(psk_view) = psk_discover(&req, &state)? {
        return Ok(psk_view);
    }
    let claims = check_auth(&req, &state)?; // token extraction/verification reused (same issuance chain)
    // Identity gate: only JWTs with valid account roles pass. No role claim (Legacy etc.)/unknown role -> 401.
    // Device identities are not issued via REST and naturally never appear here (explicit reject = guards against future device tokens).
    let role = claims.role.as_deref().and_then(CockpitRole::parse).ok_or_else(|| {
        log_event(AuditEvent::AuthorizationDenied {
            action: "api_rooms".into(),
            peer_id: claims.sub.clone(),
            detail: "non-account or unknown role token".into(),
        });
        (StatusCode::UNAUTHORIZED, Json(ErrorResponse { error: "account role required".into() }))
    })?;
    // The allowlist follows the accounts registry's current value (C33 hot effect: change the entry, the set changes). Privileged roles
    // (admin/dispatcher) do not depend on the allowlist (roles.rs semantics); a missing account table does not block -- otherwise
    // the built-in admin (not in accounts.yaml scenarios) would be caught too; a viewer/operator token whose account was deleted
    // (revocation window) -> empty list (not 401 -- connection semantics match the admin plane; the UI's "please re-login" is the fallback, F-S-7).
    let account = state.accounts.list_accounts().into_iter().find(|a| a.username == claims.sub);
    let privileged = matches!(role, CockpitRole::Admin | CockpitRole::Dispatcher);
    // apikey form (T1/T2 exchange issuance, sub="apikey:<id>") has no account row by design: authorization
    // is fully embedded in the claims (same source as the RoomJoin gate; revoke = natural TTL death F10); vehicles come from the claims.
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
        return Ok(Json(RoomsResponse { rooms: Vec::new() })); // revocation window: non-privileged and account deleted
    }
    // W4d derived set: base rooms (non-audio, with owner) x their StatusReport connected streams.
    // Registered stream rooms (a consumer joined) merge into the same-source judgment; no duplicates, kind consistent.
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
            // A registered room matching the derived naming = the stream room was joined by a consumer (same-source re-check)
            let Some((base, _)) = id.rsplit_once('_') else { return false };
            !id.starts_with("audio-")
                && !online_stream_ids(&state.signaling.status_registry, base).is_empty()
                && derived.iter().any(|(d, _, _)| d == id)
        }))
        .collect();
    let rooms: Vec<RoomEntry> = state
        .signaling
        .room_manager
        .list_rooms()
        .into_iter()
        .filter_map(|r| {
            let owner = state.signaling.room_owner_of(&r.id);
            // room-grouping-semantics T1: stream rooms (in the derived set) -> video + parent back-reference;
            // vehicle rooms -> control + embedded streams (visible streams after member-level owner filtering).
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
    // Derived stream rooms (mostly unregistered -- producers do not RoomJoin): owner inherits base.
    // T1: unregistered stream rooms are **no longer flattened** -- merged into the vehicle room's embedded streams above (member-level visibility filtering).
    Ok(Json(RoomsResponse { rooms }))
}

/// Separate assembly (main.rs merge; must **not** be mounted into admin_router's layer).
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

    /// Invariant forward: listed visible => join gate must allow (compared under the same owner input against roles.rs's existing check).
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

    /// Invariant reverse (exempt family made explicit): the only case where join allows but the list hides is the owner=None family.
    #[test]
    fn invariant_hidden_only_for_ownerless_or_denied() {
        let allowed = allowed(&["ms-car1"]);
        for role in [CockpitRole::Viewer, CockpitRole::Operator] {
            let ident = SessionIdentity::Account(AccountIdentity {
                username: "u".into(),
                role: role.clone(),
                vehicles: allowed.clone(),
            });
            // Owned but join-denied room: the list denies it too (two-way consistent)
            assert!(ident.join_vehicle_room(Some("ms-car2")).is_some());
            assert!(!room_visible(&role, &allowed, Some("ms-car2")));
            // Exempt family: owner=None joins allowed, list hidden -- the G16 ruling pinned here.
            assert!(ident.join_vehicle_room(None).is_none());
            assert!(!room_visible(&role, &allowed, None));
        }
    }

    // -- integration through the axum stack (tower oneshot, same method as admin.rs:1459) --

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
        // 2026-09-29 live pin: a stream room joined by a consumer (a truly registered room) must not appear twice as control and
        // derived video (the same name twice -> viewer dedup drops the video item).
        let state = crate::admin::tests::make_state().await;
        *state.psk_state.write().unwrap() = Some("psk".into());
        state
            .signaling
            .room_manager
            .join_room("vehicle", "c-1", &mediaservo_common::protocol::PeerRole::Host)
            .unwrap();
        state.signaling.set_room_owner_for_test("vehicle", "ms-car1");
        // A stream room joined by a consumer = truly registered room
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
        assert_eq!(ids.len(), sorted.len(), "no duplicate room names: {ids:?}");
        assert!(ids.contains(&"vehicle_gen1"), "{ids:?}");
    }

    #[tokio::test]
    async fn http_psk_discover_full_visibility_and_401_paths() {
        // psk-discover: a PSK holder = Legacy omnipotent (join gate fully allowed) -> discovery equally full.
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

        // Hit -> 200 full (including the three audio-prefix kinds)
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

        // Wrong key -> 401 (no slide into the JWT chain with a misleading message)
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
            "wrong-PSK message matches unknown key (anti-enumeration)"
        );

        // Server without PSK: Psk scheme still gets 401 (no silent pass)
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
        // T5 live same-shape regression pin: apikey form has no account row; visibility = direct claims.vehicles check (non-empty list bypass).
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
        // Allowlist excludes the owner -> empty (not everything).
        let (_, body) = get_rooms(&state, Some(&mk(vec!["ms-other".into()]))).await;
        assert_eq!(body, r#"{"rooms":[]}"#, "claims allowlist miss must be empty");
        // Hit -> visible (including derived kind).
        let (_, body) = get_rooms(&state, Some(&mk(vec!["ms-car1".into()]))).await;
        assert!(body.contains(r#""room_id":"vehicle_demo""#), "{body}");
        assert!(
            body.contains(r#""kind":"control""#),
            "base room without online streams = control kind"
        );
    }

    #[tokio::test]
    async fn http_empty_rooms_shape_and_owned_visibility() {
        let state = crate::admin::tests::make_state().await;
        let tok = crate::admin::tests::token_of_role(&state, "admin", Some("admin"));
        // No rooms -> empty list + wire shape
        let (code, body) = get_rooms(&state, Some(&tok)).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        assert_eq!(body, r#"{"rooms":[]}"#);
        // Room built but owner not registered (vehicle-offline family) -> even admin cannot see it (G16)
        state
            .signaling
            .room_manager
            .join_room("vehicle_t1", "c-1", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        assert_eq!(body, r#"{"rooms":[]}"#, "ownerless hidden even for admin (G16)");
        // Owner registered -> admin sees everything, including kind derivation
        state.signaling.set_room_owner_for_test("vehicle_t1", "ms-car1");
        state
            .signaling
            .room_manager
            .join_room("audio-ms-car1", "c-2", &mediaservo_common::protocol::PeerRole::Consumer)
            .unwrap();
        state.signaling.set_room_owner_for_test("audio-ms-car1", "ms-car1");
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        // DashMap iteration order is not guaranteed: look up by room_id; never assert array order.
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

    /// W4d: per-stream room derivation -- producers do not RoomJoin, so the registered list cannot see them = SDK
    /// consumers cannot discover playable rooms. Stream rooms are synthesized from the base room's StatusReport(connected),
    /// owner inherited from base; offline streams not derived; registered same-name stream rooms not duplicated; audio rooms not derived.
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
            config_running: None,
            config_file: None,
            config_last_push: None,
            config_flags: None,
        };
        let flow = |id: &str, connected: bool| StreamFlowJson {
            id: id.into(),
            bytes_sent: 1,
            frames_encoded: 1,
            frame_width: 1280,
            frame_height: 720,
            connected,
        };
        // Vehicle room registered + owner; stream test online / stream cam0 offline
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
        // audio rooms do not participate in derivation
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
        // room-grouping-semantics T1: derived stream rooms are **not flattened** -- expressed via the vehicle room's embedded streams;
        // offline streams do not enter streams. Vehicle room = control + embedded visible streams.
        let streams_of = |id: &str| {
            v["rooms"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["room_id"] == id)
                .map(|e| e["streams"].as_array().map(|a| a.len()).unwrap_or(0))
        };
        assert_eq!(kind_of("ms-car1_test"), None, "{body} derived stream rooms not flattened");
        assert_eq!(kind_of("ms-car1_cam0-stream"), None, "{body} offline streams do not appear");
        assert_eq!(
            kind_of("ms-car1").as_deref(),
            Some("control"),
            "{body} vehicle room = control plane"
        );
        assert_eq!(
            streams_of("ms-car1"),
            Some(1),
            "{body} streams embed online streams (test online/cam0 offline)"
        );
        assert_eq!(kind_of("audio-ms-car1").as_deref(), Some("audio"), "{body}");
        // G16 forward invariant (stream-room side): listed visible => join allowed -- stream room owner unregistered
        // (producers do not RoomJoin) -> join_vehicle_room(None) exempt family allows, holds.
        let ident = SessionIdentity::Account(AccountIdentity {
            username: "v".into(),
            role: CockpitRole::Viewer,
            vehicles: vec!["ms-car1".into()],
        });
        assert!(
            ident.join_vehicle_room(None).is_none(),
            "stream-room join exempt family = listed subset-of joinable"
        );
        // viewer allowlist hits the base owner -> stream room equally visible (owner inheritance, same direction)
        state
            .accounts
            .create_account("v2", "pw", "viewer", &["ms-car1".to_string()].to_vec())
            .unwrap();
        let vt = crate::admin::tests::token_of_role(&state, "v2", Some("viewer"));
        let (_, body) = get_rooms(&state, Some(&vt)).await;
        assert!(
            body.contains(r#""streams":["test"]"#) || body.contains(r#""streams": ["test"]"#),
            "{body} streams of allowlisted vehicles embedded via the vehicle room"
        );
        // Not in the allowlist -> stream room not visible
        state
            .accounts
            .create_account("v3", "pw", "viewer", &["ms-car2".to_string()].to_vec())
            .unwrap();
        let vt3 = crate::admin::tests::token_of_role(&state, "v3", Some("viewer"));
        let (_, body) = get_rooms(&state, Some(&vt3)).await;
        assert!(!body.contains("ms-car1_test"), "{body} unauthorized stream room must be hidden");
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
        // C33 hot effect: changing the allowlist changes the visible set
        state.accounts.update_account("v1", None, Some(&["ms-car2".to_string()]), None).unwrap();
        let (_, body) = get_rooms(&state, Some(&tok)).await;
        assert!(body.contains("vehicle_t2") && !body.contains("vehicle_t1"), "{body} (hot)");
        // Unknown/missing role tokens always 401 (Legacy side)
        let bad = crate::admin::tests::token_of_role(&state, "v1", None);
        let (code, _) = get_rooms(&state, Some(&bad)).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    /// Wire contract: exactly the two fields {room_id, kind} (an extra/missing field = serde output change = this test goes red first).
    #[test]
    fn wire_shape_exactly_two_fields() {
        let json = serde_json::to_value(RoomEntry {
            room_id: "vehicle_test1".into(),
            kind: "video",
            parent: None,
            streams: None,
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({ "room_id": "vehicle_test1", "kind": "video" }));
        assert_eq!(room_kind("audio-v1", false), "audio");
        assert_eq!(room_kind("vehicle_test1", true), "video");
        assert_eq!(room_kind("vehicle", false), "control");
    }
}
