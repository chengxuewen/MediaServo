//! field 推流面 napi 绑定 — PushSession（async 全流程）。

use std::sync::Arc;

use mediaservo_field::{PublishOptions, PushConfig, PushSession};
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// 推流配置（对应 Rust PushConfig；width 等省略时用默认值）。
#[napi(object)]
pub struct JsPushConfig {
    pub url: String,
    pub psk: String,
    pub room: String,
    #[napi(ts_type = "number")]
    pub width: Option<u32>,
    #[napi(ts_type = "number")]
    pub height: Option<u32>,
    #[napi(ts_type = "number")]
    pub framerate: Option<u32>,
    #[napi(ts_type = "number")]
    pub bitrate_kbps: Option<u32>,
    #[napi(ts_type = "number")]
    pub keyframe_interval: Option<i64>,
}

/// 推流会话（async；内部 tokio Mutex 序列化——field-c C ABI 同款包装先例）。
#[napi]
pub struct JsPushSession {
    inner: Arc<tokio::sync::Mutex<Option<PushSession>>>,
    cfg: PushConfig,
}

// SAFETY: 所有方法经 tokio Mutex 序列化访问内部会话（field-c 同款先例，跨线程安全）。
unsafe impl Send for JsPushSession {}
unsafe impl Sync for JsPushSession {}

fn closed_err() -> napi::Error {
    napi::Error::from_reason("session closed")
}

#[napi]
impl JsPushSession {
    /// 连接信令 + 创建会话（async）。
    #[napi(factory)]
    pub async fn connect(cfg: JsPushConfig) -> Result<Self> {
        let mut push_cfg = PushConfig::new(&cfg.url, &cfg.psk, &cfg.room);
        if let Some(w) = cfg.width {
            push_cfg.width = w;
        }
        if let Some(h) = cfg.height {
            push_cfg.height = h;
        }
        if let Some(f) = cfg.framerate {
            push_cfg.framerate = f;
        }
        if let Some(b) = cfg.bitrate_kbps {
            push_cfg.bitrate_kbps = b;
        }
        if let Some(k) = cfg.keyframe_interval {
            push_cfg.keyframe_interval = k as u64;
        }
        let (session, _events) = PushSession::connect(push_cfg.clone())
            .await
            .map_err(|e| napi::Error::from_reason(format!("connect: {e}")))?;
        Ok(Self { inner: Arc::new(tokio::sync::Mutex::new(Some(session))), cfg: push_cfg })
    }

    /// 发布视频轨（SFU 协商；返回 track id）。
    #[napi]
    pub async fn publish_video(&self) -> Result<String> {
        let mut guard = self.inner.lock().await;
        let opts = PublishOptions::default();
        guard
            .as_mut()
            .ok_or_else(closed_err)?
            .publish_video(&self.cfg, &opts)
            .await
            .map_err(|e| napi::Error::from_reason(format!("publish_video: {e}")))
    }

    /// 启动视频帧生成（Squares + 时间戳水印）。
    #[napi]
    pub async fn start_video_frames(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        guard
            .as_mut()
            .ok_or_else(closed_err)?
            .start_video_frames(&self.cfg)
            .map_err(|e| napi::Error::from_reason(format!("start_video_frames: {e}")))
    }

    /// 停止视频帧生成（幂等）。
    #[napi]
    pub async fn stop_video_frames(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_mut() {
            s.stop_video_frames();
        }
        Ok(())
    }

    /// 关闭会话（幂等；此后任何调用报错）。
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
