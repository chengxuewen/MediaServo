//! FrameBus (iceoryx2 SHM zero-copy frame bus, latest-frame overwrite semantics).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iceoryx2::port::publisher::Publisher;
use iceoryx2::prelude::*;

/// Per-topic subscriber cap (static config frozen at service creation; iceoryx2 0.9.3 defaults to only 8):
/// after D282 defaults, "N streams sharing one source" is a legal steady state (09-21 incident: 8 streamers
/// sharing camera/generator hit the 8-slot cap + zombie nodes occupied slots -> deterministic ExceedsMaxSupportedSubscribers,
/// and old services' static config survives restarts = restarting does not help).
/// ponytail: fixed constant; 32 suffices - no known edge-box scenario shares one source across >32 streams. Upgrade path: host.yaml config key.
const MAX_SUBSCRIBERS_PER_TOPIC: usize = 32;

use crate::acl::NodeAcl;
use crate::error::LinkError;
use crate::frame::{FrameMeta, FrameRef, FrameStream, StreamInner};
use crate::id::{FrameTopic, NodeId};
use crate::registry::{NodeInfo, Registry};
use crate::token::{CapabilityToken, Ed25519VerifyingKey};

/// Per-frame byte cap (1080p I420 ~ 3.1MB, with headroom).
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

type TopicPublisher = Publisher<ipc_threadsafe::Service, [u8], ()>;

/// Cross-process topic discovery result (E1 topology-monitoring data source, Momus MEDIUM-2 option 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredTopic {
    pub topic: FrameTopic,
    /// Active node instances on the service (0 = no live process connected to this topic).
    pub alive_nodes: usize,
}

/// Frame bus (one instance per node; register-on-attach, D235).
pub struct FrameBus {
    node: Node<ipc_threadsafe::Service>,
    acl: NodeAcl,
    node_id: NodeId,
    /// Inner state of created streams (shutdown on close).
    streams: Mutex<Vec<Arc<StreamInner>>>,
    /// Cached publishers (held to keep the max_publishers(1) single-publisher lock + delivery reliability).
    publishers: Mutex<HashMap<String, TopicPublisher>>,
}

impl FrameBus {
    /// Cross-process publisher enumeration (static, no attach needed - for host-agent topology monitoring).
    ///
    /// Data source: iceoryx2 0.9.3 service-registry enumeration `Service::list(Config::global_config(), ...)`
    /// (official example: iceoryx2 source `src/service/mod.rs` `Service::list` doc example),
    /// filtered to PublishSubscribe mode (FrameBus topic semantics).
    ///
    /// Limit: the 0.9.3 public API does not distinguish publisher/subscriber nodes (`ServiceDynamicDetails::nodes`
    /// lists all connected nodes on the service) -> `alive_nodes > 0` = some live process is connected;
    /// publisher-process liveness is decided by the caller (host monitor) combined with oxmgr.
    pub fn list_topics() -> Result<Vec<DiscoveredTopic>, LinkError> {
        let mut out = Vec::new();
        ipc_threadsafe::Service::list(Config::global_config(), |service| {
            let static_cfg = &service.static_details;
            if matches!(
                static_cfg.messaging_pattern(),
                iceoryx2::service::static_config::messaging_pattern::MessagingPattern::PublishSubscribe(_)
            ) {
                let alive = service
                    .dynamic_details
                    .as_ref()
                    .map(|d| {
                        d.nodes
                            .iter()
                            .filter(|n| matches!(n, NodeState::Alive(_)))
                            .count()
                    })
                    .unwrap_or(0);
                out.push(DiscoveredTopic {
                    topic: FrameTopic::new(static_cfg.name().as_str()),
                    alive_nodes: alive,
                });
            }
            CallbackProgression::Continue
        })
        .map_err(|e| LinkError::Bus(format!("service list: {e:?}")))?;
        Ok(out)
    }

    /// Register-on-attach (D235): verify token (fail-closed) -> load ACL -> iceoryx2 node -> `Registry::register`.
    pub fn attach(
        endpoint: &str,
        token: &CapabilityToken,
        verifying_key: &Ed25519VerifyingKey,
    ) -> Result<Self, LinkError> {
        let _ = endpoint; // Phase 1 reservation (iceoryx2 uses the global config, no explicit endpoint)
        let claims = token.verify(verifying_key).map_err(|e| LinkError::Attach(e.to_string()))?;
        let acl = claims.acl.clone();
        let node_id = NodeId::new(claims.node_id.clone());
        let node = NodeBuilder::new()
            .create::<ipc_threadsafe::Service>()
            .map_err(|e| LinkError::Attach(format!("create iceoryx2 node: {e:?}")))?;
        Registry::register(&NodeInfo::from_claims(&claims))
            .map_err(|e| LinkError::Attach(e.to_string()))?;
        Ok(Self {
            node,
            acl,
            node_id,
            streams: Mutex::new(Vec::new()),
            publishers: Mutex::new(HashMap::new()),
        })
    }

    /// Unified topic-service builder (pub/sub must share config, review C1):
    /// buffer_size=1 + enable_safe_overflow(true) -> latest-frame overwrite;
    /// max_publishers(1) -> single-publisher backstop (D239, enforced cross-process by iceoryx2);
    /// max_subscribers -> see [`MAX_SUBSCRIBERS_PER_TOPIC`] (root fix for the 09-21 cap incident).
    fn topic_service(
        &self,
        topic: &FrameTopic,
    ) -> Result<
        iceoryx2::service::port_factory::publish_subscribe::PortFactory<
            ipc_threadsafe::Service,
            [u8],
            (),
        >,
        LinkError,
    > {
        let name = topic
            .as_str()
            .try_into()
            .map_err(|_| LinkError::Bus(format!("invalid topic name: {}", topic.as_str())))?;
        // open_or_create may hit transient SystemInFlux (service being created/destroyed concurrently); retry
        let mut last_err = None;
        for _ in 0..5 {
            match self
                .node
                .service_builder(&name)
                .publish_subscribe::<[u8]>()
                .subscriber_max_buffer_size(1)
                .enable_safe_overflow(true)
                .max_publishers(1)
                .max_subscribers(MAX_SUBSCRIBERS_PER_TOPIC)
                .open_or_create()
            {
                Ok(service) => return Ok(service),
                Err(e) => {
                    last_err = Some(format!("{e:?}"));
                    if format!("{e:?}").contains("SystemInFlux") {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    return Err(LinkError::Bus(format!("open topic service: {e:?}")));
                }
            }
        }
        Err(LinkError::Bus(format!("open topic service failed after retries: {:?}", last_err)))
    }

    /// Publish one frame: ACL check -> single-publisher check -> cached publisher -> loan into SHM -> send -> record active publisher.
    pub fn publish(
        &self,
        topic: &FrameTopic,
        payload: &[u8],
        meta: &FrameMeta,
    ) -> Result<(), LinkError> {
        if !self.acl.can_publish(topic) {
            return Err(LinkError::AclDenied { topic: topic.as_str().into() });
        }
        // D239 single publisher: another node already publishes this topic -> conflict (process-local fast check)
        if let Some(existing) =
            Registry::topic_publisher(topic).map_err(|e| LinkError::Bus(e.to_string()))?
            && existing != self.node_id
        {
            return Err(LinkError::TopicConflict { topic: topic.as_str().into() });
        }
        let buf_len = FrameMeta::WIRE_LEN + payload.len();
        if buf_len > MAX_FRAME_BYTES {
            return Err(LinkError::Bus(format!("frame too large: {buf_len} > {MAX_FRAME_BYTES}")));
        }
        // Get or create the cached publisher (holding it keeps the max_publishers(1) lock; cross-process conflicts fail here)
        let mut publishers = self
            .publishers
            .lock()
            .map_err(|_| LinkError::Bus("publishers lock poisoned".into()))?;
        if !publishers.contains_key(topic.as_str()) {
            let service = self.topic_service(topic)?;
            let publisher = service
                .publisher_builder()
                .initial_max_slice_len(MAX_FRAME_BYTES)
                .allocation_strategy(AllocationStrategy::Static)
                .create()
                .map_err(|e| {
                    tracing::warn!(topic = %topic.as_str(), "publisher create failed: {e:?}");
                    // iceoryx2 max_publishers(1) backstop: a publisher already exists (cross-process)
                    LinkError::TopicConflict { topic: topic.as_str().into() }
                })?;
            publishers.insert(topic.as_str().to_string(), publisher);
        }
        let publisher = publishers.get(topic.as_str()).expect("just inserted");
        let mut buf = Vec::with_capacity(buf_len);
        buf.extend_from_slice(&meta.encode());
        buf.extend_from_slice(payload);
        let sample = publisher
            .loan_slice_uninit(buf_len)
            .map_err(|e| LinkError::Bus(format!("loan: {e:?}")))?;
        let sample = sample.write_from_slice(&buf);
        sample.send().map_err(|e| LinkError::Bus(format!("send: {e:?}")))?;
        Registry::mark_publisher(topic, &self.node_id)
            .map_err(|e| LinkError::Bus(e.to_string()))?;
        Ok(())
    }

    /// Subscribe a topic: ACL check -> subscriber (buffer_size=1) -> background thread delivers into the latest-slot.
    pub fn subscribe(&self, topic: &FrameTopic) -> Result<FrameStream, LinkError> {
        if !self.acl.can_subscribe(topic) {
            return Err(LinkError::AclDenied { topic: topic.as_str().into() });
        }
        let service = self.topic_service(topic)?;
        let subscriber = service
            .subscriber_builder()
            .buffer_size(1)
            .create()
            .map_err(|e| LinkError::Bus(format!("subscriber create: {e:?}")))?;
        let stream = FrameStream::new();
        let weak = stream.weak_inner();
        let topic_for_thread = topic.clone();
        // Background thread: receive() -> parse meta+payload -> deliver into latest-slot (exits when the stream is dropped)
        // PortFactory(service) moves into the thread so the subscriber borrow stays valid there
        std::thread::spawn(move || {
            // C5 crash recovery: iceoryx2 0.9.3 auto-reconnects when the publisher restarts (proven: at restart
            // seq resets to zero and later frames run on); but the publisher list changes once per restart, and if
            // that connection setup is swallowed by the degradation handler (transient failure) it never retries -> subscriber goes permanently stale.
            // Backstop here: silence longer than REBUILD_IDLE -> rebuild the subscriber (FrameStream handle unchanged,
            // D241); on rebuild failure keep the old handle and retry after cooldown.
            const REBUILD_IDLE: std::time::Duration = std::time::Duration::from_secs(5);
            const REBUILD_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
            let mut subscriber = subscriber;
            let mut last_frame = std::time::Instant::now();
            let mut last_rebuild = std::time::Instant::now() - REBUILD_COOLDOWN;
            // Rebuild returns true on success. On failure keep the old subscriber (may be transient) and retry after cooldown.
            let try_rebuild = |sub: &mut iceoryx2::port::subscriber::Subscriber<
                ipc_threadsafe::Service,
                [u8],
                (),
            >,
                               last_frame: &mut std::time::Instant,
                               last_rebuild: &mut std::time::Instant|
             -> bool {
                match service.subscriber_builder().buffer_size(1).create() {
                    Ok(new_sub) => {
                        *sub = new_sub;
                        *last_rebuild = std::time::Instant::now();
                        *last_frame = std::time::Instant::now();
                        tracing::warn!(topic = %topic_for_thread.as_str(), "subscriber rebuilt (publisher restart / transient connect failure self-healed)");
                        true
                    }
                    Err(e) => {
                        tracing::warn!(topic = %topic_for_thread.as_str(), "subscriber rebuild failed, keeping old handle, retry after cooldown: {e:?}");
                        false
                    }
                }
            };
            while let Some(inner) = weak.upgrade() {
                match subscriber.receive() {
                    Ok(Some(sample)) => {
                        last_frame = std::time::Instant::now();
                        let data: &[u8] = &sample;
                        if data.len() >= FrameMeta::WIRE_LEN {
                            match FrameMeta::decode(&data[..FrameMeta::WIRE_LEN]) {
                                Ok(meta) => {
                                    // Phase 1: one copy into an owned Vec (Phase 2 can hold the Sample for true zero-copy)
                                    let payload = data[FrameMeta::WIRE_LEN..].to_vec();
                                    inner.deliver(FrameRef::new(meta, payload));
                                }
                                // N4: rejected frames must speak up (C15); once per subscriber thread to avoid hot-path flooding
                                Err(e) => {
                                    static VERSION_WARNED: std::sync::atomic::AtomicBool =
                                        std::sync::atomic::AtomicBool::new(false);
                                    if !VERSION_WARNED
                                        .swap(true, std::sync::atomic::Ordering::Relaxed)
                                    {
                                        tracing::warn!("FrameBus meta rejected frame: {e}");
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // Publisher silent for a long time: the connection may be dead without a rebuild trigger (see above) -> rebuild
                        if last_frame.elapsed() > REBUILD_IDLE
                            && last_rebuild.elapsed() > REBUILD_COOLDOWN
                        {
                            try_rebuild(&mut subscriber, &mut last_frame, &mut last_rebuild);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(e) => {
                        // Connection-layer errors: no rebuild during cooldown (avoids a hot loop) and no log spam; a failed rebuild
                        // terminates (old behavior, stream stops delivering). After a successful rebuild receive may keep returning Err (transient/
                        // persistent) -> yield at least 1ms per error to prevent a 100% CPU spin.
                        if last_rebuild.elapsed() > REBUILD_COOLDOWN {
                            tracing::warn!(topic = %topic_for_thread.as_str(), "subscribe receive error: {e:?}");
                            if !try_rebuild(&mut subscriber, &mut last_frame, &mut last_rebuild) {
                                break;
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            }
        });
        self.streams
            .lock()
            .map_err(|_| LinkError::Bus("streams lock poisoned".into()))?
            .push(stream.inner());
        Ok(stream)
    }

    /// This node's ID.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Close: shutdown all streams (recv returns None) + unregister the node (releases publishers and the single-publisher lock).
    pub fn close(self) -> Result<(), LinkError> {
        let streams =
            self.streams.lock().map_err(|_| LinkError::Bus("streams lock poisoned".into()))?;
        for s in streams.iter() {
            s.shutdown();
        }
        Registry::unregister(&self.node_id).map_err(|e| LinkError::Bus(e.to_string()))?;
        Ok(())
    }
}
