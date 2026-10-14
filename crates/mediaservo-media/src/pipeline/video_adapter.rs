//! VideoAdapter — source adaptation component (frame drop + resolution adaptation).
//!
//! Consumers: non-SFU local sinks (recording/preview/webrtc-rs backend) — on the SFU production path
//! the libwebrtc backend already does AdaptFrame internally, so this component is not involved (PIT-62 review conclusion).
//!
//! Design mirrors OpenCTK `video_adapter.hpp`:
//! - Frame drop decision delegated to [`FramerateController`] (VFR→CFR)
//! - Resolution adaptation: aspect-ratio-preserving scale into the target resolution
//!   (full crop-then-scale needs crop support at the buffer layer — currently provides scale math, TODO)

use super::framerate_controller::FramerateController;

/// Adaptation counters (observability — mirrors OpenCTK video_adaptation_counters).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdaptationCounters {
    /// Frames dropped due to framerate throttling.
    pub frames_dropped: u64,
    /// Frames scaled due to resolution adaptation.
    pub frames_scaled: u64,
}

/// Source adapter: adapts frames to consumer demands (fps cap + resolution cap).
#[derive(Debug, Clone)]
pub struct VideoAdapter {
    framerate_controller: FramerateController,
    /// Target max resolution (aspect-preserving scale); None = no scaling.
    scale_resolution_down_to: Option<(u32, u32)>,
    counters: AdaptationCounters,
}

impl Default for VideoAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoAdapter {
    pub fn new() -> Self {
        Self {
            framerate_controller: FramerateController::new(),
            scale_resolution_down_to: None,
            counters: AdaptationCounters::default(),
        }
    }

    /// Sets the frame rate cap (VFR→CFR throttling).
    pub fn set_max_framerate(&mut self, fps: f64) {
        self.framerate_controller.set_max_framerate(fps);
    }

    /// Sets the maximum resolution (aspect-preserving scale; None disables).
    pub fn set_scale_resolution_down_to(&mut self, max: Option<(u32, u32)>) {
        self.scale_resolution_down_to = max;
    }

    /// Drop decision: returns true = this frame should be dropped (framerate throttle).
    pub fn should_drop_frame(&mut self, ts_ns: i64) -> bool {
        let drop = self.framerate_controller.should_drop_frame(ts_ns);
        if drop {
            self.counters.frames_dropped += 1;
        }
        drop
    }

    /// Resolution adaptation: returns the scaled (w, h); None = no scaling needed.
    ///
    /// Scales preserving aspect ratio into `scale_resolution_down_to`.
    /// TODO(crop): full crop-then-scale needs crop support at the buffer layer (OpenCTK adaptFrameResolution).
    pub fn adapt_resolution(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        let (max_w, max_h) = self.scale_resolution_down_to?;
        if width <= max_w && height <= max_h {
            return None;
        }
        // Preserve aspect ratio: take the smaller of the two scale factors
        let scale_w = max_w as f64 / width as f64;
        let scale_h = max_h as f64 / height as f64;
        let scale = scale_w.min(scale_h);
        let out_w = ((width as f64 * scale).round() as u32).max(2);
        let out_h = ((height as f64 * scale).round() as u32).max(2);
        if out_w == width && out_h == height {
            None
        } else {
            self.counters.frames_scaled += 1;
            Some((out_w, out_h))
        }
    }

    /// Current adaptation counters (debug/observability).
    pub fn counters(&self) -> AdaptationCounters {
        self.counters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_frame_throttles_by_framerate() {
        let mut a = VideoAdapter::new();
        a.set_max_framerate(30.0);
        assert!(!a.should_drop_frame(0), "first frame kept");
        assert!(a.should_drop_frame(10_000_000), "early arrival dropped");
        assert!(!a.should_drop_frame(33_333_333), "output time kept");
    }

    #[test]
    fn adapt_resolution_scales_down_preserving_aspect() {
        let mut a = VideoAdapter::new();
        // Not set → no scaling
        assert_eq!(a.adapt_resolution(640, 480), None);

        let mut a = VideoAdapter::new();
        a.set_scale_resolution_down_to(Some((320, 240)));
        // 640x480 → 320x240 (proportional)
        assert_eq!(a.adapt_resolution(640, 480), Some((320, 240)));
        // 1280x720 → 320x180 (16:9 proportional)
        assert_eq!(a.adapt_resolution(1280, 720), Some((320, 180)));
        // Already smaller than the target → no scaling
        assert_eq!(a.adapt_resolution(160, 120), None);
        // One side over the limit → scale by the smaller factor (100x400 → 60x240, keeps 1:4)
        assert_eq!(a.adapt_resolution(100, 400), Some((60, 240)));
    }

    #[test]
    fn adapt_resolution_handles_non_square_targets() {
        let mut a = VideoAdapter::new();
        a.set_scale_resolution_down_to(Some((640, 360))); // 16:9 target
        assert_eq!(a.adapt_resolution(1920, 1080), Some((640, 360)));
    }
}

#[test]
fn counters_track_drops_and_scales() {
    let mut a = VideoAdapter::new();
    a.set_max_framerate(30.0);
    assert!(!a.should_drop_frame(0));
    assert!(a.should_drop_frame(10_000_000)); // drop
    assert_eq!(a.counters().frames_dropped, 1);

    a.set_scale_resolution_down_to(Some((320, 240)));
    assert_eq!(a.adapt_resolution(640, 480), Some((320, 240))); // scale
    assert_eq!(a.counters().frames_scaled, 1);
    assert_eq!(a.adapt_resolution(160, 120), None); // no scale
    assert_eq!(a.counters().frames_scaled, 1);
}
