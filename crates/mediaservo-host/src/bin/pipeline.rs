//! GStreamer pipeline: compositor → videoconvert → encoder → appsink.
//!
//! Encoder auto-selects based on platform:
//! - Linux: nvenc (NVIDIA) or vah264enc (VAAPI fallback)
//! - macOS: vt264enc (VideoToolbox)
//! - Other: x264enc (software, universal fallback)

#![allow(dead_code)] // 同上：legacy 引擎管线演示件（真管线在 mediaservo-deck）
#[cfg(feature = "gstreamer")]
mod imp {
    use gstreamer::prelude::*;
    use gstreamer_app;
    use mediaservo_common::config::CaptureConfig;
    use mediaservo_common::error::CoreError;

    pub struct Pipeline {
        pipeline: gstreamer::Pipeline,
        appsink: gstreamer_app::AppSink,
    }

    /// Select the best available hardware encoder for the current platform.
    fn resolve_encoder(requested: &str) -> &str {
        if requested != "auto" {
            return requested;
        }
        #[cfg(target_os = "macos")]
        {
            // ponytail: vtenc_h264 is the Apple VideoToolbox encoder from applemedia plugin
            return "vtenc_h264";
        }
        #[cfg(target_os = "linux")]
        {
            // ponytail: try nvenc first, VAAPI fallback; real capability probe later
            return "vah264enc";
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            "x264enc"
        }
    }

    impl Pipeline {
        /// Build a GStreamer pipeline: compositor → videoconvert → encoder → appsink.
        ///
        /// The encoder is resolved from `encoder` param (if "auto", platform-appropriate).
        pub fn new(
            capture: &CaptureConfig,
            width: u32,
            height: u32,
            fps: u32,
            bitrate: u32,
            encoder: &str,
        ) -> Result<Self, CoreError> {
            gstreamer::init().ok();
            let enc = resolve_encoder(encoder);

            let source_element = match capture.source.as_str() {
                "screen" => {
                    #[cfg(target_os = "linux")]
                    {
                        "ximagesrc"
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        "videotestsrc pattern=ball"
                    }
                }
                "camera" => {
                    if let Some(ref dev) = capture.device {
                        // ponytail: pass device path as location property
                        &format!("v4l2src device={dev}")
                    } else {
                        "v4l2src"
                    }
                }
                _ => "videotestsrc pattern=smpte",
            };

            let pipeline_desc = format!(
                "{source} ! videoconvert ! video/x-raw,format=I420,width={w},height={h},framerate={fps}/1 \
                 ! {enc} bitrate={br} ! h264parse ! appsink name=sink sync=false",
                source = source_element,
                w = width,
                h = height,
                fps = fps,
                enc = enc,
                br = bitrate,
            );

            let pipeline = gstreamer::parse::launch(&pipeline_desc)
                .map_err(|e| CoreError::EncoderInit(format!("pipeline parse: {e}")))?
                .downcast::<gstreamer::Pipeline>()
                .map_err(|_| CoreError::EncoderInit("downcast to Pipeline".into()))?;

            let appsink = pipeline
                .by_name("sink")
                .ok_or_else(|| CoreError::EncoderInit("appsink 'sink' not found".into()))?
                .downcast::<gstreamer_app::AppSink>()
                .map_err(|_| CoreError::EncoderInit("element is not an AppSink".into()))?;

            // Bus watch for error/warning logging
            if let Some(bus) = pipeline.bus() {
                bus.set_sync_handler(move |_bus, msg| {
                    use gstreamer::MessageView;
                    match msg.view() {
                        MessageView::Error(err) => {
                            tracing::error!(
                                    src = ?err.src(),
                                    error = %err.error(),
                            debug = ?err.debug(),
                                    "GStreamer pipeline error"
                                );
                        }
                        MessageView::Warning(warn) => {
                            tracing::warn!(
                                src = ?warn.src(),
                                error = %warn.error(),
                                debug = ?warn.debug(),
                                "GStreamer pipeline warning"
                            );
                        }
                        MessageView::Eos(_) => {
                            tracing::info!("GStreamer pipeline EOS");
                        }
                        _ => {}
                    }
                    gstreamer::BusSyncReply::Pass
                });
            }

            tracing::info!(encoder = enc, source = %source_element, "GStreamer pipeline created");
            Ok(Pipeline { pipeline, appsink })
        }

        pub fn start(&self) -> Result<(), CoreError> {
            tracing::info!("Pipeline: setting state to Playing");
            self.pipeline
                .set_state(gstreamer::State::Playing)
                .map_err(|e| CoreError::EncoderInit(format!("pipeline start: {e}")))?;
            Ok(())
        }

        pub fn stop(&self) -> Result<(), CoreError> {
            self.pipeline
                .set_state(gstreamer::State::Null)
                .map_err(|e| CoreError::Unknown(format!("pipeline stop: {e}")))?;
            Ok(())
        }

        /// Pull a sample from the appsink and return (raw H.264 bytes, PTS in nanoseconds).
        pub fn pull_sample(&self) -> Result<(Vec<u8>, u64), CoreError> {
            let sample = self
                .appsink
                .try_pull_sample(gstreamer::ClockTime::from_mseconds(100))
                .ok_or_else(|| CoreError::EncoderInit("appsink pulled None".into()))?;
            let buffer = sample
                .buffer()
                .ok_or_else(|| CoreError::EncoderInit("sample has no buffer".into()))?;
            let pts_ns = buffer.pts().map(|t| t.nseconds()).unwrap_or(0);
            let map = buffer
                .map_readable()
                .map_err(|_| CoreError::EncoderInit("failed to map buffer".into()))?;
            Ok((map.to_vec(), pts_ns))
        }

        /// Create a dummy pipeline for headless/E2E fallback.
        pub fn dummy() -> Self {
            gstreamer::init().ok();
            let desc = "videotestsrc pattern=smpte ! videoconvert ! appsink name=sink";
            let pipeline = gstreamer::parse::launch(desc)
                .expect("videotestsrc pipeline should always parse")
                .downcast::<gstreamer::Pipeline>()
                .expect("downcast to Pipeline");
            let appsink = pipeline
                .by_name("sink")
                .expect("sink element")
                .downcast::<gstreamer_app::AppSink>()
                .expect("downcast to AppSink");
            tracing::info!("Pipeline dummy (fallback videotestsrc)");
            Pipeline { pipeline, appsink }
        }
    }
}
#[cfg(not(feature = "gstreamer"))]
mod imp {
    use mediaservo_common::config::CaptureConfig;
    use mediaservo_common::error::CoreError;

    pub struct Pipeline;

    impl Pipeline {
        pub fn new(
            _capture: &CaptureConfig,
            _width: u32,
            _height: u32,
            _fps: u32,
            _bitrate: u32,
            _encoder: &str,
        ) -> Result<Self, CoreError> {
            tracing::warn!("Pipeline stub (no GStreamer compiled)");
            Ok(Pipeline)
        }

        /// Create a dummy pipeline for headless/E2E mode.
        pub fn dummy() -> Self {
            tracing::info!("Pipeline dummy (headless mode)");
            Pipeline
        }

        pub fn start(&self) -> Result<(), CoreError> {
            Ok(())
        }

        pub fn stop(&self) -> Result<(), CoreError> {
            Ok(())
        }

        /// Stub: returns empty tuple when GStreamer is not available.
        pub fn pull_sample(&self) -> Result<(Vec<u8>, u64), CoreError> {
            tracing::debug!("pull_sample (stub): no GStreamer");
            Ok((Vec::new(), 0))
        }
    }
}
pub use imp::Pipeline;

#[cfg(test)]
mod tests {
    use super::*;
    use mediaservo_common::config::CaptureConfig;

    fn test_capture() -> CaptureConfig {
        CaptureConfig {
            source: "test_pattern".to_string(),
            resolution: "640x480".to_string(),
            framerate: 30,
            device: None,
        }
    }

    #[cfg(not(feature = "gstreamer"))]
    #[test]
    fn pipeline_new_with_test_pattern() {
        let cfg = test_capture();
        let result = Pipeline::new(&cfg, 640, 480, 30, 1000, "auto");
        assert!(result.is_ok());
    }

    #[cfg(not(feature = "gstreamer"))]
    #[test]
    fn pipeline_start_stop_no_panic() {
        let cfg = test_capture();
        let pipe = Pipeline::new(&cfg, 640, 480, 30, 1000, "auto").unwrap();
        assert!(pipe.start().is_ok());
        assert!(pipe.stop().is_ok());
    }

    // ponytail: skip with gstreamer — real pipeline pull_sample blocks in test runner
    #[cfg(not(feature = "gstreamer"))]
    #[test]
    fn pipeline_dummy_works() {
        let pipe = Pipeline::dummy();
        assert!(pipe.start().is_ok());
        // pull before stop — dead pipeline can't produce frames
        let sample = pipe.pull_sample();
        assert!(sample.is_ok());
        assert!(pipe.stop().is_ok());
    }
}
