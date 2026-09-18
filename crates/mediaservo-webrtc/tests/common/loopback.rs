//! Loopback test harness — in-process SDP/ICE exchange between two PeerConnections.
//!
//! Provides helpers to create a connected RTCPeerConnection pair
//! without a signaling server, plus a FPS counter and test-frame generator.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use mediaservo_webrtc::RTCError;
use mediaservo_webrtc::factory::RTCPeerConnectionFactory;
use mediaservo_webrtc::peer_connection::{RTCAnswerOptions, RTCConfiguration, RTCOfferOptions};
use mediaservo_webrtc::traits::PeerConnectionApi;

/// FPS counter — tracks frame count over time.
///
/// # Example
/// ```ignore
/// let c = FpsCounter::new();
/// for _ in 0..30 { c.tick(); }
/// println!("{:.1} fps", c.fps());
/// ```
pub struct FpsCounter {
    count: AtomicU64,
    started: Instant,
}

impl FpsCounter {
    pub fn new() -> Self {
        Self { count: AtomicU64::new(0), started: Instant::now() }
    }

    pub fn tick(&self) {
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn fps(&self) -> f64 {
        let elapsed = self.started.elapsed().as_secs_f64();
        if elapsed > 0.0 { self.count() as f64 / elapsed } else { 0.0 }
    }
}

/// Complete SDP exchange between two PeerConnections.
///
/// PC1 creates an offer, PC2 answers. No ICE candidate exchange —
/// sufficient for stub backend and in-process loopback testing.
///
/// # Errors
/// Returns `RTCError` if any SDP operation fails.
pub async fn exchange_sdp(
    pc1: &mediaservo_webrtc::peer_connection::RTCPeerConnection,
    pc2: &mediaservo_webrtc::peer_connection::RTCPeerConnection,
) -> Result<(), RTCError> {
    // 1. PC1 creates offer
    let offer = pc1.create_offer(&RTCOfferOptions::default()).await?;
    pc1.set_local_description(&offer).await?;

    // 2. PC2 receives offer, creates answer
    pc2.set_remote_description(&offer).await?;
    let answer = pc2.create_answer(&RTCAnswerOptions).await?;
    pc2.set_local_description(&answer).await?;

    // 3. PC1 receives answer
    pc1.set_remote_description(&answer).await?;

    Ok(())
}

/// Create a pair of PeerConnections with SDP already exchanged.
///
/// Uses the active backend (default: `backend-webrtc-sys`).
/// Both PCs share the same `RTCPeerConnectionFactory`.
///
/// # Panics
/// If SDP exchange fails, this function panics.
pub async fn create_connected_pair() -> Result<
    (
        mediaservo_webrtc::peer_connection::RTCPeerConnection,
        mediaservo_webrtc::peer_connection::RTCPeerConnection,
    ),
    RTCError,
> {
    let factory = RTCPeerConnectionFactory::new();
    let pc1 = factory.create_peer_connection(RTCConfiguration::default()).await?;
    let pc2 = factory.create_peer_connection(RTCConfiguration::default()).await?;

    exchange_sdp(&pc1, &pc2).await?;

    Ok((pc1, pc2))
}

/// Generate a simple test-pattern frame (moving color bars) in I420 format.
///
/// Returns raw Y+U+V planar data:
/// - Y plane: shifting color bars (black/gray/light-gray/white)
/// - U, V planes: flat gray (128)
///
/// Total size: `width * height * 3 / 2` bytes.
pub fn generate_test_frame(width: u32, height: u32, frame_index: u64) -> Vec<u8> {
    let y_size = (width * height) as usize;
    let uv_size = (width * height / 4) as usize;
    let total = y_size + 2 * uv_size;
    let mut frame = vec![0u8; total];

    // Y plane: color bars that shift with frame_index
    let bar_width = width / 4;
    let shift = (frame_index % 100) as u32;
    for y in 0..height {
        for x in 0..width {
            let bar = ((x + shift) / bar_width) % 4;
            let y_val: u8 = match bar {
                0 => 0,   // black
                1 => 128, // gray
                2 => 200, // light gray
                _ => 255, // white
            };
            frame[(y * width + x) as usize] = y_val;
        }
    }

    // U and V planes: flat gray (no chroma)
    let uv_offset = y_size;
    for i in 0..2 * uv_size {
        frame[uv_offset + i] = 128;
    }

    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_counter_counts_ticks() {
        let c = FpsCounter::new();
        for _ in 0..30 {
            c.tick();
        }
        assert_eq!(c.count(), 30);
    }

    #[test]
    fn fps_counter_reports_positive_rate() {
        let c = FpsCounter::new();
        for _ in 0..30 {
            c.tick();
        }
        assert!(c.fps() > 0.0, "fps should be positive");
    }

    #[test]
    fn generate_frame_has_correct_i420_size() {
        for (w, h) in [(320, 240), (640, 480), (1920, 1080)] {
            let frame = generate_test_frame(w, h, 0);
            let expected = (w * h) as usize + 2 * ((w * h) / 4) as usize;
            assert_eq!(frame.len(), expected, "size mismatch for {w}x{h}");
        }
    }

    #[test]
    fn generate_frame_changes_with_index() {
        let frame0 = generate_test_frame(320, 240, 0);
        let frame50 = generate_test_frame(320, 240, 50);
        assert_ne!(frame0, frame50, "frames with different indices should differ");
    }
}
