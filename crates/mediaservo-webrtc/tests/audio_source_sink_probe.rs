//! The PIT-105 evidence probe (the H2 I4 review commit): AudioTrackSource -> capture_frame -> AudioTrack
//! sink delivery chain -- the source-side hookup field-proven.
//!
//! Background (PIT-105): of the five segments of the audio send chain, this probe proves the "source -> track sink" segment works
//! (capture_frame succeeding + the sink callback firing 70 times for 20 frames + silence fill); the loss point sits in libwebrtc's
//! audio send channel (the LocalAudioSinkAdapter mount / the channel StartSend state) -- the vendor domain.
//! `#[ignore]`: the probe does not depend on the PIT-105 fix (the sink chain was always working), but keeps running at low frequency
//! (FFI-probe semantics + the discriminator when the fix is verified).

// only backend-webrtc-sys has the webrtc_sys dependency (default = the stub backend, no FFI surface) --
// without this gate the default build fails to compile (the I1 re-review blocker).
#![cfg(feature = "backend-webrtc-sys")]
#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

extern "C" fn noop_complete(_ctx: *const webrtc_sys::audio_track::SourceContext) {}

struct Counter(Arc<AtomicU64>);

impl webrtc_sys::audio_track::AudioSink for Counter {
    fn on_data(&self, _data: &[i16], _sample_rate: i32, _nb_channels: usize, _nb_frames: usize) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

/// The PCM pushed by capture_frame must reach the AudioTrack's sink (the source -> track chain evidence).
/// Kept when fixing PIT-105: if this test fails = a source-side wiring regression; if it passes but e2e byte_count=0
/// = the loss point is still in libwebrtc's send channel.
#[test]
#[ignore = "the PIT-105 evidence probe: low-frequency runs; depends on the webrtc-sys FFI runtime"]
fn audio_source_sink_probe() {
    use webrtc_sys::audio_track::ffi as at;

    let factory = webrtc_sys::peer_connection_factory::ffi::create_peer_connection_factory();
    let source = at::new_audio_track_source(
        at::AudioSourceOptions {
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
        },
        48000,
        1,
        100,
    );
    let track = factory.create_audio_track("probe".into(), source.clone());
    let counter = Arc::new(AtomicU64::new(0));
    let sink = at::new_native_audio_sink(
        Box::new(webrtc_sys::audio_track::AudioSinkWrapper::new(Arc::new(Counter(
            counter.clone(),
        )))),
        48000,
        1,
    );
    track.add_sink(&sink);

    // push 20 frames of 10ms PCM (mimicking the tone cadence)
    let pcm: Vec<i16> = vec![0i16; 480];
    for i in 0..20 {
        let ok = unsafe {
            source.capture_frame(
                &pcm,
                48000,
                1,
                480,
                std::ptr::null(),
                webrtc_sys::audio_track::CompleteCallback(noop_complete),
            )
        };
        assert!(ok, "capture_frame[{i}] rejected");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // wait for the queue task to drain
    std::thread::sleep(std::time::Duration::from_millis(500));
    let n = counter.load(Ordering::Relaxed);
    eprintln!("audio_source_sink_probe: sink callbacks = {n}");
    assert!(
        n > 0,
        "capture_frame's data must reach the track sink (the source -> track line is broken)"
    );
}
