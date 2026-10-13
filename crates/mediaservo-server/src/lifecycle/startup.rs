//! `startup on|off|status`——开机锚点（frontend-process-split T19）。
//!
//! 与 host 完全同构：systemd user unit 拉**实例 oxmgr daemon**（Restart=always），
//! daemon 持久化 app 状态 → 开机自动复活整簇（server + caddy）。非 Linux 提示 oxmgr service。
//!
//! startup-hardening S1：冲突闸 v2——身份核对（brand 感知前缀，禁 contains 匹配——
//! TalkServo 同族 unit 误认事故面）+ 端口比对（三源求交）+ 交互接管纪律（PLAN §2.1）。

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::templates::{
    instance_daemon_port, parse_listen_port, parse_weaknet_port, parse_web_port, server_namespace,
    server_product,
};
use super::{absolute, oxmgr_bin, parse_dir};

// ── S1: 身份核对（单元身份分类——brand 感知前缀，精确匹配禁 contains）────────

/// unit 身份分类（纯函数；测试与 host 侧同语义镜像）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum UnitIdentity {
    /// 本产品 server 域（当前品牌 ns 前缀 ∪ 默认品牌 `oxmgr-mediaservo-server-`——§6 minor#2 ∪ 规则）。
    Server,
    /// 本产品 host 域（brand unit_prefix 已含 legacy `oxmgr-host-` 与品牌化 `oxmgr-<brand>-` 两形）。
    Host,
    /// 本产品 unknown 前缀 `oxmgr-*.service` 但不落上述两域（TalkServo 同族/残件）——WARN 列表，
    /// 永不静默、永不做接管候选。
    Unknown,
    /// 非 oxmgr unit——无关，忽略。
    Foreign,
}

/// unit 文件名 → 身份（不含 `.service` 后缀亦可——两态都收，调用方传全名）。
pub(super) fn classify_unit(name: &str) -> UnitIdentity {
    let stem = name.strip_suffix(".service").unwrap_or(name);
    let Some(rest) = stem.strip_prefix("oxmgr-").or((stem == "oxmgr").then_some("")) else {
        return UnitIdentity::Foreign;
    };
    let brand = mediaservo_common::brand::media_brand();
    let server_ns = server_namespace();
    // server 域 = 当前品牌形 `oxmgr-{ns}-`（ns = bin_prefix+server）∪ 默认品牌形
    // `oxmgr-mediaservo-server-`（§6 minor#2 ∪ 规则——跨品牌换装歧义消除）。
    let mut server_forms = vec![format!("{server_ns}-")];
    if server_ns != "mediaservo-server" {
        server_forms.push("mediaservo-server-".to_string());
    }
    if server_forms.iter().any(|f| rest.starts_with(f.as_str())) {
        return UnitIdentity::Server;
    }
    // host 域 = brand unit_prefix（默认 `oxmgr-host-` 已盖 legacy 同前缀形——§5 B1）
    // ∪ 品牌化 `oxmgr-{brand}-`（brand.id）。
    let host_forms =
        [brand.unit_prefix.trim_start_matches("oxmgr-").to_string(), format!("{}-", brand.id)];
    if host_forms.iter().any(|f| rest.starts_with(f.as_str())) {
        return UnitIdentity::Host;
    }
    UnitIdentity::Unknown
}

/// unit 文本 → 实例目录（env 双键 OXMGR_HOME= 优先，miss 落 OXMGR_DATA_DIR=——reap.rs 先例）。
/// 值 = `<实例>/run/oxmgr`，上跳两级回实例根；解析失败 → None。
pub(super) fn unit_instance_dir(unit_text: &str) -> Option<PathBuf> {
    let home = unit_env_value(unit_text, "OXMGR_HOME");
    let dir = home.or_else(|| unit_env_value(unit_text, "OXMGR_DATA_DIR"))?;
    let p = PathBuf::from(dir);
    // `<dir>/run/oxmgr` → `<dir>`；非两跳形（手工损形）按原值返回（调用方可探 run/oxfile 判存活）
    Some(p.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or(p))
}

/// unit 文本中 `Environment=KEY=value` 行的 value（trim 后；首个命中）。
fn unit_env_value(unit_text: &str, key: &str) -> Option<String> {
    let prefix = format!("Environment={key}=");
    unit_text.lines().find_map(|l| l.trim().strip_prefix(&prefix).map(|v| v.trim().to_string()))
}

// ── S1: 端口比对（三源——与 stop 同源 parse 函数；parse 失败 skip 该源）────────

/// 实例三探针口（server 域：yaml listen + Caddyfile web + oxfile weaknet）。
/// 从 stop_ports 抽出共享（stop 与 startup 冲突闸同源——PLAN §2.1）。
pub(super) fn instance_ports(dir: &Path) -> Vec<u16> {
    let mut ports = Vec::new();
    if let Some(p) = std::fs::read_to_string(dir.join("etc").join("server.yaml"))
        .ok()
        .and_then(|c| parse_listen_port(&c))
    {
        ports.push(p);
    }
    if let Some(p) = std::fs::read_to_string(dir.join("etc").join("Caddyfile"))
        .ok()
        .and_then(|cf| parse_web_port(&cf))
    {
        ports.push(p);
    }
    if let Ok(ox) = std::fs::read_to_string(dir.join("run").join("oxfile.toml"))
        && let Some(p) = parse_weaknet_port(&ox)
    {
        ports.push(p);
    }
    ports.sort();
    ports.dedup();
    ports
}

/// 两端口集交集（升序）。
fn overlap(a: &[u16], b: &[u16]) -> Vec<u16> {
    let mut out: Vec<u16> = a.iter().filter(|p| b.contains(p)).copied().collect();
    out.sort();
    out.dedup();
    out
}

// ── S1: 决策（纯函数——交互 I/O 留在调用方）────────────────────────────────

/// 单个冲突对方。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Conflict {
    pub peer_unit: String,
    pub peer_dir: PathBuf,
    pub peer_kind: PeerKind,
    pub overlapping: Vec<u16>,
}

/// 对方域：同类（同 CLI 域）/ 跨域（server 对 host）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PeerKind {
    SameDomain,
    CrossDomain,
}

/// 决策四态（decide_startup v2——§2.1 钉形）。Coexist 携共存说明行。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Install,
    Coexist(String),
    PromptTakeover,
    PromptCrossDomainWarn,
    Abort,
}

/// 聚合决策（§2.1）：同类撞优先弹接管（列全部），否则跨域撞弹警告，全空集共存安装。
pub(super) fn decide_v2(conflicts: &[Conflict], mine: &[u16], tty: bool) -> Decision {
    let _ = mine; // 交集已在 Conflict.overlapping 预算
    let same: Vec<&Conflict> =
        conflicts.iter().filter(|c| c.peer_kind == PeerKind::SameDomain).collect();
    let cross: Vec<&Conflict> =
        conflicts.iter().filter(|c| c.peer_kind == PeerKind::CrossDomain).collect();
    let same_hit = same.iter().any(|c| !c.overlapping.is_empty());
    let cross_hit = cross.iter().any(|c| !c.overlapping.is_empty());
    if same_hit {
        return if tty { Decision::PromptTakeover } else { Decision::Abort };
    }
    if cross_hit {
        return if tty { Decision::PromptCrossDomainWarn } else { Decision::Abort };
    }
    let mut peers: Vec<String> =
        conflicts.iter().map(|c| format!("{}（无端口交集）", c.peer_dir.display())).collect();
    peers.sort();
    peers.dedup();
    match peers.len() {
        0 => Decision::Install,
        1 => Decision::Coexist(format!("与实例 {} 共存（无端口交集）", peers[0])),
        _ => Decision::Coexist(format!("与实例 {} 共存（无端口交集）", peers.join("、"))),
    }
}

/// unit 名（目录 slug 派生——多实例各自可装、全局唯一检测按前缀反查）。
pub fn startup_unit_name(dir: &Path) -> String {
    let raw = dir.to_string_lossy().replace(['/', '\\', ' '], "-");
    let raw = raw.trim_start_matches('-');
    format!("oxmgr-{}-{raw}.service", server_namespace())
}

/// unit 内容（ExecStart=oxmgr daemon run + 实例 env——design 修正版模型：锚点拉 daemon，
/// daemon 持久化 app 状态、开机自动复活整簇；Restart=always 盯 daemon 死活。
/// 注：任务书曾拟 ExecStart=`bin start <dir>`——start 短退出 × Restart=always = 重启风暴，
/// 且 host 生产先例（oxmgr-*-host unit）即 daemon run，遵 design.md 修正版）。
pub fn render_startup_unit(dir: &Path, oxmgr: &str) -> String {
    let home = absolute(&dir.join("run").join("oxmgr")).to_string_lossy().into_owned();
    let port = instance_daemon_port(Path::new(&home));
    format!(
        "[Unit]
Description={product} oxmgr daemon ({dir})
After=network.target

[Service]
Type=simple
Environment=OXMGR_HOME={home}
Environment=OXMGR_DAEMON_ADDR=127.0.0.1:{port}
Environment=OXMGR_API_ADDR=127.0.0.1:{api}
ExecStart={oxmgr} daemon run
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
",
        product = server_product(),
        dir = dir.to_string_lossy(),
        api = port + 1000,
    )
}

fn units_dir() -> PathBuf {
    Path::new(&std::env::var("HOME").unwrap_or_default()).join(".config/systemd/user")
}

/// 扫描其他实例的本产品锚点 unit（S1 v2：身份核对 + 双键 env 反推实例目录）。
/// 返回 (unit 路径, 实例目录, 身份)。unknown/foreign 已滤（unknown 由调用方 WARN——
/// 本函数仅收 Server/Host 两域；unknown 集单独走 `scan_unknown_units`）。
fn scan_product_units(dir: &Path) -> (Vec<(PathBuf, PathBuf, UnitIdentity)>, Vec<String>) {
    let mine = startup_unit_name(dir);
    let mut found = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir(units_dir()) else {
        return (found, unknown);
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".service") || name == mine {
            continue;
        }
        let id = classify_unit(name);
        match id {
            UnitIdentity::Foreign => continue,
            UnitIdentity::Unknown => {
                unknown.push(name.to_string());
                continue;
            }
            other => {
                let path = e.path();
                let d = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|c| unit_instance_dir(&c))
                    .unwrap_or_default();
                found.push((path, d, other));
            }
        }
    }
    (found, unknown)
}

pub(super) fn cmd_startup(args: &mut dyn Iterator<Item = String>) -> i32 {
    let sub = args.next().unwrap_or_default();
    if !matches!(sub.as_str(), "on" | "off" | "status") {
        eprintln!("用法: {} startup <on|off|status> [<dir>]", server_product());
        return 2;
    }
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let dir = absolute(&dir);
    match sub.as_str() {
        "on" => startup_install(&dir),
        "off" => startup_uninstall(&dir),
        _ => startup_status(&dir),
    }
}

/// 冲突聚合（IO 薄壳：读对方实例端口 + 求交）。返回 (冲突表, 未知 unit 名集)。
fn collect_conflicts(dir: &Path) -> (Vec<Conflict>, Vec<String>) {
    let (units, unknown) = scan_product_units(dir);
    let mine = instance_ports(dir);
    let mut conflicts = Vec::new();
    for (path, peer_dir, id) in units {
        let kind = match id {
            UnitIdentity::Server => PeerKind::SameDomain,
            _ => PeerKind::CrossDomain,
        };
        let peer_ports =
            if peer_dir.as_os_str().is_empty() { Vec::new() } else { instance_ports(&peer_dir) };
        let overlapping = overlap(&mine, &peer_ports);
        conflicts.push(Conflict {
            peer_unit: path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
            peer_dir,
            peer_kind: kind,
            overlapping,
        });
    }
    (conflicts, unknown)
}

/// 冲突/对方明细行（明细前置纪律——动作与拒绝都先打这个）。
fn conflict_lines(conflicts: &[Conflict]) -> Vec<String> {
    conflicts
        .iter()
        .map(|c| {
            format!(
                "unit={} 实例={} 域={} 冲突端口={}",
                c.peer_unit,
                c.peer_dir.display(),
                match c.peer_kind {
                    PeerKind::SameDomain => "同类",
                    PeerKind::CrossDomain => "跨域",
                },
                if c.overlapping.is_empty() {
                    "无".to_string()
                } else {
                    format!("{:?}", c.overlapping)
                }
            )
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn startup_install(dir: &Path) -> i32 {
    let (conflicts, unknown) = collect_conflicts(dir);
    for u in &unknown {
        eprintln!("警告: 检测到未知 oxmgr unit（不识别域，未处理）: {u}");
    }
    for line in conflict_lines(&conflicts) {
        eprintln!("检测到本产品其他自启锚点: {line}");
    }
    let tty = std::io::stdin().is_terminal();
    match decide_v2(&conflicts, &instance_ports(dir), tty) {
        Decision::Abort => {
            eprintln!("  非交互环境——退出（端口冲突需处置；先 `startup off <旧dir>` 或交互接管）");
            return 1;
        }
        Decision::PromptTakeover => {
            eprintln!("同类实例端口冲突——三选一: [1]算了不装 [2]卸对方接管 [3]手工处置");
            eprint!("选择: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            match ans.trim() {
                "2" => {
                    for c in &conflicts {
                        if c.peer_kind == PeerKind::SameDomain {
                            eprintln!(
                                "  接管: 卸载 {}（实例 {}，端口明细见上）",
                                c.peer_unit,
                                c.peer_dir.display()
                            );
                            let _ = Command::new("systemctl")
                                .args(["--user", "disable", "--now", &c.peer_unit])
                                .status();
                            let _ = std::fs::remove_file(units_dir().join(&c.peer_unit));
                        }
                    }
                    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
                }
                "3" => {
                    eprintln!("已退出——请手工处置上述冲突后重试（exit 1）");
                    return 1;
                }
                _ => {
                    println!("已取消");
                    return 0;
                }
            }
        }
        Decision::PromptCrossDomainWarn => {
            eprintln!("警告: 跨服务域端口重叠——不同服务域，通常为配置错误");
            eprintln!("选择: [1]覆盖(卸对方) [2]取消本项 [3]允许并行(已知晓风险)");
            eprint!("选择: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            match ans.trim() {
                "1" => {
                    for c in &conflicts {
                        if c.peer_kind == PeerKind::CrossDomain && !c.overlapping.is_empty() {
                            eprintln!(
                                "  覆盖: 卸载 {}（实例 {}，端口明细见上）",
                                c.peer_unit,
                                c.peer_dir.display()
                            );
                            let _ = Command::new("systemctl")
                                .args(["--user", "disable", "--now", &c.peer_unit])
                                .status();
                            let _ = std::fs::remove_file(units_dir().join(&c.peer_unit));
                        }
                    }
                    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
                }
                "3" => {
                    eprintln!("警告: 允许并行——端口重叠风险已知晓");
                }
                _ => {
                    println!("已取消");
                    return 0;
                }
            }
        }
        Decision::Coexist(note) => {
            println!("startup on: {note}");
        }
        Decision::Install => {}
    }
    let unit_name = startup_unit_name(dir);
    let unit_path = units_dir().join(&unit_name);
    if let Some(parent) = unit_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let oxmgr = oxmgr_bin().to_string_lossy().into_owned();
    if let Err(e) = std::fs::write(&unit_path, render_startup_unit(dir, &oxmgr)) {
        eprintln!("startup on: 写入 unit 失败: {e}");
        return 1;
    }
    println!("startup on: 已安装 {}（daemon 锚点，Restart=always）", unit_path.display());
    for args in [
        vec!["systemctl", "--user", "daemon-reload"],
        vec!["systemctl", "--user", "enable", "--now", unit_name.as_str()],
    ] {
        match Command::new(args[0]).args(&args[1..]).status() {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("startup on: {} 退出码 {}", args.join(" "), s.code().unwrap_or(-1));
                return 1;
            }
            Err(e) => {
                eprintln!(
                    "startup on: 执行 {} 失败: {e}（systemd 用户服务不可用？）",
                    args.join(" ")
                );
                return 1;
            }
        }
    }
    println!("startup on: 开机自启已启用（{unit_name}）——建议 loginctl enable-linger 保跨会话存活");
    0
}

#[cfg(target_os = "linux")]
fn startup_uninstall(dir: &Path) -> i32 {
    let unit_name = startup_unit_name(dir);
    let unit_path = units_dir().join(&unit_name);
    if !unit_path.exists() {
        println!("startup off: 未安装自启（{} 不存在）", unit_path.display());
        return 0;
    }
    let _ = Command::new("systemctl").args(["--user", "disable", "--now", &unit_name]).status();
    let _ = std::fs::remove_file(&unit_path);
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!("startup off: 已停用并移除 {}", unit_path.display());
    0
}

#[cfg(target_os = "linux")]
fn startup_status(dir: &Path) -> i32 {
    let unit_name = startup_unit_name(dir);
    let unit_path = units_dir().join(&unit_name);
    if !unit_path.exists() {
        println!(
            "startup: 未启用（{} 无锚点 unit——`{} startup on` 启用）",
            dir.display(),
            server_product()
        );
        return 1;
    }
    println!("startup: 已安装（{}）", unit_path.display());
    if let Ok(o) = Command::new("systemctl").args(["--user", "is-active", &unit_name]).output() {
        println!("  daemon 状态: {}", String::from_utf8_lossy(&o.stdout).trim());
    }
    0
}

#[cfg(not(target_os = "linux"))]
fn startup_install(_dir: &Path) -> i32 {
    eprintln!(
        "startup on: 非 Linux——用 oxmgr service install（macOS launchd / Windows Task Scheduler）"
    );
    1
}
#[cfg(not(target_os = "linux"))]
fn startup_uninstall(_dir: &Path) -> i32 {
    eprintln!("startup off: 非 Linux——用 oxmgr service uninstall");
    1
}
#[cfg(not(target_os = "linux"))]
fn startup_status(_dir: &Path) -> i32 {
    eprintln!("startup status: 非 Linux——用 oxmgr service status");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_unit_render_and_naming() {
        let u = render_startup_unit(Path::new("/opt/msr"), "/opt/msr/bin/oxmgr");
        assert!(u.contains("ExecStart=/opt/msr/bin/oxmgr daemon run"));
        assert!(u.contains("After=network.target"));
        assert!(u.contains("Restart=always"));
        assert!(u.contains("Environment=OXMGR_HOME=/opt/msr/run/oxmgr"));
        let name = startup_unit_name(Path::new("/opt/msr"));
        assert!(name.starts_with("oxmgr-mediaservo-server-"), "{name}");
        assert!(name.ends_with("opt-msr.service"));
    }

    // ── S1: 身份分类（§5 B1 mandate——brand 感知 + TalkServo 反例 + legacy fixture）──

    /// legacy 形（motivating fixture）：`oxmgr-host-` 前缀 + OXMGR_DATA_DIR 键 + 死 bin。
    const LEGACY_HOST_UNIT: &str = "\
[Unit]
Description=MediaServo host oxmgr daemon

[Service]
Environment=OXMGR_DATA_DIR=/home/maxsense/Documents/MediaServo/install/host/run/oxmgr
ExecStart=/old/bin/oxmgr daemon run
";

    /// current-server 形：OXMGR_HOME 键。
    const CURRENT_SERVER_UNIT: &str = "\
[Service]
Environment=OXMGR_HOME=/home/x/out/server/run/oxmgr
ExecStart=/home/x/out/server/bin/oxmgr daemon run
";

    #[test]
    fn identity_classification_table() {
        // server 域：当前品牌 ns 形 + 默认品牌形
        assert_eq!(
            classify_unit("oxmgr-mediaservo-server-home-x-out-server.service"),
            UnitIdentity::Server,
            "default-brand ns form"
        );
        // 当前品牌 ns 形——本测试进程品牌=默认，ns=mediaservo-server（同串故合并断言）
        assert_eq!(
            classify_unit(&format!("oxmgr-{}-foo.service", server_namespace())),
            UnitIdentity::Server,
            "ns={} (env may rebrand this test binary)",
            server_namespace()
        );
        // host 域：legacy/默认 `oxmgr-host-`（unit_prefix）
        assert_eq!(
            classify_unit("oxmgr-host-home-maxsense-Documents-MediaServo-install-host.service"),
            UnitIdentity::Host
        );
        // TalkServo 同族 → Unknown（B1 反例——绝不能判 Server/Host）
        assert_eq!(classify_unit("oxmgr-talkservo-server-home-t.service"), UnitIdentity::Unknown);
        // 裸残件 → Unknown（stem == "oxmgr"，rest 空串不落任何域）
        assert_eq!(classify_unit("oxmgr"), UnitIdentity::Unknown);
        assert_eq!(classify_unit("oxmgr.service"), UnitIdentity::Unknown);
        assert_eq!(classify_unit("oxmgr-mediaservo-server-x"), UnitIdentity::Server);
        // 非 oxmgr → Foreign
        assert_eq!(classify_unit("nginx.service"), UnitIdentity::Foreign);
        assert_eq!(classify_unit("myapp-oxmgr.service"), UnitIdentity::Foreign);
    }

    #[test]
    fn dual_key_extraction_home_first() {
        // HOME 优先（reap.rs:274 先例）
        assert_eq!(
            unit_instance_dir(CURRENT_SERVER_UNIT),
            Some(PathBuf::from("/home/x/out/server"))
        );
        // miss HOME → 落 DATA_DIR；值 = <实例>/run/oxmgr 上跳两级
        assert_eq!(
            unit_instance_dir(LEGACY_HOST_UNIT),
            Some(PathBuf::from("/home/maxsense/Documents/MediaServo/install/host"))
        );
        // 双键全缺 → None
        assert_eq!(unit_instance_dir("[Service]\nExecStart=x\n"), None);
    }

    #[test]
    fn overlap_empty_and_nonempty() {
        assert!(overlap(&[9800, 8080], &[17980, 9810]).is_empty());
        assert_eq!(overlap(&[9800, 8080], &[8080, 1234]), vec![8080]);
        assert_eq!(overlap(&[8080], &[8080]), vec![8080]);
        assert!(overlap(&[], &[9800]).is_empty());
    }

    fn conflict(kind: PeerKind, ports: &[u16]) -> Conflict {
        Conflict {
            peer_unit: "oxmgr-x.service".into(),
            peer_dir: PathBuf::from("/tmp/peer"),
            peer_kind: kind,
            overlapping: ports.to_vec(),
        }
    }

    #[test]
    fn decide_v2_four_states() {
        // 同类撞 + tty → 弹接管；非 tty → 拒
        assert_eq!(
            decide_v2(&[conflict(PeerKind::SameDomain, &[9800])], &[9800], true),
            Decision::PromptTakeover
        );
        assert_eq!(
            decide_v2(&[conflict(PeerKind::SameDomain, &[9800])], &[9800], false),
            Decision::Abort
        );
        // 跨域撞 + tty → 警告确认；非 tty → 拒
        assert_eq!(
            decide_v2(&[conflict(PeerKind::CrossDomain, &[8080])], &[8080], true),
            Decision::PromptCrossDomainWarn
        );
        assert_eq!(
            decide_v2(&[conflict(PeerKind::CrossDomain, &[8080])], &[8080], false),
            Decision::Abort
        );
    }

    #[test]
    fn decide_v2_coexist_and_clean_install() {
        // 同类无交集 → 共存（带说明）
        match decide_v2(&[conflict(PeerKind::SameDomain, &[])], &[9800], false) {
            Decision::Coexist(note) => {
                assert!(note.contains("共存（无端口交集）"), "{note}");
                assert!(note.contains("/tmp/peer"), "{note}");
            }
            other => panic!("期望 Coexist，得 {other:?}"),
        }
        // 跨域无交集 → 共存
        assert!(matches!(
            decide_v2(&[conflict(PeerKind::CrossDomain, &[])], &[9800], false),
            Decision::Coexist(_)
        ));
        // 零对方 → 纯 Install
        assert_eq!(decide_v2(&[], &[9800], false), Decision::Install);
    }

    #[test]
    fn decide_v2_aggregate_multi_conflict() {
        // 同类撞 + 跨域撞并存 → 同类优先（接管）
        assert_eq!(
            decide_v2(
                &[
                    conflict(PeerKind::SameDomain, &[9800]),
                    conflict(PeerKind::CrossDomain, &[8080])
                ],
                &[9800, 8080],
                true
            ),
            Decision::PromptTakeover
        );
        // 仅跨域多撞 → 警告
        assert_eq!(
            decide_v2(
                &[conflict(PeerKind::CrossDomain, &[8080]), conflict(PeerKind::CrossDomain, &[])],
                &[8080],
                true
            ),
            Decision::PromptCrossDomainWarn
        );
    }

    /// 禁 contains("server") 回归钉：talkservo-server 域 unit 即使内容带端口也不进冲突表
    /// （classify 层已判 Unknown——scan 层滤除；此处钉 classify 反例防倒退）。
    #[test]
    fn talkservo_family_never_same_domain() {
        assert_ne!(classify_unit("oxmgr-talkservo-server-x.service"), UnitIdentity::Server);
        assert_ne!(classify_unit("oxmgr-talkservo-server-x.service"), UnitIdentity::Host);
    }
}
