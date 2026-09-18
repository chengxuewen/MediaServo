use crate::codec::VideoFormat;

/// A single plane of raw video data.
#[derive(Debug, Clone)]
pub struct Plane {
    pub data: Vec<u8>,
    pub stride: u32,
}

/// Raw video frame with I420/NV12 plane data (codec-crate internal, C5 boundary).
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub format: VideoFormat,
    pub planes: Vec<Plane>,
    pub pts: u64,
    pub keyframe: bool,
}

impl VideoFrame {
    pub fn width(&self) -> u32 {
        self.format.width
    }
    pub fn height(&self) -> u32 {
        self.format.height
    }
    pub fn plane_data(&self, index: usize) -> Option<&[u8]> {
        self.planes.get(index).map(|p| p.data.as_slice())
    }
    pub fn plane_stride(&self, index: usize) -> Option<u32> {
        self.planes.get(index).map(|p| p.stride)
    }
}
