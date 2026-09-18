#![allow(dead_code)]

use crate::error::MediaError;
use crate::pipeline::core::{
    FormatSpec, MediaSource, MediaType, NodeCapability, NodeInfo, PipelineNode,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

type Result<T> = std::result::Result<T, MediaError>;

// ── 1. FragmentBroadcaster ──

/// Metadata describing what a broadcaster carries.
#[derive(Debug, Clone)]
pub struct BroadcasterMeta {
    pub source_id: String,
    pub track_id: String,
    pub codec: String,
}

/// Single-producer, multi-subscriber broadcaster.
/// Based on tokio::sync::broadcast (LVQR pattern).
pub struct FragmentBroadcaster<P: Clone + Send + 'static> {
    tx: broadcast::Sender<P>,
    meta: BroadcasterMeta,
}

impl<P: Clone + Send + 'static> FragmentBroadcaster<P> {
    pub fn new(capacity: usize, meta: BroadcasterMeta) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx, meta }
    }

    /// Publish a packet. Returns current subscriber count. Never blocks.
    pub fn emit(&self, packet: P) -> usize {
        self.tx.send(packet).unwrap_or(0);
        self.tx.receiver_count()
    }

    /// Create a new subscriber stream.
    #[allow(clippy::type_complexity)] // 广播回调签名（Arc<dyn Fn>）
    pub fn subscribe(&self) -> BroadcastStream<P> {
        BroadcastStream { rx: self.tx.subscribe(), _meta: self.meta.clone() }
    }

    pub fn meta(&self) -> &BroadcasterMeta {
        &self.meta
    }
}

// ── 2. BroadcastStream ──

/// Subscriber stream that implements MediaSource.
pub struct BroadcastStream<P: Clone + Send + 'static> {
    rx: broadcast::Receiver<P>,
    _meta: BroadcasterMeta,
}

// NodeInfo impl (stub — BroadcastStream is auto-generated, no meaningful name)
impl<P: Clone + Send + 'static> NodeInfo for BroadcastStream<P> {
    fn name(&self) -> &str {
        "broadcast-stream"
    }
    fn capabilities(&self) -> NodeCapability {
        NodeCapability {
            input: FormatSpec { media_type: MediaType::Both, codecs: None, pixel_formats: vec![] },
            output: FormatSpec { media_type: MediaType::Both, codecs: None, pixel_formats: vec![] },
        }
    }
}

impl<P: Clone + Send + 'static> PipelineNode for BroadcastStream<P> {
    fn on_start(&mut self) -> Result<()> {
        Ok(())
    }
    fn on_stop(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<P: Clone + Send + 'static> MediaSource for BroadcastStream<P> {
    type Output = P;

    fn poll_fragment(&mut self) -> Result<Option<Self::Output>> {
        match self.rx.try_recv() {
            Ok(packet) => Ok(Some(packet)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => Ok(None),
            Err(broadcast::error::TryRecvError::Lagged(_n)) => {
                // ponytail: skip lagged frames, retry for latest
                match self.rx.try_recv() {
                    Ok(packet) => Ok(Some(packet)),
                    _ => Ok(None),
                }
            }
        }
    }
}

// ── 3. PipelineRegistry ──

/// Central registry for dynamic source broadcasters.
#[allow(clippy::type_complexity)] // 广播器注册表键值形（(topic,kind)->broadcaster）
pub struct PipelineRegistry {
    broadcasters: RwLock<HashMap<(String, String), Arc<FragmentBroadcaster<Vec<u8>>>>>,
}

impl PipelineRegistry {
    pub fn new() -> Self {
        Self { broadcasters: RwLock::new(HashMap::new()) }
    }

    /// Get or create a broadcaster for a (source_id, track_id) pair.
    /// Default capacity: 1024 packets (~1-2 seconds at 60fps).
    pub fn get_or_create(
        &self,
        source_id: &str,
        track_id: &str,
        codec: &str,
    ) -> Arc<FragmentBroadcaster<Vec<u8>>> {
        let key = (source_id.to_string(), track_id.to_string());
        let mut map = self.broadcasters.write().unwrap();
        map.entry(key.clone())
            .or_insert_with(|| {
                Arc::new(FragmentBroadcaster::new(
                    1024,
                    BroadcasterMeta {
                        source_id: source_id.to_string(),
                        track_id: track_id.to_string(),
                        codec: codec.to_string(),
                    },
                ))
            })
            .clone()
    }

    /// Remove a broadcaster by (source_id, track_id).
    pub fn remove(&self, source_id: &str, track_id: &str) {
        let key = (source_id.to_string(), track_id.to_string());
        self.broadcasters.write().unwrap().remove(&key);
    }

    /// Get the number of active broadcasters.
    pub fn len(&self) -> usize {
        self.broadcasters.read().unwrap().len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.broadcasters.read().unwrap().is_empty()
    }
}

impl Default for PipelineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── 4. Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broadcaster_emit_subscribe() {
        let meta = BroadcasterMeta {
            source_id: "cam1".into(),
            track_id: "video0".into(),
            codec: "h264".into(),
        };
        let bc = FragmentBroadcaster::<Vec<u8>>::new(16, meta);

        let mut sub = bc.subscribe();
        assert_eq!(bc.emit(vec![1, 2, 3]), 1);

        let packet = sub.poll_fragment().unwrap();
        assert_eq!(packet, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_registry_get_or_create() {
        let reg = PipelineRegistry::new();
        assert!(reg.is_empty());

        let bc1 = reg.get_or_create("src1", "track1", "h264");
        assert_eq!(reg.len(), 1);
        assert_eq!(bc1.meta().source_id, "src1");

        // Same key returns existing broadcaster (Arc clone)
        let bc2 = reg.get_or_create("src1", "track1", "h264");
        assert_eq!(reg.len(), 1);
        assert!(Arc::ptr_eq(&bc1, &bc2));

        // Different key creates new broadcaster
        let _bc3 = reg.get_or_create("src2", "track1", "vp8");
        assert_eq!(reg.len(), 2);

        reg.remove("src1", "track1");
        assert_eq!(reg.len(), 1);
    }
}
