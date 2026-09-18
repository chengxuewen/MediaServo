//! link 信令面 napi 绑定 — SignalSession（事件经 ThreadsafeFunction → JS 主线程，
//! livekit async_queue 同构：Rust broadcast 线程 → tsfn → JS 回调）。

use std::sync::Arc;

use mediaservo_common::protocol::{PeerRole, SignalingMessage};
use mediaservo_link::{SignalClient, SignalEvent, SignalSession};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

/// 信令配置（role: "Host"/"Pusher"→Host, "Client"/"Puller"→Remote; 默认 Host）。
#[napi(object)]
pub struct JsSignalConfig {
    pub url: String,
    pub psk: String,
    pub room: String,
    pub role: Option<String>,
}

fn closed_err() -> napi::Error {
    napi::Error::from_reason("session closed")
}

fn event_to_json(ev: &SignalEvent) -> String {
    use mediaservo_link::SignalEvent::*;
    match ev {
        Connected { room_id } => serde_json::json!({"type": "connected", "room_id": room_id}),
        Message(m) => serde_json::json!({"type": "message", "message": m}),
        Disconnected { reason } => serde_json::json!({"type": "disconnected", "reason": reason}),
        Error(e) => serde_json::json!({"type": "error", "error": e}),
        _ => serde_json::json!({"type": "unknown"}),
    }
    .to_string()
}

/// 信令会话（async；事件订阅经 onEvent 回调——C 泵线程安全转发 JS 主线程）。
#[napi]
pub struct JsSignalSession {
    inner: Arc<tokio::sync::Mutex<Option<SignalSession>>>,
}

// SAFETY: 所有方法经 tokio Mutex 序列化（field-c 同款先例）。
unsafe impl Send for JsSignalSession {}
unsafe impl Sync for JsSignalSession {}

#[napi]
impl JsSignalSession {
    /// 连接信令 + 加入房间（async，PSK 认证内建）。
    #[napi(factory)]
    pub async fn connect(cfg: JsSignalConfig) -> Result<Self> {
        let role = match cfg.role.as_deref().unwrap_or("Host") {
            "Host" | "Pusher" => PeerRole::Host,
            "Client" | "Puller" => PeerRole::Remote,
            other => return Err(napi::Error::from_reason(format!("unknown role: {other}"))),
        };
        let client = SignalClient::new(&cfg.url, &cfg.psk, &cfg.room, role);
        let session = client
            .connect()
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect: {e}")))?;
        Ok(Self { inner: Arc::new(tokio::sync::Mutex::new(Some(session))) })
    }

    /// 发送信令消息（JSON 字符串，SignalingMessage serde 格式）。
    #[napi]
    pub async fn send(&self, json: String) -> Result<()> {
        let msg: SignalingMessage = serde_json::from_str(&json)
            .map_err(|e| napi::Error::from_reason(format!("invalid message: {e}")))?;
        let mut guard = self.inner.lock().await;
        guard
            .as_mut()
            .ok_or_else(closed_err)?
            .send(msg)
            .await
            .map_err(|e| napi::Error::from_reason(format!("send: {e}")))
    }

    /// 订阅事件（JSON 字符串回调；connect 后可随时注册，替换旧回调）。
    #[napi]
    #[allow(clippy::while_let_loop)] // 泵循环 match 形保留 None 分支日志钩位，不改 while-let
    pub fn on_event(&self, cb: Function<String, ()>) -> Result<()> {
        // TSFN 的 T 自动转 JS 参数（JsValuesTupleIntoVec）；无闭包 build 即可
        let tsfn = cb.build_threadsafe_function::<String>().build()?;

        let session = self.inner.clone();
        // 泵: broadcast receiver → tsfn.call（JS 主线程）；broadcast 关闭（session 关闭）后退出。
        // 同步方法无 tokio 上下文 → 用全局共享 runtime（field-c 同款模式）。
        super::event_runtime().spawn(async move {
            let (room_id, rx) = {
                let guard = session.lock().await;
                match guard.as_ref() {
                    Some(s) => (s.room_id().to_string(), Some(s.events())),
                    None => (String::new(), None),
                }
            };
            let Some(mut rx) = rx else { return };
            // Connected 事件在 connect() 内已发出（订阅前丢失）——合成补发（link-c 同款方案）
            let _ = tsfn.call(
                serde_json::json!({"type": "connected", "room_id": room_id}).to_string(),
                ThreadsafeFunctionCallMode::Blocking,
            );
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let json = event_to_json(&ev);
                        let _ = tsfn.call(json, ThreadsafeFunctionCallMode::Blocking);
                    }
                    Err(_) => break, // broadcast closed（session 关闭）
                }
            }
        });
        Ok(())
    }

    /// 关闭会话（幂等）。
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        match guard.take() {
            Some(session) => {
                session.close().await.map_err(|e| napi::Error::from_reason(format!("close: {e}")))
            }
            None => Ok(()),
        }
    }
}
