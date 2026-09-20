//! 信令抽象面——session/control 统一形状，对 mock 测试（I2 语义由夹具钉）。
//!
//! 生产实现 = [`LinkSignal`]（封装 mediaservo_link::SignalSession）；
//! 测试实现 = tests/sfu_surface.rs 内 MockSignal（内存 broadcast + uplink 收集）。

use async_trait::async_trait;
use mediaservo_common::protocol::SignalingMessage;
use mediaservo_link::{SignalEvent, SignalSession};
use tokio::sync::broadcast;

use crate::error::ClientError;

/// 信令传输面——session/control 用同一 trait，测试注入 MockSignal（I2）。
#[async_trait]
pub trait Signal: Send + Sync {
    /// 发送一条上行信令消息。
    async fn send(&self, msg: SignalingMessage) -> Result<(), ClientError>;
    /// 订阅下行事件流（broadcast：多订阅者独立消费）。
    fn events(&self) -> broadcast::Receiver<SignalEvent>;
    /// 房间 ID（connect 后固定）。
    fn room_id(&self) -> &str;
    /// 本端 SFU peer key（"remote"/"consumer"/"host"）。
    fn sfu_peer_id(&self) -> &str;
    /// 协商方言版本（S0）。
    fn negotiated(&self) -> u32;
    /// 关闭会话（释放 WS 连接+后台任务）。
    async fn close(&self) -> Result<(), ClientError>;
}

/// 生产实现：封装 link::SignalSession（tokio::sync::Mutex 保 close 所有权）。
pub struct LinkSignal {
    session: tokio::sync::Mutex<Option<SignalSession>>,
    room: String,
    sfu_peer: String,
    negotiated: u32,
}

impl LinkSignal {
    pub fn new(session: SignalSession, sfu_peer: String) -> Self {
        let room = session.room_id().to_string();
        let negotiated = session.negotiated_protocol();
        Self { session: tokio::sync::Mutex::new(Some(session)), room, sfu_peer, negotiated }
    }
}

#[async_trait]
impl Signal for LinkSignal {
    async fn send(&self, msg: SignalingMessage) -> Result<(), ClientError> {
        let guard = self.session.lock().await;
        match guard.as_ref() {
            Some(s) => s.send(msg).await.map_err(ClientError::Signal),
            None => Err(ClientError::InvalidState("session closed".into())),
        }
    }

    fn events(&self) -> broadcast::Receiver<SignalEvent> {
        // std::sync::Mutex short lock — broadcast::subscribe is sync, no await
        let guard = self.session.blocking_lock();
        match guard.as_ref() {
            Some(s) => s.events(),
            None => {
                // 返回一个永不产出的 receiver（已关闭态兜底）
                let (_, rx) = broadcast::channel(1);
                rx
            }
        }
    }

    fn room_id(&self) -> &str {
        &self.room
    }

    fn sfu_peer_id(&self) -> &str {
        &self.sfu_peer
    }

    fn negotiated(&self) -> u32 {
        self.negotiated
    }

    async fn close(&self) -> Result<(), ClientError> {
        let session = self.session.lock().await.take();
        if let Some(s) = session {
            s.close().await.map_err(ClientError::Signal)?;
        }
        Ok(())
    }
}
