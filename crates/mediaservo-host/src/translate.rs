//! host.yaml → oxfile.toml 翻译器（Task A2 + C1）。
//!
//! 输入 host.yaml 文本，输出 OxMgr oxfile.toml 文本（`version = 1` + `[defaults]` +
//! `[[apps]]`，字段对齐官方 [OXFILE.md](https://github.com/Vladimir-Urik/OxMgr)）。
//! apps 含 7 类 host 进程 + 每 source 一个 capturer 实例 + 每 stream 一个 streamer
//! 实例（command 参数化）。Phase A 输出占位进程骨架；C1 起 capturer 实例追加

use std::path::{Path, PathBuf};

use mediaservo_common::protocol::SignalingMessage;
use serde::Deserialize;

/// host.yaml 解析模型（Phase A 子集：只需 sources/streams 做实例化）。
#[derive(Debug, Default, Deserialize)]
struct HostConfig {
    /// 视频源列表（替代旧 cameras 键；旧键名 `cameras` 兼容解析）。
    #[serde(default, alias = "cameras")]
    sources: Vec<Source>,
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    record: Option<RecordSection>,
    #[serde(default)]
    signaling: Option<SignalingSection>,
}
/// 视频源逻辑类别（mode 四类；`backend`/`input` 按 mode 生效）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceMode {
    /// 真实相机采集（backend v4l2/mipi；采集后端本期未实现）。
    Camera,
    /// 测试/演示彩条生成（VideoFrameGenerator 真实产帧）。
    Generator,
    /// 屏幕采集（本期未实现）。
    Desktop,
    /// 订阅外部源（FrameBus/ROS topic；本期未实现）。
    Subscriber,
}

impl SourceMode {
    /// 配置字面（capturer 错误/ready 日志显示）。
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceMode::Camera => "camera",
            SourceMode::Generator => "generator",
            SourceMode::Desktop => "desktop",
            SourceMode::Subscriber => "subscriber",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Source {
    id: String,
    /// 逻辑类别（缺省 generator —— 旧配置无 mode 字段 = 原 stub 生成语义）。
    #[serde(default)]
    mode: Option<SourceMode>,
    /// 采集后端（仅 mode=camera 生效：v4l2 | mipi）。
    #[serde(default)]
    backend: Option<String>,
    /// 源地址统一字段：v4l2=设备路径 / subscriber=FrameBus/ROS topic / desktop=显示标识（可空）。
    #[serde(default)]
    input: Option<String>,
    /// 帧宽（缺省 1280；subscriber 可省略——帧自带元数据）。
    #[serde(default)]
    width: Option<u32>,
    /// 帧高（缺省 720；subscriber 可省略）。
    #[serde(default)]
    height: Option<u32>,
    /// 帧率（缺省 30）。
    #[serde(default)]
    fps: Option<u32>,
    /// 相机重连间隔 ms（预留；本期未消费）。
    #[serde(default)]
    reconnect_ms: Option<u64>,
    /// 旧字段（旧 `camera.source: "stub"` 兼容吞掉；非 "stub" 拒绝——迁移提示）。
    #[serde(default)]
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    id: String,
    /// 引用的源 id（缺省 = 流 id 自身，topic camera/<id> 直连）。旧键名 `camera` 兼容。
    #[serde(default, alias = "camera")]
    source: Option<String>,
    /// 编码格式（缺省 vp8；对齐 field PublishOptions 默认）。
    #[serde(default)]
    codec: Option<String>,
    /// 编码器后端（auto/software/hardware/nvenc/vaapi；缺省 auto 运行时选择）。
    #[serde(default)]
    encoder_backend: Option<String>,
    /// 编码码率 kbps（缺省 2000——field PushConfig 默认）。
    #[serde(default)]
    bitrate_kbps: Option<u32>,
    /// 弱网策略档（smooth|balanced|quality；缺省 balanced=现状行为，qos-framerate-priority）。
    #[serde(default)]
    stream_mode: Option<String>,
    /// 码率地板 kbps（显式覆盖位；缺省随 stream_mode bundle 填充）。
    #[serde(default)]
    min_bitrate_kbps: Option<u32>,
    /// 关键帧间隔秒（GOP；缺省 2——field PushConfig 默认）。
    #[serde(default)]
    keyframe_interval: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RecordSection {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    out_dir: Option<String>,
}

/// `[signaling]` 段（D1: agent 网关本地端口; I1 review: 整车房间）。
#[derive(Debug, Deserialize)]
struct SignalingSection {
    #[serde(default)]
    local_port: Option<u16>,
    /// 整车房间（D3 TODO 关闭 — host-agent --room；缺省 = agent 内置 "vehicle"）。
    #[serde(default)]
    room: Option<String>,
    /// 远程 MediaServo server WS URL（缺省 agent 内置 ws://127.0.0.1:9800/ws）。
    #[serde(default)]
    server_url: Option<String>,
    /// 信令 PSK（缺省 agent 内置 mediaservo-dev）。
    #[serde(default)]
    psk: Option<String>,
}

/// 网关本地端口（[signaling] local_port；缺省 None → agent 内置 17980）。
pub fn signaling_local_port(cfg: &str) -> Result<Option<u16>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.local_port))
}

/// 整车房间（[signaling] room；缺省 None → host-agent 内置默认 "vehicle"）。
/// D3 TODO（gateway.rs Default）关闭 — translate 负责把配置翻译进 oxfile。
pub fn signaling_room(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.room))
}

/// 远程 server URL（[signaling] server_url；缺省 None → host-agent 内置默认）。
pub fn signaling_server_url(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.server_url))
}

/// 信令 PSK（[signaling] psk；缺省 None → host-agent 内置默认）。
pub fn signaling_psk(cfg: &str) -> Result<Option<String>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok(cfg.signaling.and_then(|s| s.psk))
}

/// 子进程网关 URL（D2）：`ws://127.0.0.1:{port}/ws`，port = [signaling] local_port
/// 或缺省 17980（与 host-agent 内置默认一致）。
pub fn signaling_gateway_url(cfg: &str) -> Result<String, String> {
    let port = signaling_local_port(cfg)?.unwrap_or(crate::gateway::DEFAULT_LOCAL_PORT);
    Ok(format!("ws://127.0.0.1:{port}/ws"))
}

/// 固定 5 类进程 base 名（无品牌前缀——默认品牌映射 legacy "host-*"，见 brand.rs）。
const FIXED_APP_BASES: [&str; 5] = ["agent", "recorder", "controller", "emergency", "audio"];

/// app 名 = 品牌前缀 + base（默认 "host-" → "host-agent"；品牌化 "cp-agent"）。
fn app_name(base: &str) -> String {
    format!("{}{}", mediaservo_common::brand::media_brand().app_prefix, base)
}

/// host.yaml → oxfile.toml 文本。
///
/// 实例名一律 `类型-id`（如 `host-capturer-cam0`）——E4 配置演进下 app 身份稳定：
/// 增删相机/流不得改名既有实例（1↔N 边界 rename 会丢 oxmgr 历史 + 留残留进程）。
/// ——OxMgr validate 拒绝重复 app 名（CLI.md "duplicate app name" 硬错误）。
/// 无路径变体：capturer 实例仅 `--camera <id>`（A2 形态，doctor/测试用）。
pub fn to_oxfile(cfg: &str) -> Result<String, String> {
    to_oxfile_with_paths(cfg, Path::new(""), Path::new(""))
}

/// host.yaml → oxfile.toml，capturer 实例追加 `--config <dir>/etc/host.yaml`
/// 与 `--token <dir>/etc/link/<cam>.token` 绝对路径（Task C1）。
pub fn to_oxfile_in_dir(cfg: &str, dir: &Path) -> Result<String, String> {
    let config_path = std::path::absolute(dir.join("etc").join("host.yaml"))
        .unwrap_or_else(|_| dir.join("etc").join("host.yaml"));
    let token_dir = std::path::absolute(dir.join("etc").join("link"))
        .unwrap_or_else(|_| dir.join("etc").join("link"));
    to_oxfile_with_paths(cfg, &config_path, &token_dir)
}

// ── E4 云端配置闭环: 校验/备份/写入/apply 共享实现（host CLI 与 host-agent 同源） ──

/// 校验 host.yaml（与各消费进程同一解析路径：camera/stream/record/signaling；
/// 另含重复 id 守卫——重复 → oxfile app 名重复 → oxmgr apply 硬错误）。
/// F2: id 字符集守卫 [A-Za-z0-9_-]+ —— 非法字符（引号/换行/路径穿越）会产出
/// 畸形 oxfile（push_app 未转义）或投毒令牌路径，必须拒绝。
pub fn validate(cfg: &str) -> Result<(), String> {
    let sources = camera_configs(cfg)?;
    let streams = stream_configs(cfg)?;
    record_config(cfg)?;
    signaling_local_port(cfg)?;
    signaling_room(cfg)?;
    let mut seen = std::collections::HashSet::new();
    for c in &sources {
        check_id("视频源", &c.id)?;
        if !seen.insert(c.id.clone()) {
            return Err(format!("host.yaml 解析失败: 视频源 id 重复: {}", c.id));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for s in &streams {
        check_id("流", &s.id)?;
        if !seen.insert(s.id.clone()) {
            return Err(format!("host.yaml 解析失败: 流 id 重复: {}", s.id));
        }
    }
    Ok(())
}

/// F2: id 字符集守卫——仅允许 [A-Za-z0-9_-]+（oxfile app 名/令牌文件名/
/// FrameBus topic 均直接拼入 id，非法字符导致畸形输出或路径穿越）。
fn check_id(kind: &str, id: &str) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(format!(
            "host.yaml 解析失败: {kind} id 非法: {id:?}（仅允许 [A-Za-z0-9_-]+）"
        ));
    }
    Ok(())
}

/// 原子写（tmp + rename）— oxmgr file-watch 只见完整文件，读端不见半写状态。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

/// 校验 + 翻译 + 原子写 run/oxfile.toml，返回 oxfile 路径。
pub fn write_oxfile(cfg: &str, dir: &Path) -> Result<PathBuf, String> {
    validate(cfg)?;
    let ox = to_oxfile_in_dir(cfg, dir)?;
    let run_dir = dir.join("run");
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("创建 {} 失败: {e}", run_dir.display()))?;
    let oxfile = run_dir.join("oxfile.toml");
    atomic_write(&oxfile, &ox)?;
    Ok(oxfile)
}

/// F1: 从 etc/host.yaml.bak-<version> 备份恢复最近应用版本（取最大版本）。
/// agent 被 oxfile [defaults].watch 重启后 config_version 不归零——磁盘上已应用
/// 的版本与 StatusReport.config_version 的关联契约。无备份 → 0。
pub fn recover_config_version(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir.join("etc")) else { return 0 };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_prefix("host.yaml.bak-")
                .and_then(|v| v.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0)
}

/// 备份当前 host.yaml → etc/host.yaml.bak-<version>（ConfigPush 应用前置）。
/// 备份当前 host.yaml → etc/host.yaml.bak-<version>（ConfigPush 应用前置）。
pub fn backup_host_config(dir: &Path, version: u64) -> Result<PathBuf, String> {
    let cfg_path = dir.join("etc").join("host.yaml");
    let bak = dir.join("etc").join(format!("host.yaml.bak-{version}"));
    std::fs::copy(&cfg_path, &bak).map_err(|e| format!("备份 {} → {} 失败: {e}", cfg_path.display(), bak.display()))?;
    Ok(bak)
}

/// E4 ConfigPush 应用（agent 侧；纯进程内可单测）：校验 → 备份 → 写 host.yaml →
/// 重生成 oxfile。任一失败返回 Err（拒绝原因；文件未被部分改写——校验先行）。
pub fn apply_config_push(dir: &Path, config: &str, version: u64) -> Result<(), String> {
    validate(config)?;
    backup_host_config(dir, version)?;
    atomic_write(&dir.join("etc").join("host.yaml"), config)?;
    write_oxfile(config, dir)?;
    Ok(())
}

/// oxmgr apply <run/oxfile.toml>（CLI 与 agent 共享；增量: 新增 Start/变更 Recreate/未变 Noop）。
/// oxmgr 二进制：host CLI 同目录优先（install 打包于 bin/），回落 PATH。
fn oxmgr_cmd() -> std::process::Command {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("oxmgr")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("oxmgr"));
    std::process::Command::new(bin)
}

/// 实例 daemon env（OXMGR_HOME 派生端口隔离 daemon——多实例/测试并行不互斥）。
/// oxmgr_apply 与 live_host_apps/delete 必须同源，否则 list 连默认 daemon 为空。
pub fn oxmgr_env(dir: &Path) -> Vec<(String, String)> {
    let home = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
    let sum: u32 = home.to_string_lossy().bytes().map(u32::from).sum();
    let port = 18000 + (sum % 400);
    vec![
        ("OXMGR_HOME".to_string(), home.to_string_lossy().into_owned()),
        ("OXMGR_DAEMON_ADDR".to_string(), format!("127.0.0.1:{port}")),
        ("OXMGR_API_ADDR".to_string(), format!("127.0.0.1:{}", port + 1000)),
    ]
}

fn oxmgr_cmd_with_env(env: &[(String, String)]) -> std::process::Command {
    let mut cmd = oxmgr_cmd();
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

pub fn oxmgr_apply(dir: &Path) -> Result<(), String> {
    let oxfile = dir.join("run").join("oxfile.toml");
    if !oxfile.exists() {
        return Err(format!("{} 不存在 — 先 write_oxfile/host apply", oxfile.display()));
    }
    let env = oxmgr_env(dir);
    let out = oxmgr_cmd_with_env(&env)
        .arg("apply")
        .arg(&oxfile)
        .output()
        .map_err(|e| format!("oxmgr 执行失败: {e} — 请先安装 OxMgr 并加入 PATH"))?;
    if !out.status.success() {
        return Err(format!(
            "oxmgr apply 失败 (exit {}): {}",
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // 删除路径: 清理不再存在于新 oxfile 的 host 命名空间 app。oxmgr apply 默认只
    // 增量不动缺省 app，而 --prune 是全量跨命名空间（实证会误杀其他工具 app）——
    // 故按名逐个 delete 精确同步。
    for name in removed_apps(&live_host_apps(&env)?, &oxfile_app_names(&oxfile)?) {
        match oxmgr_cmd_with_env(&env).arg("delete").arg(&name).output() {
            Ok(out) if out.status.success() => {
                tracing::info!(app = %name, "oxmgr delete 已移除配置外 app");
            }
            Ok(out) => tracing::warn!(
                app = %name,
                "oxmgr delete 失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => tracing::warn!(app = %name, "oxmgr delete 执行失败: {e}"),
        }
    }
    Ok(())
}

/// 待删除 app = live host 命名空间 − 新 oxfile 内 app（纯函数，可单测）。
fn removed_apps(live: &[String], desired: &[String]) -> Vec<String> {
    live.iter().filter(|n| !desired.contains(n)).cloned().collect()
}

/// oxmgr list --json 中 host 命名空间 app 名列表（removal 清理 + host stop 兜底）。
/// env 与 oxmgr_apply 同源（per-instance daemon 端口隔离）——不带 env 会连默认 daemon
/// （空列表 → 删除路径静默失效，PIT-121 同类实证）。
pub fn live_host_apps(env: &[(String, String)]) -> Result<Vec<String>, String> {
    let out = oxmgr_cmd_with_env(env)
        .args(["list", "--json"])
        .output()
        .map_err(|e| format!("oxmgr 执行失败: {e} — 请先安装 OxMgr 并加入 PATH"))?;
    if !out.status.success() {
        return Err(format!(
            "oxmgr list 失败 (exit {}): {}",
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let val: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("oxmgr list --json 解析失败: {e}"))?;
    Ok(val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|p| p.get("namespace").and_then(|n| n.as_str()) == Some(mediaservo_common::brand::media_brand().namespace))
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// 解析 oxfile [[apps]].name（removal 对比的期望侧）。
fn oxfile_app_names(path: &Path) -> Result<Vec<String>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let val: toml::Value = toml::from_str(&text).map_err(|e| format!("解析 {} 失败: {e}", path.display()))?;
    Ok(val
        .get("apps")
        .and_then(|a| a.as_array())
        .map(|apps| {
            apps.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// `host apply` 共享实现: 读 host.yaml → 校验 → 翻译 → 写 oxfile → oxmgr apply。
pub fn apply_config(dir: &Path) -> Result<(), String> {
    let cfg = std::fs::read_to_string(dir.join("etc").join("host.yaml"))
        .map_err(|e| format!("读取 host.yaml 失败: {e} — 先运行 host init <dir>"))?;
    write_oxfile(&cfg, dir)?;
    oxmgr_apply(dir)
}

/// E4 agent 应用体（应用 + 审计日志；oxmgr apply 由调用方执行——测试/CLI 可分离）。
/// 返回成功应用的版本号；失败 Err(拒绝原因)（审计日志已打 warn，C15）。
/// F1 stale guard: version <= current_version 拒绝（旧/重复下发；审计 warn 载荷）。
pub fn handle_config_push(
    dir: &Path,
    current_version: u64,
    push: &SignalingMessage,
) -> Result<u64, String> {
    let SignalingMessage::ConfigPush { config, version, .. } = push else {
        return Err("网关待应用消息非 ConfigPush".into());
    };
    if *version <= current_version {
        return Err(format!(
            "ConfigPush 版本 {version} 已过期（当前 {current_version}）— 拒绝旧/重复下发"
        ));
    }
    match apply_config_push(dir, config, *version) {
        Ok(()) => {
            tracing::info!(
                version,
                dir = %dir.display(),
                "ConfigPush 应用成功 — host.yaml 已更新 + oxfile 已重生成"
            );
            Ok(*version)
        }
        Err(e) => {
            tracing::warn!(version, "ConfigPush 拒绝: {e} — host.yaml 未变更");
            Err(e)
        }
    }
}


fn to_oxfile_with_paths(cfg: &str, config_path: &Path, token_dir: &Path) -> Result<String, String> {
    // capturer 实例需完整源配置（reconnect_ms 透传）；流仅需 id。
    let sources = camera_configs(cfg)?;
    let (_, streams) = camera_and_stream_ids(cfg)?;

    let mut out = format!(
        "version = 1\n\n[defaults]\nnamespace = \"{}\"\nrestart_policy = \"always\"\n",
        mediaservo_common::brand::media_brand().namespace
    );

    // E4 热生效: [defaults] watch = host.yaml（内容指纹）— agent/CLI 写入新配置后
    // oxmgr file-watch 重启受影响进程（进程启动时重读 host.yaml 生效）；
    // 增删 app（相机/流）由 `oxmgr apply` 增量处理（Start/Recreate），watch 兜底
    // 纯内容变更（如 fps，命令不变 → apply Noop）。cwd 是 watch 前置要求（OxMgr
    // 源码 watch_fingerprint_for_process 实证）；无路径变体（doctor）不带 watch。
    let inst_root = config_path
        .parent()
        .and_then(|p| p.parent())
        .map(|d| d.to_string_lossy().into_owned());
    if let (Some(cwd), Some(watch)) = (
        inst_root.as_deref(),
        (!config_path.as_os_str().is_empty()).then(|| config_path.to_string_lossy().into_owned()),
    ) {
        out.push_str(&format!("cwd = \"{cwd}\"\nwatch = [\"{watch}\"]\nwatch_delay_secs = 1\n"));
    }
    out.push('\n');
    // PIT: 日志路径绝对化（实例 run/logs）——OxMgr 按 daemon cwd 解析相对路径，残留 daemon
    // 复用错位（cwd 指向已删临时目录）会 failed to create ./run/logs；绝对路径与 daemon cwd 无关。
    let log_dir = inst_root.as_deref().map(|d| format!("{d}/run/logs"));

    for base in FIXED_APP_BASES {
        let name = app_name(base);
        let mut cmd = exe_cmd(&name);
        // C3: recorder 固定 app 与 capturer/streamer 同形追加 --config/--token
        // （订阅 camera/* 录制; 令牌文件 recorder.token）。
        if name == app_name("recorder") && !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
                " --config {} --token {}/recorder.token",
                config_path.display(),
                token_dir.display()
            ));
        }
        // D1: agent 网关本地端口（[signaling] local_port 配置；缺省 agent 内置 17980）
        // E1: agent 追加 --config（拓扑监控期望态数据源，与 recorder 同形）。
        // G1: agent 追加 --token agent.token（Monitor ACL）——数据流监控不再 tokenless。
        if name == app_name("agent") {
            if let Some(port) = signaling_local_port(cfg)? {
                cmd.push_str(&format!(" --port {port}"));
            }
            // I1 review (D3 TODO 关闭): [signaling] room → --room（缺省 agent 内置 "vehicle"）
            if let Some(room) = signaling_room(cfg)? {
                cmd.push_str(&format!(" --room {room}"));
            }
            // 远程 server 配置: [signaling] server_url/psk → --remote/--psk（缺省 agent 内置）
            if let Some(url) = signaling_server_url(cfg)? {
                cmd.push_str(&format!(" --remote {url}"));
            }
            if let Some(psk) = signaling_psk(cfg)? {
                cmd.push_str(&format!(" --psk {psk}"));
            }
            if !config_path.as_os_str().is_empty() {
                cmd.push_str(&format!(" --config {}", config_path.display()));
                cmd.push_str(&format!(" --token {}/agent.token", token_dir.display()));
            }
        }
        // C4: recorder [record] enabled=false 时按设计 exit 0 — 在 oxmgr
        // restart_policy=always 下会重启风暴; 改 on_failure（崩溃重启，干净退出不重启）。
        let policy = if name == app_name("recorder") { "on_failure" } else { "always" };
        // H2: host-audio 必须带 --room audio-<room>（[signaling] room 或缺省 vehicle）。
        if name == app_name("audio") {
            let room = signaling_room(cfg)?.unwrap_or_else(|| "vehicle".to_string());
            cmd.push_str(&format!(" --room audio-{room}"));
        }
        push_app(&mut out, &name, &cmd, policy, log_dir.as_deref());
    }
    for src in &sources {
        let name = instance_name(&app_name("capturer"), &src.id);
        let mut cmd = format!("{} --camera {}", exe_cmd(&app_name("capturer")), src.id);
        // T7: mode=camera 且配置 reconnect_ms → 透传 --reconnect-ms（generator 不加）
        if src.mode == SourceMode::Camera
            && let Some(ms) = src.reconnect_ms
        {
            cmd.push_str(&format!(" --reconnect-ms {ms}"));
        }
        if !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
                " --config {} --token {}/{}.token",
                config_path.display(),
                token_dir.display(),
                src.id
            ));
        }
        push_app(&mut out, &name, &cmd, "always", log_dir.as_deref());
    }
    // 流 id → 配置映射（编码参数透传；streams 循环是 id 列表）
    let stream_cfgs: std::collections::HashMap<String, StreamConfig> =
        stream_configs(cfg)?.into_iter().map(|s| (s.id.clone(), s)).collect();
    for stream in &streams {
        let name = instance_name(&app_name("streamer"), stream);
        let mut cmd = format!("{} --stream {}", exe_cmd(&app_name("streamer")), stream);
        // D2: 子进程 WS 目标 = 本地网关（[signaling] local_port 或缺省 17980）
        cmd.push_str(&format!(" --gateway {}", signaling_gateway_url(cfg)?));
        // 编码参数透传（streams 配置面——encoder_backend/bitrate/gop；缺省 streamer 侧回落）
        if let Some(sc) = stream_cfgs.get(stream) {
            if let Some(b) = &sc.encoder_backend {
                cmd.push_str(&format!(" --encoder-backend {b}"));
            }
            // qos-framerate-priority AD-2：唯一合并裁决点——显式键 > stream_mode bundle；
            // stream_mode 合法性已在 stream_configs() 拦截，此处 expect 仅防舞。
            let mode = sc
                .stream_mode
                .as_deref()
                .map(|m| m.parse::<mediaservo_field::StreamMode>().expect("validated in stream_configs"))
                .unwrap_or(mediaservo_field::StreamMode::Balanced);
            let bundle = mode.bundle();
            let bitrate = sc.bitrate_kbps.or(bundle.bitrate_kbps);
            let min_bitrate = sc.min_bitrate_kbps.or(bundle.min_bitrate_kbps);
            use mediaservo_webrtc::rtp::{RTCDegradationPreference as Deg, RTCRtpContentHint as Hint};
            if bundle.degradation != Deg::Balanced {
                let d = match bundle.degradation {
                    Deg::Fixed => "fixed",
                    Deg::MaintainFramerate => "framerate",
                    Deg::MaintainResolution => "resolution",
                    Deg::Balanced => "balanced",
                };
                cmd.push_str(&format!(" --degradation {d}"));
            }
            if bundle.content_hint == Hint::Fluid {
                cmd.push_str(" --content-hint fluid");
            }
            if let Some(k) = bitrate {
                cmd.push_str(&format!(" --bitrate-kbps {k}"));
            }
            if let Some(k) = min_bitrate {
                cmd.push_str(&format!(" --min-bitrate-kbps {k}"));
            }
            if let Some(g) = sc.keyframe_interval {
                cmd.push_str(&format!(" --keyframe-interval {g}"));
            }
        }
        if !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
            " --config {} --token {}/{}-stream.token",
            config_path.display(),
            token_dir.display(),
            stream
        ));
        }
        push_app(&mut out, &name, &cmd, "always", log_dir.as_deref());
    }
    Ok(out)
}


/// 期望进程名列表（E1 拓扑监控期望态；与 oxfile 生成同一实例命名来源，DRY）。
pub fn expected_process_names(cfg: &str) -> Result<Vec<String>, String> {
    let (sources, streams) = camera_and_stream_ids(cfg)?;
    let mut out: Vec<String> = FIXED_APP_BASES.iter().map(|b| app_name(b)).collect();
    // [record] enabled=false（缺省）→ host-recorder 按设计 exit 0（host-recorder.rs）
    // 且 oxmgr on_failure 不重启 → 不列入期望，否则默认配置永久 ProcessMissing 误报。
    if !record_config(cfg)?.enabled {
        out.retain(|n| n != &app_name("recorder"));
    }
    for src in &sources {
        out.push(instance_name(&app_name("capturer"), src));
    }
    for stream in &streams {
        out.push(instance_name(&app_name("streamer"), stream));
    }
    Ok(out)
}

/// 提取 sources/streams 的 id 列表（host init 生成 ros_bridge.yaml 复用，单一解析点）。
pub fn camera_and_stream_ids(cfg: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok((
        cfg.sources.into_iter().map(|c| c.id).collect(),
        cfg.streams.into_iter().map(|s| s.id).collect(),
    ))
}

/// 源配置（capturer/recorder/monitor 消费；mode/width/height/fps 已落默认值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceConfig {
    pub id: String,
    pub mode: SourceMode,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// v4l2 设备路径（mode=camera 必填；其余 mode 可空）。
    pub input: Option<String>,
    /// 采集失败重连间隔 ms（mode=camera 透传 capturer --reconnect-ms；缺省 capturer 侧 5000）。
    pub reconnect_ms: Option<u64>,
}

/// 帧宽缺省（width 未配置；原 capturer DEFAULT_WIDTH）。
const DEFAULT_SOURCE_WIDTH: u32 = 1280;
/// 帧高缺省（height 未配置；原 capturer DEFAULT_HEIGHT）。
const DEFAULT_SOURCE_HEIGHT: u32 = 720;

/// 解析全部视频源配置（原职能 camera_configs——函数名保留，消费面不变）。
/// 旧 `source` 字段非 "stub" 拒绝（迁移提示）；fps=0 拒绝（generator.start(0) 线程内
/// panic → 静默挂起，C1 审查发现）。
pub fn camera_configs(cfg: &str) -> Result<Vec<SourceConfig>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    let mut out = Vec::with_capacity(cfg.sources.len());
    for c in cfg.sources {
        if let Some(src) = &c.source
            && src != "stub"
        {
            return Err(format!(
                "host.yaml 解析失败: 视频源 {} 旧字段 source={src} 未支持（仅 stub；v4l2/mipi 请改用 mode: camera + backend）",
                c.id
            ));
        }
        let fps = c.fps.unwrap_or(30);
        if fps == 0 {
            return Err(format!("host.yaml 解析失败: 视频源 {} fps=0 无效（须 > 0）", c.id));
        }
        if fps > 60 {
            return Err(format!("host.yaml 解析失败: 视频源 {} fps={fps} 超上限（<=60）", c.id));
        }
        out.push(SourceConfig {
            id: c.id,
            // 旧配置无 mode → generator（原 stub 生成语义）
            mode: c.mode.unwrap_or(SourceMode::Generator),
            width: c.width.unwrap_or(DEFAULT_SOURCE_WIDTH),
            height: c.height.unwrap_or(DEFAULT_SOURCE_HEIGHT),
            fps,
            input: c.input,
            reconnect_ms: c.reconnect_ms,
        });
    }
    Ok(out)
}
/// 录制配置（recorder 进程消费；[record] 段缺省 disabled + 默认输出目录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordConfig {
    pub enabled: bool,
    pub out_dir: PathBuf,
}

/// 默认录制输出目录（host.yaml [record] out_dir 可覆盖；开发缺省在 /tmp）。
const DEFAULT_RECORD_DIR: &str = "/tmp/mediaservo-recordings";

/// 解析录制配置（C3 recorder 用）。缺省: disabled + /tmp/mediaservo-recordings。
pub fn record_config(cfg: &str) -> Result<RecordConfig, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    let rec = cfg.record.unwrap_or(RecordSection { enabled: None, out_dir: None });
    Ok(RecordConfig {
        enabled: rec.enabled.unwrap_or(false),
        out_dir: PathBuf::from(rec.out_dir.unwrap_or_else(|| DEFAULT_RECORD_DIR.to_string())),
    })
}


/// 按 id 查单个视频源配置（不存在 → Ok(None)）。
pub fn camera_config(cfg: &str, id: &str) -> Result<Option<SourceConfig>, String> {
    Ok(camera_configs(cfg)?.into_iter().find(|c| c.id == id))
}

/// 流配置（streamer 消费；source/codec 缺省 id/vp8）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamConfig {
    pub id: String,
    /// 引用的源 id（决定 FrameBus topic camera/<id>）。
    pub source: String,
    /// 编码格式（对齐 field PublishOptions: vp8/h264/vp9/av1）。
    pub codec: String,
    /// 编码器后端（auto/software/hardware/nvenc/vaapi；None=auto）。
    pub encoder_backend: Option<String>,
    /// 编码码率 kbps（None=field 默认 2000）。
    pub bitrate_kbps: Option<u32>,
    /// 弱网策略档原文（None=balanced；合法性在 stream_configs() 校验，AD-2）。
    pub stream_mode: Option<String>,
    /// 码率地板 kbps（None=随 bundle；显式 > preset 在 oxfile 拼接处裁决）。
    pub min_bitrate_kbps: Option<u32>,
    /// 关键帧间隔秒 GOP（None=field 默认 2）。
    pub keyframe_interval: Option<u32>,
}

/// 解析全部流配置（C2 streamer 用）。
pub fn stream_configs(cfg: &str) -> Result<Vec<StreamConfig>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    cfg.streams
        .into_iter()
        .map(|s| {
            let id = s.id.clone();
            // qos-framerate-priority AD-2：非法 stream_mode 在此拦截（deploy/build 期 Err，
            // 错误信息含合法集；不进 streamer 防 oxmgr 重启风暴）。
            if let Some(m) = &s.stream_mode
                && m.parse::<mediaservo_field::StreamMode>().is_err()
            {
                return Err(format!("host.yaml 解析失败: 流 {id} stream_mode={m} 非法（合法: smooth|balanced|quality）"));
            }
            Ok(StreamConfig {
                id,
                source: s.source.unwrap_or_else(|| s.id),
                codec: s.codec.unwrap_or_else(|| "vp8".into()),
                encoder_backend: s.encoder_backend,
                bitrate_kbps: s.bitrate_kbps,
                stream_mode: s.stream_mode,
                min_bitrate_kbps: s.min_bitrate_kbps,
                keyframe_interval: s.keyframe_interval,
            })
        })
        .collect()
}

/// 按 id 查单个流配置（不存在 → Ok(None)）。
pub fn stream_config(cfg: &str, id: &str) -> Result<Option<StreamConfig>, String> {
    Ok(stream_configs(cfg)?.into_iter().find(|s| s.id == id))
}

/// 实例 app 名统一 `类型-id`（E4 稳定身份：配置增删不 rename 既有实例，
/// oxmgr 重启历史/健康记录连续；拓扑期望态同源）。
fn instance_name(kind: &str, id: &str) -> String {
    format!("{kind}-{id}")
}

/// 进程可执行文件路径：与 host CLI 同目录（同 target 产物）；测试运行时
/// current_exe 在 deps/ 下，回落裸名（test 仅断言命令行子串）。
// ponytail: 裸名依赖 PATH；部署阶段（A4 脚本/打包）再固化绝对路径。
fn exe_cmd(name: &str) -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name).to_string_lossy().into_owned()))
        .unwrap_or_else(|| name.to_string())
}

fn push_app(out: &mut String, name: &str, command: &str, restart_policy: &str, log_dir: Option<&str>) {
    out.push_str(&format!(
        "[[apps]]\nname = \"{name}\"\ncommand = \"{command}\"\nrestart_policy = \"{restart_policy}\"\n"
    ));
    // D-H13 + PIT: 日志路径绝对化（实例 run/logs）——OxMgr 按 daemon cwd 解析相对路径，
    // 残留 daemon 复用错位会 failed to create ./run/logs；无路径变体（doctor）保持相对。
    // 轮转由 OxMgr daemon env 控制（OXMGR_LOG_MAX_SIZE_MB/MAX_FILES/MAX_DAYS——默认 20MB×5×14d）
    let (stdout, stderr) = match log_dir {
        Some(d) => (format!("{d}/{name}.out.log"), format!("{d}/{name}.err.log")),
        None => (format!("logs/{name}.out.log"), format!("logs/{name}.err.log")),
    };
    out.push_str(&format!(
        "[apps.logs]\nstdout = \"{stdout}\"\nstderr = \"{stderr}\"\n\n"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 新模式配置（sources / streams.source）。
    const CFG_V0: &str = r#"
sources:
  - id: "cam0"
    mode: "generator"
    fps: 30

streams:
  - id: "s0"
    source: "cam0"
"#;

    /// 旧模式配置（cameras / source 字段 / streams.camera）——alias 兼容门。
    const CFG_LEGACY: &str = r#"
cameras:
  - id: "cam0"
    source: "stub"
    fps: 30

streams:
  - id: "s0"
    camera: "cam0"
"#;

    fn write_host_yaml(dir: &Path, cfg: &str) {
        let etc = dir.join("etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("host.yaml"), cfg).unwrap();
    }

    // ── 解析：新旧键 + 字段语义 ──

    #[test]
    fn legacy_cameras_and_camera_keys_still_parse() {
        // 已部署旧 host.yaml（cameras / source 字段 / streams.camera）无破坏地按新模式解析
        let cams = camera_configs(CFG_LEGACY).unwrap();
        assert_eq!(cams.len(), 1);
        assert_eq!(cams[0].id, "cam0");
        assert_eq!(cams[0].mode, SourceMode::Generator, "旧配置无 mode → generator（原 stub 语义）");
        assert_eq!(cams[0].width, 1280, "旧配置缺 width → 默认 1280");
        assert_eq!(cams[0].height, 720, "旧配置缺 height → 默认 720");
        assert_eq!(cams[0].fps, 30);
        let streams = stream_configs(CFG_LEGACY).unwrap();
        assert_eq!(streams[0].source, "cam0", "旧 camera 键应经 alias 解析为 source");
        assert!(validate(CFG_LEGACY).is_ok());
        assert!(to_oxfile(CFG_LEGACY).unwrap().contains("host-capturer-cam0"));
    }

    #[test]
    fn legacy_source_non_stub_rejected() {
        let old_v4l2 = "cameras:\n  - id: \"cam0\"\n    source: \"v4l2\"\n    fps: 30\n";
        let err = camera_configs(old_v4l2).unwrap_err();
        assert!(err.contains("v4l2") && err.contains("未支持"), "{err}");
    }

    #[test]
    fn source_config_resolves_mode_width_height_fps() {
        let cfg = r#"
sources:
  - id: "cam0"
    mode: "camera"
    backend: "v4l2"
    input: "/dev/video0"
    width: 1920
    height: 1080
    fps: 15
  - id: "gen0"
    mode: "generator"
    fps: 30
"#;
        let cams = camera_configs(cfg).unwrap();
        assert_eq!(cams.len(), 2);
        assert_eq!(cams[0].mode, SourceMode::Camera);
        assert_eq!(cams[0].width, 1920);
        assert_eq!(cams[0].height, 1080);
        assert_eq!(cams[0].fps, 15);
        assert_eq!(cams[1].mode, SourceMode::Generator);
        assert_eq!(cams[1].width, 1280, "缺省 width 1280");
        assert_eq!(cams[1].height, 720, "缺省 height 720");
        assert_eq!(cams[1].fps, 30);
        // 单个查找
        assert!(camera_config(cfg, "cam0").unwrap().is_some());
        assert!(camera_config(cfg, "nope").unwrap().is_none());
        assert!(camera_configs("not yaml [[[").is_err());
    }

    #[test]
    fn source_missing_mode_defaults_to_generator() {
        let cfg = "sources:\n  - id: \"s0\"\n";
        assert_eq!(camera_configs(cfg).unwrap()[0].mode, SourceMode::Generator);
    }

    #[test]
    fn camera_config_rejects_zero_fps() {
        // C1 审查发现: fps=0 → generator.start 线程内 panic → 死线程 + 主线程永久阻塞
        let cfg = "sources:\n  - id: \"cam0\"\n    fps: 0\n";
        let err = camera_configs(cfg).unwrap_err();
        assert!(err.contains("fps") && err.contains("cam0"), "{err}");
        assert!(camera_config(cfg, "cam0").is_err());
    }

    #[test]
    fn camera_config_rejects_fps_over_60() {
        // multi-stream P1: fps>60 上限（缺省 30 / 零值拒绝已有）
        let cfg = "sources:\n  - id: \"cam0\"\n    fps: 61\n";
        let err = camera_configs(cfg).unwrap_err();
        assert!(err.contains("60") && err.contains("cam0"), "{err}");
        assert!(camera_config(cfg, "cam0").is_err());
        // 边界 60 合法
        let ok = "sources:\n  - id: \"cam0\"\n    fps: 60\n";
        assert_eq!(camera_configs(ok).unwrap()[0].fps, 60);
    }

    #[test]
    fn stream_source_refers_to_sources_id() {
        // 缺省 source = 流 id 自身；显式引用 sources[].id（旧 camera 键 alias 见 legacy 测试）
        let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n  - id: \"s1\"\n    source: \"cam0\"\n    codec: \"h264\"\n";
        let streams = stream_configs(cfg).unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].source, "s0", "缺省 = 流 id 自身");
        assert_eq!(streams[0].codec, "vp8");
        assert_eq!(streams[1].source, "cam0");
        assert_eq!(streams[1].codec, "h264");
        assert!(stream_config(cfg, "s1").unwrap().is_some());
        assert!(stream_config(cfg, "nope").unwrap().is_none());
        assert!(stream_configs("not yaml [[[").is_err());
    }

    // ── 校验 ──

    #[test]
    fn validate_rejects_invalid_yaml() {
        let err = validate("host: \"unterminated").unwrap_err();
        assert!(err.contains("解析失败"), "{err}");
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let dup_src = "sources:\n  - id: \"cam0\"\n  - id: \"cam0\"\n";
        assert!(validate(dup_src).unwrap_err().contains("重复"), "sources 重复 id 必须拒绝");
        let dup_stream = "streams:\n  - id: \"s0\"\n  - id: \"s0\"\n";
        assert!(validate(dup_stream).unwrap_err().contains("重复"), "流 id 重复必须拒绝");
    }

    #[test]
    fn validate_accepts_wellformed_config() {
        validate(CFG_V0).expect("新结构合法配置应通过");
        validate(CFG_LEGACY).expect("旧结构（alias）也应通过");
    }

    #[test]
    fn validate_rejects_non_alnum_source_and_stream_ids() {
        // F2: YAML 合法但含引号/换行/路径穿越的 id → oxfile 畸形（push_app 未转义）
        // 或令牌路径投毒 → 必须拒绝（仅允许 [A-Za-z0-9_-]+）
        let quote_src = "sources:\n  - id: \"cam\\\"0\"\n";
        let err = validate(quote_src).unwrap_err();
        assert!(err.contains("非法"), "引号 source id 必须拒绝: {err}");
        let path_src = "sources:\n  - id: \"../evil\"\n";
        assert!(validate(path_src).unwrap_err().contains("非法"), "路径穿越 source id 必须拒绝");
        let newline_src = "sources:\n  - id: \"cam0\\n1\"\n";
        assert!(validate(newline_src).unwrap_err().contains("非法"), "换行 source id 必须拒绝");
        let quote_stream = "streams:\n  - id: \"s\\\"0\"\n";
        assert!(validate(quote_stream).unwrap_err().contains("非法"), "引号流 id 必须拒绝");
        // 正常 id（字母数字 + 连字符/下划线）通过
        validate("sources:\n  - id: \"cam-A_1\"\nstreams:\n  - id: \"s-2_0\"\n").expect("合法字符 id 应通过");
    }

    // ── oxfile 翻译 ──

    #[test]
    fn oxfile_wires_remote_server_and_psk() {
        let cfg = "host:\n  device_id: \"x\"\nsignaling:\n  server_url: \"ws://192.168.2.127:9800/ws\"\n  psk: \"prod-psk\"\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"cam0-stream\"\n    source: \"cam0\"\n";
        let ox = to_oxfile(cfg).unwrap();
        assert!(ox.contains("--remote ws://192.168.2.127:9800/ws"));
        assert!(ox.contains("--psk prod-psk"));
        // 未配置 → 不生成（agent 内置默认）
        let ox2 = to_oxfile("host:\n  device_id: \"x\"\nsources:\n  - id: \"cam0\"\nstreams:\n  - id: \"cam0-stream\"\n    source: \"cam0\"\n").unwrap();
        assert!(!ox2.contains("--remote"));
        assert!(!ox2.contains("--psk"));
    }

    #[test]
    fn defaults_namespace_is_concrete_not_placeholder() {
        // PIT-118 回归门: namespace 曾残留 "{ns}" 字面——oxmgr 拒收 → apply 挂起
        let ox = to_oxfile(CFG_V0).unwrap();
        assert!(ox.contains("namespace = \"mediaservo-host\""), "默认品牌 namespace 应为 mediaservo-host: {ox}");
        assert!(!ox.contains("{ns}"), "禁止占位符残留: {ox}");
    }

    #[test]
    fn write_oxfile_writes_translated_file() {
        let dir = tempfile::tempdir().unwrap();
        let oxfile = write_oxfile(CFG_V0, dir.path()).expect("write_oxfile");
        assert_eq!(oxfile, dir.path().join("run").join("oxfile.toml"));
        let ox = std::fs::read_to_string(&oxfile).unwrap();
        assert!(ox.contains("host-capturer"), "source 实例应入 oxfile: {ox}");
        assert!(ox.contains("host-streamer"), "流实例应入 oxfile");
    }

    #[test]
    fn oxfile_watches_host_yaml_with_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let ox = to_oxfile_in_dir(CFG_V0, dir.path()).expect("to_oxfile_in_dir");
        let expected_watch = format!("watch = [\"{}\"]", dir.path().join("etc").join("host.yaml").display());
        assert!(ox.contains(&expected_watch), "oxfile 应 watch host.yaml 实现热生效: {ox}");
        assert!(ox.contains("watch_delay_secs"), "watch 应带防抖: {ox}");
        assert!(ox.contains("cwd = \""), "watch 前置要求 cwd: {ox}");
        // 无路径变体（doctor 用）不带 watch
        assert!(!to_oxfile(CFG_V0).unwrap().contains("watch"), "to_oxfile 无路径变体不应 watch");
    }

    /// G1: host-agent 与 recorder 同形携带 --token（agent.token，Monitor ACL）——
    /// oxfile 全进程凭据完备，agent 数据流监控不再 tokenless。
    #[test]
    fn oxfile_wires_agent_token() {
        let dir = tempfile::tempdir().unwrap();
        let ox = to_oxfile_in_dir(CFG_V0, dir.path()).expect("to_oxfile_in_dir");
        let agent_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent 命令行: {ox}");
        assert!(
            agent_line.contains("--token") && agent_line.contains("agent.token"),
            "host-agent 应带 --token agent.token: {agent_line}"
        );
    }

    /// I1 review (D3 TODO 关闭): [signaling] room → agent --room；缺省不 emit（agent 内置 "vehicle"）。
    #[test]
    fn oxfile_wires_signaling_room_to_agent() {
        let with_room = format!("{CFG_V0}signaling:\n  room: \"ms-car7\"\n");
        let ox = to_oxfile(&with_room).expect("to_oxfile");
        let agent_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent 命令行: {ox}");
        assert!(agent_line.contains("--room ms-car7"), "host-agent 应带 --room ms-car7: {agent_line}");
        assert_eq!(signaling_room(&with_room).unwrap().as_deref(), Some("ms-car7"));
        let ox2 = to_oxfile(CFG_V0).expect("to_oxfile 默认");
        let agent_line2 = ox2.lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent 命令行: {ox2}");
        assert!(!agent_line2.contains("--room"), "未配置 room 时不得 emit --room: {agent_line2}");
        assert_eq!(signaling_room(CFG_V0).unwrap(), None);
    }

    /// H2: host-audio 必须带 --room audio-<vehicle>（音频房间约定）。
    #[test]
    fn oxfile_wires_audio_room_to_host_audio() {
        let with_room = format!("{CFG_V0}signaling:\n  room: \"ms-deploy-car1\"\n");
        let ox = to_oxfile(&with_room).expect("to_oxfile");
        let audio_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio 命令行: {ox}");
        assert!(audio_line.contains("--room audio-ms-deploy-car1"), "host-audio 应带 --room audio-ms-deploy-car1: {audio_line}");
        let ox2 = to_oxfile(CFG_V0).expect("to_oxfile 默认");
        let audio_line2 = ox2.lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio 命令行: {ox2}");
        assert!(audio_line2.contains("--room audio-vehicle"), "host-audio 未配置 room 时默认 audio-vehicle: {audio_line2}");
    }

    #[test]
    fn apply_config_push_updates_host_yaml_backs_up_and_regenerates_oxfile() {
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);
        let cfg_v1 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        apply_config_push(dir.path(), &cfg_v1, 7).expect("apply_config_push");

        // host.yaml 已更新
        let now = std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap();
        assert_eq!(now, cfg_v1, "host.yaml 应为新配置");
        // 备份含旧配置
        let bak = std::fs::read_to_string(dir.path().join("etc").join("host.yaml.bak-7")).unwrap();
        assert_eq!(bak, CFG_V0, "备份应为旧配置");
        // oxfile 重新生成（新 source 实例）
        let ox = std::fs::read_to_string(dir.path().join("run").join("oxfile.toml")).unwrap();
        assert!(ox.contains("host-capturer-cam1"), "新 source 实例应入 oxfile: {ox}");
    }

    #[test]
    fn apply_config_push_rejects_invalid_and_leaves_files_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);

        let err = apply_config_push(dir.path(), "host: \"unterminated", 9).unwrap_err();
        assert!(err.contains("解析失败"), "拒绝原因应含解析失败: {err}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap(),
            CFG_V0,
            "非法配置不得改写 host.yaml"
        );
        assert!(
            !dir.path().join("etc").join("host.yaml.bak-9").exists(),
            "非法配置不得产生备份"
        );
        assert!(
            !dir.path().join("run").join("oxfile.toml").exists(),
            "非法配置不得改写 oxfile"
        );
    }

    #[test]
    fn removed_apps_diff_live_vs_desired() {
        let live = vec!["host-agent".into(), "host-capturer-cam0".into(), "host-capturer-cam1".into()];
        let desired = vec!["host-agent".into(), "host-capturer-cam0".into()];
        assert_eq!(removed_apps(&live, &desired), vec!["host-capturer-cam1"]);
        // 全部在期望内 → 无删除
        assert!(removed_apps(&live, &live).is_empty());
    }

    #[test]
    fn instance_names_stable_across_count_change() {
        // 单源也带 id 后缀 — 增源后 cam0 的 app 名不变（身份稳定）
        let ox1 = to_oxfile(CFG_V0).unwrap();
        assert!(ox1.contains("name = \"host-capturer-cam0\""), "单源: {ox1}");
        let v2 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        let ox2 = to_oxfile(&v2).unwrap();
        assert!(ox2.contains("name = \"host-capturer-cam0\""), "双源 cam0 名不变: {ox2}");
        assert!(ox2.contains("name = \"host-capturer-cam1\""), "双源 cam1 入 oxfile: {ox2}");
    }

    #[test]
    fn recover_config_version_returns_max_backup_version_after_restart() {
        // F1 关联契约: agent 被 [defaults].watch 重启后 config_version 必须从磁盘
        // 恢复（备份文件取最大版本），不得归零
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);
        let v7 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        let v10 = "sources:\n  - id: \"cam0\"\n    mode: \"generator\"\n    fps: 30\n  - id: \"cam1\"\n    mode: \"generator\"\n    fps: 15\n  - id: \"cam2\"\n    mode: \"generator\"\n    fps: 30\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n";
        apply_config_push(dir.path(), &v7, 7).unwrap();
        apply_config_push(dir.path(), &v10, 10).unwrap();
        assert_eq!(recover_config_version(dir.path()), 10, "重启后应从备份恢复最大版本");
        // 无备份 → 0
        let fresh = tempfile::tempdir().unwrap();
        write_host_yaml(fresh.path(), CFG_V0);
        assert_eq!(recover_config_version(fresh.path()), 0, "无备份时版本为 0");
    }

    #[test]
    fn handle_config_push_rejects_stale_or_duplicate_versions() {
        // F1 stale guard: version <= current 拒绝（审计 warn 载荷；文件不改写）
        let dir = tempfile::tempdir().unwrap();
        write_host_yaml(dir.path(), CFG_V0);
        let push = |version: u64| SignalingMessage::ConfigPush {
            room_id: "r".into(),
            target: "p".into(),
            config: CFG_V0.into(),
            version,
        };
        assert_eq!(handle_config_push(dir.path(), 0, &push(7)).unwrap(), 7);
        let dup = handle_config_push(dir.path(), 7, &push(7)).unwrap_err();
        assert!(dup.contains("已过期"), "同版本重复必须拒绝: {dup}");
        let stale = handle_config_push(dir.path(), 7, &push(6)).unwrap_err();
        assert!(stale.contains("已过期"), "旧版本必须拒绝: {stale}");
        assert_eq!(handle_config_push(dir.path(), 7, &push(8)).unwrap(), 8, "新版本应接受");
        // 拒绝不得改写文件（重复推 v7 后 host.yaml 仍为初始配置）
        assert_eq!(
            std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap(),
            CFG_V0,
            "拒绝不得改写 host.yaml"
        );
    }
    /// T7: mode=camera + reconnect_ms → capturer 命令行透传 --reconnect-ms；generator 不加；
    /// 未配置 reconnect_ms 的 camera 源不 emit（capturer 侧缺省 5000）。
    #[test]
    fn oxfile_passes_reconnect_ms_for_camera_mode_only() {
        let cfg = r#"
sources:
  - id: "cam0"
    mode: "camera"
    backend: "v4l2"
    input: "/dev/video0"
    width: 1920
    height: 1080
    fps: 30
    reconnect_ms: 3000
  - id: "gen0"
    mode: "generator"
    fps: 30
    reconnect_ms: 9999
streams:
  - id: "s0"
    source: "cam0"
"#;
        let ox = to_oxfile(cfg).unwrap();
        let cam_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("--camera cam0"))
            .expect("cam0 capturer 命令行");
        assert!(cam_line.contains("--reconnect-ms 3000"), "camera 源应透传 reconnect_ms: {cam_line}");
        let gen_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("--camera gen0"))
            .expect("gen0 capturer 命令行");
        assert!(!gen_line.contains("--reconnect-ms"), "generator 源不得加 --reconnect-ms: {gen_line}");
        // 未配置 reconnect_ms 的 camera 源 → 不 emit（capturer 侧缺省 5000）
        let no_reconnect = "sources:\n  - id: \"cam1\"\n    mode: \"camera\"\n    input: \"/dev/video0\"\n";
        let ox2 = to_oxfile(no_reconnect).unwrap();
        let cam1_line = ox2.lines()
            .find(|l| l.contains("command") && l.contains("--camera cam1"))
            .expect("cam1 capturer 命令行");
        assert!(!cam1_line.contains("--reconnect-ms"), "未配置 reconnect_ms 不 emit: {cam1_line}");
    }

    /// T7: camera_configs 解析 input / reconnect_ms（缺省 None——capturer 侧回落 5000）。
    #[test]
    fn camera_config_resolves_input_and_reconnect_ms() {
        let cfg = "sources:\n  - id: \"cam0\"\n    mode: \"camera\"\n    input: \"/dev/v4l/by-id/cam0\"\n    reconnect_ms: 2500\n";
        let cams = camera_configs(cfg).unwrap();
        assert_eq!(cams[0].input.as_deref(), Some("/dev/v4l/by-id/cam0"));
        assert_eq!(cams[0].reconnect_ms, Some(2500));
        let def = "sources:\n  - id: \"cam0\"\n    mode: \"camera\"\n";
        assert_eq!(camera_configs(def).unwrap()[0].reconnect_ms, None);
        assert_eq!(camera_configs(def).unwrap()[0].input, None);
    }

    /// 编码参数透传: streams 配置 encoder_backend/bitrate_kbps/keyframe_interval → oxfile 命令追加；
    /// 未配置不 emit（streamer 侧回落 field 缺省 auto/2000/2）。
    #[test]
    fn oxfile_wires_stream_encoder_params() {
        let cfg = "sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n    codec: \"h264\"\n    encoder_backend: \"hardware\"\n    bitrate_kbps: 3500\n    keyframe_interval: 4\n  - id: \"s1\"\n    source: \"cam0\"\n";
        let ox = to_oxfile(cfg).unwrap();
        let s0_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("--stream s0"))
            .expect("s0 streamer 命令行");
        assert!(s0_line.contains("--encoder-backend hardware"), "应透传 encoder_backend: {s0_line}");
        assert!(s0_line.contains("--bitrate-kbps 3500"), "应透传 bitrate: {s0_line}");
        assert!(s0_line.contains("--keyframe-interval 4"), "应透传 gop: {s0_line}");
        let s1_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("--stream s1"))
            .expect("s1 streamer 命令行");
        assert!(!s1_line.contains("--encoder-backend"), "未配置不 emit: {s1_line}");
        assert!(!s1_line.contains("--bitrate-kbps"), "未配置不 emit: {s1_line}");
        assert!(!s1_line.contains("--keyframe-interval"), "未配置不 emit: {s1_line}");
    }

    // ── qos-framerate-priority T4: stream_mode bundle 合并/flag 下发（表驱动） ──

    fn streamer_cmd(ox: &str, stream: &str) -> String {
        ox.lines()
            .find(|l| l.contains("command") && l.contains(&format!("--stream {stream}")))
            .expect("streamer 命令行")
            .to_string()
    }

    fn qos_cfg(stream_body: &str) -> String {
        format!("sources:\n  - id: \"cam0\"\nstreams:\n  - id: \"s0\"\n    source: \"cam0\"\n{stream_body}")
    }

    #[test]
    fn oxfile_expands_smooth_bundle_flags() {
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"smooth\"\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(line.contains("--degradation framerate"), "smooth → framerate: {line}");
        assert!(line.contains("--content-hint fluid"), "smooth → Fluid: {line}");
        assert!(line.contains("--min-bitrate-kbps 400"), "smooth → min 400: {line}");
        assert!(!line.contains("--bitrate-kbps"), "smooth 不动天花板: {line}");
    }

    #[test]
    fn oxfile_expands_quality_bundle_flags() {
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"quality\"\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(line.contains("--degradation resolution"), "quality → resolution: {line}");
        assert!(line.contains("--bitrate-kbps 3000"), "quality → bundle 3000 (AD-3): {line}");
        assert!(!line.contains("--content-hint"), "quality 无 hint: {line}");
        assert!(!line.contains("--min-bitrate-kbps"), "quality 无地板: {line}");
    }

    #[test]
    fn oxfile_balanced_explicit_emits_no_qos_flags() {
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"balanced\"\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(!line.contains("--degradation"), "balanced 不发 degradation: {line}");
        assert!(!line.contains("--content-hint"), "balanced 不发 hint: {line}");
        assert!(!line.contains("--min-bitrate-kbps"), "balanced 不发 min: {line}");
        assert!(!line.contains("--bitrate-kbps"), "balanced 不填天花板: {line}");
    }

    #[test]
    fn oxfile_explicit_keys_win_bundle() {
        // 显式 bitrate/min 覆盖 bundle（smooth 写 bitrate 不被抹、quality 写 bitrate 赢 3000、
        // smooth 写 min 赢 400）
        let ox = to_oxfile(&qos_cfg("    stream_mode: \"smooth\"\n    bitrate_kbps: 2500\n    min_bitrate_kbps: 800\n")).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(line.contains("--bitrate-kbps 2500"), "显式 bitrate 赢: {line}");
        assert!(line.contains("--min-bitrate-kbps 800"), "显式 min 赢 400: {line}");
        assert!(line.contains("--degradation framerate"), "bundle 原语仍在: {line}");
        let ox2 = to_oxfile(&qos_cfg("    stream_mode: \"quality\"\n    bitrate_kbps: 5000\n")).unwrap();
        assert!(streamer_cmd(&ox2, "s0").contains("--bitrate-kbps 5000"), "quality 显式赢 bundle 3000");
    }

    #[test]
    fn oxfile_legacy_yaml_flag_sequence_unchanged() {
        // 旧 yaml（无新键）：flag 序列与 HEAD 一致——不新增 --degradation/--content-hint/
        // --min-bitrate-kbps，既有 flag（gateway/encoder-backend/bitrate/keyframe）不受影响。
        let ox = to_oxfile(CFG_V0).unwrap();
        let line = streamer_cmd(&ox, "s0");
        assert!(!line.contains("--degradation"), "现状不变: {line}");
        assert!(!line.contains("--content-hint"), "现状不变: {line}");
        assert!(!line.contains("--min-bitrate-kbps"), "现状不变: {line}");
        assert!(!line.contains("--bitrate-kbps"), "现状不变: {line}");
        assert!(line.contains("--gateway"), "既有 flag 保留: {line}");
    }

    #[test]
    fn invalid_stream_mode_rejected_at_translate() {
        let e = stream_configs(&qos_cfg("    stream_mode: \"turbo\"\n")).unwrap_err();
        assert!(e.contains("smooth|balanced|quality"), "错误信息含合法集: {e}");
        assert!(e.contains("turbo"), "错误信息含非法值: {e}");
        // to_oxfile（deploy 主路径）同步拦截
        assert!(to_oxfile(&qos_cfg("    stream_mode: \"turbo\"\n")).is_err());
    }
}
