//! deck 数据面 napi 绑定 — CameraSource（帧回调经 ThreadsafeFunction → JS 主线程，
//! 帧数据 I420 拼接 Buffer + meta JSON）。

use std::sync::Arc;

use mediaservo_deck::{CameraSource, CaptureOptions};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

/// 采集选项（省略字段用默认 1280x720@30）。
#[napi(object)]
pub struct JsCaptureOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub framerate: Option<u32>,
}

fn closed_err() -> napi::Error {
    napi::Error::from_reason("camera not started or closed")
}

/// 相机采集源（async；帧回调经 TSFN 线程安全转发）。
#[napi]
pub struct JsCameraSource {
    inner: Arc<tokio::sync::Mutex<Option<CameraSource>>>,
    stream: Arc<tokio::sync::Mutex<Option<mediaservo_deck::FrameStream>>>,
    opts: CaptureOptions,
}

// SAFETY: 所有方法经 tokio Mutex 序列化（field-c 同款先例）。
unsafe impl Send for JsCameraSource {}
unsafe impl Sync for JsCameraSource {}

#[napi]
impl JsCameraSource {
    /// 打开相机（当前 stub 设备 "stub:test-camera"）。
    #[napi(factory)]
    pub async fn open(dev_id: String, opts: JsCaptureOptions) -> Result<Self> {
        let mut cap = CaptureOptions::default();
        if let (Some(w), Some(h)) = (opts.width, opts.height) {
            cap.resolution = Some((w, h));
        }
        if let Some(f) = opts.framerate {
            cap.framerate = Some(f);
        }
        let source = CameraSource::open(mediaservo_deck::DeviceId(dev_id), cap.clone())
            .map_err(|e| napi::Error::from_reason(format!("open: {e}")))?;
        Ok(Self {
            inner: Arc::new(tokio::sync::Mutex::new(Some(source))),
            stream: Arc::new(tokio::sync::Mutex::new(None)),
            opts: cap,
        })
    }

    /// 开始产帧（只允许一次；内部生成 FrameStream）。
    #[napi]
    pub async fn start(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let source = guard.as_mut().ok_or_else(closed_err)?;
        let stream = source
            .start(&self.opts)
            .map_err(|e| napi::Error::from_reason(format!("start: {e}")))?;
        *self.stream.lock().await = Some(stream);
        Ok(())
    }

    /// 订阅帧回调（(meta_json, i420_buffer) → JS；stream 关闭后泵退出）。
    #[napi]
    #[allow(clippy::while_let_loop)] // 泵循环 match 形保留 None 分支日志钩位，不改 while-let
    pub fn on_frame(&self, cb: Function<(String, Vec<u8>), ()>) -> Result<()> {
        let tsfn = cb.build_threadsafe_function::<(String, Vec<u8>)>().build()?;
        let stream = self.stream.clone();
        super::event_runtime().spawn(async move {
            // FrameStream 非 Clone（mpsc Receiver）——take 独占；sender 全 drop 后 recv 返回 None 停泵
            let rx = { stream.lock().await.take() };
            let Some(mut rx) = rx else { return };
            loop {
                match rx.recv().await {
                    Some(frame) => {
                        // I420 三平面拼接（Y + U + V）
                        let mut data =
                            Vec::with_capacity(frame.planes.iter().map(|p| p.data.len()).sum());
                        for p in &frame.planes {
                            data.extend_from_slice(&p.data);
                        }
                        let meta = serde_json::json!({
                            "width": frame.format.width,
                            "height": frame.format.height,
                            "pts_us": frame.pts,
                            "keyframe": frame.keyframe,
                        })
                        .to_string();
                        let _ = tsfn.call((meta, data), ThreadsafeFunctionCallMode::Blocking);
                    }
                    None => break, // stream 关闭
                }
            }
        });
        Ok(())
    }

    /// 停止产帧（幂等）。
    #[napi]
    pub async fn stop(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_mut() {
            s.stop();
        }
        Ok(())
    }

    /// 关闭（幂等；停泵经 stream drop）。
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        *guard = None;
        *self.stream.lock().await = None;
        Ok(())
    }
}

/// 录制器（FFmpeg mux；record(camera) 后台任务 + stop_signal——deck-c C ABI 同款模式，
/// record 阻塞至 stop → 必须 spawn，否则 JS await 死锁）。
#[napi]
pub struct JsRecorder {
    recorder: Arc<tokio::sync::Mutex<Option<mediaservo_deck::Recorder>>>,
    stop_signal: Arc<tokio::sync::Mutex<Option<mediaservo_deck::record::StopSignal>>>,
    #[allow(dead_code)]
    task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[napi]
impl JsRecorder {
    /// 打开录制目标（RecordOptions 默认 h264/mp4）。
    #[napi(factory)]
    pub async fn open(path: String) -> Result<Self> {
        let rec = mediaservo_deck::Recorder::new(path, mediaservo_deck::RecordOptions::default())
            .map_err(|e| napi::Error::from_reason(format!("open: {e}")))?;
        Ok(Self {
            recorder: Arc::new(tokio::sync::Mutex::new(Some(rec))),
            stop_signal: Arc::new(tokio::sync::Mutex::new(None)),
            task: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    /// 开始录制（后台任务；立即返回。camera 须已 start 且活到 record 结束——C 契约同款）。
    #[napi]
    pub async fn record(&self, camera: &JsCameraSource) -> Result<()> {
        let stream = { camera.stream.lock().await.take() }
            .ok_or_else(|| napi::Error::from_reason("camera not started"))?;
        let mut recorder = { self.recorder.lock().await.take() }
            .ok_or_else(|| napi::Error::from_reason("recorder not open or already recording"))?;
        let signal = recorder.stop_signal();
        *self.stop_signal.lock().await = Some(signal);
        let task = super::event_runtime().spawn(async move {
            if let Err(e) = recorder.record(stream).await {
                eprintln!("napi recorder task: {e}");
            }
        });
        *self.task.lock().await = Some(task);
        Ok(())
    }

    /// 停止录制（幂等；触发 stop_signal → worker flush + trailer）。
    #[napi]
    pub fn stop(&self) -> Result<()> {
        if let Some(signal) = self
            .stop_signal
            .try_lock()
            .map_err(|_| napi::Error::from_reason("recorder busy"))?
            .take()
        {
            signal.stop();
        }
        Ok(())
    }

    /// 关闭（幂等；stop 残留 signal）。
    #[napi]
    pub async fn close(&self) -> Result<()> {
        if let Some(signal) = self.stop_signal.lock().await.take() {
            signal.stop();
        }
        *self.recorder.lock().await = None;
        Ok(())
    }
}

/// 回放器（demux+decode；onFrame 泵线程逐帧回调——next_frame 同步 API）。
#[napi]
pub struct JsPlayer {
    inner: Arc<tokio::sync::Mutex<Option<mediaservo_deck::Player>>>,
}

// SAFETY: 泵线程独占访问（next_frame &mut）经 Mutex 序列化。
unsafe impl Send for JsPlayer {}
unsafe impl Sync for JsPlayer {}

#[napi]
impl JsPlayer {
    /// 打开媒体文件（FFmpeg demux+decode）。
    #[napi(factory)]
    pub async fn open(path: String) -> Result<Self> {
        let p = mediaservo_deck::Player::open(path)
            .map_err(|e| napi::Error::from_reason(format!("open: {e}")))?;
        Ok(Self { inner: Arc::new(tokio::sync::Mutex::new(Some(p))) })
    }

    /// 逐帧回调（泵线程 next_frame → tsfn；EOF 或 None 停泵）。
    #[napi]
    pub fn on_frame(&self, cb: Function<(String, Vec<u8>), ()>) -> Result<()> {
        let tsfn = cb.build_threadsafe_function::<(String, Vec<u8>)>().build()?;
        let inner = self.inner.clone();
        std::thread::spawn(move || {
            let mut guard = match inner.try_lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let Some(player) = guard.as_mut() else { return };
            loop {
                match player.next_frame() {
                    Ok(Some(frame)) => {
                        let mut data =
                            Vec::with_capacity(frame.planes.iter().map(|p| p.data.len()).sum());
                        for p in &frame.planes {
                            data.extend_from_slice(&p.data);
                        }
                        let meta = serde_json::json!({
                            "width": frame.format.width,
                            "height": frame.format.height,
                            "pts_us": frame.pts,
                            "keyframe": frame.keyframe,
                        })
                        .to_string();
                        let _ = tsfn.call((meta, data), ThreadsafeFunctionCallMode::Blocking);
                    }
                    Ok(None) => break, // EOF
                    Err(e) => {
                        let _ = tsfn.call(
                            (
                                serde_json::json!({"type": "error", "error": e.to_string()})
                                    .to_string(),
                                Vec::new(),
                            ),
                            ThreadsafeFunctionCallMode::Blocking,
                        );
                        break;
                    }
                }
            }
        });
        Ok(())
    }

    /// 媒体时长（秒）。
    #[napi]
    pub fn duration_secs(&self) -> Result<f64> {
        let guard = self.inner.try_lock().map_err(|_| napi::Error::from_reason("player busy"))?;
        guard
            .as_ref()
            .ok_or_else(closed_err)?
            .duration_secs()
            .map_err(|e| napi::Error::from_reason(format!("duration: {e}")))
    }

    /// 关闭（幂等）。
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        *guard = None;
        Ok(())
    }
}
