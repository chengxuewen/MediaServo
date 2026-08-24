//! MediaServo Host 库 — 多进程架构共享基建（Phase A 骨架）。
//!
//! 当前仅提供日志初始化与占位进程运行器；媒体/信令功能在后续 Phase
//! （B/C）逐步从 host-legacy 迁移至此。单进程旧实现保留于
//! `src/bin/host-legacy.rs`，Phase C 迁移完成后删除。
/// host.yaml → oxfile.toml 翻译器（Task A2）。
pub mod translate;
/// 控制平面（Task F1）：控制信封 + 执行器接口。
pub mod control;
/// 紧急停车平面（Task F2）：急停执行器闩锁 + 强审计。
pub mod emergency;
/// 初始化日志系统，role 作为首个日志事件的标识字段。
pub fn init_logging(role: &str) {
    mediaservo_common::logging::init(mediaservo_common::logging::LoggingConfig::default());
    tracing::info!(role, "host process starting");
}
/// host-agent 信令网关（Task D1）。
pub mod gateway;
/// 设备身份 identity.json 生成/加载（Task G4，D-H13）。
pub mod identity;

/// 拓扑监控（Task E1: 声明式期望 vs 发现式实际）。
pub mod monitor;

/// H2 音频会议共享工具（host-audio 进程 + 音频 e2e）: opus SDP/produce 参数/tone 生成。
pub mod audio;

/// 占位运行器：打印 `<role> placeholder ready`，随后阻塞等待

/// 占位运行器：打印 `<role> placeholder ready`，随后阻塞等待
/// SIGINT（Ctrl-C）或 SIGTERM，收到信号后返回 `Ok(())` 优雅退出。
///
/// Phase A 各进程薄壳仅以此占位，真实功能在后续 Phase 逐个替换。
pub async fn run_placeholder(role: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("{role} placeholder ready");
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }
    Ok(())
}
