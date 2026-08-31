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

/// oxfile [defaults].namespace（status/stop 过滤锚点）。
pub fn server_namespace() -> String {
    server_product()
}

/// web 端口缺省（init 渲染进 Caddyfile；start/status 从 Caddyfile 反解，可被手工编辑覆盖）。
pub const DEFAULT_WEB_PORT: u16 = 8080;

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

# ── 鉴权凭据（init 自举，文件 0600；本文件含密钥，勿入库）──
# PSK: config 优先 + env MEDIASERVO_PSK 兜底（C35）；轮换 POST /api/admin/psk 热生效（C33）。
psk: "__PSK__"
# jwt 与 admin_jwt 同值 = accounts token 验签配对（main.rs fail-fast 门）。
jwt_secret: "__JWT__"
admin_jwt_secret: "__JWT__"

# 注册表相对路径 = 相对本文件所在目录解析（main.rs resolve_path）。
# 管理面：/api/admin/devices* /api/admin/accounts* 热生效（C33），禁止手改+重启。
devices_file: "devices.yaml"
accounts_file: "accounts.yaml"

# SFU 对外公告地址（mediasoup 0.0.0.0 必须配 announced——PIT-44/58/138；C38 排查层①）。
# 优先级: env MEDIASERVO_SFU_ANNOUNCED_IP（逗号分隔多 IP）> 此处 > 自动探测（容器内不可达）。
# sfu:
#   announced_ips:
#     - "192.168.0.10"
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

/// oxfile 静态模板头（拓扑固定 2 进程——免翻译层，design.md「修正版」）。
/// 无 watch：server.yaml 变更 = 人工 restart（热生效项走 admin API C33——psk 轮换会重写
/// server.yaml，watch 会触发重启风暴，此处刻意不加）。
pub fn render_oxfile(
    dir_abs: &Path,
    server_bin: &Path,
    caddy_cmd: &str,
    server_env: &[(String, String)],
) -> String {
    let dir = dir_abs.to_string_lossy();
    let cfg = format!("{dir}/etc/server.yaml");
    let log_dir = format!("{dir}/run/logs");
    let server_app = server_product();
    let web_app = server_web_app();
    let mut out = format!(
        "# mediaservo-server init 生成——静态模板（2 条目；变更拓扑=改本文件+restart）\n\
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
    out
}

/// 日志绝对路径（实例 run/logs——OxMgr 按 daemon cwd 解析相对路径的坑，host translate 同训）。
fn push_logs(out: &mut String, name: &str, log_dir: &str) {
    out.push_str(&format!(
        "[apps.logs]\nstdout = \"{log_dir}/{name}.out.log\"\nstderr = \"{log_dir}/{name}.err.log\"\n"
    ));
}

pub fn render_server_yaml(psk: &str, jwt: &str) -> String {
    SERVER_YAML_TEMPLATE
        .replace("__PSK__", psk)
        .replace("__JWT__", jwt)
}

pub fn render_caddyfile(web_port: u16, backend_port: u16, web_root: &Path) -> String {
    CADDYFILE_TEMPLATE
        .replace("__WEB_ADDR__", &format!(":{web_port}"))
        .replace("__BACKEND__", &format!("127.0.0.1:{backend_port}"))
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
        (!digits.is_empty() && digits.len() <= rest.len())
            .then(|| digits.parse().ok())
            .flatten()
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
        );
        let b = render_oxfile(
            Path::new("/opt/ms"),
            Path::new("/opt/ms/bin/mediaservo-server"),
            "/usr/bin/caddy",
            &[("MEDIASERVO_SFU_PORT".to_string(), "20012".to_string())],
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
        let cf = render_caddyfile(8089, 9802, Path::new("/opt/ms/web"));
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

    #[test]
    fn templates_parse_as_yaml() {
        serde_yaml::from_str::<serde_yaml::Value>(DEVICES_TEMPLATE).expect("devices yaml");
        serde_yaml::from_str::<serde_yaml::Value>(ACCOUNTS_TEMPLATE).expect("accounts yaml");
    }
}
