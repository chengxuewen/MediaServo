//! FFmpeg backend — real encoding/decoding via ffmpeg-the-third.

#[cfg(feature = "backend-ffmpeg")]
mod imp {
    use crate::codec::CodecId;
    use crate::config::{Bitrate, DecoderConfig, EncoderConfig, EncoderPreset};
    use crate::decoder::VideoDecoder;
    use crate::encoder::{EncoderStats, VideoEncoder};
    use crate::error::CodecError;
    use crate::frame::VideoFrame;
    use crate::packet::EncodedPacket;
    use ffmpeg_the_third as ffmpeg;

    fn x264_opts(config: &EncoderConfig) -> ffmpeg::Dictionary {
        let mut opts = ffmpeg::Dictionary::new();
        let preset = match config.preset {
            EncoderPreset::P1UltraFast => "ultrafast",
            EncoderPreset::P2SuperFast => "superfast",
            EncoderPreset::P3VeryFast => "veryfast",
            EncoderPreset::P4Medium => "medium",
            EncoderPreset::P5Slow => "slow",
            EncoderPreset::P6VerySlow => "veryslow",
            EncoderPreset::P7Lossless => "veryslow",
        };
        opts.set("preset", preset);
        opts.set("tune", "zerolatency");
        let br = match config.bitrate {
            Bitrate::Cbr(b) => b as i64 * 1000,
            Bitrate::Vbr { target, .. } => target as i64 * 1000,
        };
        opts.set("b", &br.to_string());
        opts
    }

    pub(crate) struct FfmpegEncoder {
        encoder: Option<ffmpeg::encoder::Video>,
        packets: Vec<EncodedPacket>,
        stats: EncoderStats,
    }

    impl Default for FfmpegEncoder {
        fn default() -> Self {
            Self { encoder: None, packets: vec![], stats: EncoderStats::default() }
        }
    }

    impl VideoEncoder for FfmpegEncoder {
        fn configure(&mut self, config: &EncoderConfig) -> Result<(), CodecError> {
            let codec = ffmpeg::codec::encoder::find(ffmpeg::codec::Id::H264)
                .ok_or_else(|| CodecError::UnsupportedCodec(CodecId::H264))?;
            let mut ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
            let mut enc =
                ctx.encoder().video().map_err(|e| CodecError::Encoder(format!("create: {e}")))?;
            enc.set_width(config.format.width);
            enc.set_height(config.format.height);
            enc.set_format(ffmpeg::format::Pixel::YUV420P);
            enc.set_time_base(ffmpeg::Rational(1, config.fps.num as i32));
            enc.set_gop(config.gop);
            enc.set_max_b_frames(0);
            self.encoder = Some(
                enc.open_with(x264_opts(config))
                    .map_err(|e| CodecError::Encoder(format!("open: {e}")))?,
            );
            self.stats = EncoderStats::default();
            Ok(())
        }

        fn push_frame(&mut self, frame: &VideoFrame) -> Result<(), CodecError> {
            let enc =
                self.encoder.as_mut().ok_or(CodecError::InvalidState("not configured".into()))?;
            let w = frame.width() as usize;
            let h = frame.height() as usize;
            let mut avframe =
                ffmpeg::util::frame::Video::new(ffmpeg::format::Pixel::YUV420P, w as u32, h as u32);
            for i in 0..3 {
                if let (Some(src), Some(dst)) = (frame.plane_data(i), Some(avframe.data_mut(i))) {
                    let n = dst.len().min(src.len());
                    dst[..n].copy_from_slice(&src[..n]);
                }
            }
            avframe.set_pts(Some(frame.pts as i64));
            enc.send_frame(&avframe).map_err(|e| CodecError::Encoder(format!("send: {e}")))?;
            self.stats.frames_encoded += 1;

            // Receive packets immediately (B-frames=0, so one frame = one packet)
            loop {
                let mut pkt = ffmpeg::codec::packet::Packet::empty();
                match enc.receive_packet(&mut pkt) {
                    Ok(_) => {
                        self.stats.packets_produced += 1;
                        let data = pkt.data().unwrap_or(&[]).to_vec();
                        self.stats.bytes_encoded += data.len() as u64;
                        self.packets.push(EncodedPacket {
                            data,
                            pts: frame.pts,
                            dts: frame.pts,
                            keyframe: pkt.is_key(),
                            codec: CodecId::H264,
                        });
                    }
                    Err(_) => break,
                }
            }
            Ok(())
        }

        fn pull_packet(&mut self) -> Result<Option<EncodedPacket>, CodecError> {
            Ok(self.packets.pop())
        }

        fn flush(&mut self) -> Result<(), CodecError> {
            if let Some(enc) = &mut self.encoder {
                enc.send_eof().map_err(|e| CodecError::Encoder(format!("flush: {e}")))?;
            }
            Ok(())
        }

        fn stats(&self) -> EncoderStats {
            self.stats.clone()
        }
    }

    pub(crate) struct FfmpegDecoder {
        decoder: Option<ffmpeg::decoder::Video>,
        frames: Vec<VideoFrame>,
    }

    impl Default for FfmpegDecoder {
        fn default() -> Self {
            Self { decoder: None, frames: vec![] }
        }
    }

    impl VideoDecoder for FfmpegDecoder {
        fn configure(&mut self, config: &DecoderConfig) -> Result<(), CodecError> {
            let _ = config;
            let codec = ffmpeg::codec::decoder::find(ffmpeg::codec::Id::H264)
                .ok_or_else(|| CodecError::UnsupportedCodec(CodecId::H264))?;
            let ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
            self.decoder = Some(
                ctx.decoder().video().map_err(|e| CodecError::Decoder(format!("create: {e}")))?,
            );
            Ok(())
        }

        fn push_packet(&mut self, data: &[u8]) -> Result<(), CodecError> {
            if data.is_empty() {
                return Ok(());
            }
            let dec =
                self.decoder.as_mut().ok_or(CodecError::InvalidState("not configured".into()))?;
            let pkt = ffmpeg::codec::packet::Packet::copy(data);
            dec.send_packet(&pkt).map_err(|e| CodecError::Decoder(format!("push: {e}")))?;

            loop {
                let mut avframe = ffmpeg::util::frame::Video::empty();
                match dec.receive_frame(&mut avframe) {
                    Ok(_) => {
                        let w = avframe.width();
                        let h = avframe.height();
                        let y_stride = avframe.stride(0) as usize;
                        let u_stride = avframe.stride(1) as usize;
                        let v_stride = avframe.stride(2) as usize;
                        let y_n = y_stride * h as usize;
                        let uv_n = u_stride * (h / 2) as usize;
                        self.frames.push(VideoFrame {
                            format: crate::codec::VideoFormat {
                                width: w,
                                height: h,
                                pixel_format: crate::codec::PixelFormat::Yuv420p,
                            },
                            planes: vec![
                                crate::frame::Plane {
                                    data: avframe.data(0)[..y_n].to_vec(),
                                    stride: y_stride as u32,
                                },
                                crate::frame::Plane {
                                    data: avframe.data(1)[..uv_n].to_vec(),
                                    stride: u_stride as u32,
                                },
                                crate::frame::Plane {
                                    data: avframe.data(2)[..uv_n].to_vec(),
                                    stride: v_stride as u32,
                                },
                            ],
                            pts: 0,
                            keyframe: false,
                        });
                    }
                    Err(_) => break,
                }
            }
            Ok(())
        }

        fn pull_frame(&mut self) -> Result<Option<VideoFrame>, CodecError> {
            Ok(self.frames.pop())
        }
        fn flush(&mut self) -> Result<(), CodecError> {
            Ok(())
        }
    }
}

#[cfg(feature = "backend-ffmpeg")]
pub(crate) use imp::*;

#[cfg(not(feature = "backend-ffmpeg"))]
pub(crate) use crate::backend::stub::{StubDecoder as FfmpegDecoder, StubEncoder as FfmpegEncoder};
