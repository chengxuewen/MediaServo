#![allow(dead_code)]
use crate::error::MediaError;
use crate::pipeline::core::{CodecId, FormatQuery, MediaType, NodeType};
use crate::pixel_format::PixelFormat;
use std::any::Any;
use std::sync::Arc;
type Result<T> = std::result::Result<T, MediaError>;

/// Plugin kind: compile-time (statically linked) or run-time (dlopen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    CompileTime,
    RunTime,
}
/// Capability declaration for capability-based node selection.
#[derive(Debug, Clone)]
pub struct PluginCapability {
    pub node_type: NodeType,
    pub media_type: MediaType,
    pub codecs: Vec<CodecId>,
    pub pixel_formats: Vec<PixelFormat>,
    pub priority: u8,
}
/// A plugin is a factory of pipeline nodes.
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> (u16, u16, u16);
    fn kind(&self) -> PluginKind;
    fn capabilities(&self) -> Vec<PluginCapability>;
    fn on_load(&self) -> Result<()> {
        Ok(())
    }
    fn on_unload(&self) -> Result<()> {
        Ok(())
    }
}
/// Central plugin registry.
pub struct PluginManager {
    compile_time: Vec<Arc<dyn Plugin>>,
}
impl PluginManager {
    pub fn new() -> Self {
        Self { compile_time: Vec::new() }
    }
    /// Register a compile-time plugin.
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) {
        self.compile_time.push(plugin);
    }
    /// Find plugins whose capabilities match the query.
    /// Returns plugin references sorted by priority (descending).
    pub fn find_nodes(&self, query: &FormatQuery) -> Vec<&dyn Plugin> {
        let mut matches: Vec<&dyn Plugin> = self
            .compile_time
            .iter()
            .filter(|p| {
                p.capabilities().iter().any(|cap| {
                    if cap.node_type != query.node_type {
                        return false;
                    }
                    if !matches_media_type(cap.media_type, query.media_type) {
                        return false;
                    }
                    if let Some(ref codec) = query.codec
                        && !cap.codecs.is_empty()
                        && !cap.codecs.contains(codec)
                    {
                        return false;
                    }
                    if let Some(ref fmt) = query.pixel_format
                        && !cap.pixel_formats.contains(fmt)
                    {
                        return false;
                    }
                    true
                })
            })
            .map(|p| p.as_ref())
            .collect();
        matches.sort_by_key(|p| {
            p.capabilities()
                .iter()
                .filter(|cap| cap.node_type == query.node_type)
                .map(|cap| cap.priority)
                .max()
                .unwrap_or(0)
        });
        matches.reverse();
        matches
    }
    /// Create a pipeline node from a plugin.
    /// Returns node as boxed Any (caller downcasts to expected trait object).
    // ponytail: Phase 2 — factory fn pointers via inventory::submit!; stub for now.
    pub fn create_node(&self, plugin: &dyn Plugin, _cap_idx: usize) -> Result<Box<dyn Any + Send>> {
        Err(MediaError::Internal(format!(
            "plugin '{}': create_node not yet implemented (Phase 2)",
            plugin.name()
        )))
    }
}
impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
fn matches_media_type(cap: MediaType, query: MediaType) -> bool {
    matches!(cap, MediaType::Both) || cap == query
}
#[cfg(test)]
mod tests {
    use super::*;
    struct TestPlugin {
        name: &'static str,
        node_type: NodeType,
        media_type: MediaType,
        codecs: Vec<CodecId>,
        priority: u8,
    }
    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            self.name
        }
        fn version(&self) -> (u16, u16, u16) {
            (0, 1, 0)
        }
        fn kind(&self) -> PluginKind {
            PluginKind::CompileTime
        }
        fn capabilities(&self) -> Vec<PluginCapability> {
            vec![PluginCapability {
                node_type: self.node_type,
                media_type: self.media_type,
                codecs: self.codecs.clone(),
                pixel_formats: vec![],
                priority: self.priority,
            }]
        }
    }
    #[test]
    fn test_find_nodes_filters_by_node_type() {
        let mut mgr = PluginManager::new();
        mgr.register(Arc::new(TestPlugin {
            name: "src",
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codecs: vec![],
            priority: 10,
        }));
        mgr.register(Arc::new(TestPlugin {
            name: "proc",
            node_type: NodeType::Processor,
            media_type: MediaType::Encoded,
            codecs: vec![],
            priority: 10,
        }));

        let query = FormatQuery {
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codec: None,
            pixel_format: None,
        };
        let found = mgr.find_nodes(&query);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name(), "src");
    }
    #[test]
    fn test_find_nodes_sorts_by_priority() {
        let mut mgr = PluginManager::new();
        mgr.register(Arc::new(TestPlugin {
            name: "low",
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codecs: vec![],
            priority: 5,
        }));
        mgr.register(Arc::new(TestPlugin {
            name: "high",
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codecs: vec![],
            priority: 20,
        }));

        let query = FormatQuery {
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codec: None,
            pixel_format: None,
        };
        let found = mgr.find_nodes(&query);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name(), "high");
        assert_eq!(found[1].name(), "low");
    }
    #[test]
    fn test_find_nodes_empty_on_no_match() {
        let mut mgr = PluginManager::new();
        mgr.register(Arc::new(TestPlugin {
            name: "src",
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codecs: vec!["avc1".into()],
            priority: 10,
        }));

        let query = FormatQuery {
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codec: Some("hevc".into()),
            pixel_format: None,
        };
        let found = mgr.find_nodes(&query);
        assert!(found.is_empty());
    }
    #[test]
    fn test_register_and_create_node() {
        let mut mgr = PluginManager::new();
        let plugin = Arc::new(TestPlugin {
            name: "test",
            node_type: NodeType::Source,
            media_type: MediaType::Encoded,
            codecs: vec![],
            priority: 10,
        });
        mgr.register(plugin.clone());

        let result = mgr.create_node(plugin.as_ref(), 0);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("test"), "expected error mentioning plugin name, got: {err}");
        assert!(err.contains("Phase 2"), "expected Phase 2 mention, got: {err}");
    }
}
