# SDD 01: CameraCapture

## 1. Overview

Camera capture module. Converts raw platform camera streams into unified I420 frames, fed into the PipelineEngine.

**Decision refs**: D64 (CameraCapture trait), D75 (I420 raw stream standard)

## 2. Interface Definition

```rust
/// Camera source type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraSource {
    V4L2(String),         // /dev/videoN (Linux)
    AVFoundation(String), // Device UID (macOS)
    DirectShow(String),   // Device name (Windows)
    RTSP(String),         // rtsp:// address
}

bitflags! {
    pub struct CameraCaps: u32 {
        const I420       = 1 << 0;  // Native I420 output
        const NV12       = 1 << 1;  // Native NV12 output
        const MJPG       = 1 << 2;  // MJPEG compressed
        const AUTO_FOCUS = 1 << 3;
        const HOTPLUG    = 1 << 4;  // Hot-plug support
    }
}

#[async_trait]
pub trait CameraCapture: Send + Sync {
    fn name(&self) -> &str;
    fn caps(&self) -> CameraCaps;
    fn list_devices(&self) -> Result<Vec<CameraDevice>>;
    async fn open(&mut self, source: &CameraSource) -> Result<()>;
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn poll_frame(&mut self) -> Result<Option<I420Frame>>;
    fn resolution(&self) -> (u32, u32);
    fn fps(&self) -> u32;
}
```

### Output Frame Structure

```rust
pub struct I420Frame {
    pub y: Vec<u8>,         // Y plane
    pub u: Vec<u8>,         // U plane (half width/height)
    pub v: Vec<u8>,         // V plane (half width/height)
    pub width: u32,
    pub height: u32,
    pub pts: u64,           // Capture timestamp (μs)
    pub source: CameraSource,
}
```

## 3. Platform Implementations

| Platform | Backend | Capture Notes |
|----------|---------|---------------|
| Linux | GStreamer v4l2src | /dev/videoN, autovideosrc |
| macOS | GStreamer avfvideosrc | AVFoundation devices |
| Windows | GStreamer dshowvideosrc | DirectShow devices |

## 4. Error Handling (D-ERR-01)

| Condition | Category | Error Code | Recovery |
|-----------|----------|------------|----------|
| Device not found | Fatal | 3001 | Exit after 3 retries |
| Frame lost | Recoverable | 3002 | Skip the frame, keep capturing |
| Unsupported resolution | Recoverable | 3003 | Fall back to 720p |
| Hot-plug disconnect | Recoverable | 3002 | Wait 1s and rebuild pipeline |

## 5. Pipeline Integration

```
CameraSource → GStreamer pipeline → appsink poll
     │              │                      │
     │              ▼                      ▼
     └── videoconvert ──▶ I420Frame ──▶ PipelineEngine
```

Integrated via the `poll_frame` polling model; the PipelineEngine drives the capture loop.

## 6. Test Plan

| Test | Type | Description |
|------|------|-------------|
| frame_format_validation | Unit | Output frame is I420, width/height match, YUV plane sizes correct |
| resolution_change | Unit | Frame dimensions update correctly after resolution switch |
| device_list | Unit | list_devices returns a non-empty list |
| open_close | Integration | Full open → start → stop → close lifecycle |
| device_hotplug | Integration | Pipeline rebuild after device disconnect (mock udev) |
| fps_stability | Integration | Measured ≥28fps with 30fps configured |
| rtsp_source | Integration | RTSP source connect and frame receive |
| error_recovery_fatal | Unit | Exit flow triggered after 3001 error repeats 3 times |
