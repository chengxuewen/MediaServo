//! WebRtcTrackSink -- the bridge from a synchronous VideoSource broadcast to an asynchronous TrackSender send.
//!
//! Design source: plan `docs/plans/video-source-unification/plan.md` (v2, dual-review passed).
//!
//! - **placement**: mediaservo-webrtc (media is a plain dependency, mirroring the webrtc -> codec pattern)
//! - **frame ownership** (v2 BLOCKER-2): `on_frame` receives a shared reference `&BoxVideoFrame` (cannot move)
//!   -> copy-assemble the I420 contiguous layout (640x480 ~ 460KB/frame, 30fps ~ 13.8MB/s memcpy -- acceptable)
//! - **backpressure** (v2): bounded(3) + try_send drop-new (latency never accumulates)
//! - **runtime**: construction must be inside a tokio runtime context (`Handle::try_current` failing returns Err)
//! - **lifetime** (v2 BLOCKER-4): the background task exits on channel closed / write errors exceeding the threshold;
//!   the frame-source thread is explicitly stopped by the caller (Host)

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::runtime::Handle;
use tokio::sync::mpsc;

use mediaservo_media::base::buffer::VideoBuffer;
use mediaservo_media::base::frame::BoxVideoFrame;
use mediaservo_media::error::MediaError;
use mediaservo_media::pipeline::sink::{VideoSink, VideoSinkWants};

use crate::RTCError;
use crate::track::TrackSender;

/// Channel capacity -- 3 frames ~ 100ms @30fps. Full = drop-new (the v2 revision).
const CHANNEL_CAPACITY: usize = 3;
/// The consecutive-write-error threshold -- beyond it the sending task stops (aligned with the B5 break semantics).
const MAX_CONSECUTIVE_ERRORS: u64 = 30;
/// Log rate limiting -- one warn per N errors.
const WARN_EVERY_N_ERRORS: u64 = 10;

/// The assembled I420 frame passed through the channel (Y+U+V contiguous, tight strides).
struct OwnedI420Frame {
    data: Vec<u8>,
    width: u32,
    height: u32,
    ts_us: i64,
}

/// The bridging sink: a frame source (VideoFrameGenerator / future camera/desktop/compositor) registers via
/// `VideoSource::add_or_update_sink`; `on_frame` is called synchronously on the source thread,
/// copy-assembles then try_sends into the bounded channel, and the background task awaits the send
/// (PIT-63: the frame capture timestamp passes through `write_raw_i420_with_ts`).
#[derive(Debug)]
pub struct WebRtcTrackSink {
    tx: mpsc::Sender<OwnedI420Frame>,
    consecutive_errors: Arc<AtomicU64>,
}

impl WebRtcTrackSink {
    /// Create the bridging sink and start the background sending task.
    ///
    /// # Errors
    /// Outside a tokio runtime context returns `RTCError::Internal` (must be constructed inside a runtime).
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

    /// The current consecutive-write-error count (observability/tests).
    pub fn consecutive_errors(&self) -> u64 {
        self.consecutive_errors.load(Ordering::Relaxed)
    }
}

impl VideoSink<BoxVideoFrame> for WebRtcTrackSink {
    fn on_frame(&self, frame: &BoxVideoFrame) -> Result<VideoSinkWants, MediaError> {
        // frame ownership: a shared reference cannot move -> copy-assemble (v2 BLOCKER-2).
        let buf = frame.buffer.as_i420().ok_or_else(|| {
            MediaError::Internal("WebRtcTrackSink only accepts I420 frames".into())
        })?;

        let width = buf.width();
        let height = buf.height();
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;

        // I420Buffer::new guarantees tight strides (stride == width / width/2);
        // future non-tight sources (camera/desktop padding) need de-stride row copies (ponytail: see plan §4).
        debug_assert_eq!(buf.stride_y as usize, width as usize, "tight Y stride");
        debug_assert_eq!(buf.stride_u as usize, (width / 2) as usize, "tight U stride");
        debug_assert_eq!(buf.stride_v as usize, (width / 2) as usize, "tight V stride");

        let mut data = Vec::with_capacity(y_size + 2 * uv_size);
        data.extend_from_slice(&buf.data_y[..y_size]);
        data.extend_from_slice(&buf.data_u[..uv_size]);
        data.extend_from_slice(&buf.data_v[..uv_size]);

        let item = OwnedI420Frame { data, width, height, ts_us: frame.timestamp_us };
        match self.tx.try_send(item) {
            Ok(()) => Ok(VideoSinkWants::default()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // backpressure: full = drop the new frame (drop-new), latency never accumulates (the v2 revision).
                tracing::debug!("WebRtcTrackSink channel full — dropping frame");
                Ok(VideoSinkWants::default())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                // the receiver exited -> notify the source to stop broadcasting to this sink.
                tracing::debug!("WebRtcTrackSink channel closed — deactivating");
                Ok(VideoSinkWants { is_active: false, ..Default::default() })
            }
        }
    }
}

// stub-backend-only tests (the TrackSender.backend recording methods exist only on StubTrack) --
// meaningless under backend-webrtc-sys/rs (real FFI covered by integration tests).
#[cfg(all(test, not(any(feature = "backend-webrtc-rs", feature = "backend-webrtc-sys"))))]
mod tests {
    use super::*;
    use crate::track::TrackKind;
    use mediaservo_media::base::buffer::I420Buffer;
    use mediaservo_media::base::frame::VideoFrame;
    use std::time::Duration;

    fn test_frame(ts_us: i64, w: u32, h: u32) -> BoxVideoFrame {
        let buf: Box<dyn VideoBuffer> = Box::new(I420Buffer::new(w, h));
        VideoFrame::new(buf).with_timestamp(ts_us)
    }

    /// the stub backend's frame-count observation (TrackSender.backend and the in-task clone share a recording Arc).
    fn written(track: &TrackSender) -> u64 {
        track.backend.frames_written()
    }

    fn ts_history(track: &TrackSender) -> Vec<i64> {
        track.backend.ts_history()
    }

    #[test]
    fn sink_requires_runtime() {
        // not a tokio context (a plain #[test]) -> construction must fail.
        let track = TrackSender::new("t-no-runtime".into(), TrackKind::Video);
        let err = WebRtcTrackSink::new(track).unwrap_err();
        assert!(matches!(err, RTCError::Internal(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn sink_passes_frames_with_timestamps() {
        let track = TrackSender::new("t1".into(), TrackKind::Video);
        let sink = WebRtcTrackSink::new(track.clone()).unwrap();

        // 30fps semantics: adjacent frame ts spacing ~ 33.3ms (the C17 contract runs through the sink).
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
        // passthrough assert: the output ts is exactly the input ts.
        assert_eq!(history[0], 1_700_000_000_000);
        // the C17 assert: the adjacent frame delta ~ 33.3ms +/- 5ms.
        let delta = history[1] - history[0];
        assert!((33_333 - delta).abs() <= 5_000, "expected ~33.3ms delta, got {delta}µs");
        let delta2 = history[2] - history[1];
        assert!((33_333 - delta2).abs() <= 5_000, "got {delta2}µs");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sink_drops_frames_when_channel_full() {
        let track = TrackSender::new("t2".into(), TrackKind::Video);
        let sink = WebRtcTrackSink::new(track.clone()).unwrap();

        // a current_thread runtime: the background task can't be polled during a synchronous on_frame ->
        // the capacity-3 channel starts drop-new from the 4th frame.
        for i in 0..5 {
            sink.on_frame(&test_frame(1_000_000 + i * 33_333, 64, 48)).unwrap();
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
        // only 3 frames sent, 2 dropped (backpressure in effect).
        assert_eq!(written(&track), 3, "channel capacity exceeded");
    }
}
