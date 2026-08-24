//! host-agent: 信令网关（Task D1，D-H6）— 各 host 进程 WS 连本地端口，
//! agent 聚合单 WS 上 Server（一车一会话）。协议语义见 [`mediaservo_host::gateway`]。
//!
//! 用法: `host-agent [--port <本地端口>] [--remote <ws url>] [--psk <psk>] [--room <整车房间>]`
//! 缺省: 端口 17980；remote/psk 走 `SFU_E2E_WS_URL`/`SFU_E2E_PSK`（缺省
//! `ws://127.0.0.1:9800/ws` / `mediaservo-dev`，对齐 streamer/e2e 约定）；room 缺省
//! `vehicle`（D3 起由 host.yaml 配置接入）。
//!
//! E1-E3 监控（拓扑/数据流/信令 + 状态上报）统一由 [`spawn_status_reporter`]
//! 单循环驱动（E3 起替换 E1/E2 独立循环；行为与日志不变）。

use mediaservo_host::gateway::{run_gateway, GatewayConfig, GatewayHandle};
use mediaservo_host::init_logging;
use mediaservo_host::monitor::signal::{spawn_status_reporter, STATUS_INTERVAL};
use mediaservo_link::{CapabilityToken, Ed25519VerifyingKey};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
const USAGE: &str = "用法: host-agent [--port <本地端口>] [--remote <ws url>] [--psk <psk>] [--room <房间>] [--config <host.yaml>] [--token <令牌文件>]";
fn parse_args() -> Result<(GatewayConfig, Option<String>, Option<String>), String> {
    let mut cfg = GatewayConfig::default();
    let mut config_path: Option<String> = None;
    let mut token_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                cfg.local_port = args
                    .next()
                    .ok_or("--port 缺值")?
                    .parse()
                    .map_err(|e| format!("--port 无效: {e}"))?
            }
            "--remote" => cfg.remote_url = args.next().ok_or("--remote 缺值")?,
            "--psk" => cfg.psk = args.next().ok_or("--psk 缺值")?,
            "--room" => cfg.room = args.next().ok_or("--room 缺值")?,
            "--config" => config_path = Some(args.next().ok_or("--config 缺值")?),
            "--token" => token_path = Some(args.next().ok_or("--token 缺值")?),
            _ => return Err(format!("未知参数: {arg}\n{USAGE}")),
        }
    }
    // 环境变量缺省（对齐 field/e2e_sfu 外部 server 约定）
    if cfg.remote_url == "ws://127.0.0.1:9800/ws" {
        cfg.remote_url =
            std::env::var("SFU_E2E_WS_URL").unwrap_or_else(|_| cfg.remote_url.clone());
    }
    if cfg.psk == "mediaservo-dev" {
        cfg.psk = std::env::var("SFU_E2E_PSK").unwrap_or_else(|_| cfg.psk.clone());
    }
    Ok((cfg, config_path, token_path))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logging("agent");
    let (mut cfg, config_path, token_path) = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // G4: 实例目录（--config <dir>/etc/host.yaml → <dir>；oxfile/备份/身份基准）
    let config_dir = config_path.as_deref().map(PathBuf::from).and_then(|p| {
        p.parent().and_then(|etc| etc.parent().map(Path::to_path_buf))
    });
    // G4: 设备身份 — identity.json 存在则远端 Join 携带（additive；缺失/损坏 →
    // warn 回落 PSK，G2 起 server 校验设备凭证）
    if let Some(dir) = &config_dir {
        match mediaservo_host::identity::load_identity(dir) {
            Ok(Some(cred)) => {
                cfg.device = Some(cred.clone());
                tracing::info!(device_id = %cred.device_id, "已加载设备身份");
            }
            Ok(None) => tracing::warn!(path = %dir.join(mediaservo_host::identity::IDENTITY_FILE).display(), "设备身份缺失 — 远端连接走 PSK 认证（G2 起需设备凭证）"),
            Err(e) => tracing::warn!("{e} — 远端连接走 PSK 认证"),
        }
    } else {
        tracing::warn!("未提供 --config（实例目录不可推导）— 设备身份未加载，远端连接走 PSK 认证");
    }
    // E1 拓扑监控: host.yaml 期望态（缺省空 → 仅固定进程期望 + 告警）
    let host_toml = match &config_path {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %p, "host.yaml 读取失败: {e} — 拓扑期望仅固定进程");
                String::new()
            }
        },
        None => {
            tracing::warn!("未提供 --config，拓扑期望仅固定进程（capturer/streamer 期望需 host.yaml）");
            String::new()
        }
    };
    // E1 审查: grace 起点 = 进程启动（main 入口，含网关慢连窗口），非 monitor 任务启动后
    let monitor_started = std::time::Instant::now();
    let (port, gateway) = run_gateway(cfg).await.map_err(std::io::Error::other)?;
    tracing::info!(port, "host-agent 网关就绪");
    // E2 数据流监控令牌（缺省 → 跳过；部署侧 oxfile 签发 Monitor 角色令牌后启用）
    let token: Option<(CapabilityToken, Ed25519VerifyingKey)> = match &token_path {
        Some(p) => match std::fs::read(p) {
            Ok(b) => match mediaservo_link::TokenFile::decode(&b) {
                Ok((vk, tok)) => Some((tok, vk)),
                Err(e) => {
                    tracing::warn!(path = %p, "令牌无效: {e} — 数据流监控跳过");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(path = %p, "令牌读取失败: {e} — 数据流监控跳过");
                None
            }
        },
        None => {
            tracing::warn!("未提供 --token，数据流监控跳过（拓扑监控不受影响）");
            None
        }
    };
    // E1-E3: 拓扑 + 数据流 + 信令监控 + StatusReport 上报（单循环, 5s）
    // E4: 配置版本（成功应用后 = 最近 ConfigPush.version；StatusReport 关联）
    // F1: 启动时从 etc/host.yaml.bak-<version> 备份恢复——agent 被 oxfile watch
    // 重启后版本不归零（磁盘上已应用版本与上报关联契约）。
    let config_version = Arc::new(AtomicU64::new(
        config_dir
            .as_deref()
            .map(mediaservo_host::translate::recover_config_version)
            .unwrap_or(0),
    ));
    // E4: 云端配置应用循环（500ms 轮询网关待应用 ConfigPush；最新覆盖旧值）
    match &config_dir {
        Some(dir) => spawn_config_applier(gateway.clone(), dir.clone(), config_version.clone()),
        None => tracing::warn!("未提供 --config（<dir>/etc/host.yaml 不可推导）— 云端配置下发停用"),
    }
    spawn_status_reporter(host_toml, monitor_started, token, gateway, STATUS_INTERVAL, config_version);
    wait_shutdown().await;
    Ok(())
}

/// E4: 云端配置应用循环 — 轮询网关待应用 ConfigPush（500ms；最新覆盖旧值）。
/// 应用成功 → 版本记入 StatusReport.config_version；拒绝 → handle_config_push 已打
/// warn 审计（C15）；oxmgr apply 失败 → error 审计（应用已落盘，下轮进程重启自愈）。
fn spawn_config_applier(handle: GatewayHandle, dir: PathBuf, config_version: Arc<AtomicU64>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            let Some(push) = handle.take_config_push() else { continue };
            if let Ok(v) = mediaservo_host::translate::handle_config_push(
                &dir,
                config_version.load(Ordering::Relaxed),
                &push,
            ) {
                config_version.store(v, Ordering::Relaxed);
                if let Err(e) = mediaservo_host::translate::oxmgr_apply(&dir) {
                    tracing::error!(version = v, "ConfigPush oxmgr apply 失败: {e}");
                }
            } // 拒绝原因已由 handle_config_push 打 warn 审计
        }
    });
}

/// 等待 SIGINT/SIGTERM（unix 主路径；其他平台仅 ctrl_c）。
async fn wait_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("ctrl_c handler");
    }
}
