//! stop 兜底扫尾纯函数组（server-stop-global-reap T1，PLAN §2.1/§2.2/§2.5）。
//!
//! 职责边界（§2.5）：**判定与计划**，零 IO——/proc 扫描、environ 读取、信号、
//! oxmgr 调用全在 T2/T3 执行层薄壳。跨用户（kill EPERM）与 daemon 已死降级
//! 均为运行时关注面，不在纯计划里表达。
//!
//! 合同（`reap_plan`）：调用方先解析"哪个目录的注册表快照归哪个 daemon"，
//! 以 `(daemon_data_dir, entries)` 对传入；pid 优先、miss 回落 cwd 归属
//! （`inspect::strays_under` 同法），双 miss = 孤儿直杀。
//!
//! T2/T3 接线前本组符号仅测试消费——dead_code 允许随接线摘除（expect 保底防陈旧）。
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use super::inspect::ManagedEntry;
// ── 进程身份签名（§2.1，唯一真值 = /proc，本函数只做纯判定）─────────────────

/// 存活进程快照（T2 扫描层的填充物；T1 只定义形状 + 测试构造器）。
#[derive(Debug, Clone)]
pub(super) struct LiveProc {
    pub pid: u32,
    pub argv: Vec<String>,
    pub exe: Option<String>,
    pub cwd: Option<PathBuf>,
    /// /proc/<pid>/environ 里 OXMGR_HOME/OXMGR_DATA_DIR 的值——Some = 实例 daemon 名下。
    pub environ_oxmgr_home: Option<PathBuf>,
}

impl LiveProc {
    #[cfg(test)]
    fn new(pid: u32, argv: &[&str]) -> Self {
        Self {
            pid,
            argv: argv.iter().map(|s| s.to_string()).collect(),
            exe: None,
            cwd: None,
            environ_oxmgr_home: None,
        }
    }
}

/// 进程身份三分类（签名判定输出；caddy 的磁盘标记是独立 IO 步，见 `caddy_marker_ok`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ProcKind {
    Server,
    Weaknet,
    /// `config` = argv 中 `--config` 的取值（实例 root 派生源，§2.1 m3）。
    Caddy {
        config: PathBuf,
    },
}

fn basename(p: &str) -> Option<&str> {
    Path::new(p).file_name().and_then(|s| s.to_str())
}

/// exe 路径含 `target/` 路径**组件**（非子串）→ cargo test/dev 二进制，排除（Q1 裁决）。
fn exe_is_target(exe: &str) -> bool {
    Path::new(exe).components().any(|c| c.as_os_str() == "target")
}

/// argv 签名判定（纯函数，无 IO）。空 argv / 非本产品名 / argv[1] 非 run|serve / exe
/// 出 target/ 组件 → None。caddy 候选在此产出（含 --config 取值），磁盘标记由
/// 调用方以 `caddy_marker_ok` 补验——IO 与纯判定在此切缝，双侧均可无 /proc 单测。
pub(super) fn proc_kind(argv: &[String], exe: Option<&str>) -> Option<ProcKind> {
    if let Some(e) = exe
        && exe_is_target(e)
    {
        return None;
    }
    let argv0 = basename(argv.first()?)?;
    let sub = argv.get(1)?;
    if SERVER_NAMES.contains(&argv0) && sub == "run" {
        return Some(ProcKind::Server);
    }
    if WEAKNET_NAMES.contains(&argv0) && sub == "serve" {
        return Some(ProcKind::Weaknet);
    }
    if basename(exe?) == Some("caddy") {
        let mut it = argv.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "--config"
                && let Some(v) = it.next()
                && v.ends_with("/etc/Caddyfile")
            {
                return Some(ProcKind::Caddy { config: PathBuf::from(v) });
            }
        }
    }
    None
}

/// caddy 磁盘标记（B1，唯一 IO 切面）：`<root>/bin/<server-bin-name>` 存在才认——
/// root = Caddyfile（etc/）上一级（§2.1 二审钉：config_path.parent().parent()）。
/// 防 TalkServo 同构布局误杀。bin 名集两枚字面量（不读品牌：测试进程与部署进程
/// 品牌编译期可不同，marker 判据必须进程无关——本次会话实锚）。
pub(super) fn caddy_marker_ok(config: &Path) -> bool {
    let Some(root) = config.parent().and_then(Path::parent) else { return false };
    SERVER_NAMES.iter().any(|n| root.join("bin").join(n).is_file())
}

const SERVER_NAMES: &[&str] = &["msrtc-server", "mediaservo-server"];
const WEAKNET_NAMES: &[&str] = &["msrtc-weaknet", "mediaservo-weaknet"];

// ── 收口计划（§2.2 纯形：经哪个 daemon / 直杀）──────────────────────────────

/// 单进程收口步骤。`ViaDaemon.daemon_data_dir` = 目标 daemon 的 OXMGR_DATA_DIR
/// （执行层据此设 env 调 oxmgr stop+delete）；直杀只剩 NoRegistry——daemon 已死
/// 降级是运行时关注面，不进纯计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReapStep {
    ViaDaemon { daemon_data_dir: PathBuf, name: String, pid: u32 },
    DirectKill { pid: u32, reason: DirectReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DirectReason {
    /// 实例/默认两表 pid+cwd 双查皆 miss（孤儿 / nohup 直启）。
    NoRegistry,
}

/// 收口计划（纯函数）。**合同**：`registries` 由调用方预解析为 (daemon_data_dir,
/// 该目录下读出的注册表快照) 对——本函数不做磁盘读取、不做 default_data_dir()
/// env 解析。命中规则：pid 精确匹配优先；miss 后按 cwd 归属兜底（条目 cwd 绝对化
/// 后 starts_with 进程 cwd，`inspect::strays_under` 同法，M2: daemon 重启后 pid
/// 陈旧形态）。双查皆 miss → `DirectKill(NoRegistry)`。
pub(super) fn reap_plan(
    procs: &[LiveProc],
    registries: &[(PathBuf, &[ManagedEntry])],
) -> Vec<ReapStep> {
    procs
        .iter()
        .map(|p| {
            for (dir, entries) in registries {
                if let Some(e) = lookup(entries, p) {
                    return ReapStep::ViaDaemon {
                        daemon_data_dir: dir.clone(),
                        name: e.name.clone(),
                        pid: p.pid,
                    };
                }
            }
            ReapStep::DirectKill { pid: p.pid, reason: DirectReason::NoRegistry }
        })
        .collect()
}

/// 注册表反查：pid 优先，miss 则 cwd 归属（双侧绝对化，组件级前缀）。
fn lookup<'a>(entries: &'a [ManagedEntry], p: &LiveProc) -> Option<&'a ManagedEntry> {
    if entries.iter().any(|e| e.pid == Some(p.pid as u64)) {
        return entries.iter().find(|e| e.pid == Some(p.pid as u64));
    }
    let Some(cwd) = &p.cwd else { return None };
    entries.iter().find(|e| e.cwd.starts_with(cwd))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── proc_kind: server / weaknet ─────────────────────────────────────────

    #[test]
    fn server_signature_both_brands() {
        for name in SERVER_NAMES {
            let argv = vec![format!("/opt/x/bin/{name}"), "run".into()];
            assert_eq!(proc_kind(&argv, Some(&argv[0])), Some(ProcKind::Server), "{name}");
        }
    }

    #[test]
    fn weaknet_signature_both_brands() {
        for name in WEAKNET_NAMES {
            let argv = vec![format!("/opt/x/bin/{name}"), "serve".into()];
            assert_eq!(proc_kind(&argv, Some(&argv[0])), Some(ProcKind::Weaknet), "{name}");
        }
    }

    #[test]
    fn wrong_subcommand_is_none() {
        let a = vec!["/opt/x/bin/msrtc-server".to_string(), "serve".into()];
        assert_eq!(proc_kind(&a, None), None, "server 进程用 serve = 不匹配");
        let b = vec!["/opt/x/bin/msrtc-weaknet".to_string(), "run".into()];
        assert_eq!(proc_kind(&b, None), None);
        // 没有 argv[1]
        let c = vec!["/opt/x/bin/msrtc-server".to_string()];
        assert_eq!(proc_kind(&c, None), None);
    }

    #[test]
    fn non_server_products_are_none() {
        for name in ["msrtc-host", "mediaservo-host", "msrtc-agent", "msrtc-streamer", "oxmgr"] {
            let argv = vec![format!("/opt/x/bin/{name}"), "run".into()];
            assert_eq!(proc_kind(&argv, Some(&argv[0])), None, "{name} 不得命中");
        }
    }

    #[test]
    fn empty_argv_is_none() {
        assert_eq!(proc_kind(&[], None), None);
    }

    #[test]
    fn trailing_slash_argv0_still_matches() {
        let argv = vec!["/opt/x/bin/msrtc-server/".to_string(), "run".into()];
        assert_eq!(proc_kind(&argv, None), Some(ProcKind::Server));
    }

    #[test]
    fn target_component_exe_excluded_but_lookalike_kept() {
        // 组件形：target/ 出现在路径中 → 排除（cargo test 二进制）
        let argv = vec!["/home/u/ms/target/debug/msrtc-server".to_string(), "run".into()];
        assert_eq!(proc_kind(&argv, Some(argv[0].as_str())), None);
        // 形近子串：/opt/targets/bin 不是 target/ 组件 → 不排除
        let lookalike = vec!["/opt/targets/bin/msrtc-server".to_string(), "run".into()];
        assert_eq!(proc_kind(&lookalike, Some(lookalike[0].as_str())), Some(ProcKind::Server));
    }

    // ── proc_kind: caddy（argv 面）+ 磁盘标记（tempdir IO 面）───────────────

    #[test]
    fn caddy_argv_candidate_without_config_value_is_none() {
        let a = vec!["/usr/bin/caddy".to_string(), "run".into()];
        assert_eq!(proc_kind(&a, Some("/usr/bin/caddy")), None);
        // config 值不以 /etc/Caddyfile 结尾
        let b = vec![
            "/usr/bin/caddy".to_string(),
            "run".into(),
            "--config".into(),
            "/x/etc/Caddyfile.dev".into(),
        ];
        assert_eq!(proc_kind(&b, Some("/usr/bin/caddy")), None);
    }

    #[test]
    fn caddy_marker_present_accepts_absent_rejects() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let cfg = root.join("etc").join("Caddyfile");
        // marker 缺失（TalkServo 同构布局形）→ argv 命中但标记拒
        std::fs::create_dir_all(root.join("etc")).unwrap();
        assert!(!caddy_marker_ok(&cfg));
        let argv = vec![
            "/usr/bin/caddy".to_string(),
            "run".into(),
            "--config".into(),
            cfg.to_string_lossy().into_owned(),
        ];
        assert_eq!(
            proc_kind(&argv, Some("/usr/bin/caddy")),
            Some(ProcKind::Caddy { config: cfg.clone() })
        );
        // marker 在（品牌形 + 上游形都认）→ 采纳
        std::fs::write(root.join("bin").join("msrtc-server"), b"").unwrap();
        assert!(caddy_marker_ok(&cfg));
        std::fs::remove_file(root.join("bin").join("msrtc-server")).unwrap();
        std::fs::write(root.join("bin").join("mediaservo-server"), b"").unwrap();
        assert!(caddy_marker_ok(&cfg));
    }

    // ── reap_plan 四形态 + M2 pid-陈旧-cwd-兜底 ─────────────────────────────

    fn entry(id: u64, name: &str, pid: Option<u64>, cwd: &str) -> ManagedEntry {
        ManagedEntry { id, name: name.into(), status: "running".into(), pid, cwd: cwd.into() }
    }

    #[test]
    fn instance_daemon_managed_routes_via_instance_daemon() {
        let p = LiveProc {
            environ_oxmgr_home: Some(PathBuf::from("/inst/run/oxmgr")),
            ..LiveProc::new(100, &["/x/msrtc-server", "run"])
        };
        let inst = vec![entry(1, "msrtc-server", Some(100), "/inst")];
        let plan = reap_plan(
            &[p],
            &[(PathBuf::from("/inst/run/oxmgr"), &inst), (PathBuf::from("/default"), &[])],
        );
        assert_eq!(
            plan,
            vec![ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/inst/run/oxmgr"),
                name: "msrtc-server".into(),
                pid: 100
            }]
        );
    }

    #[test]
    fn default_daemon_managed_routes_via_default_daemon() {
        let p = LiveProc::new(200, &["/x/msrtc-server", "run"]); // environ 无 OXMGR_*
        let default = vec![entry(2, "msrtc-server", Some(200), "/deploy/out/server")];
        let plan = reap_plan(&[p], &[(PathBuf::from("/home/u/.local/share/oxmgr"), &default)]);
        assert_eq!(
            plan,
            vec![ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/home/u/.local/share/oxmgr"),
                name: "msrtc-server".into(),
                pid: 200
            }]
        );
    }

    #[test]
    fn orphan_double_miss_direct_kill() {
        let p = LiveProc::new(300, &["/x/msrtc-server", "run"]);
        let plan = reap_plan(&[p], &[(PathBuf::from("/d"), &[])]);
        assert_eq!(plan, vec![ReapStep::DirectKill { pid: 300, reason: DirectReason::NoRegistry }]);
    }

    #[test]
    fn stale_pid_falls_back_to_cwd_membership() {
        // M2: daemon 崩溃重启后注册表 pid 陈旧（≠ 活进程 pid），cwd 归属兜底仍经 daemon
        let p = LiveProc {
            cwd: Some(PathBuf::from("/deploy/out/server")),
            ..LiveProc::new(999, &["/x/msrtc-server", "run"])
        };
        let stale = vec![entry(3, "msrtc-server", Some(3419236), "/deploy/out/server")];
        let plan = reap_plan(&[p], &[(PathBuf::from("/d/oxmgr"), &stale)]);
        assert_eq!(
            plan,
            vec![ReapStep::ViaDaemon {
                daemon_data_dir: PathBuf::from("/d/oxmgr"),
                name: "msrtc-server".into(),
                pid: 999
            }]
        );
    }

    #[test]
    fn cwd_sibling_prefix_not_matched() {
        // /deploy/out/server-2 不是 /deploy/out/server 之下（strays_under 同训）
        let p = LiveProc {
            cwd: Some(PathBuf::from("/deploy/out/server")),
            ..LiveProc::new(400, &["/x/msrtc-server", "run"])
        };
        let reg = vec![entry(4, "sibling", Some(1), "/deploy/out/server-2")];
        let plan = reap_plan(&[p], &[(PathBuf::from("/d"), &reg)]);
        assert_eq!(plan, vec![ReapStep::DirectKill { pid: 400, reason: DirectReason::NoRegistry }]);
    }
}
