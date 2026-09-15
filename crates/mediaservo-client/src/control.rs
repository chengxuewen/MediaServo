//! 控制面 SFU DataChannel 句柄（S2 重写——旧 HMAC-P2P 形随 main.rs 一并退役）。
//!
//! wire 形 = [`ControlEnvelope`]/[`ControlAck`]（common::protocol 单一真源，
//! 与 host S1 `mediaservo-host::controller` 配对）。回执经**同 label DC 回声**
//! 回来（host `DcAckSink` 双写：ack 主路 DC + 入站 DC 回声——client 侧只需回声，
//! 免建 Recv 消费链）。零 HMAC、零 PSK 签名。

use std::collections::HashMap;
use std::time::Duration;

use mediaservo_common::protocol::{ControlAck, ControlEnvelope};
use mediaservo_webrtc::data_channel::RTCDataChannel;
use mediaservo_webrtc::RTCPeerConnection;
use tokio::sync::mpsc;

use crate::error::ClientError;

impl std::fmt::Debug for ControlChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlChannel")
            .field("labels", &self.labels)
            .field("producer_ids", &self.producer_ids)
            .finish_non_exhaustive()
    }
}

/// 一次 `open_control` 建立的出程控制通道集。
///
/// 构造仅由 [`crate::RoomSession::open_control`] 完成（transport/DC/announce 序列
/// 在 session 侧）；本类型只做收发与保活（持 pc 引用防其提前释放）。
pub struct ControlChannel {
    dcs: HashMap<String, RTCDataChannel>,
    labels: Vec<String>,
    ack_rx: mpsc::Receiver<ControlAck>,
    producer_ids: Vec<String>,
    /// 出程 transport 的 PC——句柄存活期间持有，drop 即断 DC。
    _pc: RTCPeerConnection,
}

impl ControlChannel {
    pub(crate) fn new(
        dcs: HashMap<String, RTCDataChannel>,
        labels: Vec<String>,
        ack_rx: mpsc::Receiver<ControlAck>,
        producer_ids: Vec<String>,
        pc: RTCPeerConnection,
    ) -> Self {
        Self {
            dcs,
            labels,
            ack_rx,
            producer_ids,
            _pc: pc,
        }
    }

    /// 已建立的通道 label（按 open_control 入参序）。
    #[must_use]
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// server 分配的 data producer id（审计/排查用）。
    #[must_use]
    pub fn producer_ids(&self) -> &[String] {
        &self.producer_ids
    }

    /// 在指定 label 通道发一条命令信封（seq 由调用方自增维护）。
    pub async fn send(
        &self,
        label: &str,
        seq: u64,
        cmd: &str,
        payload: serde_json::Value,
    ) -> Result<(), ClientError> {
        self.send_envelope(label, &ControlEnvelope { seq, cmd: cmd.into(), payload })
            .await
    }

    /// 信封直发（已持有 [`ControlEnvelope`] 时的低层入口）。
    pub async fn send_envelope(
        &self,
        label: &str,
        env: &ControlEnvelope,
    ) -> Result<(), ClientError> {
        let dc = self
            .dcs
            .get(label)
            .ok_or_else(|| ClientError::InvalidState(format!("control DC \"{label}\" 未开启")))?;
        let text = serde_json::to_string(env)
            .map_err(|e| ClientError::MalformedResponse(format!("envelope serialize: {e}")))?;
        dc.send_text(&text)
            .await
            .map_err(|e| ClientError::WebRtc(format!("DC {label} send: {e}")))
    }

    /// 取下一条回执（host 回声/ack 广播同队），`wait` 内无回执 →
    /// [`ClientError::Timeout`]。
    pub async fn recv_ack(&mut self, wait: Duration) -> Result<ControlAck, ClientError> {
        tokio::time::timeout(wait, self.ack_rx.recv())
            .await
            .map_err(|_| ClientError::Timeout { what: "ControlAck" })?
            .ok_or_else(|| ClientError::InvalidState("ack 流已关闭（DC 断开）".into()))
    }

    /// 取配对 `want_seq` 的回执——更早的过期 ack（丢包重发窗口的产物）丢弃跳过。
    /// `wait` = 整窗超时（非单条）。
    pub async fn recv_ack_for(&mut self, want_seq: u64, wait: Duration) -> Result<ControlAck, ClientError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remain.is_zero() {
                return Err(ClientError::Timeout { what: "ControlAck" });
            }
            let ack = tokio::time::timeout(remain, self.ack_rx.recv())
                .await
                .map_err(|_| ClientError::Timeout { what: "ControlAck" })?
                .ok_or_else(|| ClientError::InvalidState("ack 流已关闭（DC 断开）".into()))?;
            if ack.ack == want_seq {
                return Ok(ack);
            }
            tracing::debug!(got = ack.ack, want = want_seq, "过期 ack 跳过（重发窗产物）");
        }
    }
}
