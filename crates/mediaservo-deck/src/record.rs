//! 录制域（record）— Recorder：I420 帧流 → H264 编码 → 容器 mux 落盘。
//!
//! MVP: FFmpeg 后端（ffmpeg-the-third 6.0），编码 H264 + MP4 mux。
//! 入口参考契约 §6：`Recorder::new(path, opts)` → `record(stream)` 消费
//! 全部帧（持续录制），`stop()` 收尾（flush + trailer + join）。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mediaservo_codec::frame::VideoFrame;

use crate::DeckError;

/// 容器格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mkv,
}

/// 录制选项。
#[derive(Debug, Clone)]
pub struct RecordOptions {
    pub codec: VideoCodec,
    pub container: Container,
    #[allow(dead_code)]
    /// 目标帧率（容器 time_base 与编码器输入节奏）。
    pub fps: u32,
    /// 关键帧间隔（帧数）。
    pub keyframe_interval: u32,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self { codec: VideoCodec::H264, container: Container::Mp4, fps: 30, keyframe_interval: 60 }
    }
}

/// 视频编解码器（MVP: H264）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
}

/// 录制器：把输入帧流的每一帧编码后写入容器文件（async 录制任务）。
pub struct Recorder {
    path: PathBuf,
    opts: RecordOptions,
    running: Arc<AtomicBool>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

impl Recorder {
    /// 创建录制器（不启动；`record` 后开始写文件）。
    pub fn new(path: impl Into<PathBuf>, opts: RecordOptions) -> Result<Self, DeckError> {
        let path = path.into();
        // 父目录必须存在（不隐式创建 — 明确失败让调用方知道路径问题）
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(DeckError::NotFound(format!(
                "parent dir {} does not exist",
                parent.display()
            )));
        }
        Ok(Self { path, opts, running: Arc::new(AtomicBool::new(false)), worker: None })
    }

    /// 停止信号（与 recorder 共享 running 标志）。
    pub fn stop_signal(&self) -> StopSignal {
        StopSignal { running: Arc::clone(&self.running) }
    }

    /// 开始录制：消费帧流直到流结束（`recv` 返回 None）或 `stop()`。
    ///
    /// 编码/mux 在 `spawn_blocking` 线程（FFmpeg 为阻塞 API）；
    /// 帧桥接在调用该 async fn 的运行时上逐帧 send。帧流结束或 stop
    /// 均触发 flush + trailer。
    pub async fn record(&mut self, mut frames: impl Frames) -> Result<(), DeckError> {
        if self.worker.is_some() {
            return Err(DeckError::InvalidState("already recording".into()));
        }
        self.running.store(true, Ordering::SeqCst);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let path = self.path.clone();
        let opts = self.opts.clone();
        let running = Arc::clone(&self.running);

        // 编码 worker（阻塞线程）
        let handle = tokio::task::spawn_blocking(move || {
            if let Err(e) = mux_worker(&path, &opts, &mut rx, &running) {
                tracing::error!("recorder worker failed: {e}");
            }
        });

        // 帧桥接循环：async 帧源 → worker 的同步通道。
        // 结束条件（任一）：① 帧源结束（recv None）② stop() 请求
        // （running=false——轮询帧源是异步语义，无法被 tokio 取消，
        // 故 stop() 后立即态检查退出）
        loop {
            if !self.running.load(Ordering::SeqCst) {
                break;
            }
            match tokio::time::timeout(std::time::Duration::from_millis(50), frames.next()).await {
                Ok(Some(frame)) => {
                    if tx.send(frame).is_err() {
                        tracing::warn!("recorder worker exited; stopping frame pump");
                        break;
                    }
                }
                Ok(None) => break,  // 帧源结束
                Err(_elapsed) => {} // 超时：继续检查 running
            }
        }
        drop(tx);

        let _ = handle.await;
        if self.running.swap(false, Ordering::SeqCst) {
            // running 由 mux_worker 结束时重置；此处仅为状态一致性
        }
        self.running.store(false, Ordering::SeqCst);
        self.worker = None;
        Ok(())
    }

    /// 请求停止：置 running=false；worker 在下个 tick 退出并 flush。
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// 可独立持有的停止信号（record() 移走 recorder 时仍能从外部停止录制）。
#[derive(Clone, Debug, Default)]
pub struct StopSignal {
    running: Arc<AtomicBool>,
}

impl StopSignal {
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub(crate) fn is_stopped(&self) -> bool {
        !self.running.load(Ordering::SeqCst)
    }
}

/// 帧流抽象（deck FrameStream 使用方提供 async 帧源）。
pub trait Frames {
    /// 取下一帧；None = 流结束。
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send;
}

impl Frames for crate::source::FrameStream {
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        crate::source::FrameStream::recv(self)
    }
}

impl Frames for &mut crate::source::FrameStream {
    fn next(&mut self) -> impl std::future::Future<Output = Option<VideoFrame>> + Send {
        (*self).recv()
    }
}

/// 持续录制循环的帧泵：迭代器语义包一层 async fn 也可。
/// （若调用方有同步帧源，可用 `tokio::task::block_in_place` 转接 — YAGNI。）
#[cfg(feature = "backend-ffmpeg")]
fn mux_worker(
    path: &PathBuf,
    opts: &RecordOptions,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<VideoFrame>,
    running: &std::sync::atomic::AtomicBool,
) -> Result<(), DeckError> {
    use ffmpeg_the_third as ffmpeg;

    ffmpeg::init().map_err(|e| DeckError::Codec(format!("ffmpeg init: {e}")))?;

    let mut out = ffmpeg::format::output(path)
        .map_err(|e| DeckError::Io(std::io::Error::other(format!("open output: {e}"))))?;

    let codec = ffmpeg::codec::encoder::find(ffmpeg::codec::Id::H264)
        .ok_or_else(|| DeckError::Codec("h264 encoder not found".into()))?;

    // 第一帧决定尺寸（阻塞等首帧）
    let first = next_frame(rx, running, None)?;
    let w = first.format.width;
    let h = first.format.height;
    let _fps = opts.fps as i32;

    // 配置并打开编码器
    let ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
    let mut enc =
        ctx.encoder().video().map_err(|e| DeckError::Codec(format!("create encoder: {e}")))?;
    enc.set_width(w);
    enc.set_height(h);
    enc.set_format(ffmpeg::format::Pixel::YUV420P);
    // pts 单位为 µs（输入帧 pts 源自 ts_mono_ns/1000）
    enc.set_time_base(ffmpeg::Rational(1, 1_000_000));
    enc.set_gop(opts.keyframe_interval);
    enc.set_max_b_frames(0);
    enc.set_bit_rate(2_000_000);
    let mut enc =
        enc.open_with(h264_dict()).map_err(|e| DeckError::Codec(format!("open encoder: {e}")))?;

    // 容器 stream：从打开的编码器复制完整 codecpar（含 SPS/PPS extradata，
    // 官方 muxing.c 模式；ctx 由于 encoder() consume 不可复用，从 enc.0 复制）
    let mut stream = out
        .add_stream(ffmpeg::codec::Id::H264)
        .map_err(|e| DeckError::Codec(format!("add stream: {e}")))?;
    stream.copy_parameters_from_context(&enc.0);
    stream.set_time_base(ffmpeg::Rational(1, 1_000_000));
    out.write_header().map_err(|e| DeckError::Codec(format!("write header: {e}")))?;

    // pts 锚定首帧基准（generator/source 的 pts 是 epoch 大值 →
    // 不加权会得到 duration= 117s 的假长文件）
    let base_pts = first.pts;
    encode_and_mux(&mut enc, &mut out, &first, base_pts)?;

    while running.load(Ordering::SeqCst) {
        match next_frame(rx, running, None) {
            Ok(f) => encode_and_mux(&mut enc, &mut out, &f, base_pts)?,
            Err(e) => {
                tracing::info!("recorder stopping: {e}");
                break;
            }
        }
    }

    enc.send_eof().map_err(|e| DeckError::Codec(format!("send_eof: {e}")))?;
    loop {
        let mut pkt = ffmpeg::codec::packet::Packet::empty();
        match enc.receive_packet(&mut pkt) {
            Ok(_) => pkt
                .write_interleaved(&mut out)
                .map_err(|e| DeckError::Codec(format!("flush pkt: {e}")))?,
            Err(_) => break,
        }
    }
    out.write_trailer().map_err(|e| DeckError::Codec(format!("write trailer: {e}")))?;
    tracing::info!("recorder finished: {:?}", path);
    Ok(())
}

/// 阻塞取下一帧；`stop` 请求或通道关闭时返回 Err（触发 flush 收尾）。
#[cfg(feature = "backend-ffmpeg")]
fn next_frame(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<VideoFrame>,
    running: &AtomicBool,
    _first: Option<&VideoFrame>,
) -> Result<VideoFrame, DeckError> {
    loop {
        match rx.try_recv() {
            Ok(f) => return Ok(f),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if !running.load(Ordering::SeqCst) {
                    return Err(DeckError::InvalidState("stopped by stop()".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // 帧流结束（帧桥 send 完后 drop tx）
                return Err(DeckError::InvalidState("frame stream ended".into()));
            }
        }
    }
}

#[cfg(feature = "backend-ffmpeg")]
fn h264_dict() -> ffmpeg_the_third::Dictionary {
    let mut d = ffmpeg_the_third::Dictionary::new();
    d.set("preset", "ultrafast");
    d.set("tune", "zerolatency");
    d
}

#[cfg(feature = "backend-ffmpeg")]
fn encode_and_mux(
    enc: &mut ffmpeg_the_third::encoder::Video,
    out: &mut ffmpeg_the_third::format::context::Output,
    frame: &VideoFrame,
    base_pts: u64,
) -> Result<(), DeckError> {
    use ffmpeg_the_third as ffmpeg;
    let w = frame.format.width as usize;
    let h = frame.format.height as usize;
    let mut avframe =
        ffmpeg::util::frame::Video::new(ffmpeg::format::Pixel::YUV420P, w as u32, h as u32);
    for i in 0..3 {
        if let (Some(src), Some(dst)) = (frame.plane_data(i), Some(avframe.data_mut(i))) {
            let n = dst.len().min(src.len());
            dst[..n].copy_from_slice(&src[..n]);
        }
    }
    avframe.set_pts(Some(frame.pts.saturating_sub(base_pts) as i64));
    enc.send_frame(&avframe).map_err(|e| DeckError::Codec(format!("send frame: {e}")))?;
    loop {
        let mut pkt = ffmpeg::codec::packet::Packet::empty();
        match enc.receive_packet(&mut pkt) {
            Ok(_) => pkt
                .write_interleaved(out)
                .map_err(|e| DeckError::Codec(format!("write pkt: {e}")))?,
            Err(_) => break,
        }
    }
    Ok(())
}

#[cfg(not(feature = "backend-ffmpeg"))]
fn mux_worker(
    _path: &PathBuf,
    _opts: &RecordOptions,
    _rx: &mut tokio::sync::mpsc::UnboundedReceiver<VideoFrame>,
    _running: &std::sync::atomic::AtomicBool,
) -> Result<(), DeckError> {
    Err(DeckError::Codec("recorder requires backend-ffmpeg feature".into()))
}
