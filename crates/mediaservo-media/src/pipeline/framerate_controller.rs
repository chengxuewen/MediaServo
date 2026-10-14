//! FramerateController — VFR → CFR framerate throttle (ported from OpenCTK / libwebrtc).
//!
//! Camera output is variable frame rate (VFR); the encoder needs a fixed frame rate (CFR). The controller
//! decides frame drops from frame timestamps (nanoseconds, e.g. V4L2 buffer timestamp), keeping the output cadence ≤ max_framerate.
//!
//! Semantics mirror OpenCTK `framerate_controller.cpp`:
//! 1. `kMinFramerate = 0.5` — max_framerate below this → drop everything
//! 2. `frame_interval_ns = 1e9 / fps` integer division; `<= 0` (huge fps) → throttling disabled
//! 3. Reset branch: `abs(time_until) >= 2×interval` → reset to `ts + interval/2`
//!    (tolerant handling on first frame / large timestamp jumps; prefer keeping frames)
//! 4. Dual-path advance: `should_drop_frame` advances next in the "output time" branch;
//!    `keep_frame` advances the drop branch too (caller forces a keep; cadence moves one frame)
//!
//! Timestamp unit: **nanoseconds** (matches the V4L2 buffer timestamp; µs values are ×1000 by the caller).

const K_MIN_FRAMERATE: f64 = 0.5;
const NSECS_PER_SEC: i64 = 1_000_000_000;

/// Framerate throttle controller (VFR→CFR).
#[derive(Debug, Clone)]
pub struct FramerateController {
    max_framerate: f64,
    next_frame_timestamp_ns: Option<i64>,
}

impl Default for FramerateController {
    fn default() -> Self {
        Self::new()
    }
}

impl FramerateController {
    /// Creates the controller with throttling off by default (max_framerate = f64::MAX).
    pub fn new() -> Self {
        Self { max_framerate: f64::MAX, next_frame_timestamp_ns: None }
    }

    /// Creates the controller and sets the maximum frame rate.
    pub fn with_max_framerate(fps: f64) -> Self {
        Self { max_framerate: fps, next_frame_timestamp_ns: None }
    }

    pub fn set_max_framerate(&mut self, fps: f64) {
        self.max_framerate = fps;
    }

    pub fn max_framerate(&self) -> f64 {
        self.max_framerate
    }

    /// Resets to the initial state (no throttling + cadence cleared).
    pub fn reset(&mut self) {
        self.max_framerate = f64::MAX;
        self.next_frame_timestamp_ns = None;
    }

    /// Framerate throttle decision: returns true = this frame should be dropped.
    ///
    /// Side effect: the output-time / first / reset branches advance `next_frame_timestamp_ns`.
    pub fn should_drop_frame(&mut self, ts_ns: i64) -> bool {
        if self.max_framerate < K_MIN_FRAMERATE {
            return true;
        }
        let frame_interval_ns = NSECS_PER_SEC / self.max_framerate as i64;
        if frame_interval_ns <= 0 {
            return false; // Throttling not enabled (huge fps → interval truncates to 0)
        }

        if let Some(next) = self.next_frame_timestamp_ns {
            let time_until_next_frame_ns = next - ts_ns;
            // Within the expected range (±2×interval) → normal throttle decision
            if time_until_next_frame_ns.abs() < 2 * frame_interval_ns {
                if time_until_next_frame_ns > 0 {
                    return true; // Frame arrived early: drop
                }
                // Output time: cadence advances one frame
                self.next_frame_timestamp_ns = Some(next + frame_interval_ns);
                return false;
            }
            // Large timestamp jump → falls to the reset branch below
        }

        // First frame / out-of-range reset: target set to ts + interval/2 (prefer keeping frames under jitter)
        self.next_frame_timestamp_ns = Some(ts_ns + frame_interval_ns / 2);
        false
    }

    /// Forces keeping this frame (caller requirement, e.g. the frame is a necessary reference frame).
    ///
    /// Internally runs the drop decision first; if judged a drop (drop branch, next not advanced), advances the cadence one frame
    /// — so subsequent frames output on the new cadence (consistent with OpenCTK dual-path advance).
    pub fn keep_frame(&mut self, ts_ns: i64) {
        let dropped = self.should_drop_frame(ts_ns);
        if !dropped {
            return;
        }
        // drop path: this frame is kept, cadence advances one frame
        if self.max_framerate < K_MIN_FRAMERATE {
            return;
        }
        let frame_interval_ns = NSECS_PER_SEC / self.max_framerate as i64;
        if frame_interval_ns <= 0 {
            return;
        }
        if let Some(next) = self.next_frame_timestamp_ns {
            self.next_frame_timestamp_ns = Some(next + frame_interval_ns);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL_30FPS_NS: i64 = 33_333_333; // 1e9/30

    #[test]
    fn keep_path_outputs_frames_at_max_rate() {
        // Under 30fps throttling, frames at 33ms intervals are all kept (first-frame reset → subsequent output-time advance)
        let mut c = FramerateController::with_max_framerate(30.0);
        let mut ts = 0i64;
        let mut kept = 0;
        for _ in 0..10 {
            if !c.should_drop_frame(ts) {
                kept += 1;
            }
            ts += INTERVAL_30FPS_NS;
        }
        // First-frame reset keeps + each subsequent frame arrives at output time → all kept
        assert_eq!(kept, 10, "33ms intervals (30fps) should all be kept");
    }

    #[test]
    fn drop_path_drops_early_frames() {
        // Frame arrives early (before the next output time) → drop
        let mut c = FramerateController::with_max_framerate(30.0);
        // First-frame reset: next = 0 + 16.67ms
        assert!(!c.should_drop_frame(0));
        // Next frame at 10ms (before next 16.67ms) → time_until > 0 → drop
        assert!(c.should_drop_frame(10_000_000), "arriving before output time should drop");
    }

    #[test]
    fn reset_path_tolerates_timestamp_jump() {
        // Large timestamp jump (> 2×interval) → reset instead of consecutive drops
        let mut c = FramerateController::with_max_framerate(30.0);
        assert!(!c.should_drop_frame(0));
        // Jump of 1s (far more than 2×33ms) → reset, no drop
        assert!(!c.should_drop_frame(1_000_000_000), "large jump should reset instead of dropping");
        // Cadence recovers after reset: the next frame arrives early → drop
        assert!(c.should_drop_frame(1_000_000_000 + 10_000_000));
    }

    #[test]
    fn keep_frame_advances_rhythm_on_drop_path() {
        // keep_frame: force keep in the drop branch and advance the cadence
        let mut c = FramerateController::with_max_framerate(30.0);
        c.keep_frame(0); // First frame kept (reset)
        c.keep_frame(20_000_000); // Early frame: judged drop but forced keep → advance
        // After advancing, the next output time = 20ms + interval ≈ 53ms
        // Arriving at 53ms → output-time branch (advance) → keep
        assert!(!c.should_drop_frame(53_333_333), "after keep advance, 53ms should output");
    }

    #[test]
    fn min_framerate_drops_everything() {
        let mut c = FramerateController::with_max_framerate(0.1); // < 0.5
        assert!(c.should_drop_frame(0));
        assert!(c.should_drop_frame(1_000_000_000));
    }

    #[test]
    fn default_disables_throttling() {
        let mut c = FramerateController::new();
        // max_framerate = MAX → interval truncates to 0 → throttling disabled
        assert!(!c.should_drop_frame(0));
        assert!(!c.should_drop_frame(1));
        assert!(!c.should_drop_frame(2));
    }
}
