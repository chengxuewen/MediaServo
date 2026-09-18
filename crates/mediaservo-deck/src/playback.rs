//! 回放域（playback）— Player：demux + decode，逐帧产出解码帧。
//!
//! MVP: FFmpeg 后端（ffmpeg-the-third 6.0），H264 → I420/原始格式解码。
//! 入口参考契约 §6：`Player::open(path)` → `next_frame()` 逐帧消费。

use std::path::PathBuf;

use mediaservo_codec::frame::{Plane, VideoFrame};

use crate::DeckError;

/// 回放器：打开媒体文件，逐帧产出解码帧。
#[allow(dead_code)] // path/w/h 在非 ffmpeg 后端姿态不读（cfg 差异 = PIT-203 族）
pub struct Player {
    path: PathBuf,
    #[cfg(feature = "backend-ffmpeg")]
    inner: Option<PlayerInner>,
    #[cfg(not(feature = "backend-ffmpeg"))]
    inner: Option<()>,
}

impl Player {
    /// 打开媒体文件（demux 初始化 + 首个视频流解码器就绪）。
    #[cfg(feature = "backend-ffmpeg")]
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DeckError> {
        let path = path.into();
        if !path.exists() {
            return Err(DeckError::NotFound(format!(
                "media file {} does not exist",
                path.display()
            )));
        }
        let inner = PlayerInner::open(&path)?;
        Ok(Self { path, inner: Some(inner) })
    }

    /// 取下一帧；None = 文件解码完毕。
    #[cfg(feature = "backend-ffmpeg")]
    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>, DeckError> {
        match self.inner.as_mut() {
            Some(inner) => inner.next_frame(),
            None => Ok(None),
        }
    }

    /// 文件时长（秒）。
    #[cfg(feature = "backend-ffmpeg")]
    pub fn duration_secs(&self) -> Result<f64, DeckError> {
        self.inner
            .as_ref()
            .ok_or_else(|| DeckError::InvalidState("not open".into()))?
            .duration_secs()
    }

    /// 打开媒体文件（无 FFmpeg 后端时明确报错）。
    #[cfg(not(feature = "backend-ffmpeg"))]
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, DeckError> {
        let path = path.into();
        if !path.exists() {
            return Err(DeckError::NotFound(format!(
                "media file {} does not exist",
                path.display()
            )));
        }
        Err(DeckError::Codec("playback requires backend-ffmpeg feature".into()))
    }
}

/// 播放器内部状态（demux + decoder）。
#[cfg(feature = "backend-ffmpeg")]
#[allow(dead_code)] // w/h 在解码器重建路径备用（ffmpeg 姿态未读 = 设计保留位）
struct PlayerInner {
    input: ffmpeg_the_third::format::context::Input,
    stream_index: usize,
    decoder: ffmpeg_the_third::decoder::Video,
    width: u32,
    height: u32,
}

#[cfg(feature = "backend-ffmpeg")]
impl PlayerInner {
    fn open(path: &PathBuf) -> Result<Self, DeckError> {
        use ffmpeg_the_third as ffmpeg;
        ffmpeg::init().map_err(|e| DeckError::Codec(format!("ffmpeg init: {e}")))?;

        let input = ffmpeg::format::input(path)
            .map_err(|e| DeckError::Io(std::io::Error::other(format!("open input: {e}"))))?;

        // 找第一个视频流
        let mut stream_index = None;
        for (idx, stream) in input.streams().enumerate() {
            if stream.parameters().medium() == ffmpeg::media::Type::Video {
                stream_index = Some(idx);
                break;
            }
        }
        let stream_index =
            stream_index.ok_or_else(|| DeckError::Codec("no video stream found".into()))?;

        // 在借用 input 的块内取出全部标量后再释放借用（input 将被 move 进结构）
        let (codec_id, w, h) = {
            let stream = input.stream(stream_index).expect("stream exists");
            let p = stream.parameters();
            (p.id(), p.width(), p.height())
        };

        // 创建解码器（video codec 从流参数识别）
        let codec = ffmpeg::decoder::find(codec_id)
            .ok_or_else(|| DeckError::Codec(format!("no decoder for codec {codec_id:?}")))?;
        let mut ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
        {
            let stream = input.stream(stream_index).expect("stream exists");
            let params = stream.parameters();
            ctx.set_parameters(params)
                .map_err(|e| DeckError::Codec(format!("set parameters: {e}")))?;
        }
        // decoder::Video(pub Opened) 即 open 后解码器（send_packet/receive_frame 经 Deref）
        let decoder =
            ctx.decoder().video().map_err(|e| DeckError::Codec(format!("create decoder: {e}")))?;

        Ok(Self { input, stream_index, decoder, width: w, height: h })
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, DeckError> {
        use ffmpeg_the_third as ffmpeg;
        let mut decoded = None;

        // 循环取包直到解码出一帧或 EOF。
        // PacketIter<Item = Result<(Stream, Packet), Error>>
        for item in self.input.packets() {
            let (stream, packet) =
                item.map_err(|e| DeckError::Codec(format!("read packet: {e}")))?;
            if stream.index() != self.stream_index {
                continue;
            }
            self.decoder
                .send_packet(&packet)
                .map_err(|e| DeckError::Codec(format!("send packet: {e}")))?;

            let mut avframe = ffmpeg::util::frame::Video::empty();
            match self.decoder.receive_frame(&mut avframe) {
                Ok(()) => {
                    decoded = Some(self.video_frame(&avframe));
                    break; // 出一帧即返回（逐帧消费）
                }
                Err(ffmpeg::Error::Other { errno: 11, .. }) => continue, // EAGAIN 等更多包
                Err(e) => return Err(DeckError::Codec(format!("receive frame: {e}"))),
            }
        }

        if decoded.is_none() {
            // flush 解码器残留帧（文件尾部）
            self.decoder.send_eof().map_err(|e| DeckError::Codec(format!("send_eof: {e}")))?;
            let mut avframe = ffmpeg::util::frame::Video::empty();
            if let Ok(()) = self.decoder.receive_frame(&mut avframe) {
                decoded = Some(self.video_frame(&avframe))
            }
        }
        Ok(decoded)
    }

    fn video_frame(&self, avframe: &ffmpeg_the_third::util::frame::Video) -> VideoFrame {
        let w = avframe.width();
        let h = avframe.height();
        // 平面格式（I420/P010 等）：逐平面复制（仅有效平面数）
        let plane_count = avframe.planes();
        let mut planes = Vec::new();
        for i in 0..plane_count {
            let stride = avframe.stride(i) as u32;
            let plane_len = match i {
                0 => w * h,
                _ => (w * h) / 4,
            };
            let data = if stride > 0 {
                let slice = avframe.data(i);
                let len = (plane_len as usize).min(slice.len());
                slice[..len].to_vec()
            } else {
                Vec::new()
            };
            if data.is_empty() {
                break;
            }
            planes.push(Plane { data, stride });
        }
        VideoFrame {
            format: mediaservo_codec::codec::VideoFormat {
                width: w,
                height: h,
                pixel_format: mediaservo_codec::codec::PixelFormat::Yuv420p,
            },
            planes,
            pts: avframe.pts().unwrap_or(0).max(0) as u64,
            keyframe: false,
        }
    }

    fn duration_secs(&self) -> Result<f64, DeckError> {
        let stream = self
            .input
            .stream(self.stream_index)
            .ok_or_else(|| DeckError::InvalidState("stream gone".into()))?;
        Ok(stream.duration() as f64 * stream.time_base().numerator() as f64
            / stream.time_base().denominator() as f64)
    }
}
