# 15. Component Architecture

> Status: Phase 1 design | Related decisions: D126–D134 | **Phase numbering note**: this document uses the phase numbering of phase3-production/plan.md. The Component framework = Phase 3, Plugin = Phase 5.

## The Three-Layer Logical Abstraction Model

At the code level MediaServo uses a three-layer logical abstraction (D126), complementary to D1's deployment-topology three layers:

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 3 — Process (deployment layer)                        │
│  An OS process hosting Component execution. Phase 1 single  │
│  process; Phase 2 multiprocess (systemd/K8s outer           │
│  supervision)                                               │
├─────────────────────────────────────────────────────────────┤
│ Layer 2 — Component (service layer)                         │
│  Service-level units with independent lifecycles.           │
│  Communicate via ComponentBus.                              │
│  Phase 1: Gateway / Signaling / Relay / Admin / Auth /      │
│  Monitor                                                    │
├─────────────────────────────────────────────────────────────┤
│ Layer 1 — Plugin (pipeline layer)                           │
│  Media pipeline elements (capture/encode/decode/render).    │
│  Communicate via MediaPort (frame queues), managed by the   │
│  PipelineEngine.                                            │
│  Phase 1 involves only control-plane Components, no Plugin  │
│  pipelines.                                                 │
└─────────────────────────────────────────────────────────────┘
```

## Component trait

```rust
/// Base trait of service-layer components.
/// Each Component has an independent init → run → shutdown lifecycle.
#[async_trait]
pub trait Component: Send + Sync {
    /// Unique component identifier
    fn id(&self) -> ComponentId;

    /// Initialization (sync in Phase 1; Phase 2 may load resources async)
    async fn init(&mut self, ctx: &ComponentContext) -> Result<(), ComponentError>;

    /// Main run loop (spawned by ComponentManager as an independent tokio task)
    async fn run(self: Arc<Self>, ctx: ComponentContext) -> Result<(), ComponentError>;

    /// Graceful shutdown
    async fn shutdown(&self) -> Result<(), ComponentError>;
}
```

### ComponentContext

The runtime context passed to every Component:

```rust
pub struct ComponentContext {
    /// Message bus (RPC + pub/sub)
    pub bus: Arc<dyn ComponentBus>,
    /// The component's own ID
    pub self_id: ComponentId,
}
```

**Phase 1 note**: ComponentContext holds no `PipelineEngine` reference. Phase 1's 6 Components are all control-plane and never create media pipeline nodes. When Phase 2 introduces data-plane Components such as RecordingComponent, nodes are created via `PluginManager::create_node()` (that `create_node` stub is implemented by then).

## ComponentBus

The unified communication bus between Components.

### Dual-Mode Routing (D132)

| Mode | Method | Use | Example |
|------|--------|-----|---------|
| RPC (1:1) | `send_rpc::<Q,R>(query) → Result<R>` | Request-response, type-safe | Gateway→Auth: validate token |
| Pub/Sub (1:N) | `publish(topic, event)` / `subscribe(topic)` | Event broadcast | Signaling→Monitor: peer_joined |

### InProcessBus Implementation (Phase 1)

```rust
pub struct InProcessBus {
    rpc_handlers: DashMap<TypeId, DashMap<ComponentId, Box<dyn AnyChannel>>>,
    subscribers: DashMap<String, Vec<broadcast::Sender<ComponentEvent>>>,
}
```

- **Channel-per-type** (D133): `register_rpc_handler::<Query, Reply>()` is compile-time type-safe with zero serialization overhead
- **Zero serialization**: Phase 1 passes Rust types directly in-process. When Phase 2 switches to ZenohBus, the Bus implementation layer decides the serialization strategy
- **Arc\<Component\> bypass**: WebSocket connections cannot be serialized through the Bus, so Gateway directly holds `Arc<SignalingComponent>` and calls `handle_socket()` — a special case, not encouraged for generalization

## ComponentManager Supervision Tree

```
ComponentManager
├── GatewayComponent     ─── crash-loop: max_restarts=5, window=60s
├── SignalingComponent   ─── crash-loop: max_restarts=5, window=60s
├── AdminComponent       ─── crash-loop: max_restarts=3, window=60s
├── RelayComponent       ─── crash-loop: max_restarts=3, window=60s
├── AuthComponent        ─── crash-loop: max_restarts=5, window=60s
└── MonitorComponent     ─── crash-loop: max_restarts=5, window=60s
```

State machine: `Created → Initializing → Running → Crashed | Stopped`

**Phase 1**: in-process supervision (D134). ComponentManager monitors JoinHandles with crash-loop protection (max_restarts + time window).

**Phase 2**: in multiprocess deployment each process keeps its own ComponentManager (inner supervision); between processes systemd Restart=on-failure or K8s restartPolicy takes over (outer supervision).

## GatewayComponent Routing (D128)

Phase 1 has a unified HTTP Gateway on a single port `:9800`. The route table is hardcoded (YAGNI; Phase 2 introduces the mount API):

```
:9800
├── /health              → MonitorComponent (status check)
├── /metrics             → MonitorComponent (Prometheus metrics)
├── /api/auth/login      → AuthComponent (JWT issuance)
├── /api/auth/validate   → AuthComponent (Token validation)
├── /ws                  → SignalingComponent (WebSocket upgrade)
├── /admin/api/*         → AdminComponent (REST API)
├── /admin/*             → Phase 2 placeholder (SPA static files)
└── /api/relay/*         → RelayComponent (Relay management)
```

**Relationship to D52**: D128's Gateway is the evolution of D52's single-port signaling service pattern — D52's axum service is absorbed into the Gateway, with SignalingComponent handling WS signaling internally.

## AuthProvider Authentication (D130)

```rust
#[async_trait]
pub trait AuthProvider: Send + Sync {
    async fn login(&self, credentials: LoginRequest) -> Result<AuthToken, AuthError>;
    async fn validate(&self, token: &AuthToken) -> Result<UserInfo, AuthError>;
    async fn authorize(&self, user: &UserInfo, permission: Permission) -> Result<bool, AuthError>;
}
```

**RBAC alignment (D88)**: three predefined roles (admin / operator / auditor) plus a feature-permission enum (host:read, host:control, server:config, record:read, record:delete, user:manage, etc.).

**Relationship to PskAuthenticator**: `PskAuthenticator` (core/auth.rs) is the Phase 1 transitional implementation — used for the WebSocket PSK handshake. `AuthProvider` handles HTTP JWT auth. The two coexist with non-overlapping responsibilities.

## Phase Breakdown

### Phase 1 (current)

| Item | Scope |
|------|-------|
| Component types | Control-plane only: Gateway / Signaling / Admin / Relay / Auth / Monitor |
| Communication | InProcessBus (tokio::mpsc) |
| Serialization | None (Rust types passed directly) |
| Plugin pipelines | Not involved. Phase 1 Components don't need the PipelineEngine |
| Deployment | Single process |
| Route registration | Hardcoded in the Gateway |
| Supervision | ComponentManager, single level |

### Phase 2 (planned)

| Item | Scope |
|------|-------|
| Component types | Data plane: RecordingComponent, RTMPComponent, etc. |
| Communication | ZenohBus (network transparent) |
| Plugin pipelines | PluginManager::create_node() implemented; Components can create PipelineNodes |
| Deployment | Multiprocess (systemd/K8s outer supervision) |
| Serialization | Decided by the ZenohBus implementation |
| Route registration | mount_routes() API |
| Admin UI | React 19 + Ant Design 5 SPA |

## Related Decisions

| Decision | Content |
|----------|---------|
| D126 | Three-layer abstraction model (Plugin / Component / Process) |
| D127 | Component trait in its own crate `mediaservo-component` |
| D128 | Unified HTTP Gateway pattern |
| D129 | tokio::mpsc → Zenoh communication middleware |
| D130 | AuthProvider trait auth architecture |
| D131 | Component three-stage lifecycle (init→run→shutdown) |
| D132 | Dual-mode routing (send_rpc + publish/subscribe) |
| D133 | Channel-per-type type-safe RPC |
| D134 | Simplified supervision tree (ComponentManager single-level) |
| D88 | RBAC role-permission model |
| D52 | Signaling service architecture |
| D10 | FlatBuffers internal protocol |
| D-ERR-01/02 | Error model (Components use ThisError enums, not the 5-digit error-code system) |

## Existing Module Migration

| Existing file | Migration approach |
|---------------|--------------------|
| `signaling.rs` | SignalingServer unchanged, wrapped by SignalingComponent |
| `monitor.rs` | monitor_router() absorbed by the Gateway, replaced by MonitorComponent |
| `relay.rs` | Relay unchanged, wrapped by RelayComponent |
| `main.rs` | Rewritten as the ComponentManager startup entry |
