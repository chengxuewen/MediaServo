#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use std::sync::{Arc, RwLock};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::error::MediaError;
use crate::pipeline::core::{InternalPacket, MediaProcessor, MediaSink, MediaSource};

type Result<T> = std::result::Result<T, MediaError>;

/// Unique chain identifier (typically source node name).
pub type ChainId = String;

// ── 1. PipelineEngine ──

/// PipelineEngine orchestrates real-time media processing chains.
///
/// # Architecture
///
/// Each chain is: `source → [processor₁ → processor₂ → ...] → [sink₁, sink₂, ...]`.
/// Source tasks run in parallel (each chain in its own tokio task).
/// Processors are applied sequentially within the task.
/// Sinks receive cloned packets (fan-out).
///
/// # Hot-Plug
///
/// Chains can be added or removed while the engine is running via
/// `add_chain()` / `remove_chain()`. All state changes are protected by RwLock.
pub struct PipelineEngine {
    /// All chain states, keyed by chain ID
    chains: RwLock<HashMap<ChainId, ChainState>>,
    /// Whether the engine is currently running
    running: AtomicBool,
    /// Tokio runtime handle for spawning tasks
    rt: tokio::runtime::Handle,
    /// Shutdown signal — one sender, all chains listen
    shutdown_tx: watch::Sender<bool>,
}

/// Internal state for a single chain, with Option wrappers for move-out support.
struct ChainState {
    source: Option<Box<dyn MediaSource<Output = InternalPacket>>>,
    processors:
        Option<Vec<Box<dyn MediaProcessor<Input = InternalPacket, Output = InternalPacket>>>>,
    sinks: Option<Vec<Box<dyn MediaSink<Input = InternalPacket>>>>,
    /// Spawned task handle (None if not started)
    task: Option<JoinHandle<()>>,
}

impl PipelineEngine {
    /// Create a new, empty PipelineEngine.
    ///
    /// Requires a tokio runtime handle for spawning source tasks.
    pub fn new(rt: tokio::runtime::Handle) -> Self {
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            chains: RwLock::new(HashMap::new()),
            running: AtomicBool::new(false),
            rt,
            shutdown_tx,
        }
    }

    /// Add a processing chain to the engine.
    ///
    /// If the engine is already running, the chain starts immediately.
    pub fn add_chain(
        &self,
        id: ChainId,
        source: Box<dyn MediaSource<Output = InternalPacket>>,
        processors: Vec<Box<dyn MediaProcessor<Input = InternalPacket, Output = InternalPacket>>>,
        sinks: Vec<Box<dyn MediaSink<Input = InternalPacket>>>,
    ) -> Result<()> {
        let mut chains = self.chains.write().unwrap();
        if chains.contains_key(&id) {
            return Err(MediaError::Internal(format!("chain '{id}' already exists")));
        }

        chains.insert(
            id.clone(),
            ChainState {
                source: Some(source),
                processors: Some(processors),
                sinks: Some(sinks),
                task: None,
            },
        );

        if self.running.load(Ordering::SeqCst) {
            drop(chains);
            self.spawn_chain(&id)?;
        }

        Ok(())
    }

    /// Remove a chain by ID. If running, the chain task is aborted.
    pub fn remove_chain(&self, id: &str) -> Result<()> {
        let mut chains = self.chains.write().unwrap();
        let state = chains
            .remove(id)
            .ok_or_else(|| MediaError::Internal(format!("chain '{id}' not found")))?;

        if let Some(handle) = state.task {
            handle.abort();
        }

        Ok(())
    }

    /// Start all chains. Idempotent.
    pub fn start(&self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let chain_ids: Vec<String> = { self.chains.read().unwrap().keys().cloned().collect() };

        for id in &chain_ids {
            self.spawn_chain(id)?;
        }

        Ok(())
    }

    /// Stop all chains gracefully. Sends shutdown signal to all tasks.
    pub async fn stop(&self) -> Result<()> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }

        let _ = self.shutdown_tx.send(true);

        let tasks: Vec<JoinHandle<()>> = {
            let mut chains = self.chains.write().unwrap();
            chains.values_mut().filter_map(|c| c.task.take()).collect()
        };

        for task in tasks {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
        }

        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn chain_count(&self) -> usize {
        self.chains.read().unwrap().len()
    }

    fn spawn_chain(&self, id: &str) -> Result<()> {
        let mut chains = self.chains.write().unwrap();
        let state = chains
            .get_mut(id)
            .ok_or_else(|| MediaError::Internal(format!("chain '{id}' not found")))?;

        let source = state.source.take().ok_or_else(|| {
            MediaError::Internal(format!("chain '{id}': source already consumed"))
        })?;
        let processors = state.processors.take().unwrap_or_default();
        let sinks = state.sinks.take().unwrap_or_default();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        let chain_id = id.to_string();
        let handle = self.rt.spawn(async move {
            Self::run_chain(chain_id, source, processors, sinks, &mut shutdown_rx).await;
        });

        state.task = Some(handle);
        Ok(())
    }

    async fn run_chain(
        id: ChainId,
        mut source: Box<dyn MediaSource<Output = InternalPacket>>,
        mut processors: Vec<
            Box<dyn MediaProcessor<Input = InternalPacket, Output = InternalPacket>>,
        >,
        mut sinks: Vec<Box<dyn MediaSink<Input = InternalPacket>>>,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        tracing::info!(chain = %id, "chain started");

        loop {
            if *shutdown_rx.borrow() {
                tracing::info!(chain = %id, "shutdown received");
                break;
            }

            let fragment = match source.poll_fragment() {
                Ok(Some(f)) => f,
                Ok(None) => {
                    tokio::task::yield_now().await;
                    continue;
                }
                Err(e) => {
                    tracing::error!(chain = %id, error = %e, "source poll failed");
                    tokio::task::yield_now().await;
                    continue;
                }
            };

            let mut current = fragment;
            for (i, processor) in processors.iter_mut().enumerate() {
                match processor.process(current) {
                    Ok(output) => current = output,
                    Err(e) => {
                        tracing::error!(chain = %id, processor_idx = i, error = %e, "processor failed");
                        current = InternalPacket::Metadata(crate::pipeline::core::PacketMetadata {
                            track_id: String::new(),
                            event: crate::pipeline::core::MetadataEvent::TrackEnded,
                        });
                        break;
                    }
                }
            }

            for (i, sink) in sinks.iter_mut().enumerate() {
                let packet = current.clone();
                if let Err(e) = sink.on_fragment(packet) {
                    tracing::error!(chain = %id, sink_idx = i, error = %e, "sink failed");
                }
            }
        }

        // Lifecycle cleanup
        if let Err(e) = source.on_stop() {
            tracing::error!(chain = %id, error = %e, "source stop failed");
        }
        for (i, p) in processors.iter_mut().enumerate() {
            if let Err(e) = p.on_stop() {
                tracing::error!(chain = %id, processor_idx = i, error = %e, "processor stop failed");
            }
        }
        for (i, s) in sinks.iter_mut().enumerate() {
            if let Err(e) = s.on_stop() {
                tracing::error!(chain = %id, sink_idx = i, error = %e, "sink stop failed");
            }
        }

        tracing::info!(chain = %id, "chain stopped");
    }
}

// ── 2. Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::core::{
        EncodedFragment, FragmentFlags, FrameTiming, MediaType, NodeCapability,
    };
    use std::sync::Mutex;

    struct TestSource {
        counter: u64,
        name: &'static str,
    }

    impl crate::pipeline::core::NodeInfo for TestSource {
        fn name(&self) -> &str {
            self.name
        }
        fn capabilities(&self) -> NodeCapability {
            NodeCapability {
                input: crate::pipeline::core::FormatSpec {
                    media_type: MediaType::Both,
                    codecs: None,
                    pixel_formats: vec![],
                },
                output: crate::pipeline::core::FormatSpec {
                    media_type: MediaType::Both,
                    codecs: None,
                    pixel_formats: vec![],
                },
            }
        }
    }

    impl crate::pipeline::core::PipelineNode for TestSource {
        fn on_start(&mut self) -> Result<()> {
            self.counter = 0;
            Ok(())
        }
        fn on_stop(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl MediaSource for TestSource {
        type Output = InternalPacket;
        fn poll_fragment(&mut self) -> Result<Option<Self::Output>> {
            self.counter += 1;
            if self.counter > 3 {
                return Ok(None);
            }
            Ok(Some(InternalPacket::Encoded(EncodedFragment {
                track_id: "test".into(),
                timing: FrameTiming {
                    dts: self.counter,
                    pts: self.counter,
                    duration: 1,
                    wall_clock: None,
                },
                flags: FragmentFlags {
                    keyframe: self.counter == 1,
                    independent: true,
                    discardable: false,
                },
                codec: "test".into(),
                init_data: None,
                payload: vec![self.counter as u8],
            })))
        }
    }

    struct TestProcessor {
        name: &'static str,
    }

    impl crate::pipeline::core::NodeInfo for TestProcessor {
        fn name(&self) -> &str {
            self.name
        }
        fn capabilities(&self) -> NodeCapability {
            NodeCapability {
                input: crate::pipeline::core::FormatSpec {
                    media_type: MediaType::Both,
                    codecs: None,
                    pixel_formats: vec![],
                },
                output: crate::pipeline::core::FormatSpec {
                    media_type: MediaType::Both,
                    codecs: None,
                    pixel_formats: vec![],
                },
            }
        }
    }

    impl crate::pipeline::core::PipelineNode for TestProcessor {
        fn on_start(&mut self) -> Result<()> {
            Ok(())
        }
        fn on_stop(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl MediaProcessor for TestProcessor {
        type Input = InternalPacket;
        type Output = InternalPacket;
        fn process(&mut self, input: Self::Input) -> Result<Self::Output> {
            match input {
                InternalPacket::Encoded(mut f) => {
                    f.payload.extend_from_within(..);
                    Ok(InternalPacket::Encoded(f))
                }
                other => Ok(other),
            }
        }
    }

    struct TestSink {
        received: Arc<Mutex<Vec<InternalPacket>>>,
        name: &'static str,
    }

    impl TestSink {
        fn new() -> (Self, Arc<Mutex<Vec<InternalPacket>>>) {
            let received = Arc::new(Mutex::new(Vec::new()));
            (Self { received: received.clone(), name: "test-sink" }, received)
        }
    }

    impl crate::pipeline::core::NodeInfo for TestSink {
        fn name(&self) -> &str {
            self.name
        }
        fn capabilities(&self) -> NodeCapability {
            NodeCapability {
                input: crate::pipeline::core::FormatSpec {
                    media_type: MediaType::Both,
                    codecs: None,
                    pixel_formats: vec![],
                },
                output: crate::pipeline::core::FormatSpec {
                    media_type: MediaType::Both,
                    codecs: None,
                    pixel_formats: vec![],
                },
            }
        }
    }

    impl crate::pipeline::core::PipelineNode for TestSink {
        fn on_start(&mut self) -> Result<()> {
            Ok(())
        }
        fn on_stop(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl MediaSink for TestSink {
        type Input = InternalPacket;
        fn on_fragment(&mut self, fragment: Self::Input) -> Result<()> {
            self.received.lock().unwrap().push(fragment);
            Ok(())
        }
    }

    #[tokio::test]
    async fn engine_single_source_to_sink() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (sink, received) = TestSink::new();
        engine
            .add_chain(
                "test".into(),
                Box::new(TestSource { counter: 0, name: "test-src" }),
                vec![],
                vec![Box::new(sink)],
            )
            .unwrap();
        engine.start().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.stop().await.unwrap();
        assert_eq!(received.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn engine_source_processor_sink() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (sink, received) = TestSink::new();
        engine
            .add_chain(
                "test".into(),
                Box::new(TestSource { counter: 0, name: "test-src" }),
                vec![Box::new(TestProcessor { name: "test-proc" })],
                vec![Box::new(sink)],
            )
            .unwrap();
        engine.start().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.stop().await.unwrap();
        let fragments = received.lock().unwrap();
        assert_eq!(fragments.len(), 3);
        if let InternalPacket::Encoded(ref f) = fragments[0] {
            assert_eq!(f.payload.len(), 2, "payload should be doubled by processor");
        }
    }

    #[tokio::test]
    async fn engine_hot_plug_add_while_running() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (sink1, received1) = TestSink::new();
        engine
            .add_chain(
                "chain1".into(),
                Box::new(TestSource { counter: 0, name: "src1" }),
                vec![],
                vec![Box::new(sink1)],
            )
            .unwrap();
        engine.start().unwrap();

        let (sink2, received2) = TestSink::new();
        engine
            .add_chain(
                "chain2".into(),
                Box::new(TestSource { counter: 0, name: "src2" }),
                vec![],
                vec![Box::new(sink2)],
            )
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.stop().await.unwrap();
        assert_eq!(received1.lock().unwrap().len(), 3);
        assert_eq!(received2.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn engine_remove_chain() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (sink, received) = TestSink::new();
        engine
            .add_chain(
                "tmp".into(),
                Box::new(TestSource { counter: 0, name: "tmp-src" }),
                vec![],
                vec![Box::new(sink)],
            )
            .unwrap();
        engine.start().unwrap();
        // Verify the chain runs (at least one fragment arrives)
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        engine.remove_chain("tmp").unwrap();
        // After removal, the chain is gone
        assert_eq!(engine.chain_count(), 0);
        engine.stop().await.unwrap();
        // Verify some fragments were received before removal
        assert!(!received.lock().unwrap().is_empty(), "should have received at least 1 fragment");
    }

    #[tokio::test]
    async fn engine_chain_count() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        assert_eq!(engine.chain_count(), 0);
        let (s1, _) = TestSink::new();
        engine
            .add_chain(
                "a".into(),
                Box::new(TestSource { counter: 0, name: "a" }),
                vec![],
                vec![Box::new(s1)],
            )
            .unwrap();
        assert_eq!(engine.chain_count(), 1);
        let (s2, _) = TestSink::new();
        engine
            .add_chain(
                "b".into(),
                Box::new(TestSource { counter: 0, name: "b" }),
                vec![],
                vec![Box::new(s2)],
            )
            .unwrap();
        assert_eq!(engine.chain_count(), 2);
        engine.remove_chain("a").unwrap();
        assert_eq!(engine.chain_count(), 1);
    }

    #[tokio::test]
    async fn engine_duplicate_chain_id_errors() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (s1, _) = TestSink::new();
        engine
            .add_chain(
                "dup".into(),
                Box::new(TestSource { counter: 0, name: "a" }),
                vec![],
                vec![Box::new(s1)],
            )
            .unwrap();
        let (s2, _) = TestSink::new();
        assert!(
            engine
                .add_chain(
                    "dup".into(),
                    Box::new(TestSource { counter: 0, name: "b" }),
                    vec![],
                    vec![Box::new(s2)]
                )
                .is_err()
        );
    }

    #[tokio::test]
    async fn engine_remove_then_readd_same_id() {
        // Verify a chain can be removed and re-added with the same ID.
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (s1, _) = TestSink::new();
        engine
            .add_chain(
                "r".into(),
                Box::new(TestSource { counter: 0, name: "a" }),
                vec![],
                vec![Box::new(s1)],
            )
            .unwrap();
        engine.start().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        engine.remove_chain("r").unwrap();
        assert_eq!(engine.chain_count(), 0);
        let (s2, r2) = TestSink::new();
        engine
            .add_chain(
                "r".into(),
                Box::new(TestSource { counter: 0, name: "b" }),
                vec![],
                vec![Box::new(s2)],
            )
            .unwrap();
        assert_eq!(engine.chain_count(), 1);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.stop().await.unwrap();
        assert_eq!(r2.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn engine_remove_nonexistent_errors() {
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        assert!(engine.remove_chain("ghost").is_err());
    }

    #[tokio::test]
    async fn engine_idempotent_start() {
        // start() called twice should not panic or double-spawn.
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (sink, received) = TestSink::new();
        engine
            .add_chain(
                "c".into(),
                Box::new(TestSource { counter: 0, name: "c" }),
                vec![],
                vec![Box::new(sink)],
            )
            .unwrap();
        engine.start().unwrap();
        engine.start().unwrap(); // idempotent
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.stop().await.unwrap();
        assert_eq!(received.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn engine_hot_add_with_processor() {
        // Add a chain with a processor while the engine is already running.
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (sink1, r1) = TestSink::new();
        engine
            .add_chain(
                "c1".into(),
                Box::new(TestSource { counter: 0, name: "c1" }),
                vec![],
                vec![Box::new(sink1)],
            )
            .unwrap();
        engine.start().unwrap();

        let (sink2, r2) = TestSink::new();
        engine
            .add_chain(
                "c2".into(),
                Box::new(TestSource { counter: 0, name: "c2" }),
                vec![Box::new(TestProcessor { name: "p" })],
                vec![Box::new(sink2)],
            )
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        engine.stop().await.unwrap();
        assert_eq!(r1.lock().unwrap().len(), 3);
        assert_eq!(r2.lock().unwrap().len(), 3);
        if let InternalPacket::Encoded(ref f) = r2.lock().unwrap()[0] {
            assert_eq!(f.payload.len(), 2, "processor should double payload");
        }
    }

    #[tokio::test]
    async fn engine_remove_all_chains_survives() {
        // Engine with 0 chains after removal should not panic on stop.
        let rt = tokio::runtime::Handle::current();
        let engine = PipelineEngine::new(rt);
        let (s1, _) = TestSink::new();
        let (s2, _) = TestSink::new();
        engine
            .add_chain(
                "a".into(),
                Box::new(TestSource { counter: 0, name: "a" }),
                vec![],
                vec![Box::new(s1)],
            )
            .unwrap();
        engine
            .add_chain(
                "b".into(),
                Box::new(TestSource { counter: 0, name: "b" }),
                vec![],
                vec![Box::new(s2)],
            )
            .unwrap();
        engine.start().unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        engine.remove_chain("a").unwrap();
        engine.remove_chain("b").unwrap();
        assert_eq!(engine.chain_count(), 0);
        engine.stop().await.unwrap(); // must not panic
    }
}
