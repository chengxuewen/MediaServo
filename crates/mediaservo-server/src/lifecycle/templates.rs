//! 生命周期模板与纯渲染函数（frontend-process-split T16 — 模板单一来源 M4）。
//!
//! **整合点（后续工作，编排者处理）**：`scripts/mediaservo_cli.py::_cmd_build_server`
//! 的 etc/ 组装段（现从 `config/server.docker.yaml` 派生 + 拷贝 `config/{accounts,devices}.yaml`）
//! 应改为委托 `mediaservo-server init <out/server>`（或共享本模块常量），消灭第二份模板漂移。
//! 在委托落地前，本模块与服务端运行时模板以注释锚定同一 schema。
//!
//! 本模块**零进程副作用**（不 spawn、不读 env 之外状态），全部纯函数可确定性单测。

use std::path::Path;

use sha2::{Digest, Sha256};
use uuid::Uuid;

use mediaservo_common::brand;

/// 品牌化产品名（USAGE/oxfile app 名同源）。默认 bin_prefix="mediaservo-" →
/// "mediaservo-server"；品牌 msrtc → "msrtc-server"（与 host 派生规则对称）。
pub fn server_product() -> String {
    format!("{}server", brand::media_brand().bin_prefix)
}

/// web（caddy）进程 app 名 — 同 bin_prefix 派生。
pub fn server_web_app() -> String {
    format!("{}web", brand::media_brand().bin_prefix)
}

/// weaknet-serve app 名——**有意品牌无关字面量**（weaknet-server-integration [mn10]：
/// inspect `ours[2]` / deploy keeper bin 白名单 / logs 目标三处稳定依赖此字面量，勿随品牌变）。
pub const WEAKNET_APP: &str = "weaknet-serve";

/// 端口缺省常量单源于 mediaservo-common::config（T6b②，DEFAULT_SFU_PORT 同模式）——
/// re-export 保持 templates:: 路径，mod.rs/inspect.rs 导入零改动。
/// weaknet DEFAULT_LISTEN 手抄同步注记见 common 侧常量注释。
pub use mediaservo_common::config::{DEFAULT_WEB_PORT, WEAKNET_SERVE_PORT};

/// 读序解析纯函数（config-unification P1）：yaml(显式配置) > env(兜底) > default(殿后)。
/// None = 未配置，由下一层生效。调用方负责读 env（本函数不碰进程环境——可测性）。
pub fn resolve_port(yaml: Option<u16>, env: Option<u16>, default: u16) -> u16 {
    yaml.or(env).unwrap_or(default)
}

/// weaknet serve 监听渲染字面量（恒 loopback——D5-2 不变式：IP 恒 127.0.0.1 不变，
/// 端口可配 = server.yaml `sfu.weaknet_port`，None 缺省 9810。语义见 render_oxfile 注释）。
pub fn weaknet_listen(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// 已渲染 oxfile 文本是否含 weaknet-serve 条目（start 探针 / --no-web 白名单共用的纯判据）。
pub fn oxfile_has_weaknet(text: &str) -> bool {
    text.contains(&format!("name = \"{WEAKNET_APP}\""))
}

/// oxfile 文本 → weaknet serve 监听口（`serve --listen 127.0.0.1:NNNN` 行；
/// 与 parse_web_port 同构纯函数——start 探针与渲染同源，漂移时以产物为准）。
pub fn parse_weaknet_port(oxfile: &str) -> Option<u16> {
    oxfile.lines().find_map(|l| {
        let t = l.trim();
        let rest = t.split("--listen").nth(1)?;
        let rest = rest.trim().strip_prefix("127.0.0.1:")?;
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
    })
}

/// oxfile [defaults].namespace（status/stop 过滤锚点）。
pub fn server_namespace() -> String {
    server_product()
}

/// 实例 daemon 端口派生基数 — host 用 18000..18399（API +1000 = 19000..19399），
/// server 用 18500 基数避开同机 host 实例（API 19500..19899）。派生规则与 host 同源
/// （路径字节和 % 400，跨进程稳定——C32 实例隔离）。
pub const DAEMON_PORT_BASE: u32 = 18500;

pub fn instance_daemon_port(home: &Path) -> u16 {
    let sum: u32 = home.to_string_lossy().bytes().map(u32::from).sum();
    DAEMON_PORT_BASE as u16 + (sum % 400) as u16
}

// ── 模板 ─────────────────────────────────────────────────────────────────────

/// server.yaml 实例模板（PIT-160：init 只在缺失时写入，永不覆盖）。
/// 凭据占位符 init 渲染一次；轮换走 admin API（C33/C35 热生效）。
pub const SERVER_YAML_TEMPLATE: &str = r#"version: 1
# MSRTC server 实例配置（mediaservo-server init 生成——已存在不覆盖，PIT-160）
# D-2=B：host-agent 直连 9800 /ws，保持 0.0.0.0；
#   迁「全收敛 A」（仅 Caddy 进入）前置 = admin API 配发 url + protocol_version 握手 → 改 "127.0.0.1"。
listen:
  host: "0.0.0.0"
  port: 9800
room_capacity: 50
rate_limit: 100
consumer_limit_per_stream: 50

# WS 单条消息字节上限（信令帧最大尺寸）。缺省 65536=64KB。重启生效
# （main.rs 建服时读入 SignalingServer::new，无 admin 热更路径）。
# ws_max_message_size: 65536

# ── 鉴权凭据（init 自举，文件 0600；本文件含密钥，勿入库）──
# PSK: config 优先 + env MEDIASERVO_PSK 兜底（C35）；轮换 POST /api/admin/psk 热生效（C33）。
psk: "__PSK__"
# jwt 与 admin_jwt 同值 = accounts token 验签配对（main.rs fail-fast 门）。
jwt_secret: "__JWT__"
admin_jwt_secret: "__JWT__"

# TLS（可选）——配置后 server 以 WSS/HTTPS 监听（main.rs:363 bind_rustls）。
# 缺省=整段不配=纯 HTTP/WS。生效：重启（启动时读，无热更）。子键均无 serde 缺省，tls 一旦启用
# 则两文件必配、缺失即启动报错：
#   cert_path  必配  PEM 证书链路径
#   key_path   必配  PEM 私钥路径
# tls:
#   cert_path: "/etc/certs/server.crt"
#   key_path: "/etc/certs/server.key"

# 注册表相对路径 = 相对本文件所在目录解析（main.rs resolve_path）。
# 管理面：/api/admin/devices* /api/admin/accounts* 热生效（C33），禁止手改+重启。
devices_file: "devices.yaml"
accounts_file: "accounts.yaml"

# API-key 免账号凭证（accountless-client-auth T1，POST /api/auth/exchange 换短 JWT 的源）。
# 缺省=本文件同目录 api_keys.yaml；文件缺失=空注册表（exchange 恒 401，账号/PSK/设备通路零影响）。
# 生效：路径重启（main.rs 建服加载）；表内容经 /api/admin API-key 端点热生效（C33 同纪律）。
# api_keys_file: "api_keys.yaml"
# exchange 签发 JWT 存活秒数。缺省 43200=12h（对齐账号登录 TTL；吊销粒度=已发 JWT 至 exp 自然死 F10）。
# 生效：重启（main.rs 读入 state.api_token_ttl_secs）。
# api_token_ttl_secs: 43200

# SFU 对外公告地址（mediasoup 0.0.0.0 必须配 announced——PIT-44/58/138；C38 排查层①）。
# 不配 = 自动探测出网 IP（裸机可整段省略本 sfu 段）；容器内探测不可达 = 必配（PIT-44/58/138）。
# 优先级: env MEDIASERVO_SFU_ANNOUNCED_IP（逗号分隔多 IP）> 此处 > 自动探测。
# sfu:
#   announced_ips:
#     - "192.168.0.10"

# Web 静态层（caddy）——缺省 8080 / 实例 web/；env MEDIASERVO_WEB_PORT 兜底（yaml 配置优先，C35 同形）。
# 生效：渲染期一次性——改 port 后需删 etc/Caddyfile 重 init（Caddyfile 是渲染产物，不随 yaml 自动重生成）。
# web:
#   port: 8080              # 缺省 8080（env MEDIASERVO_WEB_PORT 兜底）
#   static_root: "/abs/web"

# SFU/weaknet 端口 —— 缺省 20000 / 9810（env MEDIASERVO_SFU_PORT 兜底 sfu.port）。
# sfu:
#   port: 20000
#   weaknet_port: 9811
"#;

/// devices.yaml 空注册表模板（prod-safe——无占位设备；注册走 admin API C33）。
pub const DEVICES_TEMPLATE: &str = r#"# MSRTC G2 设备注册表（mediaservo-server init 生成——已存在不覆盖，PIT-160）
# 注册/吊销/重置：Admin Dashboard /devices 或 POST /api/admin/devices*（热生效，C33）。
# 手工格式：devices: {<device_id>: {secret_hash: "sha256:<64hex>"}}，
#   secret_hash = sha256("<device_id>:<device_secret>")。
devices: {}
"#;

/// accounts.yaml 空账号表模板（prod-safe——禁 dev 占位，对齐 entrypoint ③ 语义）。
pub const ACCOUNTS_TEMPLATE: &str = r#"# MSRTC G3 舱端账号注册表（mediaservo-server init 生成——已存在不覆盖，PIT-160）
# 建号/角色：Admin Dashboard /accounts 或 POST /api/admin/accounts*（热生效，C33）。
# 首启登录：server 日志的 setup-token 行（admin_jwt_secret 派生）。
# 手工格式：accounts: {<user>: {password_hash: "sha256:<64hex>", role: <admin|dispatcher|operator|viewer>, vehicles: []}}，
#   password_hash = sha256("<user>:<password>")。
accounts: {}
"#;

/// web 层 Caddyfile 模板（与 deploy/caddy/Caddyfile.native 同源，T4 产出）。
/// 占位符 init 渲染为具体值（oxmgr 拉起的 caddy 不继承 CLI env，必须去 env 占位）。
pub const CADDYFILE_TEMPLATE: &str = r#"# MSRTC server 实例 web 层（mediaservo-server init 生成——已存在不覆盖，PIT-160）
# 静态 __WEB_ROOT__ + API/WS/探针反代 __BACKEND__（D-2=B：本代理只服务浏览器，host 直连 9800 不变）
# flush_interval -1 = WS 长连不被掐（T11 门）；SPA fallback + 缓存头（T4）
__WEB_ADDR__ {
	@api path /api/* /ws /health /ready /stats /metrics
	handle @api {
		reverse_proxy __BACKEND__ {
			flush_interval -1
		}
	}
	# weaknet 面板反代（D3.1 实测定稿形，Caddy 2.9.1——`uri replace_prefix` 不存在故 handle_path）。
	# 上游 = __WEAKNET_BACKEND__ 渲染字面量（生成形不吃 env [w批2 F3②]：oxmgr 拉的 caddy 不继承
	# CLI env，{$VAR} 占位会被当主机名字面量）。header_up Host 重写 = serve Host 白名单放行
	# （sF1 BLOCKER 修；rebinding 防御职责转移 token 门）。裸 /weaknet 经 @wnroot redir 保 query [wW4]。
	@wnroot path /weaknet
	redir @wnroot "/weaknet/?{query}" 308
	handle_path /weaknet/* {
		header Cache-Control "no-store"
		reverse_proxy __WEAKNET_BACKEND__ {
			header_up Host {upstream_hostport}
			flush_interval -1
		}
	}
	handle {
		root * __WEB_ROOT__
		try_files {path} /index.html
		encode zstd gzip
		file_server
	}
	header {
		X-Content-Type-Options "nosniff"
		X-Frame-Options "DENY"
		Referrer-Policy "strict-origin-when-cross-origin"
	}
	@page path / /index.html
	header @page Cache-Control "no-cache"
	@assets path /assets/*
	header @assets Cache-Control "public, max-age=31536000, immutable"
}
"#;

/// oxfile 静态模板（2~3 进程——web/weaknet 可选，免翻译层，design.md「修正版」）。
/// 无 watch：server.yaml 变更 = 人工 restart（热生效项走 admin API C33——psk 轮换会重写
/// server.yaml，watch 会触发重启风暴，此处刻意不加）。
pub fn render_oxfile(
    dir_abs: &Path,
    server_bin: &Path,
    caddy_cmd: &str,
    server_env: &[(String, String)],
    weaknet_port: u16,
) -> String {
    let dir = dir_abs.to_string_lossy();
    let cfg = format!("{dir}/etc/server.yaml");
    let log_dir = format!("{dir}/run/logs");
    let server_app = server_product();
    let web_app = server_web_app();
    let mut out = format!(
        "# mediaservo-server init 生成——静态模板（2~3 条目，weaknet 条件渲染；变更拓扑=改本文件+restart）\n\
         # 注意: 本文件 init 后不自动重渲染（手工 env 编辑保留），删掉后 start 会重新生成\n\
         version = 1\n\n[defaults]\nnamespace = \"{}\"\nrestart_policy = \"always\"\ncwd = \"{dir}\"\n",
        server_namespace()
    );
    out.push_str(&format!(
        "\n[[apps]]\nname = \"{server_app}\"\ncommand = \"{} run --config {cfg}\"\nrestart_policy = \"always\"\n",
        server_bin.to_string_lossy()
    ));
    if !server_env.is_empty() {
        out.push_str("[apps.env]\n");
        for (k, v) in server_env {
            out.push_str(&format!("{k} = \"{v}\"\n"));
        }
    }
    push_logs(&mut out, &server_app, &log_dir);
    out.push_str(&format!(
        "\n[[apps]]\nname = \"{web_app}\"\ncommand = \"{caddy_cmd} run --config {dir}/etc/Caddyfile --adapter caddyfile\"\nrestart_policy = \"always\"\n"
    ));
    push_logs(&mut out, &web_app, &log_dir);
    // weaknet-serve 第三条目（D2）——**条件渲染职责钉死于此 [M-1]**：在场判定单点=扫
    // {dir}/bin（fresh-init 与 .bak 重渲两路共用本函数；Python 只拷贝+WARN 无渲染杠杆）。
    // 缺位不写条目——杜绝 restart_policy=always 对缺文件的空转风暴 [bF1]。
    // 不变式两行（[sF3/D5-2]，unit 与代码注释同款）：
    //   ① 恒 loopback —— IP 恒 127.0.0.1 不变（端口可配 = server.yaml `sfu.weaknet_port`，
    //      config-unification P1；缺省 9810）；
    //   ② 恒禁 --lan —— Origin 门按 listen 地址比对浏览器 Origin，反代下必失配
    //      （curl 通/浏览器死的排障黑洞）；对外唯一路径 = Caddy 同源反代 + Host 重写。
    if let Some(wnet) = discover_weaknet_bin(&dir_abs.join("bin")) {
        let wnet_path = wnet.to_string_lossy();
        out.push_str(&format!(
            "\n[[apps]]\nname = \"{WEAKNET_APP}\"\ncommand = \"{wnet_path} serve --listen {} --token-file {dir}/run/weaknet.token\"\nrestart_policy = \"always\"\n",
            weaknet_listen(weaknet_port)
        ));
        // 单 state 合同 [bF8/lck-F3]：oxmgr unit 不继承 CLI env——WEAKNET_STATEDIR（weaknet
        // state.rs 解析链首位 env 真名，非 WEAKNET_HOME [BA-5 勘误]）烘 {dir}/run/weaknet，
        // 簇与 CLI 同 state 面（面板看得见 CLI 施压、clear 打得着、撤网撤全）。token-file/
        // statedir 绝对形 [mn6]（OxMgr 相对路径坑——push_logs 同训）；重渲模板值优先、
        // carried_env 只补缺键 [lck-F14]。
        out.push_str(&format!("[apps.env]\n{WEAKNET_STATEDIR_KEY} = \"{dir}/run/weaknet\"\n"));
        push_logs(&mut out, WEAKNET_APP, &log_dir);
    }
    out
}

/// SFU env 透传键集——`sfu_env_passthrough()` 恰按本清单读 env 渲染（drill/多实例端口隔离）。
pub const SFU_ENV_KEYS: [&str; 2] = ["MEDIASERVO_SFU_PORT", "MEDIASERVO_SFU_ANNOUNCED_IP"];

/// `WEAKNET_STATEDIR` 键名——render_oxfile 烘值行与本集共用此字面量（结构级防漂移）。
pub const WEAKNET_STATEDIR_KEY: &str = "WEAKNET_STATEDIR";

/// 渲染器受管键全集 = SFU 透传两键 + weaknet statedir 烘值——**merge 排除集**：本集键的
/// 旧值 = 上次渲染输出而非用户手改，永不从旧/bak 文件复活（baked-wins；与 Python
/// [lck-F14]「模板烘值优先、carried 只补缺」同构——T4 尾刀裁决），且不计入「已合并 N 条」。
pub const MANAGED_ENV_KEYS: [&str; SFU_ENV_KEYS.len() + 1] =
    [SFU_ENV_KEYS[0], SFU_ENV_KEYS[1], WEAKNET_STATEDIR_KEY];

/// 文本是否 render_oxfile 形（`[[apps]]` 条目标记）？否 = 损坏/外来旧文件——调用方降级纯渲染 + WARN。
pub fn looks_like_oxfile(text: &str) -> bool {
    text.contains("[[apps]]")
}

/// 一条 `[apps.env]` 条目——按 app 归属所属 `[[apps]]` 条目（放错=死变量：
/// WEAKNET_ADMIN_PASS 只在 weaknet 条目下生效、dev 三行在 server 条目下——Python
/// `_reapply_carried_env` 同款 per-app 合同）。
#[derive(PartialEq, Eq)]
struct EnvEntry {
    app: String,
    key: String,
    value: String,
}

/// 同格式约定行形解析（= render_oxfile 输出形）：`[apps.env]` 块表内的
/// `KEY = "VALUE"` 行，下一 `[` 起始行终止表；`[[apps]]` 头后首个 `name = "..."`
/// 行认条目名。非此形行（数字/内联表/坏行）跳过——容错，永不报错。
fn parse_env_entries(text: &str) -> Vec<EnvEntry> {
    let mut out = Vec::new();
    let mut app: Option<String> = None;
    let mut in_env = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "[[apps]]" {
            app = None;
            in_env = false;
            continue;
        }
        if t.starts_with('[') {
            in_env = t == "[apps.env]";
            continue;
        }
        if in_env {
            if let Some((key, value)) = parse_quoted_kv(t)
                && let Some(a) = &app
            {
                out.push(EnvEntry { app: a.clone(), key, value });
            }
        } else if app.is_none()
            && let Some((k, v)) = parse_quoted_kv(t)
            && k == "name"
        {
            app = Some(v);
        }
    }
    out
}

/// `KEY = "quoted value"` → (key, 解转义 value)；非双引号基字符串形 → None。
fn parse_quoted_kv(t: &str) -> Option<(String, String)> {
    let (k, v) = t.split_once('=')?;
    let inner = v.trim().strip_prefix('"')?.strip_suffix('"')?;
    Some((k.trim().to_string(), unescape_toml(inner)))
}

/// TOML 基本串最小转义：仅 `\` 与 `"`（Task 4 步 5 裁决——不发明全 Unicode 转义族）。
fn escape_toml(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            _ => o.push(c),
        }
    }
    o
}

/// escape_toml 之逆：`\\`/`\"` → 字面字符；其余反斜杠序列按两字符原样保留。
/// （render 地面形只产这两类；手写文件若用 `\\n` 形转义不承诺解释为换行——再转义时多叠一层，
/// 值语义不变、文件不坏。）
fn unescape_toml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some(n @ ('\\' | '"')) => o.push(n),
                Some(n) => {
                    o.push('\n');
                    o.push(n);
                }
                None => o.push('\n'),
            },
            _ => o.push(c),
        }
    }
    o
}

/// 参与合并的手工 env (app, key) 清单：MANAGED_ENV_KEYS（渲染器受管全集）排除、(app,key) 首现去重。
/// write_oxfile 用旧/新两侧本清单做头差 = 已合并 / 未并（新渲染无对应条目）计数。
pub fn manual_env_keys(text: &str) -> Vec<(String, String)> {
    let mut seen: Vec<(String, String)> = Vec::new();
    for e in parse_env_entries(text) {
        if MANAGED_ENV_KEYS.contains(&e.key.as_str()) {
            continue;
        }
        if !seen.iter().any(|(a, k)| a == &e.app && k == &e.key) {
            seen.push((e.app.clone(), e.key.clone()));
        }
    }
    seen
}

/// PIT-160 根修核心纯函数：旧（live 或 .bak）oxfile 的手工 [apps.env] 键并入新渲染文本。
/// 规则（PLAN §2.4 + 尾刀裁决）：渲染器受管键（MANAGED_ENV_KEYS）不参与——旧值=上次渲染
/// 遗留，复活压死新配置（baked-wins）；其余键冲突 **旧值优先**（用户手改=用户意图）；缺键追加所属 app 条目 env 表尾
/// （无表则在 [apps.logs] 前新建——与 render 同槽位）；旧条目在新渲染缺位 → 不并
/// （调用方提示行报「另有 N 条」）。新渲染其余行逐字保留。
pub fn merge_manual_env(old_text: &str, new_text: &str) -> String {
    let entries: Vec<EnvEntry> = parse_env_entries(old_text)
        .into_iter()
        .filter(|e| !MANAGED_ENV_KEYS.contains(&e.key.as_str()))
        .collect();
    if entries.is_empty() {
        return new_text.to_string();
    }
    let mut lines: Vec<String> = new_text.lines().map(str::to_string).collect();
    let mut done: Vec<(String, String)> = Vec::new();
    for e in entries {
        if done.iter().any(|(a, k)| a == &e.app && k == &e.key) {
            continue; // 同条目重复键首现即得（合法 TOML 不产生，防手工损形双应用）
        }
        let Some((start, end)) = app_block(&lines, &e.app) else {
            continue; // 新渲染无此 app 条目（品牌改名形态）→ 不并，计 lost
        };
        done.push((e.app.clone(), e.key.clone()));
        let line = format!("{} = \"{}\"", e.key, escape_toml(&e.value));
        match env_table(&lines, start, end) {
            Some((header, tail)) => {
                match (header + 1..tail)
                    .find(|&i| line_key(&lines[i]).as_deref() == Some(e.key.as_str()))
                {
                    Some(idx) => lines[idx] = line,
                    None => lines.insert(tail, line),
                }
            }
            None => {
                let anchor =
                    (start..end).find(|&i| lines[i].trim() == "[apps.logs]").unwrap_or(end);
                lines.insert(anchor, format!("[apps.env]\n{line}"));
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// 指定条目名的 [[apps]] 块行范围 [start,end)：判据=块内含 `name = "<app>"` 行（render 地面形精确）。
fn app_block(lines: &[String], app: &str) -> Option<(usize, usize)> {
    let want = format!("name = \"{app}\"");
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() != "[[apps]]" {
            i += 1;
            continue;
        }
        let start = i;
        let end = (start + 1..lines.len())
            .find(|&j| lines[j].trim() == "[[apps]]")
            .unwrap_or(lines.len());
        if lines[start..end].iter().any(|l| l.trim() == want) {
            return Some((start, end));
        }
        i = end;
    }
    None
}

/// 条目内 [apps.env] 表：(头索引, 表尾索引)——表尾=头后首个 `[` 起始行，无则条目尾。
fn env_table(lines: &[String], start: usize, end: usize) -> Option<(usize, usize)> {
    let h = (start..end).find(|&i| lines[i].trim() == "[apps.env]")?;
    let t = (h + 1..end).find(|&i| lines[i].trim().starts_with('[')).unwrap_or(end);
    Some((h, t))
}

/// `KEY = ...` 行的键部分（merge 行定位；无 `=` → None）。
fn line_key(line: &str) -> Option<String> {
    line.split_once('=').map(|(k, _)| k.trim().to_string())
}

/// weaknet bin 在场扫描（条件渲染输入）：品牌派生名优先（= Python `_weaknet_bin_name` 同构：
/// `{brand}-weaknet`，brand 读同一 MEDIASERVO_BRAND env/编译期值——deploy 装配产物名），
/// 上游名 mediaservo-weaknet 兜底（keeper 双名 [BA-2①]；brand 空 = 两名同串）。
/// 不 glob 任意 `*-weaknet`——旧品牌残件由 deploy 白名单清理，指过去=死路径。
/// `-x` 判定（is_executable 与 lifecycle::which 同源），防非可执行残档假在场。
fn discover_weaknet_bin(dir_bin: &Path) -> Option<std::path::PathBuf> {
    let exe = std::env::consts::EXE_SUFFIX;
    let branded = format!("{}weaknet{exe}", brand::media_brand().bin_prefix);
    let upstream = format!("mediaservo-weaknet{exe}");
    [branded.as_str(), upstream.as_str()]
        .into_iter()
        .map(|n| dir_bin.join(n))
        .find(|p| p.is_file() && super::is_executable(p))
}

/// 日志绝对路径（实例 run/logs——OxMgr 按 daemon cwd 解析相对路径的坑，host translate 同训）。
fn push_logs(out: &mut String, name: &str, log_dir: &str) {
    out.push_str(&format!(
        "[apps.logs]\nstdout = \"{log_dir}/{name}.out.log\"\nstderr = \"{log_dir}/{name}.err.log\"\n"
    ));
}

pub fn render_server_yaml(psk: &str, jwt: &str) -> String {
    SERVER_YAML_TEMPLATE.replace("__PSK__", psk).replace("__JWT__", jwt)
}

/// 渲染 web Caddyfile。`weaknet_backend` = 面板上游 host:port 字面量（init 传
/// `weaknet_listen(port)`（config-unification P1 端口可配）；生成形不吃 env [w批2 F3②]）。
pub fn render_caddyfile(
    web_port: u16,
    backend_port: u16,
    web_root: &Path,
    weaknet_backend: &str,
) -> String {
    CADDYFILE_TEMPLATE
        .replace("__WEB_ADDR__", &format!(":{web_port}"))
        .replace("__BACKEND__", &format!("127.0.0.1:{backend_port}"))
        .replace("__WEAKNET_BACKEND__", weaknet_backend)
        .replace("__WEB_ROOT__", &web_root.to_string_lossy())
}

/// 凭据生成（compose entrypoint `openssl rand -hex 32` 的原生等价——uuid v4 双拼 = 64 hex）。
pub fn gen_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// admin 引导账号行（entrypoint ⑤ 语义归位：sha256("<user>:<password>")）。
pub fn render_admin_account(user: &str, password: &str) -> String {
    let mut h = Sha256::new();
    h.update(format!("{user}:{password}").as_bytes());
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("  {user}:\n    password_hash: \"sha256:{hex}\"\n    role: admin\n")
}

// ── 解析（status/doctor 消费；纯函数）───────────────────────────────────────

/// server.yaml 文本 → listen.port（解析失败 None——调用方决定回退/报错）。
pub fn parse_listen_port(yaml: &str) -> Option<u16> {
    serde_yaml::from_str::<mediaservo_common::config::ServerConfig>(yaml)
        .ok()
        .map(|c| c.listen.port)
}

/// Caddyfile 文本 → 站点端口（首个 `:NNNN` 行；渲染/手工编辑双兼容）。
pub fn parse_web_port(caddyfile: &str) -> Option<u16> {
    caddyfile.lines().find_map(|l| {
        let t = l.trim();
        let rest = t.strip_prefix(':')?;
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        (!digits.is_empty() && digits.len() <= rest.len()).then(|| digits.parse().ok()).flatten()
    })
}

/// Caddyfile 文本 → 静态 root 路径（`root * <path>` 行——doctor dist 检查用）。
pub fn parse_web_root(caddyfile: &str) -> Option<String> {
    caddyfile.lines().find_map(|l| {
        let t = l.trim();
        t.strip_prefix("root *").map(|r| r.trim().to_string())
    })
}

/// 解析后的配置 → env 优先合并（与 main.rs 兜底序一致：env > yaml，C35 注释锚定）。
pub fn effective_announced(present_in_yaml: bool) -> bool {
    present_in_yaml || std::env::var("MEDIASERVO_SFU_ANNOUNCED_IP").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oxfile_render_deterministic_and_complete() {
        let a = render_oxfile(
            Path::new("/opt/ms"),
            Path::new("/opt/ms/bin/mediaservo-server"),
            "/usr/bin/caddy",
            &[("MEDIASERVO_SFU_PORT".to_string(), "20012".to_string())],
            WEAKNET_SERVE_PORT,
        );
        let b = render_oxfile(
            Path::new("/opt/ms"),
            Path::new("/opt/ms/bin/mediaservo-server"),
            "/usr/bin/caddy",
            &[("MEDIASERVO_SFU_PORT".to_string(), "20012".to_string())],
            WEAKNET_SERVE_PORT,
        );
        assert_eq!(a, b, "同输入必须逐字节一致");
        assert!(a.contains("namespace = \"mediaservo-server\""));
        assert!(a.contains("run --config /opt/ms/etc/server.yaml"));
        assert!(a.contains("caddy run --config /opt/ms/etc/Caddyfile --adapter caddyfile"));
        assert!(a.contains("MEDIASERVO_SFU_PORT = \"20012\""));
        assert!(a.contains("/opt/ms/run/logs/mediaservo-server.out.log"));
        // 不同目录 → 不同内容
        let c = render_oxfile(
            Path::new("/other"),
            Path::new("/opt/ms/bin/mediaservo-server"),
            "caddy",
            &[],
            WEAKNET_SERVE_PORT,
        );
        assert_ne!(a, c);
    }

    #[test]
    fn parses_listen_port_from_template() {
        let yaml = render_server_yaml("p", "j");
        assert_eq!(parse_listen_port(&yaml), Some(9800));
        let edited = yaml.replace("port: 9800", "port: 9802");
        assert_eq!(parse_listen_port(&edited), Some(9802));
        assert_eq!(parse_listen_port("listen: broken:["), None);
    }

    #[test]
    fn parses_web_port_and_root() {
        let cf = render_caddyfile(8089, 9802, Path::new("/opt/ms/web"), "127.0.0.1:9810");
        assert_eq!(parse_web_port(&cf), Some(8089));
        assert_eq!(parse_web_root(&cf).as_deref(), Some("/opt/ms/web"));
        assert_eq!(parse_web_port("no site here"), None);
    }

    #[test]
    fn secrets_and_admin_account_render() {
        let s = gen_secret();
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(s, gen_secret());
        let line = render_admin_account("admin", "pw");
        assert!(line.contains("role: admin"));
        // sha256("admin:pw") 已知值（echo -n admin:pw | sha256sum）
        assert!(line.contains("8f1e1766352d1c014616beb9dab85f1facd44b87affa68275744274967bf9fec"));
    }

    #[test]
    fn daemon_port_derivation_in_server_range() {
        for i in 0..50u32 {
            let p = instance_daemon_port(Path::new(&format!("/tmp/x{i}/run/oxmgr")));
            assert!((18500..18900).contains(&p), "避开 host 18000-18399: {p}");
        }
    }

    #[cfg(unix)]
    fn touch_exec_bin(dir: &Path, name: &str) {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir bin");
        let p = bin.join(name);
        std::fs::write(&p, b"#!/bin/sh\n").expect("write bin");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    #[cfg(unix)]
    #[test]
    fn oxfile_weaknet_conditional_render_present_absent() {
        let srv = Path::new("/x/bin/mediaservo-server");
        // 缺位形：无 bin/ → 不写条目（防 restart_policy=always 空转风暴 [bF1]）
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let absent = render_oxfile(dir, srv, "caddy", &[], WEAKNET_SERVE_PORT);
        assert!(!absent.contains(WEAKNET_APP), "bin 缺位必须无条件渲染为无条目");
        assert!(!oxfile_has_weaknet(&absent));
        // 在场形：上游名（默认品牌 mediaservo → branded==upstream 同串）
        touch_exec_bin(dir, "mediaservo-weaknet");
        let yes = render_oxfile(dir, srv, "caddy", &[], WEAKNET_SERVE_PORT);
        let d = dir.to_string_lossy().to_string();
        assert!(oxfile_has_weaknet(&yes), "在场判定单点=render_oxfile [M-1]");
        assert!(
            yes.contains(&format!(
                "command = \"{d}/bin/mediaservo-weaknet serve --listen {} --token-file {d}/run/weaknet.token\"",
                weaknet_listen(WEAKNET_SERVE_PORT)
            )),
            "全绝对形 [mn6] + token-file"
        );
        assert!(yes.contains(&format!("[apps.env]\nWEAKNET_STATEDIR = \"{d}/run/weaknet\"")));
        assert!(yes.contains(&format!("{d}/run/logs/{WEAKNET_APP}.out.log")));
        assert!(!yes.contains("--lan"), "恒禁 --lan 硬门 [sF3]");
        assert!(!yes.contains("{$"), "生成形不吃 env 占位");
        // ours[2]：weaknet 必须排第三（server→web→weaknet——inspect ours 索引稳定 [mn10]）
        let (i_srv, i_web, i_wn) = (
            yes.find("name = \"mediaservo-server\"").unwrap(),
            yes.find("name = \"mediaservo-web\"").unwrap(),
            yes.find(&format!("name = \"{WEAKNET_APP}\"")).unwrap(),
        );
        assert!(i_srv < i_web && i_web < i_wn, "条目序 = ours 序");
        // restart_policy 同 server 条目
        let blk = &yes[i_wn..];
        assert!(blk.contains("restart_policy = \"always\""));
    }

    #[cfg(unix)]
    #[test]
    fn oxfile_weaknet_non_exec_not_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        std::fs::write(bin.join("mediaservo-weaknet"), b"x").expect("write");
        // 0644 非可执行 → 不算在场
        let out = render_oxfile(
            dir,
            Path::new("/x/bin/mediaservo-server"),
            "caddy",
            &[],
            WEAKNET_SERVE_PORT,
        );
        assert!(!out.contains(WEAKNET_APP), "-x 判定：非可执行残档不得触发条目");
    }

    #[test]
    fn caddyfile_weaknet_segment_rendered_literal() {
        let cf = render_caddyfile(
            8080,
            9800,
            Path::new("/opt/ms/web"),
            &weaknet_listen(WEAKNET_SERVE_PORT),
        );
        assert!(cf.contains("@wnroot path /weaknet"));
        assert!(cf.contains("redir @wnroot \"/weaknet/?{query}\" 308"));
        assert!(cf.contains("handle_path /weaknet/*"));
        assert!(cf.contains("header_up Host {upstream_hostport}"));
        assert!(cf.contains("reverse_proxy 127.0.0.1:9810"), "渲染字面量（缺省口）");
        // 生成形不吃 env 占位 [w批2 F3②]（模板注释含 dollar-brace 字样是说明文，非指令）
        assert!(!cf.contains(r"{$MSRTC"), "生成的反代上游必须是渲染字面量");
        assert_eq!(weaknet_listen(WEAKNET_SERVE_PORT), "127.0.0.1:9810");
        // 段序：@api 之后、静态 handle 之前
        let i_api = cf.find("handle @api").unwrap();
        let i_wn = cf.find("handle_path /weaknet").unwrap();
        let i_static = cf.find("root * /opt/ms/web").unwrap();
        assert!(i_api < i_wn && i_wn < i_static, "段序 @api → @weaknet → 静态");
    }

    #[test]
    fn templates_parse_as_yaml() {
        serde_yaml::from_str::<serde_yaml::Value>(DEVICES_TEMPLATE).expect("devices yaml");
        serde_yaml::from_str::<serde_yaml::Value>(ACCOUNTS_TEMPLATE).expect("accounts yaml");
    }

    // ---- config-unification P1: resolve_port / weaknet_listen / parse_weaknet_port ----

    #[test]
    fn resolve_port_priority() {
        // yaml > env > default
        assert_eq!(resolve_port(Some(9090), Some(7070), 8080), 9090);
        assert_eq!(resolve_port(None, Some(7070), 8080), 7070);
        assert_eq!(resolve_port(None, None, 8080), 8080);
    }

    #[test]
    fn weaknet_listen_on_matches_constant() {
        assert_eq!(weaknet_listen(WEAKNET_SERVE_PORT), "127.0.0.1:9810");
        assert_eq!(weaknet_listen(9811), "127.0.0.1:9811");
    }

    #[test]
    fn parse_weaknet_port_from_oxfile() {
        // render → parse 闭环（自定义端口贯通）
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        touch_exec_bin(dir, "mediaservo-weaknet");
        let out = render_oxfile(dir, Path::new("/x/bin/mediaservo-server"), "caddy", &[], 9811);
        assert_eq!(parse_weaknet_port(&out), Some(9811));
        // 缺省口
        let out = render_oxfile(
            dir,
            Path::new("/x/bin/mediaservo-server"),
            "caddy",
            &[],
            WEAKNET_SERVE_PORT,
        );
        assert_eq!(parse_weaknet_port(&out), Some(9810));
        // 无 weaknet 条目 → None
        let tmp2 = tempfile::tempdir().expect("tempdir");
        let none = render_oxfile(tmp2.path(), Path::new("/x/bin/s"), "caddy", &[], 9810);
        assert_eq!(parse_weaknet_port(&none), None);
        assert_eq!(parse_weaknet_port("garbage"), None);
    }

    // ---- T4（PIT-160 根修）：merge_manual_env 纯函数组 ----

    /// 真 render_oxfile 地面形（tempdir 无 bin/ → weaknet 条件渲染缺席路，确定性）。
    fn t4_render(dir: &std::path::Path, server_env: &[(&str, &str)]) -> String {
        render_oxfile(
            dir,
            std::path::Path::new("/x/bin/s"),
            "caddy",
            &server_env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>(),
            WEAKNET_SERVE_PORT,
        )
    }

    /// server 条目 logs 表头两行——手工 env 表注入锚（render 的 [apps.env] 就位于 [apps.logs] 前）。
    fn t4_logs_anchor(dir: &std::path::Path) -> String {
        format!(
            "[apps.logs]\nstdout = \"{}/run/logs/{}.out.log\"",
            dir.to_string_lossy(),
            server_product()
        )
    }

    #[test]
    fn t4a_manual_key_survives_system_key_does_not_resurrect() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let new = t4_render(dir, &[]);
        let anchor = t4_logs_anchor(dir);
        assert!(new.contains(&anchor), "fixture 锚缺失（render 输出形已漂移）");
        let old = new.replace(
            &anchor,
            &format!("[apps.env]\nALLOW_X = \"1\"\nMEDIASERVO_SFU_PORT = \"20012\"\n{anchor}"),
        );
        let merged = merge_manual_env(&old, &new);
        assert!(merged.contains("ALLOW_X = \"1\""), "手工 env 键被重渲染丢失:\n{merged}");
        assert!(!merged.contains("MEDIASERVO_SFU_PORT"), "系统键陈旧值复活压死了新渲染");
    }

    #[test]
    fn t4b_conflict_non_system_key_old_value_wins() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let new = t4_render(dir, &[("RUST_LOG", "system=new")]);
        assert!(
            new.contains("RUST_LOG = \"system=new\""),
            "fixture: server_env 应渲染 RUST_LOG 行"
        );
        let old = new.replace("RUST_LOG = \"system=new\"", "RUST_LOG = \"user=old\"");
        let merged = merge_manual_env(&old, &new);
        assert!(merged.contains("RUST_LOG = \"user=old\""), "冲突应旧手工值优先:\n{merged}");
        assert!(!merged.contains("system=new"), "旧值压制后不得残留新渲染值");
    }

    #[test]
    fn t4d_foreign_old_degrades_to_pure_render() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let new = t4_render(dir, &[]);
        assert!(!looks_like_oxfile("!!! foreign @@@ not an oxfile"), "外来文本必须判不可识别");
        assert!(looks_like_oxfile(&new), "render 本形必须可识别");
        let merged = merge_manual_env("!!! foreign @@@ not an oxfile", &new);
        assert_eq!(merged, new, "损坏/外来旧文件 → 纯渲染兜底，无 panic 无 error");
    }

    #[test]
    fn t4e_toml_escape_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let new = t4_render(dir, &[]);
        let anchor = t4_logs_anchor(dir);
        // 旧文本行形：RUST_LOG = "a\"b\\c"（值 = a"b\c）
        let old =
            new.replace(&anchor, &format!("[apps.env]\nRUST_LOG = \"a\\\"b\\\\c\"\n{anchor}"));
        assert!(old.contains(r#"RUST_LOG = "a\"b\\c""#), "fixture 注入失败");
        let merged = merge_manual_env(&old, &new);
        assert!(
            merged.contains(r#"RUST_LOG = "a\"b\\c""#),
            "转义往返变形（解转义/再转义破坏值）:\n{merged}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn t4f_baked_statedir_wins_and_never_counted() {
        // 尾刀裁决（baked-wins）：WEAKNET_STATEDIR=render_oxfile 每轮烘值，旧/bak 形
        // 即便在场也不得复活旧值，且不计入「已合并 N 条手工 env」。
        let tmp = tempfile::tempdir().expect("tempdir");
        touch_exec_bin(tmp.path(), "mediaservo-weaknet");
        let new = t4_render(tmp.path(), &[]);
        let baked =
            format!("{WEAKNET_STATEDIR_KEY} = \"{}/run/weaknet\"", tmp.path().to_string_lossy());
        assert!(new.contains(&baked), "fixture: weaknet 条目烘值行在场");
        let old = new
            .replace(&baked, &format!("WEAKNET_STATEDIR = \"/old/tree/run/weaknet\""))
            .replacen("[apps.logs]", "[apps.env]\nALLOW_X = \"1\"\n[apps.logs]", 1);
        assert!(old.contains("/old/tree") && old.contains("ALLOW_X"), "fixture 注入失败");
        let merged = merge_manual_env(&old, &new);
        assert!(merged.contains(&baked), "烘值优先，旧 statedir 不得覆盖新烘值:\n{merged}");
        assert!(!merged.contains("/old/tree/run/weaknet"), "旧 statedir 值不复活");
        assert!(merged.contains("ALLOW_X = \"1\""), "真手工键照常合并");
        let keys = manual_env_keys(&old);
        assert_eq!(keys.len(), 1, "N 统计不收烘值键: {keys:?}");
    }

    #[test]
    fn sfu_defaults_match_templates() {
        // 常量与 serde 缺省互证（防漂移）
        assert_eq!(WEAKNET_SERVE_PORT, 9810);
        assert_eq!(DEFAULT_WEB_PORT, 8080);
        use mediaservo_common::config::{ServerConfig, SfuConfig, WebConfig};
        let cfg: ServerConfig = serde_yaml::from_str("listen: {}").expect("parse");
        assert_eq!(cfg.sfu, SfuConfig::default());
        assert_eq!(cfg.sfu.port, None);
        assert_eq!(cfg.sfu.weaknet_port, None);
        assert_eq!(cfg.web, WebConfig::default());
    }

    #[test]
    fn new_template_roundtrip() {
        // 模板（含新注释块）解析 ✓ 且新键全 None + 往返全等
        use mediaservo_common::config::ServerConfig;
        let yaml = render_server_yaml("p", "j");
        let cfg: ServerConfig = serde_yaml::from_str(&yaml).expect("template must parse");
        assert_eq!(cfg.web.port, None);
        assert_eq!(cfg.web.static_root, None);
        assert_eq!(cfg.sfu.port, None);
        assert_eq!(cfg.sfu.weaknet_port, None);
        // T6a 注释补全四字段解析后取 serde 缺省（注释示例不得被激活）
        assert!(cfg.tls.is_none(), "tls 注释示例不应被激活");
        assert!(cfg.api_keys_file.is_none());
        assert_eq!(cfg.ws_max_message_size, 65536);
        assert_eq!(cfg.api_token_ttl_secs, 43200);
        let re: ServerConfig =
            serde_yaml::from_str(&serde_yaml::to_string(&cfg).unwrap()).expect("roundtrip");
        assert_eq!(re.web, cfg.web);
        assert_eq!(re.sfu, cfg.sfu);
        // 模板含注释形新键示例（R5 防漂移锚点）
        assert!(yaml.contains("web:"));
        assert!(yaml.contains("weaknet_port:"));
    }

    /// ServerConfig 全部 pub 字段名——与 mediaservo-common/src/config.rs `ServerConfig`
    /// 现场逐一对表（10-12 清点 = 16 个，见数组）。**新增 pub 字段必须同笔入本数组 + 模板
    /// 注释**，漏则本审计门红（PLAN §2.6 防注释再腐烂）。config.rs 无 `#[serde(rename)]`
    /// （亲验），字段名 = YAML 键名逐字相等。
    const SERVER_CONFIG_PUB_FIELDS: &[&str] = &[
        "version",
        "listen",
        "room_capacity",
        "psk",
        "jwt_secret",
        "admin_jwt_secret",
        "rate_limit",
        "ws_max_message_size",
        "consumer_limit_per_stream",
        "tls",
        "devices_file",
        "accounts_file",
        "api_keys_file",
        "api_token_ttl_secs",
        "sfu",
        "web",
    ];

    /// 合法地只以说明文字（非 `键:` 行形）出现在模板的字段逃生阀 = (字段, 理由)。
    /// 现 sfu/web 均以注释示例 `# sfu:`/`# web:` 承载键形，走常规遍历，故本表空。
    const TEMPLATE_PROSE_ONLY_FIELDS: &[(&str, &str)] = &[];

    /// 键边界判据：去 `#` 注释前缀后有行以 `field:` 起头 = 该字段有键形示例。
    /// 比裸 `contains("field:")` 严——defeats `jwt_secret` ⊂ `admin_jwt_secret` 前缀假阳性
    /// （PLAN 以 room ⊂ room_capacity 举例，同类前缀在此升级为行首锚定）。
    fn template_has_key(field: &str) -> bool {
        let want = format!("{field}:");
        SERVER_YAML_TEMPLATE.lines().any(|l| {
            let t = l.trim_start();
            let t = t.strip_prefix('#').unwrap_or(t).trim_start();
            t.starts_with(&want)
        })
    }

    #[test]
    fn server_yaml_template_covers_all_pub_fields() {
        let mut missing = Vec::new();
        for field in SERVER_CONFIG_PUB_FIELDS {
            let excepted = TEMPLATE_PROSE_ONLY_FIELDS.iter().any(|(f, _)| f == field);
            if !excepted && !template_has_key(field) {
                missing.push(*field);
            }
        }
        assert!(
            missing.is_empty(),
            "SERVER_YAML_TEMPLATE 缺字段键形注释（新增 ServerConfig pub 字段须同笔补注释+入本表）: {missing:?}"
        );
    }
}
