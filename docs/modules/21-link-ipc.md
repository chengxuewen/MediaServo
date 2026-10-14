# link IPC Design (bus / registry / permissions / tokens)

> **Status**: design finalized (not implemented) | **Date**: 2026-08-14
> **Related decisions**: D235 (registry), D236 (ROS integration), D237 (permission carrier), D238 (token mechanism), D239 (derived topics)
> **Upstream**: 20-sdk-api-contract.md §3 (link), 04-sdk-layers.md, D222-D232 (four-SDK architecture)
> **Scope**: link's **device-side multiprocess IPC** — the FrameBus frame bus, Registry registration/discovery, ACL permissions, capability tokens. For link's signaling (SignalClient, WS toward the server) see 20-sdk-api-contract.md §3; not repeated here.

---

## 0. The Five Decisions at a Glance

| Decision | Conclusion |
|----------|------------|
| **D235 registry** | Decentralized SHM service registry + **attach-as-register**, no dedicated daemon; the data plane connects zero-copy directly over SHM |
| **D236 ROS integration** | Frame-path ROS nodes **connect directly to the FrameBus** (link SDK + capability tokens); bridging is the escape hatch for non-frame scenarios only |
| **D237 permission carrier** | **Static ACL** (role presets + per-node overrides); enforced at attach + on every operation |
| **D238 token mechanism** | **Capability tokens** (ACL signed into the JWT, device private key signs / public key verifies); reuse the link JWT, no standalone PKI |
| **D239 derived topics** | **Free creation + ACL backstop**; the registry only records, never approves; single publisher per topic |

## 1. Overall Architecture: Data Plane Decoupled from Control Plane

```
┌───────────────────── Control plane (registry/discovery/permissions) ─────────────────────┐
│  Registry: decentralized SHM service registry, attach-as-register, no daemon             │
│  Permission: static ACL + capability tokens, signature verified at attach + checked per  │
│  operation                                                                               │
└──────────────────────────────────────────────────────────────────────────────────────────┘
                               │ constrains/records
┌───────────────────── Data plane (frame transport) ──────────────────────────────────────┐
│  FrameBus: SHM zero-copy direct — publishers write SHM, subscribers read directly; no    │
│  broker in between                                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```
- **Data plane**: frames travel over SHM zero-copy direct connections (publishers write SHM, subscribers read directly) — **zero-copy is a hard requirement** (vehicle-side multi-stream HD); broker-style relays are excluded
- **Control plane**: registry/discovery/permissions sit outside the data path; **decentralized, no central daemon** (vehicle-side 7x24 reliability, avoiding a single point)
- **Premise**: the underlying SHM IPC (iceoryx2 or homegrown) **provides no permissions** — the Permission layer must be self-built, regardless of the underlying choice

## 2. Node Model & Roles

Each worker / ROS node = an independent process holding a **NodeId + Role + capabilities + capability token**.

| Role | Typical process | publish allowed | subscribe allowed |
|------|-----------------|-----------------|-------------------|
| `capture` | Image-production node | `camera/*` | — (no subscription) |
| `perception` | ROS perception node | `perception/*` | `camera/*` |
| `processor` | ROS stitch node | `video/*` | `camera/*` |
| `pusher` | Push node | — (WebRTC only, not on the bus) | `camera/*`, `video/*` |
| `recorder` | Recording node | — | `camera/*`, `video/*` |
| `control` | Control node | `control/cmd` | `control/telemetry`, `status/*` |
| `puller` | Cockpit pull | — (via field, remote) | — (outside the local bus) |

> Reading: a stitch node = `sub camera/* + pub video/*`; push nodes only subscribe, never publish (their "publish" is WebRTC); the control node has exclusive write on `control/cmd`. Least privilege + isolation (a compromised perception node still cannot publish `control/cmd`).

## 3. Topic Namespace

```
camera/{front,rear,cabin}/raw        Capture nodes' raw frames
camera/*/encoded                     Encoded frames
video/stitched                       Stitch-derived stream (processor output)
perception/{objects,lane}            Perception results (perception output)
control/{cmd,telemetry,emergency}    Control/telemetry
status/*                             Node status
```
- Hierarchical naming + wildcards (`camera/*`)
- **Derived topics** (processor output) use hierarchical naming + a producer-semantic prefix to reduce conflicts

## 4. FrameBus Publish/Subscribe

**Semantics** (as settled):
- **Latest-frame overwrite** (immune to backlog/startup order); multiple subscribers consume independently
- **SHM zero-copy**: metadata FlatBuffers + raw pixel memory
- **Single publisher per topic** (D239): a latecomer to an existing same-named topic is rejected

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    #[error("attach failed: {0}")]   Attach(String),
    #[error("acl denied: {0}")]      AclDenied(String),
    #[error("bus error: {0}")]       Bus(String),
    #[error("topic conflict: {0}")]  TopicConflict(FrameTopic),
    #[error("closed")]               Closed,
}

pub struct FrameBus;
impl FrameBus {
    /// attach-as-register: verify the capability token + self-describing registration + load the ACL
    pub fn attach(endpoint: &str, credential: CapabilityToken) -> Result<FrameBus, LinkError>;
    /// publish checks the ACL first (violation → AclDenied + audit)
    pub fn publish(&self, topic: &FrameTopic, frame: FrameRef) -> Result<(), LinkError>;
    /// subscribe checks the ACL first
    pub fn subscribe(&self, topic: &FrameTopic) -> Result<FrameStream, LinkError>;
    pub fn node_id(&self) -> NodeId;
    pub fn close(self) -> Result<(), LinkError>;
}
pub struct FrameTopic(String);
/// latest-slot: slow consumers skip to the newest frame (Arc<Mutex<Option<FrameRef>>> + Notify, replacement semantics).
/// Unbounded queues are forbidden — unbounded reintroduces backlog, violating the latest-frame overwrite semantics (H5 review correction).
pub struct FrameStream { /* latest-slot */ }
```

## 5. Registry (decentralized SHM, attach-as-register)

- **Registration**: automatic in `FrameBus::attach` — the node writes `{id, role, caps, publishes, subscribes}` into the SHM registration area
- **Discovery**: query the SHM registration area (who publishes `camera/*`, which processors are online)
- **Liveness**: heartbeat/lease written into the registration area; disconnects are removed automatically → subscribers get `TopicLost`
- **No daemon**: iceoryx2-style decentralized discovery (free if using iceoryx2); a homegrown version keeps the SHM registration area simple (flat topic table + node table)

```rust
pub struct Registry;
impl Registry {
    pub fn discover_topics(prefix: &str) -> Result<Vec<TopicInfo>, LinkError>;
    pub fn discover_nodes(role: Role) -> Result<Vec<NodeInfo>, LinkError>;
    pub fn topic_publisher(topic: &FrameTopic) -> Result<Option<NodeId>, LinkError>;
}
pub struct NodeInfo { pub id: NodeId, pub role: Role, pub caps: Capabilities,
                      pub publishes: Vec<TopicPattern>, pub subscribes: Vec<TopicPattern> }
pub struct TopicInfo { pub topic: FrameTopic, pub publisher: NodeId }
```

## 6. Permissions: Static ACL + Enforcement Points

**ACL structure** (topic-level + wildcards):
```rust
pub struct NodeAcl {
    pub node_id: NodeId,
    pub role: Role,
    pub publish_allow:  Vec<TopicPattern>,   // e.g. ["camera/*"]
    pub subscribe_allow: Vec<TopicPattern>,  // e.g. ["video/*", "camera/*"]
}
impl NodeAcl {
    pub fn can_publish(&self, topic: &FrameTopic) -> bool;
    pub fn can_subscribe(&self, topic: &FrameTopic) -> bool;
}
pub struct TopicPattern(String);   // Wildcard-aware, e.g. camera/*
```

**Enforcement points**:
1. `FrameBus::attach`: verify the capability token → extract the ACL → load it
2. **Every `publish`/`subscribe`**: check the ACL; violations are rejected + audit-logged

**ACL source config**: role presets (the §2 matrix, fixed) + per-node overrides; managed under the device config with version control. Permission changes go "edit source config → re-sign tokens offline → restart the node" (low frequency, acceptable).

## 7. Capability Tokens (ACL signed into the JWT)

```rust
pub struct CapabilityToken;   // Signed JWT: node_id/role/acl
impl CapabilityToken {
    /// Verify with the device public key → extract claims
    pub fn verify(&self, device_pubkey: &DevicePublicKey) -> Result<Claims, LinkError>;
}
pub struct Claims { pub node_id: NodeId, pub role: Role, pub acl: NodeAcl, pub exp: u64 }
```
- **Issuance**: the device's authoritative component (provisioning / host supervisor) **issues offline** at node deployment; **device private key signs, public key verifies** (asymmetric Ed25519/ES256)
- **Reuses the link JWT** (claims carry `node_id/role/acl`); no standalone node-certificate PKI
- **PSK does not participate in token signing** (PSK is symmetric and for authenticating to the server — a separate concern)
- **Audit**: the ACL source config is written in one place; tokens are signed snapshots
- **Decentralized self-containment**: verification needs only the token + public key — no center, no config-distribution dependency

## 8. Processing Node Pattern (subscribe→transform→republish)

"A ROS stitch node publishes stitched video" = the node is both consumer and producer, creating a **derived topic**:
```
camera/front/raw ─┐
camera/rear/raw  ─┼─► [ros-stitcher node] ─► video/stitched ─► pusher / recorder
camera/cabin/raw ─┘     (subscribes 3 streams → stitch)   (derived topic)
```
- A node may hold `subscribe` + `publish` simultaneously (not mutually exclusive)
- A derived topic is a **newly registered entity**: the Registry records that `video/stitched` is published by the stitcher and depends on `camera/*`
- This forms a **topic DAG / processing graph** (same as the ROS node graph)
- **Governance (D239)**: free creation + ACL backstop — a token allowing `pub video/*` passes `video/stitched` with no approval; the registry only records, never approves; single publisher per topic

## 9. ROS Integration (direct)

- **Frame-path ROS nodes** (perception/stitch) use the link SDK to call `FrameBus::attach(endpoint, credential)` directly:
  - py nodes → ctypes/pyo3 (D227 two-step plan); C++ nodes → cxx
  - The endpoint comes from device config / env vars / the conventional SHM path
  - They hold role tokens (perception/processor); the ACL is enforced at attach
- **Non-frame ROS subsystems**: may use a bridge (a bridge process translating ROS2 topics ↔ FrameBus) to stay pure ROS — escape hatch only
- **Dual protocol stacks**: ROS nodes hold both ROS-DDS (toward ROS internals) + FrameBus (toward MediaServo), with clean responsibilities

## 10. C ABI Binding Forms

Continuing D109 (opaque handles + int error codes + callbacks):
```c
typedef struct ms_link_bus_t ms_link_bus_t;                 /* opaque */
typedef int ms_err_t;                                        /* 0=ok, <0=error */

ms_err_t ms_link_bus_attach(const char* endpoint, const ms_capability_token_t* tok,
                            ms_link_bus_t** out);
ms_err_t ms_link_bus_publish(ms_link_bus_t*, const char* topic, const ms_frame_t* frame);
ms_err_t ms_link_bus_subscribe(ms_link_bus_t*, const char* topic, ms_frame_stream_t** out);
void     ms_link_bus_on_frame(ms_frame_stream_t*, ms_frame_cb cb, void* user);  /* frame callback */
ms_err_t ms_link_bus_close(ms_link_bus_t*);

ms_err_t ms_link_registry_discover_topics(const char* prefix, ms_topic_info_t** out, size_t* n);
ms_err_t ms_last_error(char* buf, size_t len);
```
- Frames cross FFI: pointer + a metadata struct (zero-copy preferred)
- py loads the cdylib via ctypes; C++ uses header-only RAII

## 11. Full-Chain Scenario Walkthrough (stitched video)

1. **Capture node** attaches (role=capture, token `pub camera/*`) → `CameraSource.frames()` → `publish("camera/front/raw")` (SHM zero-copy)
2. **ROS stitch node** attaches (role=processor, token `sub camera/*` + `pub video/*`) → `subscribe` to 3 streams → stitch → `publish("video/stitched")` (free creation, ACL permits, registry self-describes, single publisher)
3. **Push node** attaches (role=pusher, token `sub video/*`) → `subscribe("video/stitched")` → `field::PushSession` pushes WebRTC

Permissions throughout: attach verifies the signature → publish/subscribe check the ACL → violations rejected + audited.

## 12. Backward Compatibility & Evolution

| Item | Rule |
|------|------|
| Public enums `#[non_exhaustive]` | `LinkError`, `Role` |
| New roles/topics appended | Additive; existing ones untouched |
| Evolution (no pre-embedding) | Node scale/consistency needs rising → a lightweight registry may be introduced; runtime revocation → a lightweight revoke list; derived-topic strong governance → whitelists/quotas added to the ACL source config |
