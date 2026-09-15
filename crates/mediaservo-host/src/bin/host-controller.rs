//! host-controller: 控制进程（Task F1 → S1 SFU-DC 迁移）— 控制通道走 SFU SCTP。
//!
//! 用法: `host-controller [--gateway <本地网关 ws url>] [--token <FrameBus 令牌>]`
//! （缺省 `ws://127.0.0.1:17980/ws`，D2 本地网关）。
//!
//! 流程（S1 起，替代旧 P2P offer/answer 死路——all-SFU 决策后注册房间的 Sdp/ICE
//! 一律被 server 帧过滤丢弃，永等不到舱端 answer）: SignalClient 经本地网关
//! （信封 wire 无 PSK；整车 PSK 在 host-agent 远端）加入本地房间 `control`（网关
//! 拦截并重写为整车房间）→ [`mediaservo_host::controller::control_loop`]：
//! Send transport + create_data_channel × labels（chassis/gimbal/light/ack，
//! D-H3 可靠性语义）+ CreateDataProducer announce → Recv transport +
//! NewDataProducer→ConsumeData → 收令 parse_envelope → Actuator → ControlAck
//! （ack DC 主路 + 同通道回声）→ 旁路镜像 FrameBus control/cmd、control/ack。
//!
//! 失败语义（C15 + PIT-87 自愈惯例）：信令断开 / ICE Failed / 建立失败 →
//! 打日志退出 1，部署侧 restart_policy=always 拉起。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use mediaservo_common::protocol::PeerRole;
use mediaservo_host::control::{Actuator, StubActuator};
use mediaservo_host::controller::{ControllerConfig, control_loop};
use mediaservo_link::{FrameBus, SignalClient, TokenFile};

/// 本地房间名（网关拦截并重写为整车房间；子进程本地房间仅作下行改写目标）。
const ROOM: &str = "control";
/// 本地信封 src（网关子进程标识）。
const SRC: &str = "host-controller";

const USAGE: &str =
    "用法: host-controller [--gateway <本地网关 ws url>] [--token <FrameBus 令牌路径>]";

#[derive(Debug, Default)]
struct Args {
    gateway: Option<String>,
    /// FrameBus 令牌（可选镜像面：缺省/失败仅关总线旁路，不影响执行）。
    token: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    args_from(std::env::args().skip(1))
}

/// 纯参数解析（可单测）：`--gateway <url>` / `--token <path>`；缺省本地网关（D2）。
fn args_from(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = args.peekable();
    let mut out = Args::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gateway" => out.gateway = Some(args.next().ok_or("--gateway 缺值")?),
            "--token" => out.token = Some(PathBuf::from(args.next().ok_or("--token 缺值")?)),
            _ => return Err(format!("未知参数: {arg}\n{USAGE}")),
        }
    }
    Ok(out)
}

/// 令牌 → FrameBus attach（命令/回执镜像面）。任何失败仅 warn——总线是旁路，
/// 永不阻塞控制执行（deploy 面令牌签发为增强项，缺令牌 = 纯 DC 链路照常）。
fn attach_bus(token: Option<&PathBuf>) -> Option<Arc<FrameBus>> {
    let Some(path) = token else {
        tracing::info!("未提供 --token — FrameBus 镜像面关闭（控制链路不受影响）");
        return None;
    };
    let bus = (|| -> Result<FrameBus, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("读取令牌 {} 失败: {e}", path.display()))?;
        let (verifying_key, cap) =
            TokenFile::decode(&bytes).map_err(|e| format!("令牌 {} 无效: {e}", path.display()))?;
        FrameBus::attach("", &cap, &verifying_key)
            .map_err(|e| format!("FrameBus attach 失败: {e}"))
    })();
    match bus {
        Ok(b) => Some(Arc::new(b)),
        Err(e) => {
            tracing::warn!("{e} — FrameBus 镜像面关闭（控制链路不受影响）");
            None
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    mediaservo_host::init_logging("controller");
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let gateway = args
        .gateway
        .unwrap_or_else(|| "ws://127.0.0.1:17980/ws".to_string());

    // 信令：经本地网关（D2 信封 wire；网关拦截 RoomJoin 合成 RoomJoined）
    let signal = match SignalClient::new_gateway(&gateway, SRC, ROOM, PeerRole::Host)
        .connect()
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("controller: 信令连接失败: {e}");
            return ExitCode::from(1);
        }
    };
    tracing::info!(room = %signal.room_id(), "controller 已加入本地房间");

    let bus = attach_bus(args.token.as_ref());
    let actuator: Arc<dyn Actuator> = Arc::new(StubActuator);
    let code = control_loop(signal, ControllerConfig::default(), actuator, bus).await;
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_from_override_wins() {
        let gw = args_from(
            vec!["--gateway".into(), "ws://127.0.0.1:18888/ws".into()].into_iter(),
        )
        .unwrap()
        .gateway
        .unwrap();
        assert_eq!(gw, "ws://127.0.0.1:18888/ws");
    }

    #[test]
    fn gateway_from_defaults_to_local_gateway() {
        // 无参数 → 缺省本地网关（D2）；token 缺省 None
        let args = args_from(vec![].into_iter()).unwrap();
        assert_eq!(
            args.gateway.unwrap_or_else(|| "ws://127.0.0.1:17980/ws".to_string()),
            "ws://127.0.0.1:17980/ws"
        );
        assert_eq!(args.token, None);
    }

    #[test]
    fn token_flag_parsed() {
        let args = args_from(
            vec!["--gateway".into(), "ws://h/ws".into(), "--token".into(), "t.bin".into()]
                .into_iter(),
        )
        .unwrap();
        assert_eq!(args.token, Some(PathBuf::from("t.bin")));
    }

    #[test]
    fn args_from_rejects_unknown_arg() {
        assert!(args_from(vec!["--bogus".into(), "x".into()].into_iter()).is_err());
    }

    #[test]
    fn args_from_requires_values() {
        assert!(args_from(vec!["--gateway".into()].into_iter()).is_err());
        assert!(args_from(vec!["--token".into()].into_iter()).is_err());
    }
}
