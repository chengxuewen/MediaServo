//! server.rs —— serve 控制面（design §API 契约 + §serve 安全栈，rev-2.2 全家桶）。
//!
//! 门槛栈：Host 白名单恒开 →（--lan 时 Origin 白名单）→ Bearer token（唯一豁免 = SSE
//! `?token=`，EventSource 无 header 能力）；bind 门槛 = 非 loopback 未 --lan 启动拒绝；
//! CORS 全关（无跨源需求，面板同源伺服）。门序 Host 先于 token：Host 是源隔离形门槛，
//! 先把 rebinding 流量拒在无信息泄露处（不暴露「token 对/错」信号）。
//!
//! 写路径与 CLI 同构：flock 瞬持（engine::take_write_lock）→ engine::replay；
//! verify Deferred——200 仅要求回读指纹过，实测双采样移入后台任务，结果落 timeline
//! `verify` 事件（SSE 状态帧 ev 透传播报，design §server「verify 移出响应路径」）。
//! 读路径零副作用：state.json 直读 / tc 回读仅经 state.teardown 重建通道（不重探、
//! 不 ensure——GET 永不建容器/起进程）/ stats 复用 scope::StatsClient（token 缓存进程级）。
//!
//! 错误体统一 {"error":...}；Fail.code 映射：4→400 · 3→409 · 2→500（main.rs 头注语义沿用）。
//! scenario run/stop = 诚实 501（T8 到场），但 basename/inline 校验先行——400 是真的。

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::Read as _;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Instant;

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
use crate::scope::{self, Capabilities, StatsClient, Targeting};
use crate::spec::{self, Dir, ImpairSpec, LossSpec};
use crate::state::{self, Dirs, State};

/// bash/CLI 同规缺省保险丝（apply 未带 duration 时）。
const DEFAULT_DURATION: u64 = 300;
/// C20 豁免形：文档化缺省常量 + `--listen` 显式覆写（design §serve 安全 1）。
const DEFAULT_LISTEN: &str = "127.0.0.1:9810";
/// SSE 状态帧节拍（design §API：2s）。
const FRAME_SECS: u64 = 2;
/// scenario inline 上限（design §serve 安全 6）。
const MAX_INLINE_BYTES: usize = 32 * 1024;
/// ev 字段单帧事件上限。ponytail: 防 CLI 刷屏单帧爆量，50 条足够面板滚动，超限丢弃老行
/// （cursor 照推进）。
const MAX_EV_LINES: usize = 50;

// ---------- 装配与共享态 ----------

/// serve 装配参数（tests 直构零 env 依赖；生产由 [`serve_main`] 从 env+flag 组装）。
#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub token: String,
    pub dirs: Dirs,
    pub env: Env,
    pub lan: bool,
    pub listen_host: String,
    pub listen_port: u16,
    /// server_url 三级链解析结果（None = /v1/streams 与 stats 定向报因降级）。
    pub server_url: Option<String>,
    /// capabilities 注入（api_matrix 免真探；T7 的 WEAKNET_FAKE_CAPS 走同一字段）。
    pub caps_override: Option<Capabilities>,
    /// SSE 帧数上限（api_matrix 发满即收口断 body；None = 无限）。
    pub sse_frames_limit: Option<u32>,
}

struct Ctx {
    cfg: ServeConfig,
    /// 进程级 stats 客户端（server_url 可解析 ∧ WEAKNET_ADMIN_PASS 在位时才建；登录 token 缓存复用）。
    stats: Option<Mutex<StatsClient>>,
    /// capabilities 缓存（refresh 重探后覆写）。
    caps: Mutex<Option<Capabilities>>,
}

impl Ctx {
    fn new(cfg: ServeConfig) -> Self {
        let stats = cfg.server_url.as_deref().and_then(|url| {
            StatsClient::from_env(url)
                .map_err(|e| {
                    println!("weaknet(serve): /v1/streams 降级（stats 客户端不可建）：{}", e.msg);
                })
                .ok()
                .map(Mutex::new)
        });
        Self { cfg, stats, caps: Mutex::new(None) }
    }
}

// ---------- 路由 ----------

/// 路由工厂（注入固定 Config，测试 oneshot 不触网；全 query 化 C34，零路径参数）。
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

// ---------- 内嵌面板（T7：rust-embed ui/；静态资产免 token——401 引导页须先于凭证可载，Host 白名单仍生效） ----------

/// 面板资产编译期内嵌（design §Files：src/ui，零构建链）。debug-embed 已开：调试构建同样内嵌，二进制自足。
#[derive(Embed)]
#[folder = "src/ui/"]
struct UiAssets;

/// 扩展名 → MIME（embed 键查表，不涉文件系统；未知一律 octet-stream 防嗅探执行）。
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
        None => err_json(StatusCode::NOT_FOUND, "资源不存在"),
    }
}

/// GET / —— index.html。
async fn get_index() -> Response {
    asset_response("index.html", true)
}

/// GET /assets/* —— app.js/css/vendor。穿越防御双保险：显式拒 `..` 与前导 '/'，
/// 且 embed 键为精确哈希查找——路径永不触盘，未命中仅 404 零泄露。
async fn get_asset(Path(p): Path<String>) -> Response {
    if p.contains("..") || p.starts_with('/') {
        return err_json(StatusCode::BAD_REQUEST, "非法资源路径");
    }
    asset_response(&p, false)
}

/// Host → Origin(--lan) → Bearer 三道门槛（/v1* 才查 token；Host 恒查）。
async fn guard(AxumState(ctx): AxumState<Arc<Ctx>>, req: Request, next: Next) -> Response {
    if !host_allowed(&ctx, req.headers()) {
        return err_json(
            StatusCode::FORBIDDEN,
            "Host 头不在白名单（loopback/localhost/::1；跨机需 --lan）",
        );
    }
    if ctx.cfg.lan && !origin_allowed(&ctx, req.headers()) {
        return err_json(StatusCode::FORBIDDEN, "Origin 与监听地址不符（--lan Origin 白名单）");
    }
    if req.uri().path().starts_with("/v1") && !auth_ok(&ctx, &req) {
        return err_json(StatusCode::UNAUTHORIZED, "缺失或错误的 token（横幅行整行复制）");
    }
    next.run(req).await
}

fn auth_ok(ctx: &Ctx, req: &Request) -> bool {
    if let Some(t) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
    {
        return constant_time_eq(t.as_bytes(), ctx.cfg.token.as_bytes());
    }
    // 唯一豁免：SSE EventSource 无 header 能力 → ?token=（design §API 鉴权条）。
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

/// 等时比较（无早退；长度差亦并入 diff 累加）。ponytail: 代替 timing_guard 依赖——
/// 本地 dev 面板，微秒级偏差可接受，注释即文档。
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

/// 剥端口 → 主机串。仅「单冒号且尾段全数字」剥端口（裸 IPv6 `::1` 不剥）；`[::1]:9810` 取括号段。
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

/// --lan Origin 白名单：浏览器必带且须等于监听 host[:port]；非浏览器（curl 无 Origin）放行
/// （design 裁决：Origin 仅约束浏览器跨源，服务端面已由 token 把守）。
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

// ---------- 错误映射 ----------

struct AppErr {
    status: StatusCode,
    msg: String,
}

impl AppErr {
    fn bad(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, msg: msg.into() }
    }
    fn not_impl(msg: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_IMPLEMENTED, msg: msg.into() }
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

/// 引擎/FS 全同步（std::process / TcpStream）→ 统一进 spawn_blocking；JoinError = 500 报因。
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Wn<T> + Send + 'static) -> Wn<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Fail::env(format!("阻塞任务崩溃/中止: {e}")))?
}

// ---------- GET 面 ----------

/// state.json 全量 JSON；无 state = `{"active": false}`（design §API）。
async fn get_state(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let dirs = ctx.cfg.dirs.clone();
    let st = blocking(move || State::read_from(&dirs.state_json()).map_err(Fail::env)).await?;
    match st {
        Some(s) => serde_json::to_value(&s)
            .map(Json)
            .map_err(|e| AppErr::from(Fail::env(format!("state 序列化失败: {e}")))),
        None => Ok(Json(json!({"active": false}))),
    }
}

async fn get_profiles() -> Result<Json<Value>, AppErr> {
    let v = blocking(|| Ok::<Value, Fail>(list_yaml_dir("profiles"))).await?;
    Ok(Json(v))
}

/// 剧本只读列举（T7 S 页签数据源；与 /v1/profiles 同形同目录寻径，零业务逻辑）。
async fn get_scenarios() -> Result<Json<Value>, AppErr> {
    let v = blocking(|| Ok::<Value, Fail>(list_yaml_dir("scenarios"))).await?;
    Ok(Json(v))
}

fn list_yaml_dir(sub: &str) -> Value {
    let Some(root) = scope::weaknet_d_root() else {
        return json!({sub: [], "note": "无资产目录（WEAKNET_ASSETS_DIR / 二进制同级 weaknet.d / scripts/weaknet.d）"});
    };
    let dir = root.join(sub);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| {
                e.ok().and_then(|e| {
                    let p = e.path();
                    (p.is_file() && p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
                        .then(|| p.file_stem().and_then(|s| s.to_str().map(str::to_string)))
                        .flatten()
                })
            })
            .collect()
        })
        .unwrap_or_default();
    names.sort();
    json!({sub: names})
}

/// capabilities：缓存优先；`?refresh=1` 重探（seed 版本门；ifb 位 = false +「T9 到场」——
/// 先读后探链随 T9 进 scope::capabilities 单点替换，此处不复制探测结构）。
/// 重探在写锁瞬持内执行（design §引擎 capability：防拆活跃会话，T9 生效面先行钉住）。
async fn get_capabilities(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, AppErr> {
    if let Some(c) = &ctx.cfg.caps_override {
        return Ok(Json(caps_json(c)));
    }
    let refresh = q.get("refresh").is_some_and(|v| v == "1" || v == "true");
    let cached = ctx.caps.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(c) = cached.filter(|_| !refresh) {
        return Ok(Json(caps_json(&c)));
    }
    let c = ctx.clone();
    let caps = blocking(move || {
        let _lk = engine::take_write_lock(&c.cfg.dirs)?;
        let iface = State::read_from(&c.cfg.dirs.state_json())
            .ok()
            .flatten()
            .map(|s| s.iface)
            .unwrap_or_else(|| engine::resolve_iface(None));
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

/// stats 代理（design §API：{room, owner, live, remote_ports[], local_port}）；
/// 不可达/未配置 = 200 `{"streams": [], "note": 报因}`（禁 500——面板降级渲染）。
async fn get_streams(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let c = ctx.clone();
    let out = blocking(move || {
        let m = c
            .stats
            .as_ref()
            .ok_or_else(|| Fail::env("stats 未配置：需 server_url（--server-url/env WEAKNET_SERVER_URL/out 探测）+ WEAKNET_ADMIN_PASS"))?;
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

// ---------- POST 面 ----------

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
    v.as_u64().ok_or_else(|| Fail::bad_param(format!("{k} 需非负整数，得 {v}")))
}

fn req_u16(v: Value, k: &str) -> Wn<u16> {
    v.as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .ok_or_else(|| Fail::bad_param(format!("{k} 需 0-65535 整数，得 {v}")))
}

fn req_str(v: Value, k: &str) -> Wn<String> {
    v.as_str().map(str::to_owned).ok_or_else(|| Fail::bad_param(format!("{k} 需字符串，得 {v}")))
}

fn req_ports(v: Value) -> Wn<Vec<u16>> {
    let arr = v.as_array().ok_or_else(|| Fail::bad_param(format!("ports 需数组，得 {v}")))?;
    arr.iter().map(|n| req_u16(n.clone(), "ports[]")).collect()
}

fn req_csv(v: Value, k: &str, field: &str) -> Wn<String> {
    let o = v
        .as_object()
        .ok_or_else(|| Fail::bad_param(format!("{k} 需对象 {{\"{field}\": [...]}}")))?;
    let arr = o
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| Fail::bad_param(format!("{k}.{field} 需字符串数组")))?;
    let mut items: Vec<&str> = Vec::new();
    for x in arr {
        items.push(
            x.as_str().ok_or_else(|| Fail::bad_param(format!("{k}.{field} 项需字符串，得 {x}")))?,
        );
    }
    if items.is_empty() {
        return Err(Fail::bad_param(format!("{k}.{field} 空集")));
    }
    Ok(items.join(","))
}

/// (spec 余键, 定向, iface, sig_port)。兼容形：`sig_port`（design §API 词）与
/// `signaling_port`（CLI flag 词）同义，前者优先。
type BodySplit = (Map<String, Value>, TargetingCmd, Option<String>, Option<u16>);

/// body 拆壳：额外键先取，余下 = ImpairSpec flat 形（未知键由 from_flat_json 拒绝——typos 显性化）。
fn split_body(raw: &[u8]) -> Wn<BodySplit> {
    let v: Value =
        serde_json::from_slice(raw).map_err(|e| Fail::bad_param(format!("body JSON 非法: {e}")))?;
    let mut obj = match v {
        Value::Object(m) => m,
        other => return Err(Fail::bad_param(format!("body 需 JSON 对象，得 {other}"))),
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

/// POST /v1/apply —— 200 = 已下发 + 回读指纹过（verify 后台化）；watchdog 由 replay Apply 分支
/// spawn（REST 与 CLI 同规，design §fuse）。
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

/// duration 在 split 后余 map 里（属 spec 邻键）——先摘出再交 from_flat_json。
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

/// scenario job 独占门（design §server rev-2.2 Momus-B2）：属主存活 = 409/exit3；
/// 死主旗标自清放行（Momus-minor#1，T8 前也保 /v1/state 帧 job 不撒谎）。
fn job_gate(prior: &Option<State>) -> Wn<()> {
    if let Some(st) = prior
        && let Some(j) = &st.job
        && fuse::read_proc_starttime(j.pid) == Some(j.starttime)
    {
        return Err(Fail::conflict("scenario 进行中（先 stop）"));
    }
    Ok(())
}

fn sanitize_job(st: &mut State) {
    if let Some(j) = &st.job
        && fuse::read_proc_starttime(j.pid) != Some(j.starttime)
    {
        eprintln!("weaknet(serve): WARN 陈旧 job「{}」（pid {} 已亡）——清旗标放行", j.name, j.pid);
        st.job = None;
    }
}

/// POST /v1/set —— 无活跃 spec = 409 引导总闸（design §UI 写路径协议）；字段覆盖合并
/// 基底后全量重放（同 CLI：不续命；scope 缺省复用 state.pairs 免重拉 stats）。
async fn post_set(
    AxumState(ctx): AxumState<Arc<Ctx>>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, AppErr> {
    let (mut obj, target, iface, sig_port) = split_body(&body)?;
    let duration = target_duration(&mut obj)?;
    if duration.is_some() {
        return Err(AppErr::from(Fail::bad_param(
            "set 不续命（duration 归 apply）——改存活请重新 apply",
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
    engine::autoheal_if_expired(&c.cfg.dirs, &c.cfg.env)?; // CLI Set 同规（apply/clear 免检、set/status 开场先验）
    let mut prior = State::read_from(&c.cfg.dirs.state_json())
        .map_err(Fail::env)?
        .ok_or_else(|| Fail::conflict("no active spec — 先 apply/开总闸"))?;
    sanitize_job(&mut prior);
    let has_override = !cmd.overrides.is_empty()
        || cmd.iface.is_some()
        || cmd.sig_port.is_some()
        || !cmd.target.ports.is_empty()
        || cmd.target.stream.is_some()
        || cmd.target.device.is_some();
    if !has_override {
        return Err(Fail::bad_param("set 需至少一个覆盖字段（spec 键/iface/sig_port/定向）"));
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
        Targeting { scope: prior.scope, ports: prior.ports.clone(), pairs: prior.pairs.clone() }
    };
    if targeting.ports.is_empty() && targeting.pairs.is_empty() {
        return Err(Fail::env(
            "无可用媒体口/配对（基底 state 为空且未给 ports/stream_scope/device_scope）——重新 apply",
        ));
    }
    let before = state::param_summary(&prior.spec);
    let iface = cmd.iface.clone().unwrap_or_else(|| prior.iface.clone());
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
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
    let spec_v =
        serde_json::to_value(&spec).map_err(|e| Fail::env(format!("spec 序列化失败: {e}")))?;
    Ok((spec_v, out.expires_at_ms, iface, out.leaf.0))
}

/// ImpairSpec → from_flat_json 键形（set 合并基底用；两向无损，单测钉住）。
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

/// POST /v1/clear —— 免锁救火通道（与 CLI 同规），幂等。
async fn post_clear(AxumState(ctx): AxumState<Arc<Ctx>>) -> Result<Json<Value>, AppErr> {
    let c = ctx.clone();
    let msg = blocking(move || engine::clear(&c.cfg.dirs, &c.cfg.env, false)).await?;
    Ok(Json(json!({"ok": true, "detail": msg})))
}

/// 后台实测双采样（verify 异步化）：1s 窗 leaf 增量 → timeline `verify` 事件（SSE ev 播报；
/// 失败横幅素材）。通道仅经 state.teardown 重建（零探测副作用）。
fn spawn_verify(ctx: Arc<Ctx>, iface: String, pre: u64, phase: &'static str, with_reset: bool) {
    tokio::spawn(async move {
        let c = ctx.clone();
        let r = blocking(move || {
            let st = State::read_from(&c.cfg.dirs.state_json())
                .map_err(Fail::env)?
                .ok_or_else(|| Fail::env("verify：state 已失（并发 clear？）"))?;
            let chan = engine::channel_of(&st.teardown)
                .ok_or_else(|| Fail::env("verify：teardown 通道不可重建"))?;
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
            eprintln!("weaknet(serve): WARN 后台 verify 落事件失败: {}", e.msg);
        }
    });
}

// ---------- scenario（501 诚实位，400 校验先行——tasks T6「穿越案必须真」） ----------

fn is_basename(f: &str) -> bool {
    !f.is_empty() && f != "." && f != ".." && !f.contains(['/', '\\'])
}

async fn post_scenario_run(body: axum::body::Bytes) -> Result<Json<Value>, AppErr> {
    let v: Value = serde_json::from_slice(&body)
        .map_err(|e| AppErr::from(Fail::bad_param(format!("body JSON 非法: {e}"))))?;
    if let Some(f) = v.get("file") {
        let f = f.as_str().ok_or_else(|| AppErr::bad("file 需字符串（basename-only）"))?;
        if !is_basename(f) {
            return Err(AppErr::bad(
                "file 必须为 basename（禁路径穿越/绝对路径，design §serve 安全 6）",
            ));
        }
    } else if let Some(inl) = v.get("inline") {
        let s = inl.as_str().ok_or_else(|| AppErr::bad("inline 需字符串"))?;
        if s.len() > MAX_INLINE_BYTES {
            return Err(AppErr::bad(format!("inline 超 32KB 上限（{} 字节）", s.len())));
        }
    } else {
        return Err(AppErr::bad(
            "body 需 file{{\"file\":\"<basename>\"}} 或 {{\"inline\":\"<yaml>\"}}",
        ));
    }
    Err(AppErr::not_impl("scenario 引擎随 T8 到场（job 调度/独占旗标/baseline 配对）"))
}

async fn post_scenario_stop() -> Result<Json<Value>, AppErr> {
    Err(AppErr::not_impl("scenario stop 随 T8 到场（job 状态面）"))
}

// ---------- SSE ----------

/// 状态帧节拍（design §API）。tokio::time::interval 首拍即发——文档化：面板秒开。
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

/// 每连接 FrameState（ev 尾标/差分样本独立——无跨连接共享可变态，消竞态）。
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
            eprintln!("weaknet(serve): WARN SSE 游标初始化失败: {}", e.msg);
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
            return; // 浏览器断开
        }
        sent += 1;
        if limit.is_some_and(|n| sent >= n) {
            return; // 测试收口（api_matrix 单帧断言）；生产 limit=None 恒无限
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

/// 状态帧（design §API 全字段）：expires_at = state.expires_at_ms 顶层镜像；job 形 =
/// state.job（{name,pid,starttime}，done/total 富化随 T8）；无 state = spec/scope/dir/
/// expires_at/job 全 null——「键恒在、值可 null」与 kbps null 语义一致。
fn build_frame(ctx: &Ctx, mut fs: FrameState) -> (Value, FrameState) {
    let st = State::read_from(&ctx.cfg.dirs.state_json()).ok().flatten();
    let mut frame =
        json!({"spec": null, "scope": null, "dir": null, "expires_at": null, "job": null});
    let mut tc = json!({"sent": null, "dropped": null});
    if let Some(s) = &st {
        frame["spec"] = serde_json::to_value(&s.spec).unwrap_or(Value::Null);
        frame["scope"] = serde_json::to_value(s.scope).unwrap_or(Value::Null);
        frame["dir"] = serde_json::to_value(s.dir).unwrap_or(Value::Null);
        frame["expires_at"] = json!(s.expires_at_ms);
        frame["job"] = serde_json::to_value(&s.job).unwrap_or(Value::Null);
        // 活性计数仅经 state.teardown 重建通道（不 probe_channel——读面零 ensure 副作用）。
        if let Some(chan) = engine::channel_of(&s.teardown)
            && let Ok(show) = engine::tc_exec(&chan, &["-s", "qdisc", "show", "dev", &s.iface])
            && let Some(ls) = engine::leaf_stats(&show)
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
        return json!([]); // 未配置（无 url/无凭证）= 空表；降级提示面归 UI（T7）
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
                        _ => None, // 首点/新出现/计数回退 = null（两语义禁混 0，design §API）
                    };
                    json!({"room": a.room, "owner": a.owner, "kbps": kbps, "stale": false, "live": a.live})
                })
                .collect();
            fs.prev = Some((Instant::now(), cur));
            fs.cache = agg;
            Value::Array(out)
        }
        Err(e) => {
            // stats 失败 → 上帧行保留 + stale:true（不跳帧、不假 0——kbps 语义条款）。
            eprintln!("weaknet(serve): WARN stats 拉取失败（本帧 stale）: {}", e.msg);
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
            _ => None, // 任一成员行缺 byte_count（旧 server）→ 整房不可差分 = None
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

/// timeline 尾标增量 → ev 数组（CLI 写操作/自愈/verify 结果的通路，design §API）。
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

// ---------- serve 主体 ----------

/// `serve` 子命令入口（main.rs 建 runtime 后 block_on）：门槛校验 → token → 启动交叉判定
/// → 横幅 → bind + 惰性过期 tick + axum::serve。
pub async fn serve_main(
    listen: Option<&str>,
    lan: bool,
    token_opt: Option<&str>,
    server_url_flag: Option<&str>,
) -> Wn<()> {
    let (host, port) = parse_listen(listen.unwrap_or(DEFAULT_LISTEN))?;
    if !is_loopback_host(&host) && !lan {
        return Err(Fail::env(format!(
            "bind 门槛：--listen {host}:{port} 非 loopback 且未 --lan——启动拒绝（design §serve 安全 1）"
        )));
    }
    let dirs = Dirs::from_env();
    let env = Env::from_env();
    if env.channel_pref == "local" {
        // 启动即验（免运行到写路径才炸）；local 判据零副作用，auto/sidecar 的 ensure 留按需。
        engine::probe_channel(&env, &engine::resolve_iface(None)).map(|_| ())?;
    }
    let token = std::env::var("WEAKNET_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| token_opt.map(str::to_owned).filter(|s| !s.is_empty()))
        .map_or_else(gen_token, Ok)?;
    if lan && token.len() < 16 {
        println!(
            "weaknet(serve): WARN --lan 且 token 短于 16 字符——跨机面建议 WEAKNET_TOKEN 给足熵"
        );
    }
    let server_url = match scope::server_url_or_default(server_url_flag) {
        Ok(u) => Some(u),
        Err(e) => {
            println!("weaknet(serve): /v1/streams 与 stats 定向下线：{}", e.msg);
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
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| Fail::env(format!("bind {host}:{port} 失败: {e}（占用？--listen 换端口）")))?;
    axum::serve(listener, build_router_with_ctx(ctx))
        .await
        .map_err(|e| Fail::env(format!("serve 异常退出: {e}")))
}

/// 解析 `--listen`（host:port；支持 [::1]:9810 括号形）。
fn parse_listen(s: &str) -> Wn<(String, u16)> {
    let (h, p) = s
        .rsplit_once(':')
        .ok_or_else(|| Fail::bad_param(format!("--listen 需 host:port，得 {s:?}")))?;
    // 括号形 [::1]:9810 剥两侧；裸 IPv6 无括号形按末冒号分端口。
    let h = h.trim_start_matches('[').trim_end_matches(']').to_string();
    if h.is_empty() {
        return Err(Fail::bad_param(format!("--listen 主机为空: {s:?}")));
    }
    let p = p
        .trim_end_matches(']')
        .parse::<u16>()
        .map_err(|e| Fail::bad_param(format!("--listen 端口非法 {s:?}: {e}")))?;
    Ok((h, p))
}

fn is_loopback_host(h: &str) -> bool {
    matches!(h, "127.0.0.1" | "localhost" | "::1" | "[::1]") || h.starts_with("127.")
}

/// CSPRNG ≥128bit：/dev/urandom 32B → hex（零新依赖，std 即可）。
fn gen_token() -> Wn<String> {
    let mut buf = [0u8; 32];
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| Fail::env(format!("/dev/urandom 不可读: {e}")))?;
    f.read_exact(&mut buf).map_err(|e| Fail::env(format!("/dev/urandom 读取失败: {e}")))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// 呈现规则（design §API Token 条）：loopback 横幅 = 整行可复制 URL；--lan = 仅 token
/// （URL 带 token 会进 shell 历史/日志）。
fn print_banner(cfg: &ServeConfig) {
    if cfg.lan {
        println!(
            "weaknet serve (--lan) 监听 {}:{} — token（仅此一现，浏览器打开后粘贴）:",
            cfg.listen_host, cfg.listen_port
        );
        println!("  {}", cfg.token);
    } else {
        println!(
            "weaknet serve 面板（复制即用）: http://{}:{}/?token={}",
            cfg.listen_host, cfg.listen_port, cfg.token
        );
    }
    println!(
        "weaknet serve 安全栈: Host 白名单恒开 + Bearer（SSE 唯一 ?token= 豁免）+ CORS 全关 + 写路径 flock 瞬持"
    );
}

/// serve 启动交叉判定（design §Error handling，与 CLI status 共用 assert_fingerprint 原语）：
/// state × tc 回读，分歧 → 清 state 文件 + WARN——绝不清内核盲重放（qdisc 现况归 watchdog/人工 clear）。
fn startup_cross_check(dirs: &Dirs) -> Wn<()> {
    // 无 state：启动不探 tc（通道探测有 ensure 副作用且无判定对象）；残留面由 apply 守卫/status 收口。
    if let Some(mut st) = State::read_from(&dirs.state_json()).map_err(Fail::env)? {
        if st.job.is_some() {
            sanitize_job(&mut st);
            if st.job.is_none() {
                st.write_to(&dirs.state_json()).map_err(Fail::env)?;
            }
        }
        let Some(chan) = engine::channel_of(&st.teardown) else {
            println!(
                "weaknet(serve): WARN state.teardown 通道不可重建（state 损坏？）——交叉判定跳过"
            );
            return Ok(());
        };
        match engine::tc_exec(&chan, &["qdisc", "show", "dev", &st.iface]) {
            Ok(show) => {
                if let Err(e) =
                    engine::assert_fingerprint(&show, &spec::render_netem_spec_leg(&st.spec))
                {
                    eprintln!(
                        "weaknet(serve): WARN 启动交叉判定分歧（qdisc 被外部改写/已消失）：{}——清 state 文件复位（未动内核；现况残留归 watchdog/clear）",
                        e.msg
                    );
                    if let Err(re) = std::fs::remove_file(dirs.state_json())
                        && re.kind() != std::io::ErrorKind::NotFound
                    {
                        return Err(Fail::env(format!("清 state 失败: {re}")));
                    }
                } else {
                    println!("weaknet(serve): 启动交叉判定 state × tc 一致 ✓");
                }
            }
            Err(e) => {
                eprintln!(
                    "weaknet(serve): WARN 启动 tc 不可读（{e}）——state 保留，惰性过期自愈/后台 verify 兜底"
                );
            }
        }
    }
    Ok(())
}

/// 惰性过期 fuse 层在 serve 内的定时身（design §fuse「serve tokio 定时器」——独立于 SSE 连接
/// 数，无面板也自愈）。取锁瞬持再检；锁忙 = 写进行中，共法排队跳过本拍（C15：非冲突错误才 WARN）。
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
                eprintln!("weaknet(serve): WARN 惰性过期检查异常: {}", e.msg);
            }
        }
    });
}

/// dev-only 注入（tasks T7 Verify：灰态断言依赖）：`WEAKNET_FAKE_CAPS="ifb_ingress=false,seed=false"`。
/// 键 ∈ {seed, dir_lo, ifb_ingress}，值 true|false；未给键取保守缺省（seed=false/dir_lo=true/
/// ifb_ingress=false——假灰不假亮，禁虚构能力）。结果进 ServeConfig.caps_override →
/// /v1/capabilities 直返缓存跳过真探测。生产勿设——它把能力面钉成谎。
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
        ifb_reason: "dev 注入（WEAKNET_FAKE_CAPS）：宿主未加载 ifb——补救=宿主 root modprobe ifb；真判定随 T9 探测链到场".into(),
    };
    for pair in raw.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            eprintln!("weaknet(serve): WARN WEAKNET_FAKE_CAPS 忽略项 {pair:?}（形如 key=true|false）");
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
                eprintln!("weaknet(serve): WARN WEAKNET_FAKE_CAPS 未知键 {v:?}（合法: seed|dir_lo|ifb_ingress）");
            }
            _ => eprintln!("weaknet(serve): WARN WEAKNET_FAKE_CAPS 非法值 {pair:?}（值需 true|false）"),
        }
    }
    Some(c)
}

// ---------- 单元面纯函数测 ----------

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
        assert_eq!(agg[1].bytes, None, "成员行缺 byte_count → 整房不可差分");
    }

    #[test]
    fn fake_caps_parses_and_defaults_conservative() {
        let c = parse_fake_caps(Some("ifb_ingress=false")).unwrap();
        assert!(!c.ifb_ingress);
        assert!(!c.seed, "未给键=保守缺省");
        assert!(c.dir_lo);
        assert!(c.ifb_reason.contains("宿主未加载") && c.ifb_reason.contains("T9"), "灰显 tooltip 两词面");
        let g = parse_fake_caps(Some(" seed=true , dir_lo=false ,ifb_ingress=true ")).unwrap();
        assert!(g.seed && !g.dir_lo && g.ifb_ingress && g.ifb_reason.is_empty());
        assert_eq!(parse_fake_caps(None), None);
        assert_eq!(parse_fake_caps(Some("  ")), None);
        // 垃圾项 WARN 后忽略，不毁其余
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
        assert!(res.headers()[header::CONTENT_SECURITY_POLICY].as_bytes().starts_with(b"default-src"));
        let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&body).to_ascii_lowercase().contains("<!doctype html"));
        let res = app.clone().oneshot(req("/assets/app.js")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/javascript; charset=utf-8");
        let res = app.clone().oneshot(req("/assets/vendor/uPlot.iife.min.js")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // 穿越：显式 `..`（含解码形）= 400；未命中键 = 404 零泄露
        assert_eq!(app.clone().oneshot(req("/assets/../server.rs")).await.unwrap().status(), StatusCode::BAD_REQUEST);
        assert_eq!(app.clone().oneshot(req("/assets/%2e%2e/Cargo.toml")).await.unwrap().status(), StatusCode::BAD_REQUEST);
        assert_eq!(app.clone().oneshot(req("/assets/nope.js")).await.unwrap().status(), StatusCode::NOT_FOUND);
        // /v1* 鉴权不变：无 token 401、带 token 200
        assert_eq!(app.clone().oneshot(req("/v1/state")).await.unwrap().status(), StatusCode::UNAUTHORIZED);
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
}
