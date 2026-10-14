//! Playback domain — Player: demux + decode, yielding decoded frames one by one.
//!
//! MVP: FFmpeg backend (ffmpeg-the-third 6.0), H264 → I420/raw-format decoding.
//! Entry point per contract §6: `Player::open(path)` → `next_frame()` consumed frame by frame.

use std::path::PathBuf;

use mediaservo_codec::frame::{Plane, VideoFrame};

use crate::DeckError;

/// Player: opens a media file and yields decoded frames one by one.
#[allow(dead_code)] // path/w/h unread in the non-ffmpeg backend posture (cfg drift = PIT-203 family)
pub struct Player {
    path: PathBuf,
    #[cfg(feature = "backend-ffmpeg")]
    inner: Option<PlayerInner>,
    #[cfg(not(feature = "backend-ffmpeg"))]
    inner: Option<()>,
}

impl Player {
    /// Opens a media file (demux init + first video stream decoder ready).
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

    /// Returns the next frame; None = file fully decoded.
    #[cfg(feature = "backend-ffmpeg")]
    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>, DeckError> {
        match self.inner.as_mut() {
            Some(inner) => inner.next_frame(),
            None => Ok(None),
        }
    }

    /// File duration (seconds).
    #[cfg(feature = "backend-ffmpeg")]
    pub fn duration_secs(&self) -> Result<f64, DeckError> {
        self.inner
            .as_ref()
            .ok_or_else(|| DeckError::InvalidState("not open".into()))?
            .duration_secs()
    }

    /// Opens a media file (explicit error when no FFmpeg backend).
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

/// Player internal state (demux + decoder).
#[cfg(feature = "backend-ffmpeg")]
#[allow(dead_code)] // w/h kept for the decoder rebuild path (unread in the ffmpeg posture = reserved by design)
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

        // Find the first video stream
        let mut stream_index = None;
        for (idx, stream) in input.streams().enumerate() {
            if stream.parameters().medium() == ffmpeg::media::Type::Video {
                stream_index = Some(idx);
                break;
            }
        }
        let stream_index =
            stream_index.ok_or_else(|| DeckError::Codec("no video stream found".into()))?;

        // Extract all scalars from the borrowed input inside the borrow scope before releasing it (input is moved into the struct)
        let (codec_id, w, h) = {
            let stream = input.stream(stream_index).expect("stream exists");
            let p = stream.parameters();
            (p.id(), p.width(), p.height())
        };

        // Create the decoder (video codec identified from stream parameters)
        let codec = ffmpeg::decoder::find(codec_id)
            .ok_or_else(|| DeckError::Codec(format!("no decoder for codec {codec_id:?}")))?;
        let mut ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
        {
            let stream = input.stream(stream_index).expect("stream exists");
            let params = stream.parameters();
            ctx.set_parameters(params)
                .map_err(|e| DeckError::Codec(format!("set parameters: {e}")))?;
        }
        // decoder::Video(pub Opened) is the post-open decoder (send_packet/receive_frame via Deref)
        let decoder =
            ctx.decoder().video().map_err(|e| DeckError::Codec(format!("create decoder: {e}")))?;

        Ok(Self { input, stream_index, decoder, width: w, height: h })
    }

    fn next_frame(&mut self) -> Result<Option<VideoFrame>, DeckError> {
        use ffmpeg_the_third as ffmpeg;
        let mut decoded = None;

        // Loop over packets until a frame decodes or EOF.
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
                    break; // One frame decoded: return (frame-by-frame consumption)
                }
                Err(ffmpeg::Error::Other { errno: 11, .. }) => continue, // EAGAIN: wait for more packets
                Err(e) => return Err(DeckError::Codec(format!("receive frame: {e}"))),
            }
        }

        if decoded.is_none() {
            // Flush residual decoder frames (end of file)
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
        // Planar formats (I420/P010 etc.): copy plane by plane (valid plane count only)
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
