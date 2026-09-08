//! WebRtcTrackSink — 同步 VideoSource 广播 → 异步 TrackSender 发送的桥接。
//!
//! 设计来源: 计划 `docs/plans/video-source-unification/plan.md` (v2, 双审核通过)。
//!
//! - **放置**: mediaservo-webrtc（media 为 plain dependency，镜像 webrtc → codec 模式）
//! - **帧所有权** (v2 BLOCKER-2): `on_frame` 收到共享引用 `&BoxVideoFrame`（不可 move）
//!   → 拷贝组装 I420 连续布局（640×480 ≈ 460KB/帧, 30fps ≈ 13.8MB/s memcpy — 可接受）
//! - **背压** (v2): bounded(3) + try_send drop-new（延迟不堆积）
//! - **运行时**: 构造必须处于 tokio 运行时上下文（`Handle::try_current` 失败返回 Err）
//! - **生命周期** (v2 BLOCKER-4): 后台任务在 channel closed / 连续写错误超阈值时退出；
//!   帧源线程由调用方（Host）显式 stop

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::sync::mpsc;

use mediaservo_media::base::buffer::VideoBuffer;
use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::error::MediaError;
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};

use crate::track::TrackSender;
use crate::RTCError;

/// Channel 容量 — 3 帧 ≈ 100ms @30fps。满则 drop-new（v2 修订）。
const CHANNEL_CAPACITY: usize = 3;
/// 连续写错误阈值 — 超过则停止发送任务（对齐 B5 break 语义）。
const MAX_CONSECUTIVE_ERRORS: u64 = 30;
/// 限频日志 — 每 N 次错误打一条 warn。
const WARN_EVERY_N_ERRORS: u64 = 10;

/// 经 channel 传递的已组装 I420 帧（Y+U+V 连续布局, tight strides）。
struct OwnedI420Frame {
    data: Vec<u8>,
    width: u32,
    height: u32,
    ts_us: i64,
}

/// 桥接 sink: 帧源（VideoFrameGenerator / 未来 camera/desktop/compositor）经
/// `VideoSource::add_or_update_sink` 注册；`on_frame` 在源线程同步调用，
/// 拷贝组装后 try_send 到 bounded channel，后台任务 await 发送
/// （PIT-63: 帧捕获时间戳透传 `write_raw_i420_with_ts`）。
#[derive(Debug)]
pub struct WebRtcTrackSink {
    tx: mpsc::Sender<OwnedI420Frame>,
    consecutive_errors: Arc<AtomicU64>,
}

impl WebRtcTrackSink {
    /// 创建桥接 sink 并启动后台发送任务。
    ///
    /// # Errors
    /// 非 tokio 运行时上下文时返回 `RTCError::Internal`（须在 runtime 内构造）。
    pub fn new(track: TrackSender) -> Result<Self, RTCError> {
        let handle = Handle::try_current().map_err(|_| {
            RTCError::Internal("WebRtcTrackSink requires tokio runtime context".into())
        })?;
        let (tx, mut rx): (mpsc::Sender<OwnedI420Frame>, mpsc::Receiver<OwnedI420Frame>) =
            mpsc::channel(CHANNEL_CAPACITY);
        let consecutive_errors = Arc::new(AtomicU64::new(0));
        let errs = Arc::clone(&consecutive_errors);

        handle.spawn(async move {
            while let Some(frame) = rx.recv().await {
                let result = track
                    .write_raw_i420_with_ts(
                        &frame.data,
                        frame.width,
                        frame.height,
                        Some(frame.ts_us),
                    )
                    .await;
                match result {
                    Ok(()) => errs.store(0, Ordering::Relaxed),
                    Err(e) => {
                        let n = errs.fetch_add(1, Ordering::Relaxed) + 1;
                        if n.is_multiple_of(WARN_EVERY_N_ERRORS) {
                            tracing::warn!("WebRtcTrackSink write error (x{n}): {e}");
                        }
                        if n >= MAX_CONSECUTIVE_ERRORS {
                            tracing::error!(
                                "WebRtcTrackSink giving up after {n} consecutive write errors"
                            );
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self { tx, consecutive_errors })
    }

    /// 当前连续写错误数（可观测性/测试）。
    pub fn consecutive_errors(&self) -> u64 {
        self.consecutive_errors.load(Ordering::Relaxed)
    }
}

impl VideoSink<BoxVideoFrame> for WebRtcTrackSink {
    fn on_frame(&self, frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
        // 帧所有权: 共享引用不可 move → 拷贝组装（v2 BLOCKER-2）。
        let buf = frame.buffer.as_i420().ok_or_else(|| {
            MediaError::Internal("WebRtcTrackSink only accepts I420 frames".into())
        })?;

        let width = buf.width();
        let height = buf.height();
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;

        // I420Buffer::new 保证 tight strides（stride == width / width/2）；
        // 未来非 tight 源（相机/桌面 padding）需 de-stride 行拷贝（ponytail: 见计划 §4）。
        debug_assert_eq!(buf.stride_y as usize, width as usize, "tight Y stride");
        debug_assert_eq!(buf.stride_u as usize, (width / 2) as usize, "tight U stride");
        debug_assert_eq!(buf.stride_v as usize, (width / 2) as usize, "tight V stride");

        let mut data = Vec::with_capacity(y_size + 2 * uv_size);
        data.extend_from_slice(&buf.data_y[..y_size]);
        data.extend_from_slice(&buf.data_u[..uv_size]);
        data.extend_from_slice(&buf.data_v[..uv_size]);

        let item = OwnedI420Frame {
            data,
            width,
            height,
            ts_us: frame.timestamp_us,
        };
        match self.tx.try_send(item) {
            Ok(()) => Ok(VideoSinkWants::default()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // 背压: 满则丢弃新帧（drop-new），延迟不堆积（v2 修订）。
                tracing::debug!("WebRtcTrackSink channel full — dropping frame");
                Ok(VideoSinkWants::default())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // 接收端已退出 → 通知源停止广播本 sink。
                tracing::debug!("WebRtcTrackSink channel closed — deactivating");
                Ok(VideoSinkWants {
                    is_active: false,
                    ..Default::default()
                })
            }
        }
    }
}

// stub 后端专属测试（TrackSender.backend 记录方法仅 StubTrack 有）—
// backend-webrtc-sys/rs 下这些测试无意义（真实 FFI 由集成测试覆盖）。
#[cfg(all(test, not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))))]
mod tests {
    use super::*;
    use mediaservo_media::base::buffer::I420Buffer;
    use mediaservo_media::base::frame::VideoFrame;
    use crate::track::TrackKind;
    use std::time::Duration;

    fn test_frame(ts_us: i64, w: u32, h: u32) -> BoxVideoFrame {
        let buf: Box<dyn VideoBuffer> = Box::new(I420Buffer::new(w, h));
        VideoFrame::new(buf).with_timestamp(ts_us)
    }


    /// stub 后端帧数观测（TrackSender.backend 与任务内 clone 共享 Arc 记录）。
    fn written(track: &TrackSender) -> u64 {
        track.backend.frames_written()
    }

    fn ts_history(track: &TrackSender) -> Vec<i64> {
        track.backend.ts_history()
    }

    #[test]
    fn sink_requires_runtime() {
        // 非 tokio 上下文（普通 #[test]）→ 构造必须失败。
        let track = TrackSender::new("t-no-runtime".into(), TrackKind::Video);
        let err = WebRtcTrackSink::new(track).unwrap_err();
        assert!(matches!(err, RTCError::Internal(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn sink_passes_frames_with_timestamps() {
        let track = TrackSender::new("t1".into(), TrackKind::Video);
        let sink = WebRtcTrackSink::new(track.clone()).unwrap();

        // 30fps 语义: 相邻帧 ts 间隔 ≈ 33.3ms（C17 契约贯穿 sink）。
        let mut ts = 1_700_000_000_000i64;
        let step = 33_333i64;
        for i in 0..3 {
            sink.on_frame(&test_frame(ts, 64, 48)).unwrap();
            assert_eq!(sink.consecutive_errors(), 0, "frame {i}");
            ts += step;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(written(&track), 3, "all frames written");
        let history = ts_history(&track);
        assert_eq!(history.len(), 3);
        // 透传断言: 输出 ts 与输入完全一致。
        assert_eq!(history[0], 1_700_000_000_000);
        // C17 断言: 相邻帧差值 ≈ 33.3ms ± 5ms。
        let delta = history[1] - history[0];
        assert!(
            (33_333 - delta).abs() <= 5_000,
            "expected ~33.3ms delta, got {delta}µs"
        );
        let delta2 = history[2] - history[1];
        assert!((33_333 - delta2).abs() <= 5_000, "got {delta2}µs");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sink_drops_frames_when_channel_full() {
        let track = TrackSender::new("t2".into(), TrackKind::Video);
        let sink = WebRtcTrackSink::new(track.clone()).unwrap();

        // current_thread runtime: 同步 on_frame 期间后台任务无法被 poll →
        // 容量 3 的 channel 在第 4 帧起 drop-new。
        for i in 0..5 {
            sink.on_frame(&test_frame(1_000_000 + i * 33_333, 64, 48))
                .unwrap();
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
        // 仅 3 帧被发送，2 帧被 drop（背压生效）。
        assert_eq!(written(&track), 3, "channel capacity exceeded");
    }
}
