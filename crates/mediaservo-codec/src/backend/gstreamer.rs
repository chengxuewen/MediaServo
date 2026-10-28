//! GStreamer backend — real H.264 encoding/decoding via GStreamer 1.24+.
//!
//! Encoder pipeline: appsrc ! videoconvert ! x264enc ! h264parse ! appsink
//! Decoder pipeline: appsrc ! h264parse ! avdec_h264 ! videoconvert ! appsink

use crate::codec::{CodecId, PixelFormat, VideoFormat};
use crate::config::{Bitrate, DecoderConfig, EncoderConfig, EncoderPreset};
use crate::decoder::VideoDecoder;
use crate::encoder::{EncoderStats, VideoEncoder};
use crate::error::CodecError;
use crate::frame::{Plane, VideoFrame};
use crate::packet::EncodedPacket;

use gstreamer::prelude::*;
use gstreamer_video::prelude::*;

// ── Preset mapping (kept as documentation; GStreamer 1.28 x264enc uses
// GEnum properties not settable from Rust bindings) ──

#[allow(dead_code)]
fn _preset_str(preset: EncoderPreset) -> &'static str {
    match preset {
        EncoderPreset::P1UltraFast => "ultrafast",
        EncoderPreset::P2SuperFast => "superfast",
        EncoderPreset::P3VeryFast => "veryfast",
        EncoderPreset::P4Medium => "medium",
        EncoderPreset::P5Slow => "slow",
        EncoderPreset::P6VerySlow | EncoderPreset::P7Lossless => "slow",
    }
}

// ── I420 pack helper ──

fn pack_i420(frame: &VideoFrame) -> Result<Vec<u8>, CodecError> {
    let w = frame.width() as usize;
    let h = frame.height() as usize;
    let y_size = w * h;
    let uv_size = (w / 2) * (h / 2);
    let total = y_size + 2 * uv_size;
    let mut data = vec![0u8; total];

    let y_stride = frame.plane_stride(0).unwrap_or(w as u32) as usize;
    if let Some(plane) = frame.plane_data(0) {
        for row in 0..h {
            let src = row * y_stride;
            let dst = row * w;
            let len = w.min(plane.len().saturating_sub(src));
            data[dst..dst + len].copy_from_slice(&plane[src..src + len]);
        }
    }

    let uv_stride = frame.plane_stride(1).unwrap_or((w / 2) as u32) as usize;
    let h2 = h / 2;
    let w2 = w / 2;
    let u_off = y_size;
    if let Some(plane) = frame.plane_data(1) {
        for row in 0..h2 {
            let src = row * uv_stride;
            let dst = u_off + row * w2;
            let len = w2.min(plane.len().saturating_sub(src));
            data[dst..dst + len].copy_from_slice(&plane[src..src + len]);
        }
    }

    let v_off = y_size + uv_size;
    if let Some(plane) = frame.plane_data(2) {
        for row in 0..h2 {
            let src = row * uv_stride;
            let dst = v_off + row * w2;
            let len = w2.min(plane.len().saturating_sub(src));
            data[dst..dst + len].copy_from_slice(&plane[src..src + len]);
        }
    }

    Ok(data)
}

// ── Appsink drain helpers ──

fn drain_appsink_packets(
    appsink: &gstreamer_app::AppSink,
    buffer: &mut Vec<EncodedPacket>,
    stats: &mut EncoderStats,
    timeout: gstreamer::ClockTime,
) -> Result<(), CodecError> {
    while let Some(sample) = appsink.try_pull_sample(timeout) {
        if let Some(buf) = sample.buffer() {
            let map = buf.map_readable().map_err(|e| CodecError::Encoder(format!("map: {e}")))?;
            let data = map.as_slice().to_vec();
            let pts = buf.pts().map(|t| t.nseconds()).unwrap_or(0);
            let keyframe = !buf.flags().contains(gstreamer::BufferFlags::DELTA_UNIT);
            stats.packets_produced += 1;
            stats.bytes_encoded += data.len() as u64;
            buffer.push(EncodedPacket { data, pts, dts: pts, keyframe, codec: CodecId::H264 });
        }
    }
    Ok(())
}

fn drain_appsink_frames(
    appsink: &gstreamer_app::AppSink,
    buffer: &mut Vec<VideoFrame>,
    timeout: gstreamer::ClockTime,
) -> Result<(), CodecError> {
    while let Some(sample) = appsink.try_pull_sample(timeout) {
        let caps = sample.caps().ok_or_else(|| CodecError::Decoder("sample has no caps".into()))?;
        let video_info = gstreamer_video::VideoInfo::from_caps(caps)
            .map_err(|_| CodecError::Decoder("failed to parse video caps".into()))?;
        let buf =
            sample.buffer().ok_or_else(|| CodecError::Decoder("sample has no buffer".into()))?;
        let vframe = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(&buf, &video_info)
            .map_err(|_| CodecError::Decoder("failed to map video frame".into()))?;

        let w = vframe.width();
        let h = vframe.height();
        let strides = vframe.plane_stride();
        let mut planes = Vec::with_capacity(3);
        for i in 0..3u32 {
            if let Ok(data) = vframe.plane_data(i) {
                planes.push(Plane { data: data.to_vec(), stride: strides[i as usize] as u32 });
            }
        }

        buffer.push(VideoFrame {
            format: VideoFormat { width: w, height: h, pixel_format: PixelFormat::Yuv420p },
            planes,
            pts: buf.pts().map(|t| t.nseconds()).unwrap_or(0),
            keyframe: !buf.flags().contains(gstreamer::BufferFlags::DELTA_UNIT),
        });
    }
    Ok(())
}

// ══════════════════════════════════════════════════════════════════════
// GstEncoder
// ══════════════════════════════════════════════════════════════════════

pub(crate) struct GstEncoder {
    pipeline: Option<gstreamer::Pipeline>,
    appsrc: Option<gstreamer_app::AppSrc>,
    appsink: Option<gstreamer_app::AppSink>,
    buffer: Vec<EncodedPacket>,
    stats: EncoderStats,
}

impl Default for GstEncoder {
    fn default() -> Self {
        gstreamer::init().ok();
        Self {
            pipeline: None,
            appsrc: None,
            appsink: None,
            buffer: Vec::new(),
            stats: EncoderStats::default(),
        }
    }
}

impl VideoEncoder for GstEncoder {
    fn configure(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
        if config.codec != CodecId::H264 {
            return Err(CodecError::UnsupportedCodec(config.codec));
        }

        let pipeline = gstreamer::Pipeline::default();

        let src_caps = gstreamer::Caps::builder("video/x-raw")
            .field("format", "I420")
            .field("width", config.format.width as i32)
            .field("height", config.format.height as i32)
            .field(
                "framerate",
                gstreamer::Fraction::new(config.fps.num as i32, config.fps.den as i32),
            )
            .build();
        let appsrc = gstreamer_app::AppSrc::builder()
            .caps(&src_caps)
            .format(gstreamer::Format::Time)
            .build();

        let videoconvert = gstreamer::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| CodecError::Encoder(format!("videoconvert: {e}")))?;
        let x264enc = gstreamer::ElementFactory::make("x264enc")
            .build()
            .map_err(|e| CodecError::Encoder(format!("x264enc: {e}")))?;
        let h264parse = gstreamer::ElementFactory::make("h264parse")
            .build()
            .map_err(|e| CodecError::Encoder(format!("h264parse: {e}")))?;

        // Map EncoderPreset to x264enc speed-preset (GEnum, set via set_property_from_str)
        let preset_str = match config.preset {
            EncoderPreset::P1UltraFast => "ultrafast",
            EncoderPreset::P2SuperFast => "superfast",
            EncoderPreset::P3VeryFast => "veryfast",
            EncoderPreset::P4Medium => "medium",
            EncoderPreset::P5Slow => "slow",
            EncoderPreset::P6VerySlow => "veryslow",
            EncoderPreset::P7Lossless => "placebo",
        };
        x264enc.set_property_from_str("speed-preset", preset_str);
        x264enc.set_property_from_str("tune", "zerolatency");
        let br_kbps = match config.bitrate {
            Bitrate::Cbr(b) => b,
            Bitrate::Vbr { target, .. } => target,
        };
        x264enc.set_property("bitrate", br_kbps);
        x264enc.set_property("key-int-max", config.gop);

        let sink_caps = gstreamer::Caps::builder("video/x-h264")
            .field("stream-format", "avc")
            .field("alignment", "au")
            .build();
        let appsink = gstreamer_app::AppSink::builder().caps(&sink_caps).build();

        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &videoconvert,
                &x264enc,
                &h264parse,
                appsink.upcast_ref(),
            ])
            .map_err(|e| CodecError::Encoder(format!("add: {e}")))?;
        gstreamer::Element::link_many([
            appsrc.upcast_ref(),
            &videoconvert,
            &x264enc,
            &h264parse,
            appsink.upcast_ref(),
        ])
        .map_err(|e| CodecError::Encoder(format!("link: {e}")))?;

        pipeline
            .set_state(gstreamer::State::Playing)
            .map_err(|e| CodecError::Encoder(format!("play: {e}")))?;

        self.pipeline = Some(pipeline);
        self.appsrc = Some(appsrc);
        self.appsink = Some(appsink);
        self.buffer.clear();
        self.stats = EncoderStats::default();
        Ok(())
    }

    fn push_frame(&mut self, frame: &VideoFrame) -> Result<(), CodecError> {
        let appsrc = self
            .appsrc
            .as_ref()
            .ok_or_else(|| CodecError::InvalidState("encoder not configured".into()))?;
        let appsink = self
            .appsink
            .as_ref()
            .ok_or_else(|| CodecError::InvalidState("encoder not configured".into()))?;

        let i420 = pack_i420(frame)?;
        let mut buf = gstreamer::Buffer::from_mut_slice(i420);
        {
            let bref =
                buf.get_mut().ok_or_else(|| CodecError::Encoder("buffer not writable".into()))?;
            bref.set_pts(gstreamer::ClockTime::from_nseconds(frame.pts));
        }

        appsrc.push_buffer(buf).map_err(|e| CodecError::Encoder(format!("push_buffer: {e}")))?;
        self.stats.frames_encoded += 1;

        let timeout = gstreamer::ClockTime::from_mseconds(5);
        drain_appsink_packets(appsink, &mut self.buffer, &mut self.stats, timeout)?;
        Ok(())
    }

    fn pull_packet(&mut self) -> Result<Option<EncodedPacket>, CodecError> {
        Ok(self.buffer.pop())
    }

    fn flush(&mut self) -> Result<(), CodecError> {
        // ponytail: drain buffered packets without EOS — caller may push again
        if let Some(appsink) = &self.appsink {
            let timeout = gstreamer::ClockTime::from_mseconds(100);
            let mut dummy_stats = EncoderStats::default();
            let _ = drain_appsink_packets(appsink, &mut self.buffer, &mut dummy_stats, timeout);
        }
        Ok(())
    }

    fn stats(&self) -> EncoderStats {
        self.stats.clone()
    }
}

// ══════════════════════════════════════════════════════════════════════
// GstDecoder
// ══════════════════════════════════════════════════════════════════════

pub(crate) struct GstDecoder {
    pipeline: Option<gstreamer::Pipeline>,
    appsrc: Option<gstreamer_app::AppSrc>,
    appsink: Option<gstreamer_app::AppSink>,
    buffer: Vec<VideoFrame>,
}

impl Default for GstDecoder {
    fn default() -> Self {
        gstreamer::init().ok();
        Self { pipeline: None, appsrc: None, appsink: None, buffer: Vec::new() }
    }
}

impl VideoDecoder for GstDecoder {
    fn configure(&mut self, config: &DecoderConfig) -> Result<(), CodecError> {
        if config.codec != CodecId::H264 {
            return Err(CodecError::UnsupportedCodec(config.codec));
        }

        let pipeline = gstreamer::Pipeline::default();

        let src_caps = gstreamer::Caps::builder("video/x-h264")
            .field("stream-format", "avc")
            .field("alignment", "au")
            .build();
        let appsrc = gstreamer_app::AppSrc::builder()
            .caps(&src_caps)
            .format(gstreamer::Format::Time)
            .build();

        let h264parse = gstreamer::ElementFactory::make("h264parse")
            .build()
            .map_err(|e| CodecError::Decoder(format!("h264parse: {e}")))?;
        let avdec_h264 = gstreamer::ElementFactory::make("avdec_h264")
            .build()
            .map_err(|e| CodecError::Decoder(format!("avdec_h264: {e}")))?;
        let videoconvert = gstreamer::ElementFactory::make("videoconvert")
            .build()
            .map_err(|e| CodecError::Decoder(format!("videoconvert: {e}")))?;

        let sink_caps = gstreamer::Caps::builder("video/x-raw").field("format", "I420").build();
        let appsink = gstreamer_app::AppSink::builder().caps(&sink_caps).build();

        pipeline
            .add_many([
                appsrc.upcast_ref(),
                &h264parse,
                &avdec_h264,
                &videoconvert,
                appsink.upcast_ref(),
            ])
            .map_err(|e| CodecError::Decoder(format!("add: {e}")))?;
        gstreamer::Element::link_many([
            appsrc.upcast_ref(),
            &h264parse,
            &avdec_h264,
            &videoconvert,
            appsink.upcast_ref(),
        ])
        .map_err(|e| CodecError::Decoder(format!("link: {e}")))?;

        pipeline
            .set_state(gstreamer::State::Playing)
            .map_err(|e| CodecError::Decoder(format!("play: {e}")))?;

        self.pipeline = Some(pipeline);
        self.appsrc = Some(appsrc);
        self.appsink = Some(appsink);
        self.buffer.clear();
        Ok(())
    }

    fn push_packet(&mut self, data: &[u8]) -> Result<(), CodecError> {
        if data.is_empty() {
            return Ok(());
        }
        let appsrc = self
            .appsrc
            .as_ref()
            .ok_or_else(|| CodecError::InvalidState("decoder not configured".into()))?;
        let appsink = self
            .appsink
            .as_ref()
            .ok_or_else(|| CodecError::InvalidState("decoder not configured".into()))?;

        let buf = gstreamer::Buffer::from_slice(data.to_vec());
        appsrc.push_buffer(buf).map_err(|e| CodecError::Decoder(format!("push_buffer: {e}")))?;

        let timeout = gstreamer::ClockTime::from_mseconds(5);
        drain_appsink_frames(appsink, &mut self.buffer, timeout)?;
        Ok(())
    }

    fn pull_frame(&mut self) -> Result<Option<VideoFrame>, CodecError> {
        Ok(self.buffer.pop())
    }

    fn flush(&mut self) -> Result<(), CodecError> {
        // ponytail: drain buffered frames without EOS — caller may push again
        if let Some(appsink) = &self.appsink {
            let timeout = gstreamer::ClockTime::from_mseconds(100);
            let _ = drain_appsink_frames(appsink, &mut self.buffer, timeout);
        }
        Ok(())
    }
}
