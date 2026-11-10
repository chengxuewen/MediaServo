# Phase 1 — link IPC Implementation Plan (iceoryx2 foundation + FrameBus thin layer) [revision v2]

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **v2 revision** (after team review): task order re-sequenced (eliminates stubs/backfill) · adds `enable_safe_overflow` · adds D239 single-publisher enforcement · attach-as-register wiring · tokens use Ed25519 · FrameStream switched to latest-slot · Registry uses iceoryx2 built-in liveness/discovery · adds audit logs/close/kill-9 · SignalClient split out · FrameMeta gains format+version.

**Goal:** Establish the `mediaservo-link` crate: an iceoryx2-based cross-process zero-copy frame bus (FrameBus) + decentralized registry (Registry) + static ACL permissions + capability tokens, supporting the local IPC of the vehicle-side multiprocess topology (image production/push/ROS stitching). **Excludes SignalClient (WS signaling toward the server) — split into a Phase 1b follow-up plan.**

**Architecture:** iceoryx2 0.9.3 as the SHM transport foundation (zero-copy pub/sub, service discovery, node liveness out of the box); FrameBus as a thin layer on top (latest-frame overwrite semantics, `camera/*` topics, frame metadata, ACL enforcement points, capability-token attach, single-publisher enforcement). Decentralized with no daemon (D235); authorization via signed capability tokens validated locally (D237/D238).

**Tech Stack:** Rust edition **2024**, iceoryx2 `=0.9.3`, jsonwebtoken (Ed25519/EdDSA), tokio, thiserror, tracing, serde, mediaservo-common.

**Governing decisions:** D235/D236/D237/D238/D239/D242 + D243 (fixed-length LE frame metadata, FlatBuffers deferred).

## Global Constraints

- **Rust edition `2024`** (hard requirement of iceoryx2 0.9.3; `edition.workspace = true`)
- **iceoryx2 pinned `=0.9.3`** (pre-1.0 drift guard, D242)
- **latest-frame mechanism**: every topic service is created via the **unified `topic_service()` helper**; the service builder must explicitly `.subscriber_max_buffer_size(1).enable_safe_overflow(true)` (the default safe_overflow=false stalls publish with ReceiverCacheIsFull when the buffer fills — verified by C1 review)
- **FrameStream = latest-slot** (bounded-1 replacement; slow consumers skip to the newest frame), **unbounded queues forbidden** (unbounded reintroduces backlog, violating latest-frame)
- **Single publisher (D239)**: check the Registry before creating a publisher; if the topic already has one → `LinkError::TopicConflict`
- **attach-as-register (D235)**: `FrameBus::attach` completes signature verification → ACL load → `Registry::register`
- **Tokens are Ed25519 asymmetric** (D238): signed by the device private key, verified by each node's public key; **the verifying public key comes from device config** (`MEDIASERVO_DEVICE_PUBKEY` env or a config file path); **HS256 symmetric is forbidden** (key possession = forgery)
- **Token TTL**: verified once at attach; 7x24 nodes use a long TTL (or re-attach on expiry); already-attached nodes are unaffected by mid-life expiry (static ACL semantics)
- **Audit logs**: every ACL deny path `tracing::warn!` (D237 + C15)
- **Errors**: thiserror, `Result<T, LinkError>`, no `unwrap()` on production paths
- **Zero-copy discipline**: subscribers read only the iceoryx2 sample view; frame memcpy forbidden
- **Threat model (one line)**: ACL is a **library-level check**, guarding against "honest but misconfigured" nodes; a malicious process can open same-named services directly with iceoryx2 and bypass it (the foundation has no permission layer — known D237/D238 limitation)
- **Testing**: TDD; multiprocess integration tests must actually run; `cargo test` doesn't build examples — pre-build child-process helpers with `cargo build --example` + pass paths via `env!("CARGO_BIN_EXE_...")` or env vars
- **Commits**: one commit per task; conventional commits; **creating a crate / changing the workspace = structural change, requires user confirmation (execution gate)**

---

## File Structure

```
crates/mediaservo-link/                 # new crate (structural change, needs confirmation)
├── Cargo.toml                          # edition.workspace; iceoryx2=0.9.3, jsonwebtoken, tokio, thiserror, tracing, serde; depends on common
├── src/
│   ├── lib.rs                          # pub mod + re-exports
│   ├── error.rs                        # LinkError (incl. TopicConflict)
│   ├── id.rs                           # NodeId, FrameTopic (branded + wildcard matching)
│   ├── frame.rs                        # FrameMeta (with format+version) + FrameRef + FrameStream (latest-slot)
│   ├── acl.rs                          # Role, NodeAcl, can_publish/can_subscribe (audit logs)
│   ├── token.rs                        # CapabilityToken (Ed25519 sign/verify, ACL claims)
│   ├── registry/mod.rs                 # Registry (attach-as-register + Service::list discovery + Node::list liveness)
│   └── bus/framebus.rs                 # FrameBus (topic_service helper + safe_overflow + single publisher + attach wiring)
└── tests/
    ├── frame.rs / acl.rs / token.rs / registry.rs
    ├── framebus.rs                     # single-process loopback
    ├── framebus_multiproc.rs           # multiprocess zero-copy (cargo build --example helper)
    └── e2e_link.rs                     # image→stitch→push + ACL negatives + single-publisher conflict + kill-9 crash-restart
examples/
    └── framebus_pub.rs                 # child-process publisher for multiprocess tests
```

**Execution order (v2 re-sequence, no stubs/backfill):** Task 0 → 1 → 2(ACL) → 3(Token) → 4(Registry) → 5(FrameBus) → 6(e2e). ACL/Token are pure data + crypto with no iceoryx2 dependency, so they go first; FrameBus builds on them in one pass.

---

## Task 0: dependency wiring + crate skeleton

**Files:**
- Modify: root `Cargo.toml` (workspace members + `[workspace.dependencies]` add iceoryx2/jsonwebtoken)
- Create: `crates/mediaservo-link/Cargo.toml`, `src/lib.rs`, `src/error.rs`
- ⚠️ **Structural change (new crate) — requires user confirmation before executing**

**Interfaces:**
- Produces: a compilable `mediaservo-link` crate; `LinkError` (incl. `TopicConflict`)

- [ ] **Step 1: user confirms creating the `mediaservo-link` crate (execution gate)**
- [ ] **Step 2: add workspace dependencies to the root Cargo.toml**

```toml
# [workspace.dependencies]
iceoryx2 = "=0.9.3"
jsonwebtoken = "9"
```
and add `crates/mediaservo-link` to `members`.

- [ ] **Step 3: write crates/mediaservo-link/Cargo.toml** (edition 2024, drop bytes)

```toml
[package]
name = "mediaservo-link"
version = "0.1.0"
edition.workspace = true   # = 2024 (hard requirement of iceoryx2 0.9.3)

[dependencies]
mediaservo-common = { path = "../mediaservo-common" }
iceoryx2 = { workspace = true }
jsonwebtoken = { workspace = true }   # v9 supports EdDSA (Ed25519)
thiserror = "2"
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
tokio = { version = "1", features = ["rt", "sync", "time", "macros"] }

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

- [ ] **Step 4: write src/error.rs + src/lib.rs skeleton**

```rust
// src/error.rs
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LinkError {
    #[error("attach failed: {0}")]
    Attach(String),
    #[error("acl denied: {topic}")]
    AclDenied { topic: String },
    #[error("topic already has a publisher: {topic}")]
    TopicConflict { topic: String },          // D239 single publisher
    #[error("token invalid: {0}")]
    Token(String),
    #[error("registry error: {0}")]
    Registry(String),
    #[error("bus error: {0}")]
    Bus(String),
    #[error("closed")]
    Closed,
}
```
```rust
// src/lib.rs
pub mod error; pub mod id; pub mod frame; pub mod acl; pub mod token; pub mod registry; pub mod bus;
pub use error::LinkError;
pub use id::{NodeId, FrameTopic};
pub use frame::{FrameMeta, FrameRef, FrameStream};
pub use acl::{Role, NodeAcl};
pub use token::CapabilityToken;
pub use registry::Registry;
pub use bus::framebus::FrameBus;
```

- [ ] **Step 5: verify compile + cargo deny audit of iceoryx2 (D242 impact)**
Run: `cargo check -p mediaservo-link` then `cargo deny check` (confirm iceoryx2's ~40 transitive deps pass the audit)
Expected: check PASS; deny shows no new CRITICAL

- [ ] **Step 6: Commit**
```bash
git add Cargo.toml Cargo.lock crates/mediaservo-link deny.toml
git commit -m "feat(link): mediaservo-link crate skeleton + LinkError(incl TopicConflict) (D235/D242)"
```

---

## Task 1: branded IDs + FrameMeta (with format+version) + FrameStream (latest-slot)

**Files:**
- Create: `src/id.rs`, `src/frame.rs`
- Test: `tests/frame.rs`

**Interfaces:**
- Produces: `NodeId`, `FrameTopic (matches wildcard)`, `FrameMeta (seq/width/height/format/ts_mono_ns/ts_epoch_ns/is_keyframe/version)`, `FrameRef`, `FrameStream (latest-slot)`
- Consumes: none

- [ ] **Step 1: write the failing tests**

```rust
// tests/frame.rs
use mediaservo_link::{FrameTopic, FrameMeta};

#[test]
fn topic_wildcard_match() {
    let t = FrameTopic::new("camera/front/raw");
    assert!(t.matches("camera/*"));
    assert!(!t.matches("video/*"));
}

#[test]
fn frame_meta_has_format_and_version_roundtrip() {
    let m = FrameMeta { seq: 7, width: 1920, height: 1080, format: 1 /* I420 */,
                        ts_mono_ns: 123, ts_epoch_ns: 456, is_keyframe: true, version: 1 };
    let d = FrameMeta::decode(&m.encode()).unwrap();
    assert_eq!((d.seq, d.format, d.version), (7, 1, 1));
    assert!(d.is_keyframe);
}
```

- [ ] **Step 2: run tests, confirm failure**
Run: `cargo test -p mediaservo-link --test frame`
Expected: FAIL

- [ ] **Step 3: implement id.rs + frame.rs**

```rust
// src/id.rs — branded IDs + wildcard matching
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct NodeId(String);
impl NodeId { pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
              pub fn as_str(&self) -> &str { &self.0 } }

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct FrameTopic(String);
impl FrameTopic {
    pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
    pub fn as_str(&self) -> &str { &self.0 }
    /// `camera/*` prefix wildcard matches `camera/front/raw`
    pub fn matches(&self, pattern: &str) -> bool {
        match pattern.strip_suffix("/*") {
            Some(pfx) => self.0.starts_with(&format!("{pfx}/")),
            None      => self.0 == pattern,
        }
    }
}
```
```rust
// src/frame.rs — FrameMeta fixed-length LE (with format+version, M4/D243) + FrameStream latest-slot (H5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct FrameMeta {
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub format: u8,        // pixel format (0=unknown,1=I420,2=NV12,3=RGBA...)
    pub version: u8,       // metadata version (for evolution, D243)
    pub is_keyframe: bool,
    pub ts_mono_ns: u64,
    pub ts_epoch_ns: u64,
}
impl FrameMeta {
    pub const WIRE_LEN: usize = 8+4+4+1+1+1+1+8+8; // fixed-length LE (incl. alignment padding)
    pub fn encode(&self) -> [u8; Self::WIRE_LEN] { /* LE encoding */ todo_impl() }
    pub fn decode(b: &[u8]) -> Result<Self, crate::LinkError> { todo_impl() }
}

/// FrameRef = a zero-copy view of an iceoryx2 sample (FrameMeta + payload &[u8], holds the sample to prevent early release)
pub struct FrameRef { /* meta + payload view */ }
impl FrameRef { pub fn meta(&self) -> &FrameMeta { todo_impl() }
                pub fn payload(&self) -> &[u8] { todo_impl() } }

/// FrameStream = latest-slot (H5: slow consumers skip to the newest frame, not an unbounded queue)
/// Implementation: Arc<Mutex<Option<FrameRef>>> + tokio::sync::Notify;
/// a background thread replaces the slot's frame on every sample received and notify_waiters; consumers await notify then take the latest.
pub struct FrameStream { /* slot + notify */ }
impl FrameStream {
    pub async fn recv(&self) -> Option<FrameRef> { todo_impl() } // await notify -> take latest
    pub(crate) fn spawn(/* iceoryx2 subscriber */) -> Self { todo_impl() }
}
```
> **FlatBuffers deferred in favor of fixed-length LE, recorded as D243** (cross-language ROS consumers parse the fixed layout; future evolution adds FlatBuffers, made compatible via the version field).

- [ ] **Step 4: run tests, confirm pass**
Run: `cargo test -p mediaservo-link --test frame`
Expected: PASS

- [ ] **Step 5: Commit**
```bash
git add crates/mediaservo-link/src/id.rs crates/mediaservo-link/src/frame.rs crates/mediaservo-link/tests/frame.rs crates/mediaservo-link/src/lib.rs
git commit -m "feat(link): branded IDs + FrameMeta(format/version) + FrameStream latest-slot (D243/H5)"
```

---

## Task 2: ACL (static ACL + wildcards + audit logs)

**Files:**
- Create: `src/acl.rs`
- Test: `tests/acl.rs`

**Interfaces:**
- Consumes: `FrameTopic` (Task 1)
- Produces: `Role`, `NodeAcl { node_id, role, publish_allow, subscribe_allow }`, `for_role/can_publish/can_subscribe`

- [ ] **Step 1: write the failing tests** (D237 permission matrix line by line)

```rust
// tests/acl.rs
use mediaservo_link::{NodeAcl, Role, FrameTopic};

#[test]
fn capture_pub_camera_not_control_no_sub() {
    let acl = NodeAcl::for_role(Role::Capture);
    assert!(acl.can_publish(&FrameTopic::new("camera/front/raw")));
    assert!(!acl.can_publish(&FrameTopic::new("control/cmd")));
    assert!(!acl.can_subscribe(&FrameTopic::new("camera/front/raw")));
}

#[test]
fn processor_sub_camera_pub_video() {
    let acl = NodeAcl::for_role(Role::Processor);
    assert!(acl.can_subscribe(&FrameTopic::new("camera/front/raw")));
    assert!(acl.can_publish(&FrameTopic::new("video/stitched"))); // derived topic (D239)
    assert!(!acl.can_publish(&FrameTopic::new("control/cmd")));
}
```

- [ ] **Step 2: run tests, confirm failure**
Run: `cargo test -p mediaservo-link --test acl`
Expected: FAIL

- [ ] **Step 3: implement acl.rs** (role preset matrix + node overrides + deny audit logging M1)

```rust
// src/acl.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Role { Capture, Processor, Pusher, Puller, Recorder, Control, Perception }

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeAcl {
    pub node_id: crate::NodeId,
    pub role: Role,
    pub publish_allow: Vec<String>,   // wildcard patterns, e.g. "camera/*"
    pub subscribe_allow: Vec<String>,
}
impl NodeAcl {
    /// D237 permission matrix presets: capture pub camera/*; processor sub camera/* + pub video/*;
    /// pusher sub camera/*,video/*; recorder sub camera/*,video/*;
    /// control pub control/cmd + sub control/telemetry,status/*; perception pub perception/* + sub camera/*; puller none.
    pub fn for_role(role: Role) -> Self { todo_impl() }
    pub fn can_publish(&self, topic: &FrameTopic) -> bool {
        let ok = self.publish_allow.iter().any(|p| topic.matches(p));
        if !ok { tracing::warn!(node=%self.node_id.as_str(), topic=%topic.as_str(), "ACL deny publish"); } // M1 audit
        ok
    }
    pub fn can_subscribe(&self, topic: &FrameTopic) -> bool {
        let ok = self.subscribe_allow.iter().any(|p| topic.matches(p));
        if !ok { tracing::warn!(node=%self.node_id.as_str(), topic=%topic.as_str(), "ACL deny subscribe"); }
        ok
    }
}
```

- [ ] **Step 4: run tests, confirm pass**
Run: `cargo test -p mediaservo-link --test acl`
Expected: PASS

- [ ] **Step 5: Commit**
```bash
git add crates/mediaservo-link/src/acl.rs crates/mediaservo-link/tests/acl.rs crates/mediaservo-link/src/lib.rs
git commit -m "feat(link): static ACL role matrix + wildcards + deny audit logs (D237)"
```

---

## Task 3: capability tokens (Ed25519 sign/verify, ACL claims)

**Files:**
- Create: `src/token.rs`
- Test: `tests/token.rs`

**Interfaces:**
- Consumes: `NodeAcl`, `Role`, `NodeId` (Task 2); jsonwebtoken EdDSA
- Produces: `CapabilityToken::sign/verify`, `Claims { node_id, role, acl, exp }`, `test_keypair()`

- [ ] **Step 1: write the failing tests** (roundtrip + tamper rejection + expiry rejection)

```rust
// tests/token.rs
use mediaservo_link::{CapabilityToken, NodeAcl, Role};

#[test]
fn sign_verify_roundtrip_ed25519() {
    let acl = NodeAcl::for_role(Role::Processor);
    let (sk, vk) = CapabilityToken::test_keypair();   // random Ed25519 keypair
    let tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    let claims = tok.verify(&vk).unwrap();
    assert_eq!(claims.role, Role::Processor);
    assert!(claims.acl.can_publish(&"video/stitched".into()));
}

#[test]
fn tampered_rejected() {
    let acl = NodeAcl::for_role(Role::Capture);
    let (sk, vk) = CapabilityToken::test_keypair();
    let mut tok = CapabilityToken::sign(&acl, 3600, &sk).unwrap();
    tok.tamper();
    assert!(tok.verify(&vk).is_err());
}
```

- [ ] **Step 2: run tests, confirm failure**
Run: `cargo test -p mediaservo-link --test token`
Expected: FAIL

- [ ] **Step 3: implement token.rs** (Ed25519, HS256 forbidden; public key source in Global Constraints)

```rust
// src/token.rs — D238: ACL signed into the JWT; Ed25519 asymmetric (device private key signs, each node's public key verifies)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Claims {
    pub node_id: String,
    pub role: Role,
    pub acl: NodeAcl,      // ACL signed into the token
    pub exp: u64,
}
pub struct CapabilityToken(String); // JWT
impl CapabilityToken {
    /// Ed25519 signing (jsonwebtoken Algorithm::EdDSA). HS256 forbidden (symmetric; key possession = forgery).
    pub fn sign(acl: &NodeAcl, ttl_secs: u64, signing_key: &Ed25519SigningKey) -> Result<Self, LinkError> { todo_impl() }
    /// Public-key verification + exp check. Verifying key source: MEDIASERVO_DEVICE_PUBKEY env / device config file.
    pub fn verify(&self, verifying_key: &Ed25519VerifyingKey) -> Result<Claims, LinkError> { todo_impl() }
    #[cfg(test)] pub fn test_keypair() -> (Ed25519SigningKey, Ed25519VerifyingKey) { todo_impl() }
    #[cfg(test)] pub fn tamper(&mut self) { /* flip one payload bit */ todo_impl() }
}
```
> TTL policy: verified once at attach; 7x24 nodes use a long TTL or re-attach on expiry; already-attached nodes are unaffected by mid-life expiry.

- [ ] **Step 4: run tests, confirm pass**
Run: `cargo test -p mediaservo-link --test token`
Expected: PASS

- [ ] **Step 5: Commit**
```bash
git add crates/mediaservo-link/src/token.rs crates/mediaservo-link/tests/token.rs crates/mediaservo-link/src/lib.rs
git commit -m "feat(link): capability tokens Ed25519 sign/verify + ACL claims (D238)"
```

---

## Task 4: Registry (attach-as-register + built-in discovery/liveness)

**Files:**
- Create: `src/registry/mod.rs`
- Test: `tests/registry.rs`

**Interfaces:**
- Consumes: `NodeId`, `Role` (Tasks 1/2); iceoryx2 `Node::list`, `ipc::Service::list`
- Produces: `Registry::register/discover_topics/discover_nodes/topic_publisher`, `NodeInfo { id, role, publishes, subscribes }`, `TopicInfo { topic, publisher }`

- [ ] **Step 1: write the failing tests** (registered topics/publishers discoverable)

```rust
// tests/registry.rs
use mediaservo_link::{Registry, registry::NodeInfo, NodeId, Role};

#[test]
fn register_then_discover() {
    let info = NodeInfo { id: NodeId::new("capture-0"), role: Role::Capture,
                          publishes: vec!["camera/front/raw".into()], subscribes: vec![] };
    Registry::register(&info).unwrap();
    let topics = Registry::discover_topics("camera/").unwrap();
    assert!(topics.iter().any(|t| t.topic.as_str() == "camera/front/raw"));
}
```

- [ ] **Step 2: run tests, confirm failure**
Run: `cargo test -p mediaservo-link --test registry`
Expected: FAIL

- [ ] **Step 3: implement registry** (use iceoryx2 built-ins, delete hand-written heartbeat — M5)
> **Discovery**: `ipc::Service::list(Config::global_config(), cb)` lists active services (= topics).
> **Liveness**: `Node::<ipc::Service>::list(Config::global_config(), cb)` returns `NodeState::Alive/Dead` — built-in via the iceoryx2 filesystem lock mechanism, **no hand-written heartbeat/lease needed**. `DeadNodeView::try_remove_stale_resources()` cleans zombie resources.
> **Node self-description** (publishes/subscribes/role): written to iceoryx2 service attributes or an agreed-upon registration service; **register is called by `FrameBus::attach` (wired in Task 5, D235 attach-as-register)**.
> `topic_publisher(topic)`: queries the topic service's publisher (for Task 5's single-publisher check).

- [ ] **Step 4: run tests, confirm pass**
Run: `cargo test -p mediaservo-link --test registry`
Expected: PASS

- [ ] **Step 5: add a liveness test** (kill a node process → `Node::list` reports Dead + resource cleanup → restart recovers) + run it green
- [ ] **Step 6: Commit**
```bash
git add crates/mediaservo-link/src/registry crates/mediaservo-link/tests/registry.rs crates/mediaservo-link/src/lib.rs
git commit -m "feat(link): Registry attach-as-register + Service::list discovery + Node::list liveness (D235)"
```

---

## Task 5: FrameBus (core: safe_overflow + single publisher + attach wiring + latest-slot)

**Files:**
- Create: `src/bus/mod.rs`, `src/bus/framebus.rs`
- Create: `examples/framebus_pub.rs` (multiprocess test child)
- Test: `tests/framebus.rs` (single process), `tests/framebus_multiproc.rs` (multiprocess zero-copy)

**Interfaces:**
- Consumes: `FrameTopic/FrameMeta/FrameRef/FrameStream` (Task 1), `NodeAcl` (Task 2), `CapabilityToken` (Task 3), `Registry` (Task 4)
- Produces: `FrameBus::attach/publish/subscribe/close/node_id`

- [ ] **Step 1: write the single-process loopback failing test**

```rust
// tests/framebus.rs
use mediaservo_link::{FrameBus, FrameTopic, FrameMeta, CapabilityToken, NodeAcl, Role};

#[tokio::test]
async fn pubsub_roundtrip_latest_frame() {
    let (sk, vk) = CapabilityToken::test_keypair();
    let tok = CapabilityToken::sign(&NodeAcl::for_role(Role::Processor), 3600, &sk).unwrap();
    let bus = FrameBus::attach("test", &tok, &vk).unwrap(); // verify vk (local public key for tests)
    let topic = FrameTopic::new("camera/test/raw");
    let stream = bus.subscribe(&topic).unwrap();
    bus.publish(&topic, &[1u8,2,3], &FrameMeta::default()).unwrap();
    let frame = stream.recv().await.unwrap();   // latest-slot recv
    assert_eq!(frame.payload(), &[1u8,2,3]);
}
```

- [ ] **Step 2: run tests, confirm failure**
Run: `cargo test -p mediaservo-link --test framebus`
Expected: FAIL

- [ ] **Step 3: implement framebus.rs**

```rust
// src/bus/framebus.rs
use iceoryx2::prelude::*;

pub struct FrameBus {
    node: Node<ipc::Service>,
    acl: NodeAcl,
    node_id: crate::NodeId,
}
impl FrameBus {
    /// attach-as-register (D235): verify → load ACL (fail-closed: verification failure rejects) → Registry::register
    pub fn attach(endpoint: &str, token: &CapabilityToken, vk: &Ed25519VerifyingKey) -> Result<Self, LinkError> {
        let claims = token.verify(vk)?;                 // verification failure → LinkError::Token (fail-closed)
        let acl = claims.acl;                            // ACL from the signed token
        let node = NodeBuilder::new().create::<ipc::Service>().map_err(/*...*/)?;
        let info = registry::NodeInfo::from_claims(&claims);
        Registry::register(&info)?;                      // attach-as-register wiring (H3)
        Ok(Self { node, acl, node_id: crate::NodeId::new(claims.node_id) })
    }

    /// Unified topic service construction (pub/sub must share config, C1):
    /// buffer_size=1 + enable_safe_overflow=true → latest-frame overwrite (without explicitly enabling safe_overflow it stalls)
    fn topic_service(&self, topic: &FrameTopic) -> Result</* PortFactory */ ..., LinkError> {
        self.node.service_builder(&topic.as_str().try_into()?)
            .publish_subscribe::<[u8]>()
            .subscriber_max_buffer_size(1)
            .enable_safe_overflow(true)                  // ★ C1 critical
            .open_or_create()
            .map_err(/*...*/)
    }

    pub fn publish(&self, topic: &FrameTopic, payload: &[u8], meta: &FrameMeta) -> Result<(), LinkError> {
        if !self.acl.can_publish(topic) { return Err(LinkError::AclDenied { topic: topic.as_str().into() }); } // audit warn inside
        // D239 single publisher: topic already has one → TopicConflict
        if Registry::topic_publisher(topic)?.is_some() {
            return Err(LinkError::TopicConflict { topic: topic.as_str().into() });
        }
        let svc = self.topic_service(topic)?;
        let publisher = svc.publisher_builder()
            .initial_max_slice_len(FrameMeta::WIRE_LEN + payload.len()) // Static: must >= max frame
            .allocation_strategy(AllocationStrategy::Static)
            .create().map_err(/*...*/)?;
        let mut buf = Vec::with_capacity(FrameMeta::WIRE_LEN + payload.len());
        buf.extend_from_slice(&meta.encode());
        buf.extend_from_slice(payload);
        let sample = publisher.loan_slice_uninit(buf.len()).map_err(/*...*/)?;
        let sample = sample.write_from_slice(&buf);
        sample.send().map_err(/*...*/)?;                 // frame written in place to SHM (zero-copy source)
        Ok(())
    }

    pub fn subscribe(&self, topic: &FrameTopic) -> Result<FrameStream, LinkError> {
        if !self.acl.can_subscribe(topic) { return Err(LinkError::AclDenied { topic: topic.as_str().into() }); }
        let svc = self.topic_service(topic)?;            // same config as publish (C1)
        let subscriber = svc.subscriber_builder().buffer_size(1).create().map_err(/*...*/)?;
        Ok(FrameStream::spawn(subscriber))               // background thread receive → latest-slot
    }

    pub fn node_id(&self) -> &crate::NodeId { &self.node_id }
    pub fn close(self) -> Result<(), LinkError> { /* Registry unregister + drop node, M6 */ todo_impl() }
}
```
> **Constraint**: `AllocationStrategy::Static` requires `initial_max_slice_len` >= the max frame, otherwise `LoanError::ExceedsMaxLoanSize`; variable frames use `PowerOfTwo`. FrameStream uses Task 1's latest-slot (replacement semantics).

- [ ] **Step 4: run the single-process test, confirm pass**
Run: `cargo test -p mediaservo-link --test framebus`
Expected: PASS

- [ ] **Step 5: write the multiprocess zero-copy integration test** (child-process helper: `cargo build --example framebus_pub` + path via env — L4)

```rust
// tests/framebus_multiproc.rs
// Main process subscribes camera/mp/raw; spawns the examples/framebus_pub child publishing 1080p frames;
// asserts payload.len()==3_110_400 received, and that it is an SHM view (zero-copy)
```
```rust
// examples/framebus_pub.rs — child process: attach(role=capture) → publish a 3_110_400B frame
```

- [ ] **Step 6: run the multiprocess test (must actually run)**
Run: `cargo build --example framebus_pub -p mediaservo-link && cargo test -p mediaservo-link --test framebus_multiproc -- --nocapture`
Expected: PASS, 1080p frames received

- [ ] **Step 7: Commit**
```bash
git add crates/mediaservo-link/src/bus crates/mediaservo-link/examples crates/mediaservo-link/tests/framebus*.rs crates/mediaservo-link/src/lib.rs
git commit -m "feat(link): FrameBus on iceoryx2 — safe_overflow latest-frame + single publisher + attach wiring (D242/D239/D235/C1)"
```

---

## Task 6: e2e (image→stitch→push + negatives + crash)

**Files:**
- Create: `tests/e2e_link.rs`

**Interfaces:**
- Consumes: FrameBus/Registry/ACL/CapabilityToken (Tasks 2-5)
- Produces: scenario-level acceptance (maps to 21-link-ipc.md §11)

- [ ] **Step 1: write the integration test**
```rust
// tests/e2e_link.rs (multiprocess)
// capture(role=capture) pub camera/front/raw
// processor(role=processor) sub camera/* → stitch → pub video/stitched (D239 derived)
// pusher(role=pusher) sub video/stitched
// Assert: pusher receives the derived frame
```
- [ ] **Step 2: ACL negative** (processor tries control/cmd → `AclDenied`)
- [ ] **Step 3: single-publisher conflict** (a second capture publishes the same topic → `TopicConflict`)
- [ ] **Step 4: kill-9 crash-restart** (kill the capture process → `Node::list` reports Dead + resource cleanup → restart recovers, M6)
- [ ] **Step 5: run the tests (must actually run, multiprocess)**
Run: `cargo test -p mediaservo-link --test e2e_link -- --nocapture`
Expected: PASS (including negatives and crash recovery)
- [ ] **Step 6: Commit**
```bash
git add crates/mediaservo-link/tests/e2e_link.rs
git commit -m "test(link): e2e image->stitch->push + ACL/single-publisher negatives + kill-9 crash recovery (D236/D239/D235)"
```

---

## Split-out: SignalClient (WS signaling toward the server) → Phase 1b follow-up plan

21-link-ipc.md explicitly scopes SignalClient out of this IPC plan; this plan targets local IPC. **SignalClient (WS to the server, reusing SignalingMessage, needs tokio-tungstenite) is split into an independent Phase 1b plan** and does not block Tasks 0-6; once split out, link becomes leaner for lightweight ROS consumers.

## Wrap-up: docs sync (M3)

- [ ] Sync the link section of `docs/modules/20-sdk-api-contract.md §3`: `attach(endpoint, token, vk)`, `LinkError` (incl. TopicConflict), the `Registry` API, `FrameRef` unified, remove "transport decided in Phase 3" (D242 already chose iceoryx2)
- [ ] `docs/modules/21-link-ipc.md §4`: `FrameStream` changes from `UnboundedReceiver` to **latest-slot** (H5, spec-of-record corrected in the same stroke)
- [ ] `decisions.md` records **D243** (FrameMeta fixed-length LE + format/version, FlatBuffers deferred)

---

## Self-Review (v2)

- **Decision coverage**: D235→Tasks 4+5 (attach wiring); D236→Tasks 5/6; D237→Task 2 (+audit); D238→Task 3 (Ed25519); D239→Task 5 (single publisher) + error TopicConflict; D242→Task 5 (safe_overflow); D243→Task 1 (format/version) ✅
- **Order**: 0→1→2→3→4→5→6, FrameBus built on ACL/Token/Registry, no stubs/backfill ✅
- **C1 fix**: `enable_safe_overflow(true)` + unified `topic_service()` construction ✅
- **H5 fix**: FrameStream latest-slot (defined in Task 1, used in Task 5), spec corrected in sync ✅

## Risks & De-risking

| Risk | De-risk |
|------|---------|
| iceoryx2 latest-frame (buffer=1+safe_overflow) behavior | Task 5 validates the single-process loopback first, then multiprocess |
| Multiprocess tests flaky in CI | Explicit subprocess + timeout + `cargo build --example` pre-build + path via env |
| 0.9.3 vs spike (main 0.9.999) API differences | Pin =0.9.3; Task 0 `cargo check` confirms resolution first; compare against the 0.9.3 tag before implementing |
| Malicious processes bypassing ACL | Threat model declared (library-level check, no permission layer in the foundation); solved outside this phase |

## Execution Gate Reminders

- **Task 0 creating the `mediaservo-link` crate = structural change; hands on keys only after user confirmation**
- This plan is a document only; confirm each item before writing code
