use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use super::sink::{SinkId, VideoSink, VideoSinkWants};
use super::source::VideoSource;
use crate::error::MediaError;

struct SinkEntry<F> {
    id: SinkId,
    sink: Box<dyn VideoSink<F>>,
    wants: VideoSinkWants,
}

/// Fan-out combinator that broadcasts frames to multiple downstream sinks.
///
/// `VideoBroadcaster<F>` implements both [`VideoSource<F>`] and
/// [`VideoSink<F>`]. The primary broadcast path is the inherent method
/// `on_frame(&self, &Arc<F>)`, which uses `Arc` for zero-copy sharing
/// across sinks. The [`VideoSink::on_frame`] trait impl is a collector
/// entry-point that expects the caller to already have an `Arc<F>` and
/// should call the inherent method instead.
pub struct VideoBroadcaster<F> {
    sinks: Mutex<Vec<SinkEntry<F>>>,
    next_id: AtomicU64,
}

impl<F: Send + 'static> VideoBroadcaster<F> {
    pub fn new() -> Self {
        Self { sinks: Mutex::new(Vec::new()), next_id: AtomicU64::new(1) }
    }

    /// Aggregate wants from all active sinks.
    ///
    /// - `max_pixel_count` → minimum across all sinks (zero values ignored).
    /// - `max_framerate_fps` → minimum across all sinks (zero values ignored).
    /// - `is_active` → `false` if ALL sinks are inactive.
    /// - `resolution_alignment` → LCM of all sink alignments.
    pub fn wants(&self) -> VideoSinkWants {
        let sinks = self.sinks.lock().unwrap();
        if sinks.is_empty() {
            return VideoSinkWants::default();
        }
        let mut aggregated = VideoSinkWants {
            is_active: false,
            max_pixel_count: u32::MAX,
            max_framerate_fps: u32::MAX,
            resolution_alignment: 1,
            rotation_applied: false,
        };
        for entry in sinks.iter() {
            let w = &entry.wants;
            if w.is_active {
                aggregated.is_active = true;
            }
            if w.max_pixel_count != 0 {
                aggregated.max_pixel_count = aggregated.max_pixel_count.min(w.max_pixel_count);
            }
            if w.max_framerate_fps != 0 {
                aggregated.max_framerate_fps =
                    aggregated.max_framerate_fps.min(w.max_framerate_fps);
            }
            aggregated.resolution_alignment =
                lcm(aggregated.resolution_alignment, w.resolution_alignment);
        }
        if !aggregated.is_active {
            aggregated.max_pixel_count = 0;
            aggregated.max_framerate_fps = 0;
        }
        aggregated
    }

    /// Broadcast an `Arc<F>` frame to all active sinks.
    ///
    /// Returns aggregated wants after delivery for dynamic backpressure.
    /// This is the primary broadcast API — callers should pass `Arc<F>` directly
    /// to avoid a Clone bound on `F`.
    pub fn send_frame(&self, arc_frame: &Arc<F>) -> Result<VideoSinkWants, MediaError> {
        let sinks = self.sinks.lock().unwrap();
        for entry in sinks.iter() {
            if entry.wants.is_active {
                entry.sink.on_frame(arc_frame.as_ref())?;
            }
        }
        // Re-aggregate after delivery (dynamic backpressure)
        drop(sinks);
        Ok(self.wants())
    }

    pub fn sink_count(&self) -> usize {
        self.sinks.lock().unwrap().len()
    }
}

impl<F: Send + 'static> Default for VideoBroadcaster<F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: Send + 'static> VideoSource<F> for VideoBroadcaster<F> {
    fn add_or_update_sink(&self, sink: Box<dyn VideoSink<F>>, wants: VideoSinkWants) -> SinkId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut sinks = self.sinks.lock().unwrap();
        sinks.push(SinkEntry { id, sink, wants });
        id
    }

    fn remove_sink(&self, id: SinkId) {
        let mut sinks = self.sinks.lock().unwrap();
        sinks.retain(|e| e.id != id);
    }
}

impl<F: Send + 'static> VideoSink<F> for VideoBroadcaster<F> {
    /// Entry-point for the collector role.
    ///
    /// This impl does **not** distribute the `&F` reference to downstream
    /// sinks because broadcasting requires `Arc<F>` for zero-copy sharing.
    /// Real broadcast happens via [`VideoBroadcaster::send_frame`].
    fn on_frame(&self, _frame: &F) -> Result<VideoSinkWants, MediaError> {
        Ok(self.wants())
    }
}

// ponytail: inline helpers; extract to a math util module when needed elsewhere.
fn lcm(a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 {
        return a.max(b);
    }
    a / gcd(a, b) * b
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        #[allow(clippy::manual_swap)]
        {
            let t = b;
            b = a % b;
            a = t;
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct TestSink {
        received: Arc<Mutex<Vec<u32>>>,
        wants: VideoSinkWants,
    }

    impl VideoSink<u32> for TestSink {
        fn on_frame(&self, frame: &u32) -> Result<VideoSinkWants, MediaError> {
            self.received.lock().unwrap().push(*frame);
            Ok(self.wants)
        }
    }

    fn make_sink(wants: VideoSinkWants) -> (Box<TestSink>, Arc<Mutex<Vec<u32>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Box::new(TestSink { received: received.clone(), wants });
        (sink, received)
    }

    #[test]
    fn broadcaster_add_remove_sink() {
        let b: VideoBroadcaster<u32> = VideoBroadcaster::new();
        assert_eq!(b.sink_count(), 0);

        let (sink, _recv) = make_sink(VideoSinkWants::default());
        let id = b.add_or_update_sink(sink, VideoSinkWants::default());
        assert_eq!(b.sink_count(), 1);

        b.remove_sink(id);
        assert_eq!(b.sink_count(), 0);
    }

    #[test]
    fn broadcaster_fans_out_to_all_sinks() {
        let b: VideoBroadcaster<u32> = VideoBroadcaster::new();

        let (s1, r1) = make_sink(VideoSinkWants::default());
        let (s2, r2) = make_sink(VideoSinkWants::default());
        let (s3, r3) = make_sink(VideoSinkWants::default());

        b.add_or_update_sink(s1, VideoSinkWants::default());
        b.add_or_update_sink(s2, VideoSinkWants::default());
        b.add_or_update_sink(s3, VideoSinkWants::default());

        b.send_frame(&Arc::new(42)).unwrap();

        assert_eq!(r1.lock().unwrap().len(), 1);
        assert_eq!(r2.lock().unwrap().len(), 1);
        assert_eq!(r3.lock().unwrap().len(), 1);
        assert_eq!(*r1.lock().unwrap().first().unwrap(), 42u32);
    }

    #[test]
    fn broadcaster_aggregates_wants_correctly() {
        let b: VideoBroadcaster<u32> = VideoBroadcaster::new();

        let w1 = VideoSinkWants {
            is_active: true,
            max_pixel_count: 1920 * 1080,
            max_framerate_fps: 30,
            resolution_alignment: 2,
            rotation_applied: false,
        };
        let w2 = VideoSinkWants {
            is_active: true,
            max_pixel_count: 1280 * 720,
            max_framerate_fps: 15,
            resolution_alignment: 4,
            rotation_applied: true,
        };

        let (s1, _) = make_sink(w1);
        let (s2, _) = make_sink(w2);
        b.add_or_update_sink(s1, w1);
        b.add_or_update_sink(s2, w2);

        let agg = b.wants();
        assert!(agg.is_active);
        // min pixel count wins (lower resolution constraint)
        assert_eq!(agg.max_pixel_count, 1280 * 720);
        // min framerate wins
        assert_eq!(agg.max_framerate_fps, 15);
        // LCM of alignments
        assert_eq!(agg.resolution_alignment, 4);
    }

    #[test]
    fn broadcaster_aggregates_wants_empty() {
        let b: VideoBroadcaster<u32> = VideoBroadcaster::new();
        let agg = b.wants();
        // Empty broadcaster returns default wants
        assert!(agg.is_active);
        assert_eq!(agg.max_pixel_count, 0);
        assert_eq!(agg.max_framerate_fps, 0);
        assert_eq!(agg.resolution_alignment, 1);
    }

    #[test]
    fn broadcaster_respects_inactive_sinks() {
        let b: VideoBroadcaster<u32> = VideoBroadcaster::new();

        let (s_active, r_active) = make_sink(VideoSinkWants::default());
        let (s_inactive, r_inactive) =
            make_sink(VideoSinkWants { is_active: false, ..Default::default() });

        b.add_or_update_sink(s_active, VideoSinkWants::default());
        b.add_or_update_sink(s_inactive, VideoSinkWants { is_active: false, ..Default::default() });

        b.send_frame(&Arc::new(99)).unwrap();

        // Active sink received the frame
        assert_eq!(r_active.lock().unwrap().len(), 1);
        // Inactive sink did NOT receive the frame
        assert_eq!(r_inactive.lock().unwrap().len(), 0);
    }

    #[test]
    fn broadcaster_all_inactive_returns_zero_wants() {
        let b: VideoBroadcaster<u32> = VideoBroadcaster::new();

        let (s, _) = make_sink(VideoSinkWants {
            is_active: false,
            max_pixel_count: 640 * 480,
            max_framerate_fps: 60,
            resolution_alignment: 1,
            rotation_applied: false,
        });
        b.add_or_update_sink(s, VideoSinkWants { is_active: false, ..Default::default() });

        let agg = b.wants();
        assert!(!agg.is_active);
        assert_eq!(agg.max_pixel_count, 0);
        assert_eq!(agg.max_framerate_fps, 0);
    }
}
