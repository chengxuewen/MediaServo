//! `startup on|off|status`——开机锚点（frontend-process-split T19）。
//!
//! 与 host 完全同构：systemd user unit 拉**实例 oxmgr daemon**（Restart=always），
//! daemon 持久化 app 状态 → 开机自动复活整簇（server + caddy）。非 Linux 提示 oxmgr service。

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{absolute, oxmgr_bin, parse_dir};
use super::templates::{instance_daemon_port, server_namespace, server_product};

// ── startup（T19 开机锚点——systemd user unit 拉实例 daemon，host 同构）────────

/// takeover 决策（纯函数单测）：无其他 unit → 直接装；有 + tty → 交互接管；有 + 非 tty → 拒。
#[derive(Debug, PartialEq, Eq)]
pub enum StartupDecision {
    Install,
    Takeover,
    Abort,
}

pub fn decide_startup(others_found: bool, tty: bool) -> StartupDecision {
    if !others_found {
        StartupDecision::Install
    } else if tty {
        StartupDecision::Takeover
    } else {
        StartupDecision::Abort
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

/// 扫描其他实例的 server 锚点 unit（(unit 路径, 实例目录)——从 Environment=OXMGR_HOME 反推）。
fn other_startup_units(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mine = startup_unit_name(dir);
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(units_dir()) else {
        return found;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.ends_with(".service")
            || !name.starts_with("oxmgr-")
            || !name.contains("server")
            || name == mine
        {
            continue;
        }
        let path = e.path();
        let d = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| {
                c.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("Environment=OXMGR_HOME=")
                        .map(|v| v.trim().to_string())
                })
            })
            .map(|h| PathBuf::from(h).join("..").join(".."))
            .unwrap_or_default();
        found.push((path, d));
    }
    found
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

#[cfg(target_os = "linux")]
fn startup_install(dir: &Path) -> i32 {
    let others = other_startup_units(dir);
    match decide_startup(!others.is_empty(), std::io::stdin().is_terminal()) {
        StartupDecision::Abort => {
            for (p, od) in &others {
                eprintln!(
                    "检测到其他实例已开启自启: {}（实例目录: {}）",
                    p.display(),
                    od.display()
                );
            }
            eprintln!("  非交互环境——退出（全局唯一锚点；先 `startup off <旧dir>` 或交互接管）");
            return 1;
        }
        StartupDecision::Takeover => {
            for (p, od) in &others {
                eprintln!("  接管: 卸载旧 unit {}（实例 {}）", p.display(), od.display());
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                let _ = Command::new("systemctl").args(["--user", "disable", "--now", &name]).status();
                let _ = std::fs::remove_file(p);
            }
            let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
        }
        StartupDecision::Install => {}
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
                eprintln!("startup on: 执行 {} 失败: {e}（systemd 用户服务不可用？）", args.join(" "));
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
    eprintln!("startup on: 非 Linux——用 oxmgr service install（macOS launchd / Windows Task Scheduler）");
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
        assert_eq!(decide_startup(true, false), StartupDecision::Abort);
        assert_eq!(decide_startup(true, true), StartupDecision::Takeover);
        assert_eq!(decide_startup(false, false), StartupDecision::Install);
    }
}
