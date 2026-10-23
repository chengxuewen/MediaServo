# MediaServo

MediaServo — Real-Time Media Servo Platform (MediaServo Platform). A standalone video/media service platform centered on servo-driven "precision, low latency, responsiveness", covering surveillance camera ingest and recording/playback (NVR), video conferencing, remote desktop, teleoperation, live streaming push/pull, and more. Can be deployed standalone or embedded into third-party platforms as a Docker module.

## Feature Scope

- **Remote desktop**: screen capture, GPU encoding (H.264/H.265), input injection, <100ms latency
- **Video conferencing**: multiparty audio/video calls, SFU/MCU, screen sharing
- **Streaming push/pull**: RTMP/HLS/SRT ingest and distribution, live transcoding
- **Surveillance ingest**: ONVIF/GB28181 camera discovery and stream management
- **WebRTC teleoperation**: low-latency video + DataChannel control (vehicles/robots)
- **Vehicle streaming**: vehicle cameras push to cloud / in-cabin pull

## Architecture

```
┌──────────────────────────────────────────────┐
│           MediaServo Backend Services        │
│   User mgmt · Access control · License · Signaling │
└──────────────────┬───────────────────────────┘
                   │ gRPC / REST
    ┌──────────────┼──────────────┐
    ▼              ▼              ▼
┌──────────┐ ┌──────────┐ ┌──────────────┐
│  Client  │ │   Host   │ │ Embedded/Module │
│ (GUI)    │ │(headless)│ │ Platform module │
│ Operator │ │ Remote   │ │              │
└──────────┘ └──────────┘ └──────────────┘
```

- **Client**: full-featured desktop GUI application (Tauri v2); can control others and be controlled
- **Host**: headless daemon, suited for edge devices/servers/vehicles; purely produces media streams
- **Microkernel + plugins**: mediaservo-common as the microkernel; domain features loaded as plugins
- **Auth dual mode**: standalone deployment ships its own account system; as a third-party platform module, delegates to platform RBAC/LDAP

See [`docs/architecture.md`](docs/architecture.md).

## Tech Stack

- **Native layer**: Rust (edition 2024), libwebrtc (primary) / str0m / webrtc-rs three backends
- **Bindings layer**: napi-rs (Node.js), C FFI (statically linked into the host app)
- **Signaling**: WebSocket (Phase 1) + MQTT 5.0 (Phase 2+)
- **Transport**: RTP/RTCP, SRT, WebRTC DataChannel
- **Internal protocol**: FlatBuffers (zero-copy, multi-language)

## Development Status

Phase 3 complete. 7-crate workspace, webrtc triple-backend, codec three backends (stub+FFmpeg+GStreamer), 400+ tests all passing, Docker/CI/DevContainer in place.

## Build & Development

Unified CLI entry point (available after bootstrap):

```bash
./mediaservo.sh -h            # All subcommands (Windows: mediaservo.bat)

# Verb × target matrix (target: server | host | client | all)
./mediaservo.sh build         # Build all (host/client native + server Docker)
./mediaservo.sh build host    # Host side only (alias: build-host)
./mediaservo.sh build server  # Server only (alias: build-server)
./mediaservo.sh up --env dev  # Start dev server container (idempotent, auto-injects ANNOUNCED_IP); prod: up --env prod
./mediaservo.sh up --announced-ip 10.144.0.3  # Explicit container announced address (overrides auto-detect; multiple values comma-separated —
                                  # containers only honor a single announced address, first IP wins; multiple addresses require bare-metal run)
./mediaservo.sh build server --native  # Bare-metal compile server (pixi toolchain, needs network for meson wraps; multi-IP announce takes effect at run stage)
./mediaservo.sh run server --native [--foreground]  # Retired → guidance + exit 2 (run plane = instance commands or up/down for containers; T21/C39)
# host run CLI wrapper removed (D266) — deployed instance: systemctl --user start oxmgr-<brand>-host.service / <prefix>/bin/msrtc-host start <prefix>;
# dev: target/debug/mediaservo-host init . && token issue --all . && start . (clear iceoryx2 residue first)
./mediaservo.sh stop server   # Retired → guidance + exit 2 (instance: <prefix>/bin/<brand>-server stop <dir>; containers: mediaservo down)
./mediaservo.sh stop host     # Removed (D266) — manual: systemctl --user stop oxmgr-<brand>-host.service / <prefix>/bin/msrtc-host stop <prefix>
./mediaservo.sh logs server    # Server logs (default bare-metal target/server-native.log; --env dev/--mode compose container logs; --native explicit)
./mediaservo.sh down --env dev && ./mediaservo.sh up --env dev   # Restart dev server (clear old then start; config changes take effect)
./mediaservo.sh logs host     # Host logs (/tmp/mediaservo-host.log)
./mediaservo.sh e2e sfu       # e2e_sfu regression (suites: sfu push ui host package brand bindings client smoke; prereq per suite)
./mediaservo.sh status server  # Health probe (default bare-metal native — pid+port+announced; --mode compose/--env dev container health; exit 0/1/2)
./mediaservo.sh status host    # Push processes + gateway 17980 + instance logs (C38 layer 2); environment diagnostics see doctor
./mediaservo.sh config validate   # Config validation (bash arm; serde_yaml via mediaservo-gates)
./mediaservo.sh clean         # Clean build artifacts (keeps cargo-cache volume); clean --all clears everything
./mediaservo.sh deploy host  # Assemble+deploy to prefix (--prefix required; brand via MEDIASERVO_BRAND; vehicle /opt/mediaservo-host); deploy bindings same
./mediaservo.sh package host|bindings  # dist/ dual-package release (D-H13): mediaservo-host-<ver>.tar.gz + mediaservo-sdk-<ver>.tar.gz (each tar has a versioned top-level dir + CHANGES.md = verbatim copy of repo-root CHANGELOG.md, human-readable update log for consumers)
```

> client target is registered (build works); up/down/restart/logs client pending skeleton completion.

First use: `source bootstrap.sh` (Linux/macOS) or `bootstrap.bat` (Windows).

IDE (VS Code + CMake Tools): copy `.vscode/settings.example.json` to `.vscode/settings.json` — the committed presets (`cmake --preset default|bindings|host|server|all`) drive the assembly tree; examples are the F25 `-DMEDIASERVO_BUILD_EXAMPLES=ON` route (plain shell or `mediaservo.sh build example`).
Windows: planned, not yet supported (win-64 is not declared in pixi.toml platforms; bootstrapping per `docs/modules/25-abi-baseline.md` §10.4 ticket O4 once a customer list materializes).

## License

Apache 2.0
