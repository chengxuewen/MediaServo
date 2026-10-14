//! deck data-plane napi bindings — CameraSource (frame callbacks via ThreadsafeFunction → JS main thread,
//! frame data concatenated as an I420 Buffer + meta JSON).

use std::sync::Arc;

use mediaservo_deck::{CameraSource, CaptureOptions};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::ThreadsafeFunctionCallMode;
use napi_derive::napi;

/// Capture options (omitted fields use the default 1280x720@30).
#[napi(object)]
pub struct JsCaptureOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub framerate: Option<u32>,
}

fn closed_err() -> napi::Error {
    napi::Error::from_reason("camera not started or closed")
}

/// Camera capture source (async; frame callbacks forwarded thread-safely via TSFN).
#[napi]
pub struct JsCameraSource {
    inner: Arc<tokio::sync::Mutex<Option<CameraSource>>>,
    stream: Arc<tokio::sync::Mutex<Option<mediaservo_deck::FrameStream>>>,
    opts: CaptureOptions,
}

// SAFETY: all methods are serialized through a tokio Mutex (same precedent as field-c).
unsafe impl Send for JsCameraSource {}
unsafe impl Sync for JsCameraSource {}

#[napi]
impl JsCameraSource {
    /// Open the camera (currently the stub device "stub:test-camera").
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

    /// Start producing frames (only allowed once; creates the FrameStream internally).
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

    /// Subscribe to frame callbacks ((meta_json, i420_buffer) → JS; the pump exits when the stream closes).
    #[napi]
    #[allow(clippy::while_let_loop)] // keep the match-shaped pump loop with the None branch log hook; do not rewrite as while-let
    pub fn on_frame(&self, cb: Function<(String, Vec<u8>), ()>) -> Result<()> {
        let tsfn = cb.build_threadsafe_function::<(String, Vec<u8>)>().build()?;
        let stream = self.stream.clone();
        super::event_runtime().spawn(async move {
            // FrameStream is not Clone (mpsc Receiver) — take() for exclusive access; recv returns None once all senders drop, stopping the pump.
            let rx = { stream.lock().await.take() };
            let Some(mut rx) = rx else { return };
            loop {
                match rx.recv().await {
                    Some(frame) => {
                        // Concatenate the I420 three planes (Y + U + V)
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
                    None => break, // stream closed
                }
            }
        });
        Ok(())
    }

    /// Stop producing frames (idempotent).
    #[napi]
    pub async fn stop(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if let Some(s) = guard.as_mut() {
            s.stop();
        }
        Ok(())
    }

    /// Close (idempotent; the pump stops when the stream is dropped).
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        *guard = None;
        *self.stream.lock().await = None;
        Ok(())
    }
}

/// Recorder (FFmpeg mux; record(camera) as a background task + stop_signal — same pattern as the deck-c C ABI:
/// record blocks until stop, so it must be spawned or the JS await deadlocks).
#[napi]
pub struct JsRecorder {
    recorder: Arc<tokio::sync::Mutex<Option<mediaservo_deck::Recorder>>>,
    stop_signal: Arc<tokio::sync::Mutex<Option<mediaservo_deck::record::StopSignal>>>,
    #[allow(dead_code)]
    task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[napi]
impl JsRecorder {
    /// Open the recording target (RecordOptions defaults to h264/mp4).
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

    /// Start recording (background task; returns immediately. The camera must be started and outlive the recording — same as the C contract).
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

    /// Stop recording (idempotent; signals stop → worker flush + trailer).
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

    /// Close (idempotent; drains a leftover stop signal).
    #[napi]
    pub async fn close(&self) -> Result<()> {
        if let Some(signal) = self.stop_signal.lock().await.take() {
            signal.stop();
        }
        *self.recorder.lock().await = None;
        Ok(())
    }
}

/// Player (demux+decode; onFrame called per frame on the pump thread — next_frame is a sync API).
#[napi]
pub struct JsPlayer {
    inner: Arc<tokio::sync::Mutex<Option<mediaservo_deck::Player>>>,
}

// SAFETY: exclusive pump-thread access (next_frame &mut) is serialized through a Mutex.
unsafe impl Send for JsPlayer {}
unsafe impl Sync for JsPlayer {}

#[napi]
impl JsPlayer {
    /// Open a media file (FFmpeg demux+decode).
    #[napi(factory)]
    pub async fn open(path: String) -> Result<Self> {
        let p = mediaservo_deck::Player::open(path)
            .map_err(|e| napi::Error::from_reason(format!("open: {e}")))?;
        Ok(Self { inner: Arc::new(tokio::sync::Mutex::new(Some(p))) })
    }

    /// Per-frame callback (pump-thread next_frame → tsfn; EOF or None stops the pump).
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

    /// Media duration (seconds).
    #[napi]
    pub fn duration_secs(&self) -> Result<f64> {
        let guard = self.inner.try_lock().map_err(|_| napi::Error::from_reason("player busy"))?;
        guard
            .as_ref()
            .ok_or_else(closed_err)?
            .duration_secs()
            .map_err(|e| napi::Error::from_reason(format!("duration: {e}")))
    }

    /// Close (idempotent).
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        *guard = None;
        Ok(())
    }
}
