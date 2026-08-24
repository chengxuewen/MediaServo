//! host.yaml → oxfile.toml 翻译器（Task A2 + C1）。
//!
//! 输入 host.yaml 文本，输出 OxMgr oxfile.toml 文本（`version = 1` + `[defaults]` +
//! `[[apps]]`，字段对齐官方 [OXFILE.md](https://github.com/Vladimir-Urik/OxMgr)）。
//! apps 含 7 类 host 进程 + 每 camera 一个 capturer 实例 + 每 stream 一个 streamer
//! 实例（command 参数化）。Phase A 输出占位进程骨架；C1 起 capturer 实例追加
//! `--config`/`--token` 绝对路径（`to_oxfile_in_dir`），真实命令逐 Phase 替换。

use std::path::{Path, PathBuf};

use mediaservo_common::protocol::SignalingMessage;
use serde::Deserialize;

/// host.yaml 解析模型（Phase A 子集：只需 cameras/streams 做实例化）。
#[derive(Debug, Default, Deserialize)]
struct HostConfig {
    #[serde(default)]
    cameras: Vec<Camera>,
    #[serde(default)]
    streams: Vec<Stream>,
    #[serde(default)]
    record: Option<RecordSection>,
    #[serde(default)]
    signaling: Option<SignalingSection>,
}

#[derive(Debug, Deserialize)]
struct Camera {
    id: String,
    /// 采集源（缺省 "stub"；v4l2/mipi 后接）。
    #[serde(default)]
    source: Option<String>,
    /// 帧率（缺省 30）。
    #[serde(default)]
    fps: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    id: String,
    /// 引用的相机 id（缺省 = 流 id 自身，topic camera/<id> 直连）。
    #[serde(default)]
    camera: Option<String>,
    /// 编码格式（缺省 vp8；对齐 field PublishOptions 默认）。
    #[serde(default)]
    codec: Option<String>,
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
    let cameras = camera_configs(cfg)?;
    let streams = stream_configs(cfg)?;
    record_config(cfg)?;
    signaling_local_port(cfg)?;
    signaling_room(cfg)?;
    let mut seen = std::collections::HashSet::new();
    for c in &cameras {
        check_id("相机", &c.id)?;
        if !seen.insert(c.id.clone()) {
            return Err(format!("host.yaml 解析失败: 相机 id 重复: {}", c.id));
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

pub fn oxmgr_apply(dir: &Path) -> Result<(), String> {
    let oxfile = dir.join("run").join("oxfile.toml");
    if !oxfile.exists() {
        return Err(format!("{} 不存在 — 先 write_oxfile/host apply", oxfile.display()));
    }
    let home = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).join("run").join("oxmgr");
    let daemon_port = {
        let sum: u32 = home.to_string_lossy().bytes().map(u32::from).sum();
        18000 + (sum % 400)
    };
    let out = oxmgr_cmd()
        .env("OXMGR_HOME", &home)
        .env("OXMGR_DAEMON_ADDR", format!("127.0.0.1:{daemon_port}"))
        .env("OXMGR_API_ADDR", format!("127.0.0.1:{}", daemon_port + 1000))
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
    for name in removed_apps(&live_host_apps()?, &oxfile_app_names(&oxfile)?) {
        match oxmgr_cmd().arg("delete").arg(&name).output() {
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
pub fn live_host_apps() -> Result<Vec<String>, String> {
    let out = oxmgr_cmd()
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
    let (cameras, streams) = camera_and_stream_ids(cfg)?;

    let mut out = format!(
        "version = 1\n\n[defaults]\nnamespace = \"{}\"\nrestart_policy = \"always\"\n",
        mediaservo_common::brand::media_brand().namespace
    );

    // E4 热生效: [defaults] watch = host.yaml（内容指纹）— agent/CLI 写入新配置后
    // oxmgr file-watch 重启受影响进程（进程启动时重读 host.yaml 生效）；
    // 增删 app（相机/流）由 `oxmgr apply` 增量处理（Start/Recreate），watch 兜底
    // 纯内容变更（如 fps，命令不变 → apply Noop）。cwd 是 watch 前置要求（OxMgr
    // 源码 watch_fingerprint_for_process 实证）；无路径变体（doctor）不带 watch。
    if let (Some(cwd), Some(watch)) = (
        config_path.parent().and_then(|p| p.parent()).map(|d| d.to_string_lossy().into_owned()),
        (!config_path.as_os_str().is_empty()).then(|| config_path.to_string_lossy().into_owned()),
    ) {
        out.push_str(&format!("cwd = \"{cwd}\"\nwatch = [\"{watch}\"]\nwatch_delay_secs = 1\n"));
    }
    out.push('\n');

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
        push_app(&mut out, &name, &cmd, policy);
    }
    for cam in &cameras {
        let name = instance_name(&app_name("capturer"), cam);
        let mut cmd = format!("{} --camera {}", exe_cmd(&app_name("capturer")), cam);
        if !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
                " --config {} --token {}/{}.token",
                config_path.display(),
                token_dir.display(),
                cam
            ));
        }
        push_app(&mut out, &name, &cmd, "always");
    }
    for stream in &streams {
        let name = instance_name(&app_name("streamer"), stream);
        let mut cmd = format!("{} --stream {}", exe_cmd(&app_name("streamer")), stream);
        // D2: 子进程 WS 目标 = 本地网关（[signaling] local_port 或缺省 17980）
        cmd.push_str(&format!(" --gateway {}", signaling_gateway_url(cfg)?));
        if !config_path.as_os_str().is_empty() {
            cmd.push_str(&format!(
            " --config {} --token {}/{}.token",
            config_path.display(),
            token_dir.display(),
            stream
        ));
        }
        push_app(&mut out, &name, &cmd, "always");
    }
    Ok(out)
}


/// 期望进程名列表（E1 拓扑监控期望态；与 oxfile 生成同一实例命名来源，DRY）。
pub fn expected_process_names(cfg: &str) -> Result<Vec<String>, String> {
    let (cameras, streams) = camera_and_stream_ids(cfg)?;
    let mut out: Vec<String> = FIXED_APP_BASES.iter().map(|b| app_name(b)).collect();
    // [record] enabled=false（缺省）→ host-recorder 按设计 exit 0（host-recorder.rs）
    // 且 oxmgr on_failure 不重启 → 不列入期望，否则默认配置永久 ProcessMissing 误报。
    if !record_config(cfg)?.enabled {
        out.retain(|n| n != &app_name("recorder"));
    }
    for cam in &cameras {
        out.push(instance_name(&app_name("capturer"), cam));
    }
    for stream in &streams {
        out.push(instance_name(&app_name("streamer"), stream));
    }
    Ok(out)
}

/// 提取 cameras/streams 的 id 列表（host init 生成 ros_bridge.yaml 复用，单一解析点）。
pub fn camera_and_stream_ids(cfg: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok((
        cfg.cameras.into_iter().map(|c| c.id).collect(),
        cfg.streams.into_iter().map(|s| s.id).collect(),
    ))
}

/// 相机配置（capturer 消费；source/fps 缺省 stub/30）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraConfig {
    pub id: String,
    pub source: String,
    pub fps: u32,
}

/// 解析全部相机配置（C1 capturer 用；`camera_and_stream_ids` 保持 A2/B3 消费面）。
/// fps=0 拒绝（generator.start(0) 线程内 panic → 静默挂起，C1 审查发现）。
pub fn camera_configs(cfg: &str) -> Result<Vec<CameraConfig>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    let mut out = Vec::with_capacity(cfg.cameras.len());
    for c in cfg.cameras {
        let fps = c.fps.unwrap_or(30);
        if fps == 0 {
            return Err(format!("host.yaml 解析失败: 相机 {} fps=0 无效（须 > 0）", c.id));
        }
        out.push(CameraConfig {
            id: c.id,
            source: c.source.unwrap_or_else(|| "stub".into()),
            fps,
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


/// 按 id 查单个相机配置（不存在 → Ok(None)）。
pub fn camera_config(cfg: &str, id: &str) -> Result<Option<CameraConfig>, String> {
    Ok(camera_configs(cfg)?.into_iter().find(|c| c.id == id))
}

/// 流配置（streamer 消费；camera/codec 缺省 id/vp8）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamConfig {
    pub id: String,
    /// 引用的相机 id（决定 FrameBus topic camera/<id>）。
    pub camera: String,
    /// 编码格式（对齐 field PublishOptions: vp8/h264/vp9/av1）。
    pub codec: String,
}

/// 解析全部流配置（C2 streamer 用）。
pub fn stream_configs(cfg: &str) -> Result<Vec<StreamConfig>, String> {
    let cfg: HostConfig = serde_yaml::from_str(cfg).map_err(|e| format!("host.yaml 解析失败: {e}"))?;
    Ok(cfg
        .streams
        .into_iter()
        .map(|s| {
            let id = s.id.clone();
            StreamConfig {
                id,
                camera: s.camera.unwrap_or_else(|| s.id),
                codec: s.codec.unwrap_or_else(|| "vp8".into()),
            }
        })
        .collect())
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

fn push_app(out: &mut String, name: &str, command: &str, restart_policy: &str) {
    out.push_str(&format!(
        "[[apps]]\nname = \"{name}\"\ncommand = \"{command}\"\nrestart_policy = \"{restart_policy}\"\n"
    ));
    // D-H13: 每节点日志落实例 run/logs（相对 oxfile 目录 = run/）；轮转由 OxMgr daemon
    // env 控制（OXMGR_LOG_MAX_SIZE_MB/MAX_FILES/MAX_DAYS——默认 20MB×5×14d，见 OxMgr docs）
    out.push_str(&format!(
        "[apps.logs]\nstdout = \"logs/{name}.out.log\"\nstderr = \"logs/{name}.err.log\"\n\n"
    ));
}

#[cfg(test)]
mod tests {
    // 部署配置面: [signaling] server_url/psk → host-agent --remote/--psk
    #[test]
    fn oxfile_wires_remote_server_and_psk() {
        let cfg = "host:\n  device_id: \"x\"\nsignaling:\n  server_url: \"ws://192.168.2.127:9800/ws\"\n  psk: \"prod-psk\"\ncameras:\n  - id: \"cam0\"\nstreams:\n  - id: \"cam0-stream\"\n    camera: \"cam0\"\n";
        let ox = to_oxfile(cfg).unwrap();
        assert!(ox.contains("--remote ws://192.168.2.127:9800/ws"));
        assert!(ox.contains("--psk prod-psk"));
        // 未配置 → 不生成（agent 内置默认）
        let ox2 = to_oxfile("host:\n  device_id: \"x\"\ncameras:\n  - id: \"cam0\"\nstreams:\n  - id: \"cam0-stream\"\n    camera: \"cam0\"\n").unwrap();
        assert!(!ox2.contains("--remote"));
        assert!(!ox2.contains("--psk"));
    }

    use super::*;

    const CFG_V0: &str = r#"
cameras:
  - id: "cam0"
    fps: 30

streams:
  - id: "s0"
    camera: "cam0"
"#;

    fn write_host_toml(dir: &Path, cfg: &str) {
        let etc = dir.join("etc");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("host.yaml"), cfg).unwrap();
    }

    #[test]
    fn validate_rejects_invalid_toml() {
        let err = validate("host: \"unterminated").unwrap_err();
        assert!(err.contains("解析失败"), "{err}");
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let dup_cam = "cameras:\n  - id: \"cam0\"\n  - id: \"cam0\"\n";
        assert!(validate(dup_cam).unwrap_err().contains("重复"), "相机 id 重复必须拒绝");
        let dup_stream = "streams:\n  - id: \"s0\"\n  - id: \"s0\"\n";
        assert!(validate(dup_stream).unwrap_err().contains("重复"), "流 id 重复必须拒绝");
    }

    #[test]
    fn validate_accepts_wellformed_config() {
        validate(CFG_V0).expect("合法配置应通过");
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
        assert!(ox.contains("host-capturer"), "相机实例应入 oxfile: {ox}");
        assert!(ox.contains("host-streamer"), "流实例应入 oxfile");
    }

    #[test]
    fn oxfile_watches_host_toml_with_cwd() {
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
        // 配置了 room → --room 上 oxfile
        let with_room = format!("{CFG_V0}signaling:\n  room: \"ms-car7\"\n");
        let ox = to_oxfile(&with_room).expect("to_oxfile");
        let agent_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent 命令行: {ox}");
        assert!(
            agent_line.contains("--room ms-car7"),
            "host-agent 应带 --room ms-car7: {agent_line}"
        );
        // signaling_room 解析直测
        assert_eq!(signaling_room(&with_room).unwrap().as_deref(), Some("ms-car7"));
        // 未配置 → 不 emit（agent 内置默认 "vehicle" 保持）
        let ox2 = to_oxfile(CFG_V0).expect("to_oxfile 默认");
        let agent_line2 = ox2.lines()
            .find(|l| l.contains("command") && l.contains("host-agent"))
            .expect("agent 命令行: {ox2}");
        assert!(
            !agent_line2.contains("--room"),
            "未配置 room 时不得 emit --room: {agent_line2}"
        );
        assert_eq!(signaling_room(CFG_V0).unwrap(), None);
    }

    /// H2: host-audio 必须带 --room audio-<vehicle>（音频房间约定）。
    /// [signaling] room 配置 → audio-<room>；未配置 → audio-vehicle（gateway 默认）。
    #[test]
    fn oxfile_wires_audio_room_to_host_audio() {
        // 配置了 room → --room audio-<room>
        let with_room = format!("{CFG_V0}signaling:\n  room: \"ms-deploy-car1\"\n");
        let ox = to_oxfile(&with_room).expect("to_oxfile");
        let audio_line = ox.lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio 命令行: {ox}");
        assert!(
            audio_line.contains("--room audio-ms-deploy-car1"),
            "host-audio 应带 --room audio-ms-deploy-car1: {audio_line}"
        );
        // 未配置 room → audio-vehicle（gateway 默认 "vehicle"）
        let ox2 = to_oxfile(CFG_V0).expect("to_oxfile 默认");
        let audio_line2 = ox2.lines()
            .find(|l| l.contains("command") && l.contains("host-audio"))
            .expect("host-audio 命令行: {ox2}");
        assert!(
            audio_line2.contains("--room audio-vehicle"),
            "host-audio 未配置 room 时默认 audio-vehicle: {audio_line2}"
        );
    }

    #[test]
    fn apply_config_push_updates_host_toml_backs_up_and_regenerates_oxfile() {
        let dir = tempfile::tempdir().unwrap();
        write_host_toml(dir.path(), CFG_V0);
        let cfg_v1 = "cameras:\n  - id: \"cam0\"\n    fps: 30\n  - id: \"cam1\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    camera: \"cam0\"\n";
        apply_config_push(dir.path(), &cfg_v1, 7).expect("apply_config_push");

        // host.yaml 已更新
        let now = std::fs::read_to_string(dir.path().join("etc").join("host.yaml")).unwrap();
        assert_eq!(now, cfg_v1, "host.yaml 应为新配置");
        // 备份含旧配置
        let bak = std::fs::read_to_string(dir.path().join("etc").join("host.yaml.bak-7")).unwrap();
        assert_eq!(bak, CFG_V0, "备份应为旧配置");
        // oxfile 重新生成（新相机实例）
        let ox = std::fs::read_to_string(dir.path().join("run").join("oxfile.toml")).unwrap();
        assert!(ox.contains("host-capturer-cam1"), "新相机实例应入 oxfile: {ox}");
    }

    #[test]
    fn apply_config_push_rejects_invalid_and_leaves_files_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        write_host_toml(dir.path(), CFG_V0);

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
        // 单相机也带 id 后缀 — 增相机后 cam0 的 app 名不变（身份稳定）
        let ox1 = to_oxfile(CFG_V0).unwrap();
        assert!(ox1.contains("name = \"host-capturer-cam0\""), "单相机: {ox1}");
        let v2 = "cameras:\n  - id: \"cam0\"\n    fps: 30\n  - id: \"cam1\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    camera: \"cam0\"\n";
        let ox2 = to_oxfile(&v2).unwrap();
        assert!(ox2.contains("name = \"host-capturer-cam0\""), "双相机 cam0 名不变: {ox2}");
        assert!(ox2.contains("name = \"host-capturer-cam1\""), "双相机 cam1 入 oxfile: {ox2}");
    }

    #[test]
    fn validate_rejects_non_alnum_camera_and_stream_ids() {
        // F2: TOML 合法但含引号/换行/路径穿越的 id → oxfile 畸形（push_app 未转义）
        // 或令牌路径投毒 → 必须拒绝（仅允许 [A-Za-z0-9_-]+）
        let quote_cam = "cameras:\n  - id: \"cam\\\"0\"\n";
        let err = validate(quote_cam).unwrap_err();
        assert!(err.contains("非法"), "引号相机 id 必须拒绝: {err}");
        let path_cam = "cameras:\n  - id: \"../evil\"\n";
        assert!(validate(path_cam).unwrap_err().contains("非法"), "路径穿越相机 id 必须拒绝");
        let newline_cam = "cameras:\n  - id: \"cam0\\n1\"\n";
        assert!(validate(newline_cam).unwrap_err().contains("非法"), "换行相机 id 必须拒绝");
        let quote_stream = "streams:\n  - id: \"s\\\"0\"\n";
        assert!(validate(quote_stream).unwrap_err().contains("非法"), "引号流 id 必须拒绝");
        // 正常 id（字母数字 + 连字符/下划线）通过
        validate("cameras:\n  - id: \"cam-A_1\"\nstreams:\n  - id: \"s-2_0\"\n")
            .expect("合法字符 id 应通过");
    }

    #[test]
    fn recover_config_version_returns_max_backup_version_after_restart() {
        // F1 关联契约: agent 被 [defaults].watch 重启后 config_version 必须从磁盘
        // 恢复（备份文件取最大版本），不得归零
        let dir = tempfile::tempdir().unwrap();
        write_host_toml(dir.path(), CFG_V0);
        let v7 = "cameras:\n  - id: \"cam0\"\n    fps: 30\n  - id: \"cam1\"\n    fps: 15\nstreams:\n  - id: \"s0\"\n    camera: \"cam0\"\n";
        let v10 = "cameras:\n  - id: \"cam0\"\n    fps: 30\n  - id: \"cam1\"\n    fps: 15\n  - id: \"cam2\"\n    fps: 30\nstreams:\n  - id: \"s0\"\n    camera: \"cam0\"\n";
        apply_config_push(dir.path(), &v7, 7).unwrap();
        apply_config_push(dir.path(), &v10, 10).unwrap();
        assert_eq!(recover_config_version(dir.path()), 10, "重启后应从备份恢复最大版本");
        // 无备份 → 0
        let fresh = tempfile::tempdir().unwrap();
        write_host_toml(fresh.path(), CFG_V0);
        assert_eq!(recover_config_version(fresh.path()), 0, "无备份时版本为 0");
    }

    #[test]
    fn handle_config_push_rejects_stale_or_duplicate_versions() {
        // F1 stale guard: version <= current 拒绝（审计 warn 载荷；文件不改写）
        let dir = tempfile::tempdir().unwrap();
        write_host_toml(dir.path(), CFG_V0);
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
}