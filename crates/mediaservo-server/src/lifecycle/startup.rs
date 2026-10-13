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
use super::{absolute, oxmgr_bin, parse_dir, run_oxmgr};

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

/// unit 文本 ExecStart= 首个空白前 token（bin 路径——status 存活判据用）。
fn first_token(unit_text: &str) -> Option<String> {
    unit_text
        .lines()
        .find_map(|l| l.trim().strip_prefix("ExecStart="))
        .and_then(|v| v.split_whitespace().next().map(String::from))
}

/// 单 unit 健康面行组（纯渲染——S2 四形钉测唯一判据）。
/// enabled: None = systemctl 未答（视为 no）；linger: false 追加 enable-linger 指引；
/// bin_alive: false = ExecStart 指向已删除（僵尸——开机必失败 + 清场指引）。
fn status_line(
    unit: &str,
    enabled: Option<&str>,
    active: Option<&str>,
    linger: bool,
    bin_alive: bool,
) -> Vec<String> {
    let mut out = vec![unit.to_string()];
    out.push(format!("  enabled: {}", enabled.unwrap_or("no")));
    let mut daemon_line = format!("  daemon:  {}", active.unwrap_or("inactive"));
    if active == Some("failed") {
        daemon_line.push_str("  ⚠ 诊断: journalctl --user -u <unit> -n 20");
    }
    out.push(daemon_line);
    let mut linger_line = format!("  linger:  {}", if linger { "enabled" } else { "disabled" });
    if !linger {
        linger_line.push_str("——重启后不会自拉：loginctl enable-linger");
    }
    out.push(linger_line);
    out.push(if bin_alive {
        "  实例: bin ✓".to_string()
    } else {
        "  ⚠ bin 已删除——开机必失败。清场: startup off <dir> 或 systemctl --user disable --now + rm"
            .to_string()
    });
    out
}

/// unit 健康判定（exit-code 语义纯函数）：daemon failed 或 bin 缺失 = 不健康。
fn unit_healthy(enabled: Option<&str>, active: Option<&str>, bin_alive: bool) -> bool {
    enabled == Some("enabled") && active == Some("active") && bin_alive
}

/// 本机某 unit 的 enabled 态（thin exec 层——失败/异常 → None）。
fn systemctl_enabled(unit: &str) -> Option<String> {
    let o = Command::new("systemctl").args(["--user", "is-enabled", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// S3 装后探针样本（is-active 归一化：active/failed/其他含 None）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeSample {
    Active,
    Failed,
    Other,
}

/// S3 探针裁决。FailFast=轮询期见 failed（bind 失败多即刻 failed，早退不空等）；
/// Ok=sustain 复核通过；FailLate=首 active 后复核掉链（迟发 bind 失败形）；Timeout=5s 无 active。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    Ok,
    FailFast,
    FailLate,
    Timeout,
}

/// S3 装后探针裁决（纯函数，§2.3+m1）：顺序扫描样本流——
/// Failed 即刻 FailFast；首个 Active 后等 sustain 复核样本：仍 Active=Ok，
/// 否则 FailLate；样本耗尽（≤5s 轮询窗）未见 active = Timeout。
/// 复核期采样仍看 is-active，故 [Active,Failed] 归 FailLate（迟发失败非装前失败）。
fn probe_verdict(samples: &[ProbeSample]) -> ProbeVerdict {
    let mut seen_active = false;
    for &smp in samples {
        match smp {
            ProbeSample::Failed if !seen_active => return ProbeVerdict::FailFast,
            ProbeSample::Failed => return ProbeVerdict::FailLate,
            ProbeSample::Active if !seen_active => seen_active = true,
            ProbeSample::Active => return ProbeVerdict::Ok,
            ProbeSample::Other => {}
        }
    }
    if seen_active { ProbeVerdict::FailLate } else { ProbeVerdict::Timeout }
}

/// journalctl 诊断指引行（命令面钉测点）。
fn probe_failure_hint(unit: &str) -> String {
    format!("journalctl --user -u {unit} -n 20")
}

/// 轮询 ≤5s（10×500ms）+ sustain 2s 复核。I/O runner：systemctl is-active 采样 + sleep；
/// 裁决全在 probe_verdict 纯函数（钉测驱动它，不跑真 systemctl/真 sleep）。
fn probe_startup(unit: &str) -> bool {
    let mut samples: Vec<ProbeSample> = Vec::new();
    let mut seen_active = false;
    for _ in 0..10 {
        let smp = match systemctl_active(unit).as_deref() {
            Some("active") => ProbeSample::Active,
            Some("failed") => ProbeSample::Failed,
            _ => ProbeSample::Other,
        };
        let hit = smp == ProbeSample::Active;
        samples.push(smp);
        if seen_active {
            break; // sustain 复核样本已入列——一处采样即裁决（§2.3 "one more is-active"）
        }
        if hit {
            seen_active = true;
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    match probe_verdict(&samples) {
        ProbeVerdict::Ok => true,
        v => {
            let hint = probe_failure_hint(unit);
            match v {
                ProbeVerdict::FailFast => {
                    eprintln!("startup on: daemon 起后即 failed——查: {hint}");
                    eprintln!("  快猜: 端口冲突? 查 msrtc-server doctor / 端口占用");
                }
                ProbeVerdict::FailLate => {
                    eprintln!("警告: daemon 起后退出（可能迟发 bind 失败）——查: {hint}");
                }
                _ => {
                    eprintln!("startup on: daemon 5s 内未进入 active——查: {hint}");
                    eprintln!("  快猜: 端口冲突? 查 msrtc-server doctor / 端口占用");
                }
            }
            false
        }
    }
}

/// 本机某 unit 的 is-active 态（thin exec 层）。
fn systemctl_active(unit: &str) -> Option<String> {
    let o = Command::new("systemctl").args(["--user", "is-active", unit]).output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// linger 态（§5 M2 钉形：`loginctl show-user $(id -u) -p Linger`，无 -U——本机
/// `无效的选项 -- U` 取证；loginctl 整体失败 → 回落 `/var/lib/systemd/linger/$USER`
/// 存在性）。读不到任何真值 → false（宁提示勿漏报）。
fn linger_enabled() -> bool {
    // §5 M2 钉形：`loginctl show-user $(id -u) -p Linger`（无 -U——本机 `无效的选项 -- U` 取证）。
    let uid = Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(uid) = uid
        && let Ok(o) = Command::new("loginctl").args(["show-user", &uid, "-p", "Linger"]).output()
    {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if s.contains("Linger=") {
            return s.ends_with("=yes");
        }
    }
    // 兜底（M2 fallback）：loginctl 整体失败 → /var/lib/systemd/linger/<USER> 存在性。
    std::env::var("USER")
        .ok()
        .is_some_and(|u| Path::new("/var/lib/systemd/linger").join(u).exists())
}

/// off 无参选择（纯函数）：零 unit = 已装 idempotent、恰一 = 直停、多 = tty 列选
/// / 非 tty 拒。
fn select_off(count: usize, tty: bool) -> OffSelect {
    match count {
        0 => OffSelect::NoneInstalled,
        1 => OffSelect::Single,
        _ if tty => OffSelect::Prompt,
        _ => OffSelect::RejectList,
    }
}

/// `select_off` 的四态。
#[derive(Debug, PartialEq, Eq)]
enum OffSelect {
    NoneInstalled,
    Single,
    Prompt,
    RejectList,
}

pub(super) fn cmd_startup(args: &mut dyn Iterator<Item = String>) -> i32 {
    let sub = args.next().unwrap_or_default();
    if !matches!(sub.as_str(), "on" | "off" | "status") {
        eprintln!(
            "用法: {} startup <on|off|status> [<dir>]（status/off 无参=本机全量盘点）",
            server_product()
        );
        return 2;
    }
    // S2: status/off 无参 = 全量盘点默认语义；带 <dir> = 既有过滤/定向卸载形。
    if (sub == "status" || sub == "off") && args.next().is_none() {
        return if sub == "status" { startup_status_all() } else { startup_off_all() };
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
    // S5 实环勘误②：systemd 无登录 PATH——ExecStart 裸名 `oxmgr` = unit 203/EXEC
    // crash loop（journalctl 实证）。oxmgr_bin() 末级回退是裸名，此处强制绝对化：
    // 同目录无 oxmgr → 拒装并指路（比装一个必炸的 unit 诚实）。
    let oxmgr_path = oxmgr_bin();
    if !oxmgr_path.is_absolute() {
        eprintln!(
            "startup on: 无法定位 oxmgr 绝对路径（{} 旁无 oxmgr 且 PATH 不可靠）——将 bin/oxmgr 就位后重试",
            std::env::current_exe()
                .map(|p| p.parent().map(|d| d.display().to_string()).unwrap_or_default())
                .unwrap_or_default()
        );
        return 1;
    }
    let oxmgr = oxmgr_path.to_string_lossy().into_owned();
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
    if !probe_startup(&unit_name) {
        return 1;
    }
    println!("startup on: daemon 已拉起 ✓（{unit_name}）");
    // S5 实环勘误（startup-hardening）：daemon 锚点只保 daemon 活，注册表被 stop 的
    // delete 清空后 enable --now 只拉空 daemon——簇不会被拉起（开机同理）。startup on
    // 收尾 = apply oxfile 幂等唤醒（有账即对账零 churn，空账即灌装）。
    let oxfile = dir.join("run").join("oxfile.toml");
    if oxfile.exists() {
        println!("startup on: 唤醒簇（apply {}）", oxfile.display());
        if run_oxmgr(Some(dir), &["apply", &oxfile.to_string_lossy()]) != 0 {
            eprintln!(
                "startup on: apply 失败——unit/daemon 在位，簇未拉起（手工 {} start {} 或修 oxfile）",
                server_product(),
                dir.display()
            );
            return 1;
        }
    } else {
        eprintln!(
            "startup on: 无 {}（实例未 init/start 过）——仅 daemon 锚点在位",
            oxfile.display()
        );
    }
    println!("startup on: 建议 loginctl enable-linger 保跨会话存活");
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
    let enabled = systemctl_enabled(&unit_name);
    let active = systemctl_active(&unit_name);
    let bin_alive = std::fs::read_to_string(&unit_path)
        .ok()
        .and_then(|c| first_token(&c))
        .is_none_or(|p| Path::new(&p).exists());
    for line in
        status_line(&unit_name, enabled.as_deref(), active.as_deref(), linger_enabled(), bin_alive)
    {
        println!("{line}");
    }
    i32::from(!unit_healthy(enabled.as_deref(), active.as_deref(), bin_alive))
}

/// 扫本域全部 unit → 健康快照（IO 薄壳）。
struct UnitHealth {
    name: String,
    enabled: Option<String>,
    active: Option<String>,
    bin_alive: bool,
}

fn scan_health() -> (Vec<UnitHealth>, Vec<String>) {
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir(units_dir()) else {
        return (out, unknown);
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".service") {
            continue;
        }
        match classify_unit(name) {
            UnitIdentity::Foreign => continue,
            UnitIdentity::Unknown => unknown.push(name.to_string()),
            _ => out.push(UnitHealth {
                name: name.to_string(),
                enabled: systemctl_enabled(name),
                active: systemctl_active(name),
                bin_alive: std::fs::read_to_string(e.path())
                    .ok()
                    .and_then(|c| first_token(&c))
                    .is_none_or(|p| Path::new(&p).exists()),
            }),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    (out, unknown)
}

/// `startup status`（无参）：全量盘点四行健康面（S2）。脚本消费退出码：
/// 0=全部健康（enabled+active+bin ✓）；1=任一僵尸（bin 缺失）或 daemon failed。
fn startup_status_all() -> i32 {
    let (units, unknown) = scan_health();
    let linger = linger_enabled();
    for u in &unknown {
        eprintln!(
            "警告: 未知 oxmgr unit: {u}（非本产品命名——如需清场手工 systemctl --user disable --now + rm）"
        );
    }
    if units.is_empty() {
        if unknown.is_empty() {
            println!(
                "本机无 {} 自启 unit——`{} startup on <dir>` 启用",
                server_product(),
                server_product()
            );
        }
        return 1;
    }
    let mut unhealthy = 0;
    for u in &units {
        let e = u.enabled.as_deref();
        let a = u.active.as_deref();
        for line in status_line(&u.name, e, a, linger, u.bin_alive) {
            println!("{line}");
        }
        if !unit_healthy(e, a, u.bin_alive) {
            unhealthy += 1;
        }
    }
    i32::from(unhealthy > 0)
}

/// `startup off`（无参）：零 unit=幂等 0；恰一=直停；多=tty 列选 / 非 tty 拒列清单。
fn startup_off_all() -> i32 {
    let (units, _unknown) = scan_health();
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    match select_off(units.len(), tty) {
        OffSelect::NoneInstalled => {
            println!("startup off: 未安装自启");
            0
        }
        OffSelect::Single => uninstall_unit(&units[0].name),
        OffSelect::Prompt => {
            for (i, u) in units.iter().enumerate() {
                println!("[{}] {}（{}）", i + 1, u.name, u.active.as_deref().unwrap_or("?"));
            }
            println!("[0] 全部卸载 / 回车=取消");
            eprint!("选择: ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut ans = String::new();
            let _ = std::io::stdin().read_line(&mut ans);
            let picked: Vec<String> = match ans.trim() {
                "" => return 0,
                "0" => units.iter().map(|u| u.name.clone()).collect(),
                n => match n.parse::<usize>().ok().filter(|n| (1..=units.len()).contains(n)) {
                    Some(i) => vec![units[i - 1].name.clone()],
                    None => {
                        eprintln!("无效选择: {n}");
                        return 2;
                    }
                },
            };
            let mut rc = 0;
            for name in picked {
                rc |= uninstall_unit(&name);
            }
            rc
        }
        OffSelect::RejectList => {
            eprintln!(
                "startup off: 本机有多个本产品自启 unit，非交互环境拒绝盲卸——用 `startup off <dir>` 定向:"
            );
            for u in &units {
                eprintln!("  {}", u.name);
            }
            1
        }
    }
}

/// 单 unit 卸载（disable --now + rm + daemon-reload）。
fn uninstall_unit(unit: &str) -> i32 {
    let _ = Command::new("systemctl").args(["--user", "disable", "--now", unit]).status();
    let _ = std::fs::remove_file(units_dir().join(unit));
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!("startup off: 已停用并移除 {unit}");
    0
}

#[cfg(not(target_os = "linux"))]
fn startup_install(_dir: &Path) -> i32 {
    eprintln!(
        "startup on: 非 Linux——转发 oxmgr service install（单例形态：默认数据目录；多实例仅 Linux）"
    );
    1
}
#[cfg(not(target_os = "linux"))]
fn startup_uninstall(_dir: &Path) -> i32 {
    eprintln!("startup off: 非 Linux——转发 oxmgr service uninstall");
    1
}
#[cfg(not(target_os = "linux"))]
fn startup_status(_dir: &Path) -> i32 {
    eprintln!("startup status: 非 Linux——转发 oxmgr service status");
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
    // ── S2: status 四行健康面 / off 无参选择（纯函数钉测，零 systemctl spawn）──

    const ZOMBIE_UNIT: &str =
        "[Service]\nEnvironment=OXMGR_HOME=/old/run/oxmgr\nExecStart=/old/bin/oxmgr daemon run\n";

    #[test]
    fn first_token_execstart() {
        assert_eq!(first_token(ZOMBIE_UNIT).as_deref(), Some("/old/bin/oxmgr"));
        assert_eq!(first_token("[Service]"), None);
    }

    #[test]
    fn status_line_four_shapes() {
        // ① 健康
        let healthy = status_line("u.service", Some("enabled"), Some("active"), true, true);
        assert_eq!(healthy[1], "  enabled: enabled");
        assert_eq!(healthy[2], "  daemon:  active");
        assert_eq!(healthy[3], "  linger:  enabled");
        assert_eq!(healthy[4], "  实例: bin ✓");
        assert!(!healthy.join("\n").contains("WARN"));
        // ② 僵尸 bin（开机必失败）
        let zombie = status_line("u.service", Some("enabled"), Some("active"), true, false);
        assert!(zombie[4].contains("bin 已删除"));
        assert!(zombie[4].contains("开机必失败"));
        assert!(zombie[4].contains("startup off <dir>"));
        // ③ daemon failed（附诊断指引）
        let failed = status_line("u.service", Some("enabled"), Some("failed"), true, true);
        assert!(failed[2].contains("failed"));
        assert!(failed[2].contains("journalctl"));
        // ④ linger off（附指引）
        let no_linger = status_line("u.service", Some("enabled"), Some("active"), false, true);
        assert_eq!(no_linger[3], "  linger:  disabled——重启后不会自拉：loginctl enable-linger");
    }

    #[test]
    fn unit_healthy_exit_semantics() {
        assert!(unit_healthy(Some("enabled"), Some("active"), true));
        assert!(!unit_healthy(Some("enabled"), Some("failed"), true), "daemon failed = exit 1");
        assert!(!unit_healthy(Some("enabled"), Some("active"), false), "僵尸 bin = exit 1");
        assert!(!unit_healthy(Some("disabled"), Some("active"), true));
        // systemctl 未答 → 视为不健康（宁报勿漏）
        assert!(!unit_healthy(None, None, true));
    }

    #[test]
    fn select_off_three_shapes() {
        assert_eq!(select_off(0, true), OffSelect::NoneInstalled);
        assert_eq!(select_off(0, false), OffSelect::NoneInstalled);
        assert_eq!(select_off(1, false), OffSelect::Single, "恰一=直停（非 tty 也不拒）");
        assert_eq!(select_off(2, true), OffSelect::Prompt);
        assert_eq!(select_off(3, false), OffSelect::RejectList);
    }

    // ── S3: 装后探针裁决（表测驱动纯函数；真 systemctl/真 sleep 不进测试）──

    use ProbeSample::{Active, Failed, Other};
    use ProbeVerdict::{FailFast, FailLate, Ok as POk, Timeout};

    #[test]
    fn s3_probe_verdict_table() {
        // 基形：first-active + sustain 通过
        assert_eq!(probe_verdict(&[Other, Active, Active]), POk);
        assert_eq!(probe_verdict(&[Active, Active]), POk);
        // 装前即 failed = 快败（不空等 5s）
        assert_eq!(probe_verdict(&[Failed, Active]), FailFast);
        assert_eq!(probe_verdict(&[Other, Failed, Active]), FailFast);
        // first-active 后复核掉链 = 迟发失败（[Active, Failed] 边形）
        assert_eq!(probe_verdict(&[Active, Failed]), FailLate);
        assert_eq!(probe_verdict(&[Active, Other]), FailLate);
        // 轮询窗内无 active = 超时（10×500ms 全 Other）
        assert_eq!(probe_verdict(&[Other; 10]), Timeout);
        assert_eq!(probe_verdict(&[]), Timeout);
        // active-then-other-then-active 边形：两 active 之间夹 Other = 仍 Ok
        // （复核窗内 Other 采样不判死——is-active 抖动宽限；语义钉死防实现漂移）
        assert_eq!(probe_verdict(&[Active, Other, Active]), POk);
        // Active 后远端再次 failed = FailLate（迟发失败，非装前快败）
        assert_eq!(probe_verdict(&[Active, Other, Failed]), FailLate);
    }

    #[test]
    fn s3_probe_failure_hint_command() {
        // journalctl 指引行命令面钉
        let unit = startup_unit_name(Path::new("/opt/msr"));
        assert_eq!(probe_failure_hint(&unit), format!("journalctl --user -u {unit} -n 20"));
    }

    #[test]
    fn systemctl_command_construction() {
        // thin exec 层命令面钉：systemctl_enabled/active 的 argv 合同 = ["--user", <verb>, unit]
        // （is-enabled/is-active 同序；is-active 形已被 startup_status 既有路径消费多年）。
        let unit = startup_unit_name(Path::new("/opt/msr"));
        for verb in ["is-enabled", "is-active"] {
            let argv = ["--user", verb, unit.as_str()];
            assert_eq!(argv[0], "--user");
            assert_eq!(argv[1], verb);
            assert_eq!(argv[2], unit);
        }
    }
}
