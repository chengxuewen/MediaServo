//! server.rs -- serve control plane (design §API contract + §serve security stack, full rev-2.2 set).
//!
//! Gate stack: Host allowlist always on -> (Origin allowlist with --lan) -> Bearer token (sole exemption = SSE
//! `?token=`; EventSource cannot send headers). Bind gate = non-loopback without --lan refuses startup;
//! CORS fully off (no cross-origin need, panel served same-origin). Gate order Host before token: Host is the origin-isolation gate,
//! rejecting rebinding traffic with zero information leakage (never reveals a token right/wrong signal).
//!
//! Write path mirrors the CLI: transient flock (engine::take_write_lock) -> engine::replay;
//! verify Deferred -- 200 only requires the fingerprint re-read to pass; live double sampling moved into a background task, results land as a timeline
//! `verify` event (SSE state frame ev broadcast, design §server: verify moved off the response path).
//! Read path is side-effect free: state.json read directly / tc re-read only via the state.teardown rebuilt channel (no re-probe,
//! no ensure -- GET never creates containers or processes) / stats reuses scope::StatsClient (token cached at process level).
//!
//! Error body uniformly {"error":...}; Fail.code mapping: 4->400, 3->409, 2->500 (main.rs header-comment semantics).
//! scenario run/stop = real T8 (run synchronous start phase 409/500, stop flag 200); basename/inline validation up front -- the 400 is real.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{Read as _, Write as _};
use std::path::Path as StdPath;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, Request, State as AxumState};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::Embed;
use serde_json::{Map, Value, json};

use crate::engine::{self, ApplyRequest, Env, Fail, Replay, Verify, Wn};
use crate::fuse;
use crate::scenario::{self, StepExec};
use crate::scope::{self, Capabilities, StatsClient, Targeting};
use crate::spec::{self, Dir, ImpairSpec, LossSpec};
use crate::state::{self, Dirs, State};

/// Default fuse shared with bash/CLI (apply without duration).
const DEFAULT_DURATION: u64 = 300;
/// C20 exemption shape: documented default constant + explicit `--listen` override (design §serve security 1).
const DEFAULT_LISTEN: &str = "127.0.0.1:9810";
/// SSE state frame cadence (design §API: 2s).
const FRAME_SECS: u64 = 2;
/// scenario inline size cap (design §serve security 6).
const MAX_INLINE_BYTES: usize = 32 * 1024;
/// Per-frame event cap for the ev field. ponytail: stops a single frame from flooding the CLI; 50 rows suffice for panel scrolling, overflow drops the oldest rows
/// (the cursor still advances).
const MAX_EV_LINES: usize = 50;
/// Total budget for SIGTERM/SIGINT graceful-exit step 2, drive quiet (design §D2 lck-F1): on timeout abandon clear and exit,
/// leaving teardown to the surviving watchdog -- the handler itself must never block unboundedly (otherwise it hits the oxmgr grace window then SIGKILL, turning the graceful path into a dead end).
const DRIVE_QUIET_BUDGET: Duration = Duration::from_secs(2);

// ---------- Graceful exit five-step sequence (design §D2 [sF2 lck-F1]; pure decision in [`shutdown_plan`]) ----------

/// SIGTERM/SIGINT graceful exit steps (map design §D2 five steps; step 4, state lands inactive, is handled inside [`engine::clear`]
/// via `remove_file` -- skipped when clear already did it, hence no dedicated variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownStep {
    /// 1. Set the scenario::stop flag (reuses the post/request_stop mechanism -- owner alive writes the cancel file, runner settles itself within 1s).
    WriteStopFlag,
    /// 2. Await in-flight drive quiet (within DRIVE_QUIET_BUDGET; nothing in flight counts as quiet).
    AwaitDriveQuiet,
    /// 3-4. Clear active pressure in this statedir (engine::clear is idempotent: no-queue -> Ok; step 4, state lands inactive, included).
    ClearPressure,
    /// 5. Exit (serve_main returns -> main exits 0).
    Exit,
}

/// Pure decision for the five-step sequence. `drive_settled=false` (step-2 timeout) => skip steps 3-4 and go straight to 5 (lck-F1: an in-flight step may still land tc,
/// and clearing concurrently with it would itself cause residue; teardown correctness never depends on graceful exit -- left to the deadline watchdog [design D5-3]).
/// `state_present=false` (no pressure / already torn down cleanly by drive finish_abort) => steps 3-4 naturally skip as no-ops.
pub(crate) fn shutdown_plan(drive_settled: bool, state_present: bool) -> Vec<ShutdownStep> {
    let mut steps = vec![ShutdownStep::WriteStopFlag, ShutdownStep::AwaitDriveQuiet];
    if drive_settled && state_present {
        steps.push(ShutdownStep::ClearPressure);
    }
    steps.push(ShutdownStep::Exit);
    steps
}

// ---------- Assembly and shared state ----------

/// serve assembly params (tests construct directly with zero env dependency; production assembles env+flags in [`serve_main`]).
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub token: String,
    pub dirs: Dirs,
    pub env: Env,
    pub lan: bool,
    pub listen_host: String,
    pub listen_port: u16,
    /// server_url three-level chain resolution result (None = /v1/streams and targeted stats degrade).
    pub server_url: Option<String>,
    /// capabilities injection (api_matrix skips real probing; the T7 WEAKNET_FAKE_CAPS knob feeds the same field).
    pub caps_override: Option<Capabilities>,
    /// SSE frame-count cap (api_matrix closes and asserts the body once full; None = unlimited).
    pub sse_frames_limit: Option<u32>,
}

struct Ctx {
    cfg: ServeConfig,
    /// Process-level stats client (built only when server_url resolves AND WEAKNET_ADMIN_PASS is present; login token cache reused).
    stats: Option<Mutex<StatsClient>>,
    /// capabilities cache (overwritten after a refresh re-probe).
    caps: Mutex<Option<Capabilities>>,
    /// In-flight scenario drive handle (write the stop flag, then await self-settle; at most one -- job exclusivity gate).
    drive: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Ctx {
    fn new(cfg: ServeConfig) -> Self {
        let stats = cfg.server_url.as_deref().and_then(|url| {
            StatsClient::from_env(url)
                .map_err(|e| {
                    println!(
                        "weaknet(serve): /v1/streams degraded (stats client unavailable): {}",
                        e.msg
                    );
                })
                .ok()
                .map(Mutex::new)
        });
        Self { cfg, stats, caps: Mutex::new(None), drive: Mutex::new(None) }
    }
}

// ---------- Routing ----------

/// Router factory (injects a fixed Config, test oneshots never touch the network; all-query C34, zero path params).
pub fn build_router(cfg: ServeConfig) -> Router {
    build_router_with_ctx(Arc::new(Ctx::new(cfg)))
}

fn build_router_with_ctx(ctx: Arc<Ctx>) -> Router {
    Router::new()
        .route("/v1/state", get(get_state))
        .route("/v1/profiles", get(get_profiles))
        .route("/v1/capabilities", get(get_capabilities))
        .route("/v1/scenarios", get(get_scenarios))
        .route("/v1/streams", get(get_streams))
        .route("/v1/apply", post(post_apply))
        .route("/v1/set", post(post_set))
        .route("/v1/clear", post(post_clear))
        .route("/v1/scenario/run", post(post_scenario_run))
        .route("/v1/scenario/stop", post(post_scenario_stop))
        .route("/v1/events", get(get_events))
        .route("/", get(get_index))
        .route("/assets/*path", get(get_asset))
        .layer(middleware::from_fn_with_state(ctx.clone(), guard))
        .with_state(ctx)
}

// ---------- Embedded panel (T7: rust-embed ui/; static assets are token-free -- the 401 onboarding page must load before credentials exist; Host allowlist still applies) ----------

/// Panel assets embedded at compile time (design §Files: src/ui, zero build chain). debug-embed is on: debug builds embed too, the binary is self-contained.
#[derive(Embed)]
#[folder = "src/ui/"]
struct UiAssets;

/// Extension -> MIME (lookup over embed keys, never the filesystem; unknown always octet-stream to prevent sniff-and-execute).
fn mime_for(key: &str) -> &'static str {
    match key.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn asset_response(key: &str, csp: bool) -> Response {
    match UiAssets::get(key) {
        Some(f) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime_for(key)));
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            if csp {
                headers.insert(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(
                        "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; frame-ancestors 'none'",
                    ),
                );
            }
            (StatusCode::OK, headers, f.data.to_vec()).into_response()
        }
        None => err_json(StatusCode::NOT_FOUND, "resource not found"),
    }
}

/// GET / —— index.html。
async fn get_index() -> Response {
    asset_response("index.html", true)
}

/// GET /assets/* -- app.js/css/vendor. Double traversal defense: explicitly reject `..` and a leading '/',
/// and embed keys are exact hash lookups -- the path never touches disk; a miss is a plain 404 with zero leakage.
async fn get_asset(Path(p): Path<String>) -> Response {
    if p.contains("..") || p.starts_with('/') {
        return err_json(StatusCode::BAD_REQUEST, "illegal asset path");
    }
    asset_response(&p, false)
}

/// Three gates Host -> Origin(--lan) -> Bearer (token checked only on /v1*; Host always checked).
async fn guard(AxumState(ctx): AxumState<Arc<Ctx>>, req: Request, next: Next) -> Response {
    if !host_allowed(&ctx, req.headers()) {
        return err_json(
            StatusCode::FORBIDDEN,
            "Host header not in allowlist (loopback/localhost/::1; cross-machine needs --lan)",
        );
    }
    if ctx.cfg.lan && !origin_allowed(&ctx, req.headers()) {
        return err_json(
            StatusCode::FORBIDDEN,
            "Origin does not match the listen address (--lan Origin allowlist)",
        );
    }
    if req.uri().path().starts_with("/v1") && !auth_ok(&ctx, &req) {
        // Sidebar liveness 401 de-noise (2026-09-11 small knife): unauthenticated capabilities get a **read-only cache** exemption --
        // the prober (Layout.tsx HEAD) has no token so the browser always logs a 401 resource error; semantically 401 just means
        // alive but credentials required. De-noising must not open the probe surface: refresh/cold-cache real probes = write lock + tc exec,
        // reachable via caddy by credential-less LAN clients = DoS -> flagged CapsNoProbe, handler downgrades to no-probe.
        if req.uri().path() == "/v1/capabilities" {
            let mut req = req;
            req.extensions_mut().insert(CapsNoProbe);
            return next.run(req).await;
        }
        return err_json(
            StatusCode::UNAUTHORIZED,
            "missing or wrong token (copy the whole banner line)",
        );
    }
    next.run(req).await
}

/// Guard injection: this request is an unauthenticated capabilities read (no real probes; cache read-only).
#[derive(Clone, Copy)]
struct CapsNoProbe;

fn auth_ok(ctx: &Ctx, req: &Request) -> bool {
    if let Some(t) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        return constant_time_eq(t.as_bytes(), ctx.cfg.token.as_bytes());
    }
    // Sole exemption: SSE EventSource cannot send headers -> ?token= (design §API auth item).
    if req.uri().path() == "/v1/events"
        && let Some(t) = req.uri().query().and_then(query_token)
    {
        return constant_time_eq(t.as_bytes(), ctx.cfg.token.as_bytes());
    }
    false
}

fn query_token(q: &str) -> Option<&str> {
    q.split('&').find_map(|kv| kv.strip_prefix("token=")).filter(|s| !s.is_empty())
}

/// Constant-time compare (no early exit; length difference folded into the diff accumulator). ponytail: replaces a timing_guard dependency --
/// local dev panel, microsecond-level bias is acceptable, the comment is the doc.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut diff = a.len() ^ b.len();
    for (x, y) in long.iter().zip(short.iter()) {
        diff |= (*x ^ *y) as usize;
    }
    for x in long.iter().skip(short.len()) {
        diff |= *x as usize;
    }
    diff == 0
}

fn host_allowed(ctx: &Ctx, headers: &HeaderMap) -> bool {
    let Some(raw) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let host = host_only(raw);
    const LOOPBACK: [&str; 4] = ["127.0.0.1", "localhost", "::1", "[::1]"];
    if LOOPBACK.contains(&host.as_str()) {
        return true;
    }
    ctx.cfg.lan && host == ctx.cfg.listen_host
}

/// Strip port -> host string. Only strips on a single colon with an all-numeric tail (bare IPv6 `::1` untouched); `[::1]:9810` takes the bracketed segment.
fn host_only(v: &str) -> String {
    if let Some(idx) = v.rfind(']') {
        return v[..=idx].to_string();
    }
    match v.rsplit_once(':') {
        Some((h, p))
            if !h.is_empty()
                && v.matches(':').count() == 1
                && !p.is_empty()
                && p.chars().all(|c| c.is_ascii_digit()) =>
        {
            h.to_string()
        }
        _ => v.to_string(),
    }
}

/// --lan Origin allowlist: browsers always send it and it must equal the listen host[:port]; non-browsers (curl without Origin) pass
/// (design ruling: Origin only constrains browser cross-origin; the server side is already guarded by the token).
fn origin_allowed(ctx: &Ctx, headers: &HeaderMap) -> bool {
    let Some(o) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let Some((_, rest)) = o.split_once("://") else {
        return false;
    };
    let hostport = rest.split('/').next().unwrap_or("");
    hostport == format!("{}:{}", ctx.cfg.listen_host, ctx.cfg.listen_port)
        || hostport == ctx.cfg.listen_host
}

fn err_json(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"error": msg}))).into_response()
}

// ---------- Error mapping ----------

struct AppErr {
    status: StatusCode,
    msg: String,
}

impl AppErr {
    fn bad(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, msg: msg.into() }
    }
}

impl IntoResponse for AppErr {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.msg}))).into_response()
    }
}

impl From<Fail> for AppErr {
    fn from(e: Fail) -> Self {
        let status = match e.code {
            4 => StatusCode::BAD_REQUEST,
            3 => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self { status, msg: e.msg }
    }
}

/// Engine/FS calls are all synchronous (std::process / TcpStream) -> uniformly wrapped in spawn_blocking; JoinError = 500 with cause.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Wn<T> + Send + 'static) -> Wn<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Fail::env(format!("blocking task crashed/aborted: {e}")))?
}

// ---------- GET surface ----------

/// Full state.json JSON; no state = `{"active": false}` (design §API).
async fn get_state(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let dirs = ctx.cfg.dirs.clone();
    let st = blocking(move || State::read_from(&dirs.state_json()).map_err(Fail::env)).await?;
    match st {
        Some(s) => serde_json::to_value(&s)
            .map(Json)
            .map_err(|e| AppErr::from(Fail::env(format!("state serialization failed: {e}")))),
        None => Ok(Json(json!({"active": false}))),
    }
}

async fn get_profiles() -> Result<Json<Value>, AppErr> {
    let v = blocking(|| Ok::<Value, Fail>(list_yaml_dir("profiles"))).await?;
    Ok(Json(v))
}

/// Read-only scenario listing (T7 scenarios tab data source; same shape and directory resolution as /v1/profiles, zero business logic).
async fn get_scenarios() -> Result<Json<Value>, AppErr> {
    let v = blocking(|| Ok::<Value, Fail>(list_yaml_dir("scenarios"))).await?;
    Ok(Json(v))
}

fn list_yaml_dir(sub: &str) -> Value {
    // T13 vehicle side: fs probe chain union compile-time embedded (single source = scope::list_assets, same rules as the CLI).
    let names = scope::list_assets(sub);
    if names.is_empty() {
        return json!({sub: [], "note": "no assets (WEAKNET_ASSETS_DIR / weaknet.d next to the binary / scripts/weaknet.d / embedded all empty)"});
    }
    json!({sub: names})
}

/// capabilities: cache first; `?refresh=1` re-probes (seed version gate; ifb bit = false + T9 pending --
/// the read-before-probe chain moves into scope::capabilities with T9 as a single-point replacement; the probe structure is not duplicated here).
/// Re-probe runs under a transient write lock (design §engine capability: protects live sessions; the T9 effective surface pinned in advance).
async fn get_capabilities(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    Query(q): Query<HashMap<String, String>>,
    no_probe: Option<axum::extract::Extension<CapsNoProbe>>,
) -> Result<Json<Value>, AppErr> {
    if let Some(c) = &ctx.cfg.caps_override {
        return Ok(Json(caps_json(c)));
    }
    let no_probe = no_probe.is_some();
    let refresh = !no_probe && q.get("refresh").is_some_and(|v| v == "1" || v == "true");
    let cached = ctx.caps.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(c) = cached.filter(|_| !refresh) {
        return Ok(Json(caps_json(&c)));
    }
    if no_probe {
        // Unauthenticated + no cache: explicit unknown shape with 200 (liveness probers only need the status code; no fabricated capabilities, no probing).
        return Ok(Json(serde_json::json!({ "cached": false })));
    }
    let c = ctx.clone();
    let caps = blocking(move || {
        let _lk = engine::take_write_lock(&c.cfg.dirs)?;
        let st = State::read_from(&c.cfg.dirs.state_json()).ok().flatten();
        let iface =
            st.as_ref().map(|s| s.iface.clone()).unwrap_or_else(|| engine::resolve_iface(None));
        // §capability: with an active ifb session in state, refresh is a no-op returning the cache -- the real probe already follows read-before-probe
        // protection (ifb0 present = zero create/delete); this guard keeps a no-cache first probe from spinning up inside the race window.
        if st.as_ref().is_some_and(|s| s.ifb_used)
            && let Some(cached) = c.caps.lock().unwrap_or_else(|e| e.into_inner()).clone()
        {
            return Ok(cached);
        }
        let caps = scope::capabilities(&c.cfg.env, &iface)?;
        *c.caps.lock().unwrap_or_else(|e| e.into_inner()) = Some(caps.clone());
        Ok(caps)
    })
    .await?;
    Ok(Json(caps_json(&caps)))
}

fn caps_json(c: &Capabilities) -> Value {
    json!({
        "seed": c.seed,
        "dir_lo": c.dir_lo,
        "ifb_ingress": c.ifb_ingress,
        "ifb_reason": c.ifb_reason,
    })
}

/// stats proxy (design §API: {room, owner, live, remote_ports[], local_port});
/// unreachable/unconfigured = 200 `{"streams": [], "note": cause}` (never 500 -- the panel degrades gracefully).
async fn get_streams(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let c = ctx.clone();
    let out = blocking(move || {
        let m = c
            .stats
            .as_ref()
            .ok_or_else(|| Fail::env("stats not configured: needs server_url (--server-url/env WEAKNET_SERVER_URL/out detection) + WEAKNET_ADMIN_PASS"))?;
        let rows = m.lock().unwrap_or_else(|e| e.into_inner()).fetch_streams()?;
        Ok::<Vec<Value>, Fail>(
            rows.iter()
                .map(|r| {
                    json!({
                        "room": r.room,
                        "owner": r.owner,
                        "live": r.live,
                        "remote_ports": r.remote_ports,
                        "local_port": r.local_port,
                    })
                })
                .collect(),
        )
    })
    .await;
    Ok(Json(match out {
        Ok(rows) => json!({"streams": rows}),
        Err(e) => json!({"streams": [], "note": e.msg}),
    }))
}

// ---------- POST surface ----------

#[derive(Debug)]
struct TargetingCmd {
    ports: Vec<u16>,
    stream: Option<String>,
    device: Option<String>,
    server_url: Option<String>,
}

#[derive(Debug)]
struct ApplyCmd {
    spec: ImpairSpec,
    duration: u64,
    iface: Option<String>,
    sig_port: Option<u16>,
    target: TargetingCmd,
}

#[derive(Debug)]
struct SetCmd {
    overrides: Map<String, Value>,
    iface: Option<String>,
    sig_port: Option<u16>,
    target: TargetingCmd,
}

fn take_key(obj: &mut Map<String, Value>, k: &str) -> Option<Value> {
    obj.remove(k)
}

fn req_u64(v: Value, k: &str) -> Wn<u64> {
    v.as_u64()
        .ok_or_else(|| Fail::bad_param(format!("{k} must be a non-negative integer, got {v}")))
}

fn req_u16(v: Value, k: &str) -> Wn<u16> {
    v.as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| Fail::bad_param(format!("{k} must be an integer in 0-65535, got {v}")))
}

fn req_str(v: Value, k: &str) -> Wn<String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| Fail::bad_param(format!("{k} must be a string, got {v}")))
}

fn req_ports(v: Value) -> Wn<Vec<u16>> {
    let arr =
        v.as_array().ok_or_else(|| Fail::bad_param(format!("ports must be an array, got {v}")))?;
    arr.iter().map(|n| req_u16(n.clone(), "ports[]")).collect()
}

fn req_csv(v: Value, k: &str, field: &str) -> Wn<String> {
    let o = v
        .as_object()
        .ok_or_else(|| Fail::bad_param(format!("{k} must be an object {{\"{field}\": [...]}}")))?;
    let arr = o
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| Fail::bad_param(format!("{k}.{field} must be a string array")))?;
    let mut items: Vec<&str> = Vec::new();
    for x in arr {
        items.push(x.as_str().ok_or_else(|| {
            Fail::bad_param(format!("{k}.{field} items must be strings, got {x}"))
        })?);
    }
    if items.is_empty() {
        return Err(Fail::bad_param(format!("{k}.{field} must not be empty")));
    }
    Ok(items.join(","))
}

/// (remaining spec keys, direction, iface, sig_port). Compatibility forms: `sig_port` (design §API word) and
/// `signaling_port` (CLI flag word) are synonyms; the former wins.
type BodySplit = (Map<String, Value>, TargetingCmd, Option<String>, Option<u16>);

/// Body unwrapping: extra keys taken first, the rest = ImpairSpec flat form (unknown keys rejected by from_flat_json -- typos surface loudly).
fn split_body(raw: &[u8]) -> Wn<BodySplit> {
    let v: Value = serde_json::from_slice(raw)
        .map_err(|e| Fail::bad_param(format!("invalid body JSON: {e}")))?;
    let mut obj = match v {
        Value::Object(m) => m,
        other => return Err(Fail::bad_param(format!("body must be a JSON object, got {other}"))),
    };
    let ports = take_key(&mut obj, "ports").map(req_ports).transpose()?.unwrap_or_default();
    let stream = take_key(&mut obj, "stream_scope")
        .map(|v| req_csv(v, "stream_scope", "rooms"))
        .transpose()?;
    let device = take_key(&mut obj, "device_scope")
        .map(|v| req_csv(v, "device_scope", "ids"))
        .transpose()?;
    let server_url =
        take_key(&mut obj, "server_url").map(|v| req_str(v, "server_url")).transpose()?;
    let iface = take_key(&mut obj, "iface").map(|v| req_str(v, "iface")).transpose()?;
    let sig_port = take_key(&mut obj, "sig_port")
        .or_else(|| take_key(&mut obj, "signaling_port"))
        .map(|v| req_u16(v, "sig_port"))
        .transpose()?;
    let target = TargetingCmd { ports, stream, device, server_url };
    Ok((obj, target, iface, sig_port))
}

fn resolve_cmd(c: &Ctx, t: &TargetingCmd) -> Wn<Targeting> {
    scope::resolve_targeting(
        t.server_url.as_deref().or(c.cfg.server_url.as_deref()),
        t.stream.as_deref(),
        t.device.as_deref(),
        &t.ports,
    )
}

/// POST /v1/apply -- 200 = dispatched + fingerprint re-read passed (verify async); the watchdog spawns from the replay Apply branch
/// (REST mirrors the CLI, design §fuse).
async fn post_apply(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), AppErr> {
    let (mut spec_map, target, iface, sig_port) = split_body(&body)?;
    let duration = match target_duration(&mut spec_map)? {
        Some(d) => d,
        None => DEFAULT_DURATION,
    };
    let spec = ImpairSpec::from_flat_json(&Value::Object(spec_map)).map_err(Fail::bad_param)?;
    let cmd = ApplyCmd { spec, duration, iface, sig_port, target };
    let c = ctx.clone();
    let out = blocking(move || do_apply_http(&c, cmd)).await?;
    spawn_verify(ctx, out.iface.clone(), out.pre_leaf, "apply", false);
    Ok((
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "fingerprint": out.fingerprint,
            "expires_at_ms": out.expires_at_ms,
            "channel": out.channel,
            "filters": out.filters,
        })),
    ))
}

/// duration sits in the leftover map after splitting (a spec-adjacent key) -- extract it before handing off to from_flat_json.
fn target_duration(obj: &mut Map<String, Value>) -> Wn<Option<u64>> {
    match take_key(obj, "duration") {
        None => Ok(None),
        Some(v) => {
            let d = req_u64(v, "duration")?;
            spec::validate_duration(d).map_err(Fail::bad_param)?;
            Ok(Some(d))
        }
    }
}

#[derive(Debug)]
struct HttpReplay {
    fingerprint: String,
    expires_at_ms: u64,
    channel: String,
    filters: usize,
    iface: String,
    pre_leaf: u64,
}

fn do_apply_http(c: &Arc<Ctx>, cmd: ApplyCmd) -> Wn<HttpReplay> {
    let _lk = engine::take_write_lock(&c.cfg.dirs)?;
    let mut prior = State::read_from(&c.cfg.dirs.state_json()).map_err(Fail::env)?;
    job_gate(&prior)?;
    if let Some(st) = &mut prior {
        sanitize_job(st);
    }
    let targeting = resolve_cmd(c, &cmd.target)?;
    let iface = cmd.iface.clone().unwrap_or_else(|| engine::resolve_iface(None));
    let req = ApplyRequest {
        spec: cmd.spec.clone(),
        scope: targeting.scope,
        names: targeting.scope_names(),
        iface: iface.clone(),
        ports: targeting.ports,
        pairs: targeting.pairs,
        sig_port: cmd.sig_port,
    };
    let out = engine::replay(
        &req,
        Replay::Apply { duration_secs: cmd.duration },
        &c.cfg.dirs,
        &c.cfg.env,
        Verify::Deferred,
    )?;
    Ok(HttpReplay {
        fingerprint: out.spec_string,
        expires_at_ms: out.expires_at_ms,
        channel: out.channel,
        filters: out.filter_count,
        iface,
        pre_leaf: out.leaf.0,
    })
}

/// scenario job exclusivity gate (design §server rev-2.2 Momus-B2): owner alive = 409/exit3;
/// a dead owner has its flag self-cleared and passes (Momus-minor#1; keeps the /v1/state job field honest even before T8).
fn job_gate(prior: &Option<State>) -> Wn<()> {
    if let Some(st) = prior
        && let Some(j) = &st.job
        && fuse::read_proc_starttime(j.pid) == Some(j.starttime)
    {
        return Err(Fail::conflict("scenario in progress (stop first)"));
    }
    Ok(())
}

fn sanitize_job(st: &mut State) {
    if let Some(j) = &st.job
        && fuse::read_proc_starttime(j.pid) != Some(j.starttime)
    {
        eprintln!(
            "weaknet(serve): WARN stale job '{}' (pid {} dead) -- clearing flag and passing",
            j.name, j.pid
        );
        st.job = None;
    }
}

/// POST /v1/set -- no active spec = 409 onboarding gate (design §UI write-path protocol); field-override merge
/// over the base then full replay (same as the CLI: no lifetime extension; scope defaults reuse state.pairs to skip re-pulling stats).
async fn post_set(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppErr> {
    let (mut obj, target, iface, sig_port) = split_body(&body)?;
    let duration = target_duration(&mut obj)?;
    if duration.is_some() {
        return Err(AppErr::from(Fail::bad_param(
            "set does not extend lifetime (duration belongs to apply) -- re-apply to change it",
        )));
    }
    let cmd = SetCmd { overrides: obj, iface, sig_port, target };
    let c = ctx.clone();
    let (spec_v, expires_at_ms, out_iface, pre_leaf) =
        blocking(move || do_set_http(&c, cmd)).await?;
    spawn_verify(ctx.clone(), out_iface, pre_leaf, "set", true);
    Ok(Json(json!({
        "ok": true,
        "spec": spec_v,
        "expires_at_ms": expires_at_ms,
        "job_gated": false,
    })))
}

fn do_set_http(c: &Arc<Ctx>, cmd: SetCmd) -> Wn<(Value, u64, String, u64)> {
    let _lk = engine::take_write_lock(&c.cfg.dirs)?;
    engine::autoheal_if_expired(&c.cfg.dirs, &c.cfg.env)?; // same as CLI Set (apply/clear skip the check; set/status verify up front)
    let mut prior = State::read_from(&c.cfg.dirs.state_json())
        .map_err(Fail::env)?
        .ok_or_else(|| Fail::conflict("no active spec -- apply first"))?;
    job_gate(&Some(prior.clone()))?; // rev-2.2: set 409s while a job is active too (symmetric with apply)
    sanitize_job(&mut prior);
    let has_override = !cmd.overrides.is_empty()
        || cmd.iface.is_some()
        || cmd.sig_port.is_some()
        || !cmd.target.ports.is_empty()
        || cmd.target.stream.is_some()
        || cmd.target.device.is_some();
    if !has_override {
        return Err(Fail::bad_param(
            "set needs at least one override field (spec key/iface/sig_port/direction)",
        ));
    }
    let mut flat = spec_to_flat_map(&prior.spec);
    for (k, v) in cmd.overrides {
        flat.insert(k, v);
    }
    let spec = ImpairSpec::from_flat_json(&Value::Object(flat)).map_err(Fail::bad_param)?;
    let targeting = if !cmd.target.ports.is_empty()
        || cmd.target.stream.is_some()
        || cmd.target.device.is_some()
    {
        resolve_cmd(c, &cmd.target)?
    } else {
        Targeting {
            scope: prior.scope,
            ports: prior.ports.clone(),
            pairs: prior.pairs.clone(),
            names: match prior.scope {
                spec::ScopeSel::Stream => prior.rooms.clone(),
                spec::ScopeSel::Device => prior.devices.clone(),
                spec::ScopeSel::Media => vec![],
            },
        }
    };
    if targeting.ports.is_empty() && targeting.pairs.is_empty() {
        return Err(Fail::env(
            "no usable media ports/pairs (base state empty and no ports/stream_scope/device_scope given) -- re-apply",
        ));
    }
    let before = state::param_summary(&prior.spec);
    let iface = cmd.iface.clone().unwrap_or_else(|| prior.iface.clone());
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
        names: targeting.scope_names(),
        iface: iface.clone(),
        ports: targeting.ports,
        pairs: targeting.pairs,
        sig_port: cmd.sig_port.or(prior.sig_port),
    };
    let out = engine::replay(
        &req,
        Replay::Set { prior: Box::new(prior), before },
        &c.cfg.dirs,
        &c.cfg.env,
        Verify::Deferred,
    )?;
    let spec_v = serde_json::to_value(&spec)
        .map_err(|e| Fail::env(format!("spec serialization failed: {e}")))?;
    Ok((spec_v, out.expires_at_ms, iface, out.leaf.0))
}

/// ImpairSpec -> from_flat_json key form (used by set to merge the base; lossless both ways, pinned by unit tests).
fn spec_to_flat_map(s: &ImpairSpec) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("rtt_ms".into(), json!(s.rtt_ms));
    m.insert("jitter_ms".into(), json!(s.jitter_ms));
    match &s.loss {
        Some(LossSpec::Simple(p)) => {
            m.insert("loss".into(), json!(p));
            m.insert("loss_mode".into(), json!("simple"));
        }
        Some(LossSpec::GeModel { loss, r, h, k }) => {
            m.insert("loss".into(), json!(loss));
            m.insert("loss_mode".into(), json!("gemodel"));
            m.insert("gemodel".into(), json!([r, h, k]));
        }
        None => {}
    }
    if let Some(r) = &s.reorder_pct {
        m.insert("reorder".into(), json!(r));
    }
    if let Some(r) = s.rate_mbps {
        m.insert("rate_mbps".into(), json!(r));
    }
    if let Some(seed) = s.seed {
        m.insert("seed".into(), json!(seed));
    }
    m.insert("limit".into(), json!(s.limit));
    let dir = match s.dir {
        Dir::Out => "out",
        Dir::In => "in",
        Dir::Both => "both",
    };
    m.insert("dir".into(), json!(dir));
    m
}

/// POST /v1/clear -- lock-free firefighting channel (same as the CLI), idempotent.
async fn post_clear(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let c = ctx.clone();
    let msg = blocking(move || engine::clear(&c.cfg.dirs, &c.cfg.env, false)).await?;
    Ok(Json(json!({"ok": true, "detail": msg})))
}

/// Background live double sampling (verify async): leaf delta over a 1s window -> timeline `verify` event (SSE ev broadcast;
/// banner material on failure). The channel is rebuilt only via state.teardown (zero probe side effects).
fn spawn_verify(ctx: Arc<Ctx>, iface: String, pre: u64, phase: &'static str, with_reset: bool) {
    tokio::spawn(async move {
        let c = ctx.clone();
        let r = blocking(move || {
            let st = State::read_from(&c.cfg.dirs.state_json())
                .map_err(Fail::env)?
                .ok_or_else(|| Fail::env("verify: state gone (concurrent clear?)"))?;
            let chan = engine::channel_of(&st.teardown)
                .ok_or_else(|| Fail::env("verify: teardown channel cannot be rebuilt"))?;
            let ev = match engine::verify_measure(&chan, &iface, Some(pre)) {
                Ok((p, q)) => {
                    let mut v = json!({"ev": "verify", "phase": phase, "ok": true, "leaf": [p, q]});
                    if with_reset {
                        v["counters_reset"] = json!(engine::counters_reset(p, q));
                    }
                    v
                }
                Err(e) => json!({"ev": "verify", "phase": phase, "ok": false, "err": e.msg}),
            };
            state::timeline_append(&c.cfg.dirs, ev).map_err(Fail::env)
        })
        .await;
        if let Err(e) = r {
            eprintln!("weaknet(serve): WARN background verify failed to land its event: {}", e.msg);
        }
    });
}

// ---------- scenario (T8: run = synchronous start (real 409/500 surface) + background drive; stop = flag channel) ----------

fn is_basename(f: &str) -> bool {
    !f.is_empty() && f != "." && f != ".." && !f.contains(['/', '\\'])
}

/// scenario blocking-phase artifacts (run handle tuple) -- type alias to silence clippy::type_complexity.
type PreparedRun = (Arc<scenario::RunSpec>, Arc<dyn StepExec>, Arc<Dirs>, ImpairSpec);
/// Blocking phase: exclusivity gate / dry-run / first-step apply / job persistence (start_blocking chains all real 409/500 errors).
fn scenario_prepare(
    c: &Arc<Ctx>,
    plan: scenario::ScenarioPlan,
    name: String,
    file_disp: String,
) -> Wn<PreparedRun> {
    let dirs = c.cfg.dirs.clone();
    let targeting = scope::resolve_targeting(c.cfg.server_url.as_deref(), None, None, &[])?;
    let rs = Arc::new(scenario::RunSpec {
        name,
        file_disp,
        plan,
        no_baseline: false,
        keep: false,
        targeting,
        iface: engine::resolve_iface(None),
        sig_port: None,
        recovery_dwell_secs: None,
    });
    let exec: Arc<dyn StepExec> =
        Arc::new(scenario::EngineExec::new(dirs.clone(), c.cfg.env.clone()));
    scenario::start_blocking(&rs, &exec, &dirs)?;
    let cur = State::read_from(&dirs.state_json())
        .map_err(Fail::env)?
        .ok_or_else(|| Fail::env("scenario: state missing after start"))?
        .spec;
    Ok((rs, exec, Arc::new(dirs), cur))
}

async fn post_scenario_run(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppErr> {
    let v: Value = serde_json::from_slice(&body)
        .map_err(|e| AppErr::from(Fail::bad_param(format!("invalid body JSON: {e}"))))?;
    let (plan, name, file_disp) = if let Some(f) = v.get("file") {
        let f = f.as_str().ok_or_else(|| AppErr::bad("file must be a string (basename-only)"))?;
        if !is_basename(f) {
            return Err(AppErr::bad(
                "file must be a basename (no path traversal/absolute paths, design §serve security 6)",
            ));
        }
        let (src, stem, disp) = match scenario::resolve_serve_file(f) {
            Ok((path, stem)) => {
                let src = std::fs::read_to_string(&path).map_err(|e| {
                    AppErr::from(Fail::env(format!(
                        "failed to read scenario {}: {e}",
                        path.display()
                    )))
                })?;
                (src, stem, path.display().to_string())
            }
            // T13 embedded fallback (vehicle scp single file): embedded only when fs is absent and the basename is clean.
            Err(fs_err) => {
                if !scenario::is_safe_asset_name(f) {
                    return Err(AppErr::from(fs_err));
                }
                let rel = scenario::asset_rel("scenarios", f);
                match scope::read_asset(&rel).map_err(AppErr::from)? {
                    Some(raw) => (
                        raw,
                        f.trim_end_matches(".yaml").trim_end_matches(".yml").to_string(),
                        format!("embedded {rel}"),
                    ),
                    None => return Err(AppErr::from(fs_err)),
                }
            }
        };
        let plan = scenario::parse_plan_yaml(&src).map_err(|e| AppErr::from(Fail::bad_param(e)))?;
        (plan, stem, disp)
    } else if let Some(inl) = v.get("inline") {
        let s = inl.as_str().ok_or_else(|| AppErr::bad("inline must be a string"))?;
        if s.len() > MAX_INLINE_BYTES {
            return Err(AppErr::bad(format!("inline exceeds the 32KB cap ({} bytes)", s.len())));
        }
        let plan = scenario::parse_plan_yaml(s).map_err(|e| AppErr::from(Fail::bad_param(e)))?;
        (plan, "inline".to_string(), "inline".to_string())
    } else {
        return Err(AppErr::bad(
            "body needs file{{\"file\":\"<basename>\"}} or {{\"inline\":\"<yaml>\"}}",
        ));
    };
    let total = plan.lines.len();
    let job_name = name.clone();
    let c = ctx.clone();
    let (rs, exec, dirs, cur) =
        blocking(move || scenario_prepare(&c, plan, name, file_disp)).await?;
    let handle = tokio::spawn(async move {
        let out = scenario::drive(rs, exec, dirs, cur).await;
        match &out.aborted {
            Some(r) => {
                eprintln!("weaknet(serve): scenario abort: {r}（elapsed_s={}）", out.elapsed_s)
            }
            None => println!("weaknet(serve): {}", out.summary),
        }
    });
    // Graceful-exit step 2 awaits this handle (drive sees the cancel flag -> finish_abort self-settle + clear; at most one = job exclusivity gate).
    *ctx.drive.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    Ok(Json(json!({
        "ok": true,
        "job": { "name": job_name, "done": 0, "total": total },
    })))
}

async fn post_scenario_stop(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let dirs = ctx.cfg.dirs.clone();
    let detail = blocking(move || scenario::stop(&dirs)).await?;
    Ok(Json(json!({"ok": true, "detail": detail})))
}

// ---------- SSE ----------

/// State-frame cadence (design §API). tokio::time::interval fires on the first tick -- documented: panel opens instantly.
async fn get_events(AxumState(ctx): AxumState<Arc<Ctx>>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);
    tokio::spawn(run_events(ctx, tx));
    Sse::new(EventStream(rx)).into_response()
}

struct EventStream(tokio::sync::mpsc::Receiver<Result<Event, Infallible>>);

impl futures_core::stream::Stream for EventStream {
    type Item = Result<Event, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// Per-connection FrameState (independent ev tail cursor / differential samples -- no cross-connection shared mutable state, kills races).
#[derive(Default)]
struct FrameState {
    cursor: u64,
    prev: Option<(Instant, HashMap<String, u64>)>,
    cache: Vec<AggRow>,
}

#[derive(Clone, Debug)]
struct AggRow {
    room: String,
    owner: Option<String>,
    live: bool,
    bytes: Option<u64>,
}

async fn run_events(ctx: Arc<Ctx>, tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>) {
    let limit = ctx.cfg.sse_frames_limit;
    let c = ctx.clone();
    let mut fs = match blocking(move || Ok::<FrameState, Fail>(FrameState::open(&c))).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("weaknet(serve): WARN SSE cursor init failed: {}", e.msg);
            return;
        }
    };
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(FRAME_SECS));
    let mut sent = 0u32;
    loop {
        tick.tick().await;
        let c = ctx.clone();
        let taken = std::mem::take(&mut fs);
        let (frame, fs2) = match blocking(move || Ok(build_frame(&c, taken))).await {
            Ok(v) => v,
            Err(e) => {
                let c2 = ctx.clone();
                (
                    json!({"frame_error": e.msg}),
                    blocking(move || Ok(FrameState::open(&c2))).await.unwrap_or_default(),
                )
            }
        };
        fs = fs2;
        let body = serde_json::to_string(&frame).unwrap_or_else(|_| "{}".into());
        if tx.send(Ok(Event::default().data(body))).await.is_err() {
            return; // browser disconnected
        }
        sent += 1;
        if limit.is_some_and(|n| sent >= n) {
            return; // test cutoff (api_matrix single-frame assert); production limit=None stays unlimited
        }
    }
}

impl FrameState {
    fn open(ctx: &Ctx) -> FrameState {
        let cursor = std::fs::read_to_string(ctx.cfg.dirs.timeline())
            .map(|s| s.lines().count() as u64)
            .unwrap_or(0);
        FrameState { cursor, ..Default::default() }
    }
}

/// State frame (full design §API fields): expires_at mirrors state.expires_at_ms at top level; job shape =
/// state.job ({name,pid,starttime,done,total}); no state = spec/scope/dir/expires_at/job/
/// names all null -- keys always present, values nullable, same semantics as the null kbps.
fn build_frame(ctx: &Ctx, mut fs: FrameState) -> (Value, FrameState) {
    let st = State::read_from(&ctx.cfg.dirs.state_json()).ok().flatten();
    let mut frame = json!({"spec": null, "scope": null, "dir": null, "expires_at": null, "job": null, "names": null});
    let mut tc = json!({"sent": null, "dropped": null});
    if let Some(s) = &st {
        frame["spec"] = serde_json::to_value(&s.spec).unwrap_or(Value::Null);
        frame["scope"] = serde_json::to_value(s.scope).unwrap_or(Value::Null);
        frame["dir"] = serde_json::to_value(s.dir).unwrap_or(Value::Null);
        frame["expires_at"] = json!(s.expires_at_ms);
        frame["job"] = serde_json::to_value(&s.job).unwrap_or(Value::Null);
        // E2: targeted names (the only path for panel checkbox echo -- mirrors spec, not optimistic updates).
        frame["names"] = json!({ "rooms": s.rooms, "devices": s.devices });
        // Liveness counts go only through the state.teardown rebuilt channel (no probe_channel -- the read surface has zero ensure side effects).
        // T9 observation-point routing: an ifb ingress-only session lives at the ifb0 root (ingress proof = its Sent growth).
        let (rd_dev, rd_form) = readback_target(Some(s), &s.iface);
        if let Some(chan) = engine::channel_of(&s.teardown)
            && let Ok(show) = engine::tc_exec(&chan, &["-s", "qdisc", "show", "dev", rd_dev])
            && let Some(ls) = engine::leaf_stats_form(&show, rd_form)
        {
            tc = json!({"sent": ls.sent_pkt, "dropped": ls.dropped});
        }
    }
    frame["tc"] = tc;
    frame["streams"] = build_streams(ctx, &mut fs);
    if let Some(ev) = tail_ev(ctx, &mut fs) {
        frame["ev"] = ev;
    }
    (frame, fs)
}

fn build_streams(ctx: &Ctx, fs: &mut FrameState) -> Value {
    let Some(m) = &ctx.stats else {
        return json!([]); // unconfigured (no url/no credentials) = empty list; degraded-notice surface belongs to the UI (T7)
    };
    let fetched = m.lock().unwrap_or_else(|e| e.into_inner()).fetch_streams();
    match fetched {
        Ok(rows) => {
            let agg = aggregate_streams(&rows);
            let dt =
                fs.prev.as_ref().map(|(t, _)| t.elapsed().as_secs_f64().max(0.2)).unwrap_or(0.0);
            let cur: HashMap<String, u64> =
                agg.iter().filter_map(|a| a.bytes.map(|b| (a.room.clone(), b))).collect();
            let out: Vec<Value> = agg
                .iter()
                .map(|a| {
                    let kbps = match (a.bytes, fs.prev.as_ref().and_then(|(_, m)| m.get(&a.room).copied())) {
                        (Some(c), Some(p)) if c >= p => Some(((c - p) as f64 * 8.0 / dt / 1000.0).round() as u64),
                        _ => None, // first sample/newly seen/counter reset = null (never conflated with 0, design §API)
                    };
                    json!({"room": a.room, "owner": a.owner, "kbps": kbps, "stale": false, "live": a.live})
                })
                .collect();
            fs.prev = Some((Instant::now(), cur));
            fs.cache = agg;
            Value::Array(out)
        }
        Err(e) => {
            // stats failure -> keep the previous row + stale:true (no skipped frames, no fake 0 -- the kbps semantics clause).
            eprintln!("weaknet(serve): WARN stats fetch failed (frame marked stale): {}", e.msg);
            let out: Vec<Value> = fs
                .cache
                .iter()
                .map(|a| json!({"room": a.room, "owner": a.owner, "kbps": null, "stale": true, "live": a.live}))
                .collect();
            Value::Array(out)
        }
    }
}

fn aggregate_streams(rows: &[scope::StreamInfo]) -> Vec<AggRow> {
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, (Option<String>, bool, Option<u64>)> = HashMap::new();
    for r in rows {
        let e = map.entry(r.room.clone()).or_insert_with(|| {
            order.push(r.room.clone());
            (None, false, Some(0))
        });
        if e.0.is_none() {
            e.0 = r.owner.clone();
        }
        e.1 |= r.live;
        e.2 = match (e.2, r.byte_count) {
            (Some(acc), Some(b)) => Some(acc + b),
            _ => None, // any member row missing byte_count (older server) -> room not differentiable = None
        };
    }
    order.sort();
    order
        .into_iter()
        .map(|room| {
            let (owner, live, bytes) = map.remove(&room).unwrap_or((None, false, None));
            AggRow { room, owner, live, bytes }
        })
        .collect()
}

/// timeline tail-cursor delta -> ev array (the path for CLI write ops / self-heal / verify results, design §API).
fn tail_ev(ctx: &Ctx, fs: &mut FrameState) -> Option<Value> {
    let raw = std::fs::read_to_string(ctx.cfg.dirs.timeline()).ok()?;
    let lines: Vec<&str> = raw.lines().collect();
    let n = lines.len() as u64;
    if n <= fs.cursor {
        return None;
    }
    let start = (fs.cursor as usize).min(lines.len());
    fs.cursor = n;
    let mut new: Vec<Value> =
        lines[start..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if new.len() > MAX_EV_LINES {
        new = new.split_off(new.len() - MAX_EV_LINES);
    }
    if new.is_empty() { None } else { Some(json!(new)) }
}

// ---------- serve main body ----------

/// `serve` subcommand entry (main.rs builds the runtime then block_on): gate validation -> token (incl. --token-file)
/// -> startup cross-check -> banner -> bind + lazy-expiry tick + axum::serve plus SIGTERM/SIGINT five-step graceful exit.
pub async fn serve_main(
    listen: Option<&str>,
    lan: bool,
    token_opt: Option<&str>,
    token_file: Option<&StdPath>,
    server_url_flag: Option<&str>,
) -> Wn<()> {
    let (host, port) = parse_listen(listen.unwrap_or(DEFAULT_LISTEN))?;
    if !is_loopback_host(&host) && !lan {
        return Err(Fail::env(format!(
            "bind gate: --listen {host}:{port} is non-loopback without --lan -- refusing startup (design §serve security 1)"
        )));
    }
    let dirs = Dirs::from_env();
    let env = Env::from_env();
    if env.channel_pref == "local" {
        // Validate at startup (don't wait to blow up on the write path); the local predicate is side-effect free, auto/sidecar ensure stays on demand.
        engine::probe_channel(&env, &engine::resolve_iface(None)).map(|_| ())?;
    }
    // token priority (design §D2 sF5): --token flag > WEAKNET_TOKEN env > --token-file (read/generate) > CSPRNG.
    let token = if let Some(t) = token_opt.filter(|s| !s.is_empty()) {
        t.to_owned()
    } else if let Some(t) = std::env::var("WEAKNET_TOKEN").ok().filter(|s| !s.is_empty()) {
        t
    } else if let Some(pf) = token_file {
        resolve_token_file(pf)?
    } else {
        gen_token()?
    };
    if lan && token.len() < 16 {
        println!(
            "weaknet(serve): WARN --lan with a token shorter than 16 chars -- give WEAKNET_TOKEN enough entropy for cross-machine use"
        );
    }
    let server_url = match scope::server_url_or_default(server_url_flag) {
        Ok(u) => Some(u),
        Err(e) => {
            println!("weaknet(serve): /v1/streams and targeted stats offline: {}", e.msg);
            None
        }
    };
    startup_cross_check(&dirs)?;
    let cfg = ServeConfig {
        token,
        dirs,
        env,
        lan,
        listen_host: host.clone(),
        listen_port: port,
        server_url,
        caps_override: fake_caps_from_env(),
        sse_frames_limit: None,
    };
    print_banner(&cfg);
    let host = cfg.listen_host.clone();
    let port = cfg.listen_port;
    let ctx = Arc::new(Ctx::new(cfg));
    spawn_expiry_ticker(ctx.clone());
    let listener = tokio::net::TcpListener::bind((host.as_str(), port)).await.map_err(|e| {
        Fail::env(format!("bind {host}:{port} failed: {e} (port in use? change --listen)"))
    })?;
    // SIGTERM/SIGINT graceful exit (design §D2 five-step). listener already bound (connectable); once the signal listener is up it enters
    // waiting; any termination signal -> drop the serve future (close the listener, stop accepting) -> run the five steps -> return and exit 0.
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| Fail::env(format!("failed to set up the SIGTERM listener: {e}")))?;
    tokio::select! {
        r = axum::serve(listener, build_router_with_ctx(ctx.clone())) => {
            return r.map_err(|e| Fail::env(format!("serve exited abnormally: {e}")));
        }
        _ = tokio::signal::ctrl_c() => println!("weaknet(serve): SIGINT -- five-step graceful exit"),
        _ = sigterm.recv() => println!("weaknet(serve): SIGTERM -- five-step graceful exit"),
    }
    graceful_shutdown(ctx).await;
    Ok(())
}

/// SIGTERM/SIGINT graceful-exit executor (design §D2 five steps; pure predicates for order/budget branches in [`shutdown_plan`]).
/// Semantic anchor: lck-F1 unbounded blocking = the oxmgr-grace-then-SIGKILL dead end -> await step 2 under a 2s budget, abandon clear on timeout; the runner already listens
/// for SIGTERM + finish_abort self-settle (proven in scenario.rs), so step 2 usually returns within seconds; the fallback is the surviving watchdog (spawned at apply as an
/// independent re-exec'd process, detached pgid, outlives serve) -- teardown correctness never depends on this function [design D5-3].
async fn graceful_shutdown(ctx: Arc<Ctx>) {
    let dirs = ctx.cfg.dirs.clone();
    let env = ctx.cfg.env.clone();
    // 1. Set the scenario::stop flag (reuses the post mechanism; idempotent no-op with no in-flight job).
    let _ = blocking(move || scenario::stop(&dirs)).await;
    println!("weaknet(serve): graceful exit (1) stop flag set");
    // 2. Await in-flight drive quiet (within DRIVE_QUIET_BUDGET). No in-flight handle => treat as quiet (static pressure goes through step 3).
    let drive = ctx.drive.lock().unwrap_or_else(|e| e.into_inner()).take();
    let drive_settled = match drive {
        Some(h) => tokio::time::timeout(DRIVE_QUIET_BUDGET, h).await.is_ok(),
        None => true,
    };
    println!(
        "weaknet(serve): graceful exit (2) drive quiet ({})",
        if drive_settled {
            "settled within budget"
        } else {
            "timeout -> abandoning clear, leaving it to the watchdog"
        }
    );
    // 3-4 verdict: drive quiet AND active pressure in this statedir => engine::clear (quiet: no channel ensure noise,
    // teardown per the list + rm state [= step 4 lands inactive]). Timeout or no pressure => shutdown_plan skips this step.
    let dirs_for_read = ctx.cfg.dirs.clone();
    let state_present = blocking(move || {
        Ok::<bool, Fail>(
            State::read_from(&dirs_for_read.state_json()).map(|s| s.is_some()).unwrap_or(false),
        )
    })
    .await
    .unwrap_or(false);
    for step in shutdown_plan(drive_settled, state_present) {
        if step == ShutdownStep::ClearPressure {
            let (d, e) = (ctx.cfg.dirs.clone(), env.clone());
            match blocking(move || engine::clear(&d, &e, true)).await {
                Ok(msg) => println!(
                    "weaknet(serve): graceful exit (3-4) cleared active pressure + landed inactive: {msg}"
                ),
                Err(f) => println!(
                    "weaknet(serve): graceful exit (3-4) WARN clear failed (residue goes to the watchdog/manual): {}",
                    f.msg
                ),
            }
        }
    }
    // 5. Exit: serve_main returns Ok -> main ends -> process exit 0 (runtime drop reclaims the ticker/leftover tasks).
    println!("weaknet(serve): graceful exit (5) exit 0");
}

/// Parse `--listen` (host:port; supports the [::1]:9810 bracketed form).
fn parse_listen(s: &str) -> Wn<(String, u16)> {
    let (h, p) = s
        .rsplit_once(':')
        .ok_or_else(|| Fail::bad_param(format!("--listen must be host:port, got {s:?}")))?;
    // Bracketed [::1]:9810 strips both sides; bare IPv6 without brackets splits the port at the last colon.
    let h = h.trim_start_matches('[').trim_end_matches(']').to_string();
    if h.is_empty() {
        return Err(Fail::bad_param(format!("--listen host is empty: {s:?}")));
    }
    let p = p
        .trim_end_matches(']')
        .parse::<u16>()
        .map_err(|e| Fail::bad_param(format!("--listen port invalid {s:?}: {e}")))?;
    Ok((h, p))
}

fn is_loopback_host(h: &str) -> bool {
    matches!(h, "127.0.0.1" | "localhost" | "::1" | "[::1]") || h.starts_with("127.")
}

/// CSPRNG >=128bit: /dev/urandom 32B -> hex (zero new dependencies, std suffices).
fn gen_token() -> Wn<String> {
    let mut buf = [0u8; 32];
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| Fail::env(format!("cannot read /dev/urandom: {e}")))?;
    f.read_exact(&mut buf).map_err(|e| Fail::env(format!("failed reading /dev/urandom: {e}")))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Token file permission gate (lck-F10/F11): an existing file's mode may have been loosened by a daemon umask (022 -> 644 world-readable);
/// validate before read, must be 0600 else reject with cause; for new files set_permissions(0600) after create then re-read to confirm. Non-unix has no POSIX
/// mode concept -> no-op (token still read/written, bits just not enforced).
#[cfg(unix)]
fn enforce_token_mode(path: &StdPath, apply: bool) -> Wn<()> {
    use std::os::unix::fs::PermissionsExt;
    if apply {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            Fail::env(format!("failed to chmod 0600 token file {}: {e}", path.display()))
        })?;
    }
    let got = std::fs::metadata(path)
        .map_err(|e| Fail::env(format!("failed to stat token file {}: {e}", path.display())))?
        .permissions()
        .mode()
        & 0o777;
    if got != 0o600 {
        return Err(Fail::env(format!(
            "token file {} permissions {got:#o} != 0600 (existing file may have been loosened by a daemon umask = readable leak; chmod 600 or rm and regenerate)",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_token_mode(_path: &StdPath, _apply: bool) -> Wn<()> {
    Ok(())
}

/// `--token-file` (design §D2 sF5): read if present (trim, enforced 0600 check), else generate via CSPRNG and write atomically
/// (create_new + set 0600 + re-read confirm). Parent dirs auto-created; the token source of truth in run/ gets its path passed by the unit side (T4's job).
fn resolve_token_file(path: &StdPath) -> Wn<String> {
    if path.exists() {
        enforce_token_mode(path, false)?; // existing: validate first (lck-F10: create mode does not apply to existing files)
        let raw = std::fs::read_to_string(path)
            .map_err(|e| Fail::env(format!("failed to read token file {}: {e}", path.display())))?;
        let tok = raw.trim();
        if tok.is_empty() {
            return Err(Fail::env(format!("token file {} is empty", path.display())));
        }
        return Ok(tok.to_owned());
    }
    let tok = gen_token()?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            Fail::env(format!("failed to create token dir {}: {e}", parent.display()))
        })?;
    }
    // create_new: races with a concurrent first create -> AlreadyExists treated as the other side having landed; fall through to read-and-validate (idempotent).
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => {
            f.write_all(tok.as_bytes()).map_err(|e| {
                Fail::env(format!("failed to write token file {}: {e}", path.display()))
            })?;
            f.flush().ok();
            drop(f);
            enforce_token_mode(path, true)?; // new file: chmod 0600 + re-read confirm (lck-F10)
            println!(
                "weaknet(serve): token generated and written to {} (0600; rotate = rm the file + restart the unit)",
                path.display()
            );
            Ok(tok)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => resolve_token_file(path),
        Err(e) => Err(Fail::env(format!("failed to create token file {}: {e}", path.display()))),
    }
}

/// Presentation rules (design §API Token item): loopback banner = a full copy-paste URL; --lan = token only
/// (a URL carrying the token would land in shell history/logs).
fn print_banner(cfg: &ServeConfig) {
    if cfg.lan {
        println!(
            "weaknet serve (--lan) listening on {}:{} -- token (shown once, paste after opening in the browser):",
            cfg.listen_host, cfg.listen_port
        );
        println!("  {}", cfg.token);
    } else {
        println!(
            "weaknet serve panel (copy-paste ready): http://{}:{}/?token={}",
            cfg.listen_host, cfg.listen_port, cfg.token
        );
    }
    println!(
        "weaknet serve security stack: Host allowlist always on + Bearer (SSE is the sole ?token= exemption) + CORS fully off + transient flock on the write path"
    );
    // Reverse-proxy form (wW8): through the server admin entry (Caddy) the browser cannot reach :9810 above; must use the /weaknet/ prefix.
    println!(
        "weaknet serve reverse proxy: when going through the server admin entry, open http://<current host>/weaknet/ -- token at run/weaknet.token in the deploy tree (same value as the direct token above)"
    );
}

/// serve startup cross-check (design §Error handling; shares the assert_fingerprint primitive with CLI status):
/// state x tc re-read; on divergence clear the state file + WARN -- never blindly re-apply to the kernel (current qdisc state belongs to the watchdog/manual clear).
/// Single T9 re-read observation-point predicate (shared by status/state frame/startup cross-check): an ifb ingress-only session (dir=In and
/// no egress skeleton traces -- when a sig leg exists the root was already built, observation belongs to the iface) -> ifb0 root form.
#[must_use]
pub(crate) fn readback_target<'a>(
    st: Option<&State>,
    iface: &'a str,
) -> (&'a str, engine::LeafForm) {
    match st {
        Some(s) if s.ifb_used && s.dir == spec::Dir::In && s.sig_port.is_none() => {
            (crate::ifb::IFB_DEV, engine::LeafForm::Root)
        }
        _ => (iface, engine::LeafForm::Parent110),
    }
}

fn startup_cross_check(dirs: &Dirs) -> Wn<()> {
    // No state: do not probe tc at startup (channel probing has ensure side effects and nothing to judge); residue is handled by the apply guard/status.
    if let Some(mut st) = State::read_from(&dirs.state_json()).map_err(Fail::env)? {
        if st.job.is_some() {
            sanitize_job(&mut st);
            if st.job.is_none() {
                st.write_to(&dirs.state_json()).map_err(Fail::env)?;
            }
        }
        let Some(chan) = engine::channel_of(&st.teardown) else {
            println!(
                "weaknet(serve): WARN state.teardown channel cannot be rebuilt (corrupt state?) -- skipping the cross-check"
            );
            return Ok(());
        };
        let (rd_dev, rd_form) = readback_target(Some(&st), &st.iface);
        match engine::tc_exec(&chan, &["qdisc", "show", "dev", rd_dev]) {
            Ok(show) => {
                if let Err(e) = engine::assert_fingerprint_form(
                    &show,
                    &spec::render_netem_spec_leg(&st.spec),
                    rd_form,
                ) {
                    eprintln!(
                        "weaknet(serve): WARN startup cross-check divergence (qdisc rewritten externally/gone): {} -- clearing the state file to reset (kernel untouched; current residue belongs to the watchdog/clear)",
                        e.msg
                    );
                    if let Err(re) = std::fs::remove_file(dirs.state_json())
                        && re.kind() != std::io::ErrorKind::NotFound
                    {
                        return Err(Fail::env(format!("failed to clear state: {re}")));
                    }
                } else {
                    println!("weaknet(serve): startup cross-check state x tc consistent");
                }
            }
            Err(e) => {
                eprintln!(
                    "weaknet(serve): WARN tc unreadable at startup ({e}) -- keeping state; lazy-expiry self-heal/background verify back it up"
                );
            }
        }
    }
    Ok(())
}

/// The lazy-expiry fuse layer's timer inside serve (design §fuse serve tokio timer -- independent of the SSE connection
/// count, self-heals even without the panel). Take the lock transiently then check; a busy lock = a write in progress, skip this tick (C15: WARN only on non-conflict errors).
fn spawn_expiry_ticker(ctx: Arc<Ctx>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(FRAME_SECS)).await;
            let c = ctx.clone();
            let r = blocking(move || match engine::take_write_lock(&c.cfg.dirs) {
                Ok(_lk) => engine::autoheal_if_expired(&c.cfg.dirs, &c.cfg.env).map(|_| ()),
                Err(e) if e.code == 3 => Ok(()),
                Err(e) => Err(e),
            })
            .await;
            if let Err(e) = r {
                eprintln!("weaknet(serve): WARN lazy-expiry check failed: {}", e.msg);
            }
        }
    });
}

/// dev-only injection (tasks T7 Verify: grey-state assertions depend on it): `WEAKNET_FAKE_CAPS="ifb_ingress=false,seed=false"`.
/// Keys in {seed, dir_lo, ifb_ingress}, values true|false; missing keys take conservative defaults (seed=false/dir_lo=true/
/// ifb_ingress=false -- grey not falsely green, fabricated capabilities forbidden). Results go into ServeConfig.caps_override ->
/// /v1/capabilities returns the cache directly, skipping the real probe. Never set in production -- it pins the capability surface to a lie.
fn fake_caps_from_env() -> Option<Capabilities> {
    let raw = std::env::var("WEAKNET_FAKE_CAPS").ok();
    parse_fake_caps(raw.as_deref())
}

fn parse_fake_caps(raw: Option<&str>) -> Option<Capabilities> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let mut c = Capabilities {
        seed: false,
        dir_lo: true,
        ifb_ingress: false,
        ifb_reason: "dev injection (WEAKNET_FAKE_CAPS): host has not loaded ifb -- fix = modprobe ifb as host root; real verdict = ifb::probe read-before-probe chain".into(),
    };
    for pair in raw.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            eprintln!(
                "weaknet(serve): WARN WEAKNET_FAKE_CAPS ignored entry {pair:?} (expected key=true|false)"
            );
            continue;
        };
        match (k.trim(), v.trim()) {
            ("seed", "true") => c.seed = true,
            ("seed", "false") => c.seed = false,
            ("dir_lo", "true") => c.dir_lo = true,
            ("dir_lo", "false") => c.dir_lo = false,
            ("ifb_ingress", "true") => {
                c.ifb_ingress = true;
                c.ifb_reason.clear();
            }
            ("ifb_ingress", "false") => c.ifb_ingress = false,
            (_, v @ ("true" | "false")) => {
                eprintln!(
                    "weaknet(serve): WARN WEAKNET_FAKE_CAPS unknown key {v:?} (valid: seed|dir_lo|ifb_ingress)"
                );
            }
            _ => eprintln!(
                "weaknet(serve): WARN WEAKNET_FAKE_CAPS invalid value {pair:?} (value must be true|false)"
            ),
        }
    }
    Some(c)
}

// ---------- Unit pure-function tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::LossSpec;

    fn spec(s: ImpairSpec) -> ImpairSpec {
        ImpairSpec::from_flat_json(&Value::Object(spec_to_flat_map(&s))).unwrap()
    }

    #[test]
    fn spec_to_flat_roundtrip_is_lossless() {
        let a = ImpairSpec {
            rtt_ms: 80,
            jitter_ms: 15,
            loss: Some(LossSpec::Simple("2".into())),
            reorder_pct: Some("5".into()),
            rate_mbps: Some(4.5),
            seed: Some(7),
            limit: 100000,
            dir: Dir::In,
        };
        assert_eq!(spec(a.clone()), a);
        let g = ImpairSpec {
            loss: Some(LossSpec::GeModel {
                loss: "8".into(),
                r: "25%".into(),
                h: "0.2".into(),
                k: "0.05".into(),
            }),
            ..ImpairSpec::default()
        };
        assert_eq!(spec(g.clone()), g);
        assert_eq!(spec(ImpairSpec::default()), ImpairSpec::default());
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[test]
    fn host_only_strips_port_by_shape() {
        assert_eq!(host_only("127.0.0.1:9810"), "127.0.0.1");
        assert_eq!(host_only("localhost"), "localhost");
        assert_eq!(host_only("[::1]:9810"), "[::1]");
        assert_eq!(host_only("::1"), "::1");
        assert_eq!(host_only("evil.example.com:80"), "evil.example.com");
    }

    #[test]
    fn listen_parse_shapes() {
        assert_eq!(parse_listen("127.0.0.1:9810").unwrap(), ("127.0.0.1".to_string(), 9810));
        assert_eq!(parse_listen("[::1]:19811").unwrap(), ("::1".to_string(), 19811));
        assert!(parse_listen("noport").is_err());
        assert!(parse_listen("h:99999").is_err());
    }

    #[test]
    fn basename_gate_blocks_traversal_and_absolute() {
        assert!(is_basename("cell-edge.yaml"));
        assert!(!is_basename(""));
        assert!(!is_basename("."));
        assert!(!is_basename(".."));
        assert!(!is_basename("../evil"));
        assert!(!is_basename("/etc/passwd"));
        assert!(!is_basename("sub/dir.yaml"));
    }

    #[test]
    fn aggregate_sums_rooms_and_flags_missing_bytes() {
        let row = |room: &str, owner: Option<&str>, live: bool, b: Option<u64>| scope::StreamInfo {
            room: room.into(),
            owner: owner.map(str::to_owned),
            peer_id: "p".into(),
            role: "producer".into(),
            kind: "video".into(),
            live,
            local_port: Some(1),
            remote_ports: vec![2],
            byte_count: b,
        };
        let agg = aggregate_streams(&[
            row("cam0", Some("dev-a"), true, Some(100)),
            row("cam0", None, false, Some(50)),
            row("cam1", None, true, None),
        ]);
        assert_eq!(agg.len(), 2);
        assert_eq!(agg[0].room, "cam0");
        assert_eq!(agg[0].owner.as_deref(), Some("dev-a"));
        assert_eq!(agg[0].bytes, Some(150));
        assert!(agg[0].live);
        assert_eq!(agg[1].bytes, None, "member row missing byte_count -> room not differentiable");
    }

    #[test]
    fn fake_caps_parses_and_defaults_conservative() {
        let c = parse_fake_caps(Some("ifb_ingress=false")).unwrap();
        assert!(!c.ifb_ingress);
        assert!(!c.seed, "missing key = conservative default");
        assert!(c.dir_lo);
        assert!(
            c.ifb_reason.contains("not loaded") && c.ifb_reason.contains("modprobe"),
            "grey tooltip covers both wordings (fix text = design §UI distinguishes structural vs not-loaded)",
        );
        let g = parse_fake_caps(Some(" seed=true , dir_lo=false ,ifb_ingress=true ")).unwrap();
        assert!(g.seed && !g.dir_lo && g.ifb_ingress && g.ifb_reason.is_empty());
        assert_eq!(parse_fake_caps(None), None);
        assert_eq!(parse_fake_caps(Some("  ")), None);
        // junk entries WARNed then ignored, the rest intact
        let b = parse_fake_caps(Some("nope=true,seed=true,oops")).unwrap();
        assert!(b.seed && !b.ifb_ingress);
    }

    #[tokio::test]
    async fn static_panel_unauth_asset_auth_unchanged() {
        use tower::util::ServiceExt;
        let cfg = ServeConfig {
            token: "tk".into(),
            dirs: Dirs { statedir: std::env::temp_dir().join("wnet-t7-unit-nonexistent") },
            env: Env::default(),
            lan: false,
            listen_host: "127.0.0.1".into(),
            listen_port: 9810,
            server_url: None,
            caps_override: None,
            sse_frames_limit: None,
        };
        let app = build_router(cfg);
        let req = |uri: &str| {
            axum::http::Request::builder()
                .uri(uri)
                .header("host", "127.0.0.1:9810")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let res = app.clone().oneshot(req("/")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert!(
            res.headers()[header::CONTENT_SECURITY_POLICY].as_bytes().starts_with(b"default-src")
        );
        let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&body).to_ascii_lowercase().contains("<!doctype html"));
        let res = app.clone().oneshot(req("/assets/app.js")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/javascript; charset=utf-8");
        let res = app.clone().oneshot(req("/assets/vendor/uPlot.iife.min.js")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // traversal: explicit `..` (incl. decoded forms) = 400; unknown key = 404 with zero leakage
        assert_eq!(
            app.clone().oneshot(req("/assets/../server.rs")).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            app.clone().oneshot(req("/assets/%2e%2e/Cargo.toml")).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            app.clone().oneshot(req("/assets/nope.js")).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        // /v1* auth unchanged: no token 401, valid token 200 (except the capabilities read-only exemption, below)
        assert_eq!(
            app.clone().oneshot(req("/v1/state")).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        // capabilities unauthenticated read-only exemption: cold cache must not probe (probing would Err in this test env -> non-200),
        // and refresh=1 must not slip past the gate either -- otherwise 401 de-noising becomes a credential-less DoS surface.
        for uri in ["/v1/capabilities", "/v1/capabilities?refresh=1"] {
            let res = app.clone().oneshot(req(uri)).await.unwrap();
            assert_eq!(
                res.status(),
                StatusCode::OK,
                "{uri} should be 200 (read-only cache exemption)"
            );
            let body: Value = serde_json::from_slice(
                &axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap(),
            )
            .unwrap();
            assert_eq!(
                body["cached"],
                serde_json::Value::Bool(false),
                "{uri} with no cache must return the unknown shape"
            );
        }
        // token-bearing refresh still takes the original path (no CapsNoProbe shortcut) -- deterministic asserts: probe success = real capability shape
        // (incl. seed key), probe failure = 5xx; neither state may be the {"cached":false} shortcut shape.
        let res = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/capabilities?refresh=1")
                    .header("host", "127.0.0.1:9810")
                    .header(header::AUTHORIZATION, "Bearer tk")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let st = res.status();
        let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        assert!(
            !st.is_success() || v.get("cached") != Some(&Value::Bool(false)),
            "authenticated refresh must not take the no-probe shortcut, got {st} {v}"
        );
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/state")
                    .header("host", "127.0.0.1:9810")
                    .header(header::AUTHORIZATION, "Bearer tk")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[test]
    fn drive_quiet_budget_is_two_seconds() {
        // design §D2 lck-F1 budget pin: step-2 total budget is exactly 2s (changing it = regression tripwire against the oxmgr grace window).
        assert_eq!(DRIVE_QUIET_BUDGET, Duration::from_secs(2));
    }

    #[test]
    fn shutdown_plan_writes_flag_then_awaits_drive_first() {
        let p = shutdown_plan(true, false);
        assert_eq!(p[0], ShutdownStep::WriteStopFlag, "(1) stop flag always first");
        assert_eq!(p[1], ShutdownStep::AwaitDriveQuiet, "(2) drive quiet always second");
        assert_eq!(*p.last().unwrap(), ShutdownStep::Exit, "(5) always last");
    }

    #[test]
    fn shutdown_plan_clears_only_when_settled_and_state_present() {
        // drive quiet AND pressure present => steps 3-4 (ClearPressure) show up.
        let p = shutdown_plan(true, true);
        assert!(
            p.contains(&ShutdownStep::ClearPressure),
            "settled+present should tear down actively"
        );
        // no pressure => steps 3-4 skip as no-ops (only 1, 2, 5 remain).
        let q = shutdown_plan(false, false);
        assert!(!q.contains(&ShutdownStep::ClearPressure), "no pressure should not tear down");
        assert_eq!(
            q,
            vec![ShutdownStep::WriteStopFlag, ShutdownStep::AwaitDriveQuiet, ShutdownStep::Exit]
        );
    }

    #[test]
    fn shutdown_plan_timeout_abandons_clear() {
        // lck-F1: step-2 timeout (drive_settled=false) => skip clear and exit even with state present (leave it to the watchdog),
        // never race an in-flight step that may still land tc.
        let p = shutdown_plan(false, true);
        assert!(
            !p.contains(&ShutdownStep::ClearPressure),
            "on timeout clear must be abandoned (lck-F1)"
        );
        assert_eq!(
            p,
            vec![ShutdownStep::WriteStopFlag, ShutdownStep::AwaitDriveQuiet, ShutdownStep::Exit]
        );
    }

    #[test]
    fn token_file_generates_0600_when_absent() {
        let dir = std::env::temp_dir().join(format!("wnet-t3-gen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sub").join("weaknet.token"); // parent dir sub absent -> auto-created
        let tok = resolve_token_file(&path).unwrap();
        assert_eq!(tok.len(), 64, "CSPRNG 32B hex = 64 chars");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            tok,
            "file content = return value"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600,
                "new file must be 0600"
            );
        }
        // second call takes the read branch (same value, not regenerated).
        assert_eq!(resolve_token_file(&path).unwrap(), tok);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn token_file_rejects_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("wnet-t3-rej-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("weaknet.token");
        std::fs::write(&path, "leaky-token\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = resolve_token_file(&path).unwrap_err();
        assert!(err.msg.contains("0600"), "cause must mention 0600: {}", err.msg);
        // after fixing to 0600 it passes, and the trailing newline is trimmed.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(resolve_token_file(&path).unwrap(), "leaky-token");
        std::fs::remove_dir_all(&dir).ok();
    }
}
