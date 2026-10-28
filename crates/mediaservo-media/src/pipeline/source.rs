use super::sink::{SinkId, VideoSink, VideoSinkWants};

/// Producer of video frames. Manages downstream sink registrations.
pub trait VideoSource<F>: Send {
    /// Register or update a sink with its wants. Returns the assigned SinkId.
    fn add_or_update_sink(&self, sink: Box<dyn VideoSink<F>>, wants: VideoSinkWants) -> SinkId;

    /// Remove a previously registered sink.
    fn remove_sink(&self, id: SinkId);
}
