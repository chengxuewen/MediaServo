#![allow(dead_code)]

use crate::error::MediaError;

// Type alias for all pipeline operations.
type Result<T> = std::result::Result<T, MediaError>;

// ── 1. Public API Types (SDK layer) ──

/// Unique codec identifier (RFC 6381, e.g. "avc1.640028").
pub type CodecId = String;

/// I420 is the standard internal format (D75).
use crate::pixel_format::PixelFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Encoded,
    Raw,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeType {
    Source,
    Processor,
    Sink,
}

#[derive(Debug, Clone)]
pub struct Plane {
    pub data: Vec<u8>,
    pub stride: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct TextureHandle {
    // ponytail: inner Arc<()> for now; replace with GPU handle when needed
    _inner: std::sync::Arc<()>,
}

impl TextureHandle {
    pub fn dummy() -> Self {
        Self { _inner: std::sync::Arc::new(()) }
    }
}

#[derive(Debug, Clone)]
pub struct FrameTiming {
    pub dts: u64,
    pub pts: u64,
    pub duration: u64,
    pub wall_clock: Option<std::time::Instant>,
}

#[derive(Debug, Clone)]
pub struct RawFrame {
    pub track_id: String,
    pub timing: FrameTiming,
    pub format: PixelFormat,
    pub planes: Vec<Plane>,
    pub texture: Option<TextureHandle>,
}

#[derive(Debug, Clone, Copy)]
pub struct FragmentFlags {
    pub keyframe: bool,
    pub independent: bool,
    pub discardable: bool,
}

#[derive(Debug, Clone)]
pub struct EncodedFragment {
    pub track_id: String,
    pub timing: FrameTiming,
    pub flags: FragmentFlags,
    pub codec: CodecId,
    pub init_data: Option<Vec<u8>>,
    pub payload: Vec<u8>,
}

// ── 2. Internal Pipeline Types ──

#[derive(Debug, Clone)]
pub enum InternalPacket {
    Encoded(EncodedFragment),
    Raw(RawFrame),
    Metadata(PacketMetadata),
}

#[derive(Debug, Clone)]
pub struct PacketMetadata {
    pub track_id: String,
    pub event: MetadataEvent,
}

#[derive(Debug, Clone)]
pub enum MetadataEvent {
    TrackStarted { codec: CodecId, timescale: u32 },
    TrackEnded,
    QualityChanged { target_bitrate: u32 },
}

// ── 3. Node Capability Types ──

#[derive(Debug, Clone)]
pub struct FormatSpec {
    pub media_type: MediaType,
    pub codecs: Option<Vec<CodecId>>,
    pub pixel_formats: Vec<PixelFormat>,
}

#[derive(Debug, Clone)]
pub struct NodeCapability {
    pub input: FormatSpec,
    pub output: FormatSpec,
}

/// Query used by PluginManager to find matching pipeline nodes.
#[derive(Debug, Clone)]
pub struct FormatQuery {
    pub node_type: NodeType,
    pub media_type: MediaType,
    pub codec: Option<CodecId>,
    pub pixel_format: Option<PixelFormat>,
}

// ── 4. Plugin Registry ──

/// Compile-time plugin registration entry (used with inventory::submit!).
#[derive(Debug, Clone)]
pub struct PluginRegistry {
    pub name: &'static str,
    pub version: (u16, u16, u16),
    pub node_type: NodeType,
    pub media_type: MediaType,
    pub codecs: Vec<CodecId>,
    pub pixel_formats: Vec<PixelFormat>,
    pub priority: u8,
    pub factory: fn() -> Box<dyn std::any::Any + Send>,
}

// ── 5. Node Traits ──

/// Base trait for all pipeline nodes.
pub trait NodeInfo: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> NodeCapability;
}

/// Lifecycle for runnable nodes — simplified 2-state model (Phase 1).
pub trait PipelineNode: NodeInfo {
    fn on_start(&mut self) -> Result<()>;
    fn on_stop(&mut self) -> Result<()>;
}

/// Producer: yields media data (camera, screen, network source).
/// LVQR FragmentStream::next_fragment pattern.
pub trait MediaSource: PipelineNode {
    type Output;
    fn poll_fragment(&mut self) -> Result<Option<Self::Output>>;
}

/// Processor: transforms media data (encode, decode, color convert).
/// GStreamer BaseTransformImpl::transform pattern.
pub trait MediaProcessor: PipelineNode {
    type Input;
    type Output;
    fn process(&mut self, input: Self::Input) -> Result<Self::Output>;
}

/// Sink: consumes media data (render, push, record).
/// OBS filter_video / encoder_packet callback pattern.
pub trait MediaSink: PipelineNode {
    type Input;
    fn on_fragment(&mut self, fragment: Self::Input) -> Result<()>;
}
