# MediaServo Product Reference Documentation

> Phase 0 → Phase 5 planned | 2026-07-20 | Active decisions D196+ (D1-D190 archived)
> MVP v2 relay model (D118) · Host single process (D155) · Based on the [architecture design doc](../architecture.md)

---

## Table of Contents

1. [Product Capability Overview](./01-product-capabilities.md)
2. [Deployment Modes](./02-deployment-modes.md)
3. [Client & Host](./03-client-host.md)
4. [SDK Layers & API Reference](./04-sdk-layers.md)
5. [Auth & Permissions Reference](./05-auth-permissions.md)
6. [Plugin System Reference](./06-plugin-system.md)
7. [Protocols & Communication Reference](./07-protocols.md)
8. [Pipeline Model Reference](./08-pipeline-model.md)
9. [Transport Architecture](./09-transport-architecture.md)
10. [Signaling Architecture](./10-signaling-architecture.md)
11. [NAPI Bindings](./11-napi-binding.md)
12. [Recording & Playback](./12-recording-playback.md)
13. [Server Architecture](./13-server-architecture.md)
14. [Remote Architecture](./14-remote-architecture.md)
15. [Component Architecture](./15-component-architecture.md)
16. [Admin Dashboard](./16-admin-dashboard.md)
17. [WebRTC Crate](./17-webrtc-crate.md)
18. [Codec Crate](./18-codec-crate.md)
19. [TLS Reverse Proxy & CORS Configuration](./19-tls-cors-configuration.md)
20. [SDK API Contract](./20-sdk-api-contract.md)
21. [Link IPC](./21-link-ipc.md)
22. [Field Guide](./22-field-guide.md)
23. [Binding Guide](./23-binding-guide.md)
24. [App Branding](./24-app-branding.md)
25. [ABI Baseline](./25-abi-baseline.md)
26. [Room Model](./26-room-model.md)
27. [Error Model](./27-error-model.md)
28. [Security Architecture](./28-security-architecture.md)
29. [Operations](./29-operations.md)
30. [Testing Strategy](./30-testing-strategy.md)
31. [Upgrade & Migration](./31-upgrade-migration.md)
32. [CI/CD Pipeline](./32-ci-cd-pipeline.md)
33. [SFU (mediasoup) Integration](./33-sfu-mediasoup-integration.md)
34. [Site Deployment](./34-site-deployment.md)
35. [Container Ops Manual](./35-container-ops-manual.md)

---

## Overview

MediaServo is independently deployable multimedia infrastructure offering seven product capabilities. Core crates: mediaservo-host (capture+encode+push), mediaservo-client (pull+decode+control), mediaservo-server (signaling+relay+monitoring). Phase 1-2 use the Host single-process form (D155); the overall Phase 0-5 plan is in the architecture doc.

```
Industrial control end    ──┐              ┌── Host platform (enterprise app)
                            ├── MediaServo ──┤
   references native crates │  media core   │ Docker module
```

- **Standalone deployment**: complete backend with its own user/permission system
- **Platform module**: Docker container delegating to platform RBAC/LDAP
- **Third-party platform embedding**: statically linked Rust crates, remote desktop + teleoperation only
