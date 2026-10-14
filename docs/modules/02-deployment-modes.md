# Deployment Modes

> MediaServo supports four deployment forms to fit different scenarios.

---

## Deployment Modes Overview

| Form | Architecture | Suitable Scenarios | Plugin Count |
|------|--------------|--------------------|--------------|
| **Embed** | Rust crates statically linked | Third-party platform embedding remote desktop + teleoperation | ~5 |
| **Sidecar** | Container + napi-rs bindings | Host-platform enterprise apps | ~12 |
| **Standalone** | Standalone processes + complete backend | Standalone deployment | All plugins + Web UI |
| **Platform module** | Docker container module | Integrated into the host platform | Auth delegated to the platform |

---

## 1. Embed — Rust crates statically linked

**Goal**: Embed remote desktop and teleoperation capabilities into third-party platforms.

**Characteristics**:
- Only core plugins (screen capture, encoding, transport, decoding, input injection)
- Exposed to third-party platforms via C FFI
- No Web UI, no backend services
- Lowest resource footprint

**Integration**:
```rust
// Referenced from a third-party platform project
extern crate mediaservo_core;
use mediaservo_core::remote::RemoteDesktopClient;
```

---

## 2. Sidecar — Container + napi-rs bindings

**Goal**: Multimedia extension for host-platform enterprise apps.

**Characteristics**:
- Deployed on the host platform as a container
- Node.js bindings via napi-rs
- About 12 core plugins
- Shares infrastructure with the host platform

**Deployment**:
```bash
docker run -d --name mediaservo-sidecar mediaservo/sidecar:latest
```

---

## 3. Standalone — Standalone processes + complete backend

**Goal**: Fully independent multimedia services.

**Characteristics**:
- Complete backend services (user management, permission control, License, signaling)
- Web UI (Tauri v2 desktop app)
- All plugins available
- Ships with SQLite + JWT auth

**Startup**:
```bash
mediaservo-server --config /etc/mediaservo/config.toml
mediaservo-client  # Starts the desktop GUI
```

**Phase 1 ops strategy**:
- systemd service with `Restart=always` + `RestartSec=5s` (D155)
- Single-process deployment: Host functionality cohesive in mediaservo-host (D155 decision)

---

## 4. Platform Module — Docker container

**Goal**: Run as a Docker module of the host platform, analogous to Synology Surveillance Station.

**Characteristics**:
- Zero hard dependency on the host platform
- User/permission management delegated to platform RBAC/LDAP
- Communicates with the host platform via gRPC
- Config: `auth.mode: "aude"`

**Analogy**: like Jira installed on a Synology, using DSM's LDAP accounts.

---

## Docker Phase 2 Plan

MVP Phase 1 centers on single-process systemd. Phase 2 introduces Dockerization:
- docker-compose multi-service orchestration (Server + Client + Host separated)
- Container health checks and auto-restart
- Kubernetes readiness/liveness probes
