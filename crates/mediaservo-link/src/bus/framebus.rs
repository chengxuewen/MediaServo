//! FrameBus（iceoryx2 SHM 零拷贝帧总线，latest-frame 覆盖语义）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iceoryx2::port::publisher::Publisher;
use iceoryx2::prelude::*;

/// 单 topic 订阅者上限（服务创建时定死的静态配置，iceoryx2 0.9.3 默认仅 8）：
/// D282 defaults 形态后「N 路流共用同一 source」= 合法常态（09-21 实录：8 路 streamer
/// 共享 camera/generator → 8 槽顶格 + 僵尸节点占位 → 确定性 ExceedsMaxSupportedSubscribers，
/// 且旧服务的静态配置跨 restart 存活 = 重启无效）。
/// ponytail: 常量固定；上限 = 边缘盒无 >32 路共享单源的已知场景，升级路径 = host.yaml 配置键。
const MAX_SUBSCRIBERS_PER_TOPIC: usize = 32;

use crate::acl::NodeAcl;
use crate::error::LinkError;
use crate::frame::{FrameMeta, FrameRef, FrameStream, StreamInner};
use crate::id::{FrameTopic, NodeId};
use crate::registry::{NodeInfo, Registry};
use crate::token::{CapabilityToken, Ed25519VerifyingKey};

/// 单帧字节上限（1080p I420 ≈ 3.1MB，留余量）。
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

type TopicPublisher = Publisher<ipc_threadsafe::Service, [u8], ()>;

/// 跨进程 topic 发现结果（E1 拓扑监控数据源，Momus MEDIUM-2 选项 ①）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredTopic {
    pub topic: FrameTopic,
    /// 服务上活跃节点数（0 = 无活进程连接该 topic）。
    pub alive_nodes: usize,
}

/// 帧总线（每节点一个实例；attach 即注册，D235）。
pub struct FrameBus {
    node: Node<ipc_threadsafe::Service>,
    acl: NodeAcl,
    node_id: NodeId,
    /// 已创建流的内部状态（close 时 shutdown）。
    streams: Mutex<Vec<Arc<StreamInner>>>,
    /// 缓存 publisher（持有以维持 max_publishers(1) 单发布者锁 + 交付可靠）。
    publishers: Mutex<HashMap<String, TopicPublisher>>,
}

impl FrameBus {
    /// 跨进程发布者枚举（静态，无需 attach — host-agent 拓扑监控用）。
    ///
    /// 数据源: iceoryx2 0.9.3 服务注册表枚举 `Service::list(Config::global_config(), ...)`
    /// （官方示例见 iceoryx2 源码 `src/service/mod.rs` `Service::list` doc example），
    /// 过滤 PublishSubscribe 模式（FrameBus topic 语义）。
    ///
    /// 限制: 0.9.3 公共 API 不区分发布者/订阅者节点（`ServiceDynamicDetails::nodes`
    /// 是服务上全部连接节点）→ `alive_nodes > 0` = 有活跃进程连接该 topic；
    /// 发布端进程级 liveness 由调用方（host monitor）结合 oxmgr 判定。
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

    /// attach 即注册（D235）：验签（fail-closed）→ 载 ACL → iceoryx2 节点 → `Registry::register`。
    pub fn attach(
        endpoint: &str,
        token: &CapabilityToken,
        verifying_key: &Ed25519VerifyingKey,
    ) -> Result<Self, LinkError> {
        let _ = endpoint; // Phase 1 预留（iceoryx2 用全局 config，无显式 endpoint）
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

    /// 统一 topic service 构造（pub/sub 必须同配置，审核 C1）：
    /// buffer_size=1 + enable_safe_overflow(true) → latest-frame 覆盖；
    /// max_publishers(1) → 单发布者兜底（D239，跨进程由 iceoryx2 强制）；
    /// max_subscribers → 见 [`MAX_SUBSCRIBERS_PER_TOPIC`]（09-21 顶格事故根治）。
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
        // open_or_create 可能遇 SystemInFlux 瞬态（服务正被并发创建/销毁），重试
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

    /// 发布一帧：ACL 检查 → 单发布者检查 → 缓存 publisher → loan 写入 SHM → send → 记录活跃发布者。
    pub fn publish(
        &self,
        topic: &FrameTopic,
        payload: &[u8],
        meta: &FrameMeta,
    ) -> Result<(), LinkError> {
        if !self.acl.can_publish(topic) {
            return Err(LinkError::AclDenied { topic: topic.as_str().into() });
        }
        // D239 单发布者：该 topic 已有其他节点的活跃发布者 → 冲突（进程本地快速检查）
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
        // 取或建缓存 publisher（持有它维持 max_publishers(1) 锁；跨进程冲突在此失败）
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
                    // iceoryx2 max_publishers(1) 兜底：已有发布者（跨进程）
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

    /// 订阅一个 topic：ACL 检查 → subscriber（buffer_size=1）→ 后台线程投递 latest-slot。
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
        // 后台线程：receive() → 解析 meta+payload → deliver latest-slot（流丢弃即退出）
        // PortFactory(service) 移入线程，使 subscriber 的借用在线程内有效
        std::thread::spawn(move || {
            // C5 崩溃恢复: iceoryx2 0.9.3 在发布端重启时能自动重建连接（实证：重启点
            // seq 归零且后续帧连续）；但发布端列表只变一次/重启，若那次连接创建被
            // degradation handler 吞掉（瞬态失败）则永不重试 → 订阅端永久 stale。
            // 此处兜底：长于 REBUILD_IDLE 无帧 → 重建 subscriber（FrameStream 句柄不变，
            // D241）；重建失败保留旧句柄，冷却后重试。
            const REBUILD_IDLE: std::time::Duration = std::time::Duration::from_secs(5);
            const REBUILD_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
            let mut subscriber = subscriber;
            let mut last_frame = std::time::Instant::now();
            let mut last_rebuild = std::time::Instant::now() - REBUILD_COOLDOWN;
            // 重建成功返回 true。失败保留旧 subscriber（可能只是瞬态），冷却后重试。
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
                        tracing::warn!(topic = %topic_for_thread.as_str(), "subscriber 重建完成（发布端重启/瞬态连接失败自愈）");
                        true
                    }
                    Err(e) => {
                        tracing::warn!(topic = %topic_for_thread.as_str(), "subscriber 重建失败，保留旧句柄冷却后重试: {e:?}");
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
                                    // Phase 1：一拷贝进 owned Vec（Phase 2 可持 Sample 实现真零拷贝）
                                    let payload = data[FrameMeta::WIRE_LEN..].to_vec();
                                    inner.deliver(FrameRef::new(meta, payload));
                                }
                                // N4：拒帧必出声（C15）；每订阅线程一次防热路径洪泛
                                Err(e) => {
                                    static VERSION_WARNED: std::sync::atomic::AtomicBool =
                                        std::sync::atomic::AtomicBool::new(false);
                                    if !VERSION_WARNED
                                        .swap(true, std::sync::atomic::Ordering::Relaxed)
                                    {
                                        tracing::warn!("FrameBus meta 拒帧: {e}");
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // 发布端长时间无帧：连接可能已死但重建未触发（见上）→ 重建
                        if last_frame.elapsed() > REBUILD_IDLE
                            && last_rebuild.elapsed() > REBUILD_COOLDOWN
                        {
                            try_rebuild(&mut subscriber, &mut last_frame, &mut last_rebuild);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(e) => {
                        // 连接层错误：冷却期内不重建（避免热循环）且不刷日志；重建失败则
                        // 终止（旧行为，流停投递）。重建成功后 receive 可能继续 Err（瞬态/
                        // 持续）→ 每次错误至少 1ms 让步，防止 100% CPU 自旋。
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

    /// 本节点 ID。
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// 关闭：shutdown 全部流（recv 返回 None）+ 注销节点（释放 publishers 与单发布者锁）。
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
