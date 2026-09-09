//! scope.rs —— stats 轻量客户端（std::net 明文 HTTP/1.1 小客户端，design §Dependencies
//! 去 reqwest 裁决：~40 行 GET/POST，零新增依赖）+ 流/设备定向（§dir 腿定义表 rev-2.2：
//! Stream/Device = 有序对 (local, remote) 集，Device 由 owner 分组后拍平）。
//!
//! 真值源：`scripts/weaknet.sh`（api_get/login 流、media_ports/stream_ports 解析、
//! exit2 措辞）+ server `admin.rs::sfu_stats` 列表模式 wire 形（小刀 C：行 +owner）。
//! 活性谓词：列表模式服务端已按 ICE tuple 过滤（「表内即活」，PIT-185）；客户端侧
//! `live` = transport_id 字段存在（tuple 衍生的恒有字段）——防御非列表形/未来漂移。
//!
//! 退出码：解析/目标为空/URL 不可得 = exit2（Fail::env）；参数互斥 = exit4（bad_param）。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::engine::{Env, Fail, Wn, probe_channel, tc_exec};
use crate::spec::ScopeSel;

/// 登录 token 进程内缓存窗（弱网工具生命周期短，5min 够用；过期重登）。
const TOKEN_TTL: Duration = Duration::from_secs(300);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// C20 豁免形：yaml 探测缺省端口对应的 host 回环（跨机用 --server-url/env）。
const PROBE_HOST: &str = "127.0.0.1";

// ---------- wire 形 ----------

/// stats 列表模式一行（一个 transport）。remote_ports 由 remote_port 折算
/// （design §API /v1/streams 聚形；当前服务端 tuple 单值 → 0 或 1 元）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub room: String,
    /// 小刀 C：房间注册设备 ID；旧 server 无字段/无主房 = None。
    pub owner: Option<String>,
    pub peer_id: String,
    /// "producer" | "consumer"
    pub role: String,
    /// "video" | "audio"
    pub kind: String,
    /// transport 观测行存在（tuple 衍生字段）= 活。
    pub live: bool,
    pub local_port: Option<u16>,
    pub remote_ports: Vec<u16>,
    /// stats 行字节计数（serve 状态帧 kbps 差分源；旧 server/缺字段=None——禁假 0）。
    pub byte_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawRow {
    #[serde(default)]
    room: String,
    /// 旧 server（小刀 C 前）无此键；新 server 无主房为 null——两形同为 None。
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    peer_id: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    transport_id: Option<String>,
    #[serde(default)]
    local_port: Option<u16>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default)]
    byte_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct StatsBody {
    #[serde(default)]
    streams: Vec<RawRow>,
}

/// 响应体 → 类型化行（纯函数，fixture 单测面对象；与 fetch 解耦）。
pub fn parse_streams_body(body: &str) -> Wn<Vec<StreamInfo>> {
    let v: StatsBody = serde_json::from_str(body)
        .map_err(|e| Fail::env(format!("stats JSON 解析失败: {e}（体首 {} 字节）", prefix_excerpt(body))))?;
    Ok(v.streams
        .into_iter()
        .map(|r| StreamInfo {
            room: r.room,
            owner: r.owner.filter(|s| !s.is_empty()),
            peer_id: r.peer_id,
            role: r.role,
            kind: r.kind,
            live: r.transport_id.is_some(),
            local_port: r.local_port,
            remote_ports: r.remote_port.into_iter().collect(),
            byte_count: r.byte_count,
        })
        .collect())
}

// ---------- 定向（§dir 表：pairs = (local_port, remote_port) 有序对） ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Targeting {
    pub scope: ScopeSel,
    /// scope=Media：本地媒体口集
    pub ports: Vec<u16>,
    /// scope=Stream/Device：拍平有序对集（去重升序）
    pub pairs: Vec<(u16, u16)>,
    /// E2（T8）：定向名字——Stream=房间集 / Device=设备 ID 集 / Media=空。
    /// 经 ApplyRequest.names 入 state，供 SSE 帧回显勾选（面板）与 apply 事件 stream 键（bash 小账）。
    pub names: Vec<String>,
}

impl Targeting {
    /// crate::engine::ScopeNames 映射（Stream→rooms / Device→devices / Media→空）——CLI/serve/scenario 共源。
    #[must_use]
    pub fn scope_names(&self) -> crate::engine::ScopeNames {
        match self.scope {
            ScopeSel::Stream => crate::engine::ScopeNames { rooms: self.names.clone(), devices: vec![] },
            ScopeSel::Device => crate::engine::ScopeNames { rooms: vec![], devices: self.names.clone() },
            ScopeSel::Media => crate::engine::ScopeNames::default(),
        }
    }
}

/// 单行 → (L,R) 对（缺任一端口 = 不可定向，WARN 留痕 C15）。
fn row_pairs(row: &StreamInfo, out: &mut Vec<(u16, u16)>) {
    let Some(l) = row.local_port else {
        eprintln!(
            "weaknet(scope): WARN 房间 {} {} 行 local_port 缺失（tuple 未含本地口？跳过）",
            row.room, row.role
        );
        return;
    };
    for &r in &row.remote_ports {
        out.push((l, r));
    }
}

fn finish_pairs(mut pairs: Vec<(u16, u16)>) -> Vec<(u16, u16)> {
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// 可用活房间清单（exit2 报因用，bash stream_rooms_available 同位）。
pub fn live_rooms_csv(rows: &[StreamInfo]) -> String {
    let mut rooms: Vec<&str> = rows
        .iter()
        .filter(|s| s.live)
        .map(|s| s.room.as_str())
        .collect();
    rooms.sort_unstable();
    rooms.dedup();
    if rooms.is_empty() {
        "?".into()
    } else {
        rooms.join(",")
    }
}

/// --stream：房间名逗号集 → 每活行 (L,R) 对。空集 exit2 + 活房间清单。
pub fn targeting_for_rooms(rows: &[StreamInfo], sel: &str) -> Wn<Targeting> {
    let want: Vec<&str> = sel.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if want.is_empty() {
        return Err(Fail::bad_param("--stream 需流名/房间名（逗号分隔多路）"));
    }
    let mut pairs = Vec::new();
    for row in rows.iter().filter(|s| s.live && want.contains(&s.room.as_str())) {
        row_pairs(row, &mut pairs);
    }
    let pairs = finish_pairs(pairs);
    if pairs.is_empty() {
        return Err(Fail::env(format!(
            "流 [{sel}] 无活性 transport（未连/名错；可用房间: {}）",
            live_rooms_csv(rows)
        )));
    }
    Ok(Targeting {
        scope: ScopeSel::Stream,
        ports: vec![],
        pairs,
        names: want.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// --device：owner 设备逗号集 → 其名下房间全部活行（producer+consumer）的 (L,R) 并集。
/// **peer_id 兜底**：owner 字段缺失（运行中的旧 server 早于小刀 C）时，按
/// role==producer ∧ peer_id==device 认定房间归属（信令 peer_id=设备 ID，signaling
/// room_owners 同源）——每次兜底命中打 WARN（显性化降级，C15）。空集 exit2 + 清单。
pub fn targeting_for_devices(rows: &[StreamInfo], sel: &str) -> Wn<Targeting> {
    let want: Vec<&str> = sel.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if want.is_empty() {
        return Err(Fail::bad_param("--device 需设备 ID（逗号分隔多个）"));
    }
    let mut owned_rooms: Vec<String> = Vec::new();
    let mut fallback_hits = 0usize;
    for row in rows.iter().filter(|s| s.live) {
        let owner_match = row
            .owner
            .as_deref()
            .is_some_and(|o| want.contains(&o));
        let fallback = !owner_match
            && row.owner.is_none()
            && row.role == "producer"
            && want.contains(&row.peer_id.as_str());
        if (owner_match || fallback) && !owned_rooms.contains(&row.room) {
            owned_rooms.push(row.room.clone());
        }
        if fallback {
            fallback_hits += 1;
        }
    }
    if fallback_hits > 0 {
        eprintln!(
            "weaknet(scope): WARN --device 经 peer_id 兜底匹配 {fallback_hits} 行（owner 字段缺失：server 早于小刀 C 或房间无主登记）"
        );
    }
    let mut pairs = Vec::new();
    for row in rows.iter().filter(|s| s.live && owned_rooms.contains(&s.room)) {
        row_pairs(row, &mut pairs);
    }
    let pairs = finish_pairs(pairs);
    if pairs.is_empty() {
        return Err(Fail::env(format!(
            "设备 [{sel}] 无活性流（owner/peer_id 两路匹配均空；可用房间: {}）",
            live_rooms_csv(rows)
        )));
    }
    Ok(Targeting {
        scope: ScopeSel::Device,
        ports: vec![],
        pairs,
        names: want.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// 段级：活行 local_port 并集（bash media_ports 同义）。空集 exit2 + bash L518 措辞。
pub fn targeting_media(rows: &[StreamInfo]) -> Wn<Targeting> {
    let mut ports: Vec<u16> = rows
        .iter()
        .filter(|s| s.live)
        .filter_map(|s| s.local_port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    if ports.is_empty() {
        return Err(Fail::env(
            "媒体口观测为空：server 未起 / 无在产流 / stats 不可达（起服务或重试；逃生门 --rtp-port；或 --stream 定向）",
        ));
    }
    Ok(Targeting { scope: ScopeSel::Media, ports, pairs: vec![], names: vec![] })
}

/// 作用域裁决优先级（bash resolve_ports 同序）：显式端口集 > stream/rooms > device/ids > stats 媒体口。
/// `stream="all"` = 显式回段级（bash 等价）。CLI 与 serve REST 共用核（T6 自 main.rs 提取，禁复制）。
/// stats 触达仅在需要时发生（显式端口逃生门保持零 server 可用——离线/无凭证场景）。
pub fn resolve_targeting(
    server_url: Option<&str>,
    stream: Option<&str>,
    device: Option<&str>,
    ports_flag: &[u16],
) -> Wn<Targeting> {
    if !ports_flag.is_empty() {
        if stream.is_some() || device.is_some() {
            eprintln!("weaknet: WARN --rtp-port 与 --stream/--device 并给：显式端口集优先（定向被忽略）");
        }
        return Ok(Targeting {
            scope: ScopeSel::Media,
            ports: ports_flag.to_vec(),
            pairs: vec![],
            names: vec![],
        });
    }
    let stream = stream.map(str::trim).filter(|s| !s.is_empty());
    let device = device.map(str::trim).filter(|s| !s.is_empty());
    if stream.is_some() && device.is_some() {
        return Err(Fail::bad_param("--stream 与 --device 互斥（作用域定向只能选一路）"));
    }
    let url = server_url_or_default(server_url)?;
    let mut client = StatsClient::from_env(&url)?;
    let rows = client.fetch_streams()?;
    if let Some(s) = stream.filter(|s| *s != "all") {
        return targeting_for_rooms(&rows, s);
    }
    if let Some(d) = device {
        return targeting_for_devices(&rows, d);
    }
    targeting_media(&rows)
}

/// profile/scenario 资产寻径（§车端面：二进制同级 weaknet.d 优先；内嵌兜底随 T7 embed）。
/// CLI 与 serve /v1/profiles 共用（T6 自 main.rs 上提到 lib 面，一条函数钉死）。
#[must_use]
pub fn weaknet_d_root() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("WEAKNET_ASSETS_DIR").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(v);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe_dir = std::env::current_exe().ok().and_then(|x| x.parent().map(Path::to_path_buf));
    if let Some(d) = &exe_dir {
        let sib = d.join("weaknet.d");
        if sib.is_dir() {
            return Some(sib);
        }
    }
    for base in std::iter::once(std::env::current_dir().unwrap_or_default()).chain(exe_dir) {
        for anc in base.ancestors().take(6) {
            let c = anc.join("scripts").join("weaknet.d");
            if c.is_dir() {
                return Some(c);
            }
        }
    }
    None
}

// ---------- server_url 解析链（flag > env WEAKNET_SERVER_URL > 探测 server.yaml） ----------

/// 纯链（三源全部注入——测试零环境触碰）。yaml 探测 = listen.port，host 回环缺省。
pub fn pick_server_url(flag: Option<&str>, env_url: Option<&str>, yaml: Option<&Path>) -> Wn<String> {
    for cand in [flag, env_url] {
        if let Some(u) = cand.map(str::trim).filter(|s| !s.is_empty()) {
            return normalize_url(u);
        }
    }
    if let Some(p) = yaml {
        let raw = std::fs::read_to_string(p)
            .map_err(|e| Fail::env(format!("读 {} 失败: {e}", p.display())))?;
        let doc: serde_yaml::Value = serde_yaml::from_str(&raw)
            .map_err(|e| Fail::env(format!("server.yaml 非法 {}: {e}", p.display())))?;
        let port = doc
            .get("listen")
            .and_then(|l| l.get("port"))
            .and_then(|p| p.as_u64())
            .ok_or_else(|| {
                Fail::env(format!("{} 缺 listen.port（server_url 无法派生）", p.display()))
            })?;
        let port = u16::try_from(port)
            .map_err(|_| Fail::env(format!("listen.port 越界 u16: {port}")))?;
        return normalize_url(&format!("http://{PROBE_HOST}:{port}"));
    }
    Err(Fail::env(
        "server_url 三源皆空：--server-url / env WEAKNET_SERVER_URL / out/server/etc/server.yaml（探测缺，逃生门 --rtp-port 不需 server_url）",
    ))
}

/// https → 直连报因（Out of scope，design §Dependencies：小客户端仅明文）。
fn normalize_url(u: &str) -> Wn<String> {
    if u.starts_with("https://") {
        return Err(Fail::env(format!(
            "https 属 Out 范围：需明文 http://（本工具 stats 客户端不含 TLS 栈），得 {u}"
        )));
    }
    let s = u.strip_prefix("http://").unwrap_or(u);
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err(Fail::env(format!("server_url 形态非法: {u}")));
    }
    Ok(format!("http://{}", s.trim_end_matches('/')))
}

fn host_port(url: &str) -> Wn<(String, u16)> {
    let s = url
        .strip_prefix("http://")
        .ok_or_else(|| Fail::env(format!("stats 请求内部错：非 http URL {url}")))?;
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|e| Fail::env(format!("server_url 端口非法 {p:?}: {e}")))?,
        ),
        None => (s.to_string(), 80),
    };
    Ok((host, port))
}

/// CLI 面装配：env 注入 + 仓库根祖先探测（bash SERVER_YAML 缺省位）。
pub fn server_url_or_default(flag: Option<&str>) -> Wn<String> {
    let env_url = std::env::var("WEAKNET_SERVER_URL").ok().filter(|s| !s.is_empty());
    let yaml = server_yaml_probe_path();
    pick_server_url(flag, env_url.as_deref(), yaml.as_deref())
}

fn server_yaml_probe_path() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("WEAKNET_SERVER_YAML").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(v);
        return p.is_file().then_some(p);
    }
    let exe_dir = std::env::current_exe().ok().and_then(|x| x.parent().map(Path::to_path_buf));
    for base in std::iter::once(std::env::current_dir().unwrap_or_default()).chain(exe_dir) {
        for anc in base.ancestors().take(6) {
            let cand = anc.join("out").join("server").join("etc").join("server.yaml");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

// ---------- HTTP 小客户端（std::net，Connection: close 读至 EOF） ----------

pub struct StatsClient {
    url: String,
    user: String,
    pass: String,
    token: Option<(String, Instant)>,
}

impl StatsClient {
    /// 凭证 env 注入（C20/口令纪律：绝不硬编码，缺失 exit2 报因——bash L122 同义）。
    pub fn from_env(server_url: &str) -> Wn<Self> {
        let pass = std::env::var("WEAKNET_ADMIN_PASS").map_err(|_| {
            Fail::env("admin 口令未注入：export WEAKNET_ADMIN_PASS=<dev 口令>（用户 env WEAKNET_ADMIN_USER 缺省 admin）")
        })?;
        Ok(Self {
            url: server_url.to_string(),
            user: std::env::var("WEAKNET_ADMIN_USER")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "admin".into()),
            pass,
            token: None,
        })
    }

    fn request(&mut self, method: &str, path: &str, json_body: Option<&str>) -> Wn<(u16, String)> {
        let (host, port) = host_port(&self.url)?;
        let addr = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| Fail::env(format!("解析 {host}:{port} 失败: {e}")))?
            .next()
            .ok_or_else(|| Fail::env(format!("{host}:{port} 无解析结果")))?;
        let mut sock = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|e| Fail::env(format!("连接 {host}:{port} 失败: {e}（server 未起？）")))?;
        sock.set_read_timeout(Some(IO_TIMEOUT))
            .and_then(|()| sock.set_write_timeout(Some(IO_TIMEOUT)))
            .map_err(|e| Fail::env(format!("socket 超时设定失败: {e}")))?;
        let auth = match &self.token {
            Some((t, _)) => format!("Authorization: Bearer {t}\r\n"),
            None => String::new(),
        };
        let body = json_body.unwrap_or("");
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: mediaservo-weaknet\r\n\
             {auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(req.as_bytes())
            .map_err(|e| Fail::env(format!("{method} {path} 发送失败: {e}")))?;
        let mut raw = String::new();
        sock.read_to_string(&mut raw)
            .map_err(|e| Fail::env(format!("{method} {path} 读取失败: {e}")))?;
        let (head, resp_body) = raw
            .split_once("\r\n\r\n")
            .ok_or_else(|| Fail::env(format!("{method} {path} 响应无头体分隔（非 HTTP 响应？）")))?;
        // ponytail: 只吃 content-length/EOF 体形（axum Json 恒有 content-length）；chunked 报因不裸解。
        if head.lines().any(|l| l.to_ascii_lowercase().contains("transfer-encoding: chunked")) {
            return Err(Fail::env(format!("{method} {path} 响应 chunked 编码：小客户端不支持")));
        }
        let status = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or_else(|| Fail::env(format!("{method} {path} 状态行非法: {:?}", head.lines().next())))?;
        Ok((status, resp_body.to_string()))
    }

    fn login(&mut self) -> Wn<String> {
        let body = serde_json::json!({ "username": self.user, "password": self.pass }).to_string();
        let (status, resp) = self.request("POST", "/api/auth/login", Some(&body))?;
        if status != 200 {
            return Err(Fail::env(format!(
                "登录失败：HTTP {status}（口令错误 / server 未起 / 非 dev 环境禁走此路）: {}",
                prefix_excerpt(&resp)
            )));
        }
        let token = serde_json::from_str::<serde_json::Value>(&resp)
            .ok()
            .and_then(|v| v.get("token").and_then(|t| t.as_str().map(str::to_string)))
            .ok_or_else(|| {
                Fail::env(format!("登录响应无 token 字段: {}", prefix_excerpt(&resp)))
            })?;
        self.token = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    fn token_cached(&mut self) -> Wn<()> {
        if let Some((_, at)) = &self.token
            && at.elapsed() < TOKEN_TTL
        {
            return Ok(());
        }
        self.login().map(|_| ())
    }

    /// GET /api/admin/sfu/stats 列表模式 → 类型化行。401 一次重登重试。
    pub fn fetch_streams(&mut self) -> Wn<Vec<StreamInfo>> {
        for attempt in 0..2 {
            self.token_cached()?;
            let (status, resp) = self.request("GET", "/api/admin/sfu/stats", None)?;
            match status {
                200 => return parse_streams_body(&resp),
                401 if attempt == 0 => {
                    eprintln!("weaknet(scope): WARN token 失效（401），重登一次重试");
                    self.token = None;
                }
                other => {
                    return Err(Fail::env(format!(
                        "GET stats 失败：HTTP {other}: {}",
                        prefix_excerpt(&resp)
                    )));
                }
            }
        }
        Err(Fail::env("GET stats 401：重登后仍拒（账号/角色变更？）"))
    }
}

fn prefix_excerpt(s: &str) -> String {
    let cut = s.chars().take(160).collect::<String>();
    cut.replace('\n', " ")
}

// ---------- capabilities（design §引擎 capability 表；ifb 探测 T9 到场） ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// iproute2 ≥ 6.6（seed 语义门，engine version_gate_seed 同判据）。
    pub seed: bool,
    /// lo 场景纯端口腿恒真（§dir 表注）。
    pub dir_lo: bool,
    /// 物理口 dir=in 的 ifb 镜像路径——T9 实现（先读后探链）。
    pub ifb_ingress: bool,
    pub ifb_reason: String,
}

/// tc -V 播种 seed 位；ifb 位保持单一真值源 = false +「T9 到场」（T9 替换本函数体）。
pub fn capabilities(env: &Env, iface: &str) -> Wn<Capabilities> {
    let chan = probe_channel(env, iface)?;
    let v = tc_exec(&chan, &["-V"])?;
    let seed = crate::engine::parse_iproute2_semver(&v)
        .is_some_and(|(maj, min)| (maj, min) >= (6, 6));
    Ok(Capabilities {
        seed,
        dir_lo: true,
        ifb_ingress: false,
        ifb_reason: "ifb ingress 随 T9 到场".into(),
    })
}
