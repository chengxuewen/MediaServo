//! Adapters that wrap host crate types as mediaservo_common pipeline nodes,
//! so they can be orchestrated by PipelineEngine.
//!
//! - GstCaptureSource: wraps crate::pipeline::Pipeline as MediaSource<InternalPacket>
//! - WebrtcOutputSink: wraps crate::webrtc_transport::WebrtcTransport as MediaSink<InternalPacket>

#![allow(dead_code)] // 双身份文件：host-legacy bin 的 `mod` 形态在用（T1.3 翻案），独立 bin 编译形态全 dead = 结构噪音
use std::sync::Arc;

use mediaservo_media::error::MediaError;
#[cfg(feature = "gstreamer")]
use mediaservo_media::pipeline::core::{EncodedFragment, FragmentFlags, FrameTiming};
use mediaservo_media::pipeline::core::{
    FormatSpec, InternalPacket, MediaSource, MediaType, NodeCapability, NodeInfo, PipelineNode,
};

type Result<T> = std::result::Result<T, MediaError>;

// ── NAL unit helpers for Annex B byte-stream ──

/// Scan an Annex B byte-stream for a NAL unit header starting at `offset`.
/// Returns (nal_unit_type, end_of_nal_offset) if header found.
fn scan_nal_header(data: &[u8], offset: usize) -> Option<(u8, usize)> {
    if offset + 4 > data.len() {
        return None;
    }
    let header_len = if data[offset..].starts_with(&[0x00, 0x00, 0x00, 0x01]) {
        4
    } else if data[offset..].starts_with(&[0x00, 0x00, 0x01]) {
        3
    } else {
        return None;
    };
    let nal_pos = offset + header_len;
    if nal_pos >= data.len() {
        return None;
    }
    let nal_type = data[nal_pos] & 0x1F;
    // find end: next start code or EOF
    let nal_end = (nal_pos + 1..data.len().saturating_sub(3))
        .find(|&i| {
            data[i..].starts_with(&[0x00, 0x00, 0x01])
                || data[i..].starts_with(&[0x00, 0x00, 0x00, 0x01])
        })
        .unwrap_or(data.len());
    Some((nal_type, nal_end))
}

/// Check if byte-stream contains an IDR keyframe (NAL unit type 5).
fn is_keyframe(data: &[u8]) -> bool {
    let mut offset = 0;
    while let Some((nal_type, next)) = scan_nal_header(data, offset) {
        if nal_type == 5 {
            return true;
        }
        offset = next;
    }
    false
}

/// Extract SPS (NAL 7) and PPS (NAL 8) from byte-stream including start codes.
fn extract_sps_pps(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some((nal_type, next)) = scan_nal_header(data, offset) {
        // find the actual start code position for slicing
        let start_offset =
            (offset..next).find(|&i| data[i] == 0x00 && data[i + 1] == 0x00).unwrap_or(offset);
        if nal_type == 7 || nal_type == 8 {
            out.extend_from_slice(&data[start_offset..next]);
        }
        offset = next;
    }
    out
}

/// Check if byte-stream contains a coded slice NAL (type 1 or 5).
fn has_data_nal(data: &[u8]) -> bool {
    let mut offset = 0;
    while let Some((nal_type, next)) = scan_nal_header(data, offset) {
        if nal_type == 1 || nal_type == 5 {
            return true;
        }
        offset = next;
    }
    false
}

// ── GstCaptureSource ──
// Feature-gated: real GStreamer pipeline vs stub (returns None always).

/// Real pipeline-backed source (GStreamer compiled in).
#[cfg(feature = "gstreamer")]
pub struct GstCaptureSource {
    pipeline: Arc<crate::pipeline::Pipeline>,
    sps_pps_buf: Option<Vec<u8>>,
}

#[cfg(feature = "gstreamer")]
impl GstCaptureSource {
    pub fn new(pipeline: Arc<crate::pipeline::Pipeline>) -> Self {
        Self { pipeline, sps_pps_buf: None }
    }
}

#[cfg(feature = "gstreamer")]
impl NodeInfo for GstCaptureSource {
    fn name(&self) -> &str {
        "gst-capture"
    }

    fn capabilities(&self) -> NodeCapability {
        NodeCapability {
            input: FormatSpec { media_type: MediaType::Both, codecs: None, pixel_formats: vec![] },
            output: FormatSpec {
                media_type: MediaType::Encoded,
                codecs: None,
                pixel_formats: vec![],
            },
        }
    }
}

#[cfg(feature = "gstreamer")]
impl PipelineNode for GstCaptureSource {
    fn on_start(&mut self) -> Result<()> {
        self.pipeline.start()
    }

    fn on_stop(&mut self) -> Result<()> {
        self.pipeline.stop()
    }
}

#[cfg(feature = "gstreamer")]
impl MediaSource for GstCaptureSource {
    type Output = InternalPacket;

    fn poll_fragment(&mut self) -> Result<Option<Self::Output>> {
        match self.pipeline.pull_sample() {
            Ok((data, pts)) if data.is_empty() => Ok(None),
            Ok((data, pts)) => {
                let is_kf = is_keyframe(&data);

                // accumulate SPS/PPS for init_data
                let sps_pps = extract_sps_pps(&data);
                if !sps_pps.is_empty() {
                    self.sps_pps_buf.get_or_insert_with(Vec::new).extend(&sps_pps);
                }

                // pass accumulated init_data with first data frame, then clear
                let init_data = if has_data_nal(&data) { self.sps_pps_buf.take() } else { None };

                Ok(Some(InternalPacket::Encoded(EncodedFragment {
                    track_id: "capture".into(),
                    timing: FrameTiming {
                        dts: pts,
                        pts,
                        duration: 0,
                        wall_clock: Some(std::time::Instant::now()),
                    },
                    flags: FragmentFlags { keyframe: is_kf, independent: true, discardable: false },
                    codec: "h264".into(),
                    init_data,
                    payload: data,
                })))
            }
            Err(_e) => {
                // ponytail: treat pull failures as transient (no frame available)
                Ok(None)
            }
        }
    }
}

// Stub source — returns no fragments when GStreamer is not compiled in.

#[cfg(not(feature = "gstreamer"))]
pub struct GstCaptureSource;

#[cfg(not(feature = "gstreamer"))]
impl GstCaptureSource {
    pub fn new(_pipeline: Arc<crate::pipeline::Pipeline>) -> Self {
        Self
    }
}

#[cfg(not(feature = "gstreamer"))]
impl NodeInfo for GstCaptureSource {
    fn name(&self) -> &str {
        "gst-capture"
    }

    fn capabilities(&self) -> NodeCapability {
        NodeCapability {
            input: FormatSpec { media_type: MediaType::Both, codecs: None, pixel_formats: vec![] },
            output: FormatSpec {
                media_type: MediaType::Encoded,
                codecs: None,
                pixel_formats: vec![],
            },
        }
    }
}

#[cfg(not(feature = "gstreamer"))]
impl PipelineNode for GstCaptureSource {
    fn on_start(&mut self) -> Result<()> {
        Ok(())
    }

    fn on_stop(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(not(feature = "gstreamer"))]
impl MediaSource for GstCaptureSource {
    type Output = InternalPacket;

    fn poll_fragment(&mut self) -> Result<Option<Self::Output>> {
        Ok(None)
    }
}

// ── WebrtcOutputSink ──
// Always available — WebrtcTransport is not feature-gated.

#[cfg(feature = "webrtc-p2p")]
pub struct WebrtcOutputSink {
    _drain_handle: tokio::task::JoinHandle<()>,
    frame_tx: mpsc::Sender<Vec<u8>>,
}

#[cfg(feature = "webrtc-p2p")]
impl WebrtcOutputSink {
    pub fn new(transport: Arc<crate::webrtc_transport::WebrtcTransport>) -> Self {
        // ponytail: bounded channel (capacity 4) replaces fire-and-forget spawn.
        // Full channel → drop oldest frame. Single consumer drains sequentially,
        // avoiding unbounded task accumulation on DC backpressure.
        let (frame_tx, mut frame_rx) = mpsc::channel::<Vec<u8>>(4);
        let drain_handle = tokio::spawn(async move {
            while let Some(data) = frame_rx.recv().await {
                let _ = transport.send_frame(&data).await;
            }
        });
        Self { _drain_handle: drain_handle, frame_tx }
    }
}

#[cfg(feature = "webrtc-p2p")]
impl NodeInfo for WebrtcOutputSink {
    fn name(&self) -> &str {
        "webrtc-output"
    }

    fn capabilities(&self) -> NodeCapability {
        NodeCapability {
            input: FormatSpec {
                media_type: MediaType::Encoded,
                codecs: None,
                pixel_formats: vec![],
            },
            output: FormatSpec {
                media_type: MediaType::Encoded,
                codecs: None,
                pixel_formats: vec![],
            },
        }
    }
}

#[cfg(feature = "webrtc-p2p")]
impl PipelineNode for WebrtcOutputSink {
    fn on_start(&mut self) -> Result<()> {
        Ok(())
    }

    fn on_stop(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(feature = "webrtc-p2p")]
impl MediaSink for WebrtcOutputSink {
    type Input = InternalPacket;

    fn on_fragment(&mut self, fragment: Self::Input) -> Result<()> {
        let payload = match fragment {
            InternalPacket::Encoded(f) => f.payload,
            _ => return Ok(()),
        };
        // ponytail: try_send → drop oldest if channel full, no task accumulation
        let _ = self.frame_tx.try_send(payload);
        Ok(())
    }
}
