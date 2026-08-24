//! 拓扑监控核心（Task E1，D-H4 声明式期望 + 发现式实际）。
//!
//! 期望态: host.yaml 声明（N capturer + N streamer + recorder/controller/emergency/
//! audio/agent 固定进程 + 每相机一个 `camera/<id>` 发布 topic）。
//! 实际态: oxmgr list 进程存活 + FrameBus 跨进程发布者枚举（Momus MEDIUM-2 选项 ①，
//! `mediaservo_link::FrameBus::list_topics`，iceoryx2 服务注册表）。
//! 差异列表 + grace period 抑制启动窗口（D-H14: 启动窗口 vs 故障区分）。
//!
//! 数据消费者: E2（flow 统计）/ E3（signal 状态 + 上报 Server）沿用
//! [`TopologySnapshot`] 结构。

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use mediaservo_link::FrameBus;
use mediaservo_link::FrameTopic;

use crate::translate;

/// 期望进程在 oxmgr 中缺失或非 running。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    ProcessMissing { name: String },
    /// 期望相机 topic 无活跃连接（发布端进程活着但总线无发布者）。
    PublisherMissing { topic: String },
}

/// oxmgr list --json 单条进程（未知字段忽略）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OxProcess {
    pub name: String,
    pub status: String,
}

/// 解析 `oxmgr list --json` 输出（字段超集容忍，serde 忽略未知字段）。
pub fn parse_oxmgr_json(json: &str) -> Result<Vec<OxProcess>, String> {
    serde_json::from_str(json).map_err(|e| format!("oxmgr list --json 解析失败: {e}"))
}

/// 期望 vs 实际差异（纯函数，E1 单测覆盖）。
///
/// 进程期望: oxmgr 中存在且 status == "running"；topic 期望: FrameBus 发现中
/// 有活跃连接（调用方已过滤 alive_nodes >= 1 的 topic）。
pub fn diff(
    expected_procs: &[String],
    expected_topics: &[FrameTopic],
    actual_procs: &[OxProcess],
    actual_topics: &[FrameTopic],
) -> Vec<Mismatch> {
    let mut out = Vec::new();
    for name in expected_procs {
        let running = actual_procs.iter().any(|p| &p.name == name && p.status == "running");
        if !running {
            out.push(Mismatch::ProcessMissing { name: name.clone() });
        }
    }
    for topic in expected_topics {
        if !actual_topics.iter().any(|t| t == topic) {
            out.push(Mismatch::PublisherMissing { topic: topic.as_str().into() });
        }
    }
    out
}

/// oxmgr CLI 封装（进程级实际态数据源）。
pub struct OxmgrClient {
    bin: PathBuf,
}

impl OxmgrClient {
    /// 默认构造：PATH 查找 oxmgr，回退 `~/.local/bin/oxmgr`（开发机惯例）。
    pub fn new() -> Self {
        Self::with_bin(find_oxmgr())
    }

    /// 显式指定 oxmgr 路径（测试注入）。
    pub fn with_bin(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// `oxmgr list --json` → 进程列表。失败返回 Err（调用方打日志, C15）。
    pub fn list(&self) -> Result<Vec<OxProcess>, String> {
        let out = Command::new(&self.bin)
            .args(["list", "--json"])
            .output()
            .map_err(|e| format!("oxmgr list 执行失败 ({}): {e}", self.bin.display()))?;
        if !out.status.success() {
            return Err(format!(
                "oxmgr list 非零退出: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        parse_oxmgr_json(&String::from_utf8_lossy(&out.stdout))
    }
}

impl Default for OxmgrClient {
    fn default() -> Self {
        Self::new()
    }
}

/// 拓扑监控器：周期采集（host-agent 5s 循环）+ 快照。
pub struct TopologyMonitor {
    host_toml: String,
    grace: Duration,
    started: Instant,
    oxmgr: OxmgrClient,
}

/// 单次拓扑快照（E2/E3 数据基础）。
#[derive(Debug, Clone)]
pub struct TopologySnapshot {
    /// 期望进程名（host.yaml 声明，含固定进程与实例）。
    pub expected_processes: Vec<String>,
    /// 实际进程（oxmgr list，含状态）。
    pub actual_processes: Vec<OxProcess>,
    /// 实际活跃 topic（FrameBus 发现, alive_nodes >= 1）。
    pub actual_topics: Vec<FrameTopic>,
    /// 差异列表（原始；grace 期间由调用方抑制上报）。
    pub mismatches: Vec<Mismatch>,
    /// 启动窗口是否仍生效（D-H14）。
    pub grace_active: bool,
}

/// 默认启动窗口（D-H14: 进程启动后 N 秒内抑制缺失上报）。
pub const DEFAULT_GRACE: Duration = Duration::from_secs(15);

impl TopologyMonitor {
    /// 默认 grace 15s，起点 = 当前时刻。
    pub fn new(host_toml: String) -> Self {
        Self::new_at(host_toml, DEFAULT_GRACE, Instant::now())
    }

    /// 显式 grace（测试注入短窗口），起点 = 当前时刻。
    pub fn new_with_grace(host_toml: String, grace: Duration) -> Self {
        Self::new_at(host_toml, grace, Instant::now())
    }

    /// 显式 grace + 起点（E1 审查: host-agent 起点 = main 入口，覆盖网关慢连窗口）。
    pub fn new_at(host_toml: String, grace: Duration, started: Instant) -> Self {
        Self {
            host_toml,
            grace,
            started,
            oxmgr: OxmgrClient::new(),
        }
    }

    /// 测试用：注入 oxmgr 客户端（不依赖 daemon）。
    pub fn new_for_test(host_toml: String, grace: Duration) -> Self {
        Self {
            host_toml,
            grace,
            started: Instant::now(),
            oxmgr: OxmgrClient::with_bin(PathBuf::from("/nonexistent/oxmgr")),
        }
    }

    /// 启动窗口是否仍生效。
    pub fn grace_active(&self) -> bool {
        self.started.elapsed() < self.grace
    }

    /// 采集一次拓扑快照：期望（host.yaml）+ 实际（oxmgr + FrameBus 发现）→ diff。
    pub fn collect(&self) -> TopologySnapshot {
        let expected_processes = translate::expected_process_names(&self.host_toml)
            .unwrap_or_else(|e| {
                tracing::warn!("host.yaml 期望进程解析失败: {e}");
                Vec::new()
            });
        let expected_topics: Vec<FrameTopic> = translate::camera_configs(&self.host_toml)
            .unwrap_or_else(|e| {
                tracing::warn!("host.yaml 相机解析失败: {e}");
                Vec::new()
            })
            .into_iter()
            .map(|c| FrameTopic::new(format!("camera/{}", c.id)))
            .collect();

        let actual_processes = self.oxmgr.list().unwrap_or_else(|e| {
            tracing::warn!("oxmgr list 失败: {e}");
            Vec::new()
        });
        // 发现式实际: 仅活跃连接（alive_nodes >= 1）的 topic 计入实际态
        let actual_topics: Vec<FrameTopic> = FrameBus::list_topics()
            .unwrap_or_else(|e| {
                tracing::warn!("FrameBus 发现失败: {e}");
                Vec::new()
            })
            .into_iter()
            .filter(|t| t.alive_nodes >= 1)
            .map(|t| t.topic)
            .collect();

        let mismatches = diff(
            &expected_processes,
            &expected_topics,
            &actual_processes,
            &actual_topics,
        );
        TopologySnapshot {
            expected_processes,
            actual_processes,
            actual_topics,
            mismatches,
            grace_active: self.grace_active(),
        }
    }
}

/// PATH 查找 oxmgr；未命中回退 `$HOME/.local/bin/oxmgr`。
fn find_oxmgr() -> PathBuf {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("oxmgr");
            if cand.is_file() {
                return cand;
            }
        }
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".local/bin/oxmgr")
}
