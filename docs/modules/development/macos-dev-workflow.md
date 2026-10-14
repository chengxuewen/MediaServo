# macOS Development Workflow

> Status: Phase 3 complete | Related: Docker SFU architecture, Native Host/Client

## Architecture Overview

macOS development uses a **hybrid architecture**: Host and Client run natively; the Server (SFU) runs via Docker.

```
┌───────────────────────────────────────────────┐
│                  Docker (Linux)               │
│  ┌─────────────────────────────────────────┐  │
│  │         mediaservo-server               │  │
│  │  ┌──────────┐  ┌────────────────────┐   │  │
│  │  │ Signaling│  │  mediasoup Worker  │   │  │
│  │  │  (WS)    │  │  (RTP relay)       │   │  │
│  │  └────┬─────┘  └────────┬───────────┘   │  │
│  └───────┼────────────────┼────────────────┘  │
│          │ 8000/WS        │ 40000-40100/udp   │
└──────────┼────────────────┼───────────────────┘
           │                │
     ┌─────┴────┐     ┌─────┴────┐
     │  macOS   │     │  macOS   │
     │  Host    │     │  Client  │
     │ (native) │     │ (native) │
     │ :9801    │     │ :9101    │
     └──────────┘     └──────────┘
```

- **Server**: Linux only (the mediasoup C++ Worker depends on Linux epoll + libuv). Under macOS it must run via Docker.
- **Host / Client**: pure Rust, compiled and run natively on macOS. The Host handles capture+encode+push; the Client handles pull+decode.

## Quick Start

### Prerequisites

- macOS 13+ (Intel or Apple Silicon)
- Docker Desktop 4.x or Colima
- pixi (via `curl -fsSL https://pixi.sh/install.sh | bash`)

First use:

```bash
source bootstrap.sh   # Installs pixi + GStreamer + the Rust toolchain
```

Every time after that:

```bash
source pixi.sh        # Activates the pixi environment only
```

### Three-Step Startup

```bash
# 1. Start the Docker SFU server
docker compose up -d
# Verify: docker compose logs -f server

# 2. Start the Host (macOS native)
cargo run --bin mediaservo-host -- --config config/host.conf

# 3. Start the Client (macOS native, new terminal)
source pixi.sh
cargo run --bin mediaservo-client -- --config crates/mediaservo-client/config/remote.conf
```

## Configuration Files

### host.conf

Path: `config/host.conf` (default `/opt/mediaservo/etc/host.conf`)

```yaml
host:
  id: "host-001"

signaling:
  ws_url: "ws://localhost:9800/ws"   # Points at the Server in Docker

media:
  camera: "/dev/video0"              # macOS camera device
  width: 1280
  height: 720
  fps: 30
  bitrate_kbps: 2000
  encoder: "auto"                    # or "nvh264enc" / "x264enc"
  format: "I420"

# Optional: TURN server
# turn:
#   urls: "turn:192.168.1.100:3478"
#   username: "user"
#   credential: "pass"
```

### remote.conf

Path: `crates/mediaservo-client/config/remote.conf` (default `/opt/mediaservo/etc/remote.conf`)

```yaml
version: 1

server:
  signaling_url: "ws://localhost:9800/ws"

# psk: "mediaservo-dev"        # Signaling auth pre-shared key (default value)
```

## Day-to-Day Development Flow

```
Edit code → compile check → run tests → integration tests
```

### Edit-Compile Loop

```bash
# Incremental check (fastest)
cargo check --workspace

# Check only the Host
cargo check --bin mediaservo-host

# Full build (with the SFU feature, must run inside Docker)
docker compose exec server cargo build --features sfu-mediasoup --bin mediaservo-server
```

### Running Tests

```bash
# All tests (excluding SFU; mediasoup unsupported on macOS)
cargo test --workspace

# Host tests only
cargo test -p mediaservo-host

# SFU tests inside Docker
docker compose exec server cargo test -p mediaservo-server --features sfu-mediasoup

# Lint check
cargo clippy --workspace --all-targets -- -D warnings

# Format check
cargo fmt --all -- --check
```

### Integration Test Flow

```bash
# 1. Restart the Server (applies changes)
docker compose restart server

# 2. New terminal: start the Host
cargo run --bin mediaservo-host -- --config config/host.conf

# 3. New terminal: start the Client
cargo run --bin mediaservo-client -- --config crates/mediaservo-client/config/remote.conf

# 4. View logs
docker compose logs -f server          # Server logs
# Host/Client logs print to their own terminals (JSON format)
```

## Port Notes

| Port | Process | Purpose |
|------|---------|---------|
| 8000 | Server (Docker) | WebSocket signaling |
| 9801 | Host (macOS) | metrics HTTP |
| 9101 | Client (macOS) | health/config/metrics HTTP |
| 40000-40100/udp | Server (Docker) | mediasoup RTP/RTCP media streams |
| 9999 | Host (macOS) | Emergency control UDP |

The macOS-local port 8000 is forwarded to the container by the Docker port mapping; Host/Client just connect to `ws://localhost:9800/ws`.

## Build Acceleration

### Cargo Cache

docker-compose.yml already configures a `cargo-cache` named volume to avoid re-downloads:

```bash
# Clear the cache (if corrupted)
docker volume rm mediaservo_cargo-cache
```

### macOS Native Compilation

- Speed up with `sccache`:
  ```bash
  brew install sccache
  # Add to .cargo/config.toml:
  # [build]
  # rustc-wrapper = "sccache"
  ```
- Use the `lld` linker (installed by pixi): 3-5x faster than the default ld

## Troubleshooting

### Docker Networking

**Problem**: Host/Client cannot connect to `ws://localhost:9800`

```bash
# Check whether the Server is running
docker compose ps

# View Server logs
docker compose logs server --tail 20

# Test port connectivity
curl -v http://localhost:9800/ 2>&1 | head -5

# If the port conflicts:
sudo lsof -i :9800
# Change the left-side port mapping in docker-compose.yml, e.g. "18000:9800"
```

### STUN / NAT Traversal

**Problem**: WebRTC ICE connection fails; logs show `ICE connection failed`

```bash
# 1. Confirm both ends are on the same network (localhost by default in local dev; no STUN needed)
# 2. Cross-network development: add STUN/TURN to host.conf and remote.conf
```

Uncomment in host.conf:
```yaml
turn:
  urls: "turn:your-turn-server:3478"
  username: "user"
  credential: "pass"
```

### Cargo Cache Corruption

```bash
# macOS native
cargo clean

# Docker
docker compose down -v && docker compose up -d
```

### GStreamer Missing

Under macOS, pixi installs GStreamer automatically. If pipeline initialization fails:

```bash
# Verify GStreamer is available
gst-inspect-1.0 --version

# If not installed, reinstall via pixi
pixi install
```

When GStreamer is unavailable the Host automatically degrades to headless mode (no actual capture output), designed for E2E testing.

### Apple Silicon vs Intel

pixi.toml declares both `osx-64` and `osx-arm64` platforms; pixi selects the matching conda packages automatically. No manual switching needed.

## Docker Command Reference

| Command | Description |
|---------|-------------|
| `docker compose up -d` | Start the Server container in the background |
| `docker compose down` | Stop and remove containers |
| `docker compose restart server` | Restart the Server (after code changes) |
| `docker compose exec server bash` | Enter the container shell |
| `docker compose logs -f server` | Follow Server logs |
| `docker compose build --no-cache` | Full image rebuild |
| `docker compose exec server cargo check --features sfu-mediasoup` | Compile check inside the container |
| `docker compose exec server cargo test -p mediaservo-server --features sfu-mediasoup` | Run SFU tests inside the container |

## Related Documents

- [Docker Development Workflow](docker-workflow.md) — Docker image build stages, Dev Container, Linux native workflow
- [Architecture](../../architecture.md) — overall architecture design
