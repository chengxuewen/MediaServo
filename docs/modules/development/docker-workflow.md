# Docker Development Workflow

> Status: Phase 3 complete | Related: SFU mediasoup integration, D208 build optimization

## Overview

MediaServo uses a multi-stage Dockerfile (base → dev → builder → runtime) and docker-compose to simplify the dev environment. The mediasoup SFU supports Linux only; macOS developers must compile via a Docker container.

## macOS Docker Workflow

### Prerequisites

- Docker Desktop 4.x or Colima
- VS Code + the Dev Containers extension (optional)

### Using docker-compose

```bash
# Start the dev container (source mounted + cargo cache)
docker compose up -d

# Enter the container
docker compose exec server bash

# Compile inside the container (incl. the mediasoup SFU)
cargo check --features sfu-mediasoup

# Run the server
cargo run --bin mediaservo-server
```

### Using the VS Code Dev Container

1. Install the `Dev Containers` extension
2. Open the project root
3. `Ctrl+Shift+P` → `Dev Containers: Reopen in Container`
4. rust-analyzer detects the workspace automatically, with the `sfu-mediasoup` feature enabled

### Build Acceleration

docker-compose.yml already configures a `cargo-cache` named volume to avoid re-downloading dependencies:

```bash
# Clear the cargo cache (if corrupted)
docker volume rm mediaservo_cargo-cache
```

## Linux Native Workflow

Linux compiles natively, no Docker needed (mediasoup is natively supported):

```bash
# Install dependencies (Debian/Ubuntu)
sudo apt-get install -y \
  pkg-config cmake ninja-build \
  libssl-dev libuv1-dev \
  python3 python3-pip

# Install meson (needed by the mediasoup Worker build)
pip3 install meson

# Fetch dependencies
cargo fetch

# Build the server with the SFU
cargo build --features sfu-mediasoup --bin mediaservo-server

# Run
cargo run --features sfu-mediasoup --bin mediaservo-server
```

### clippy check

```bash
cargo clippy --features sfu-mediasoup --all-targets -- -D warnings
```

## Docker Command Reference

| Command | Description |
|---------|-------------|
| `docker compose up -d` | Start the dev container in the background |
| `docker compose down` | Stop and remove containers |
| `docker compose exec server bash` | Enter the running container |
| `docker compose logs -f server` | View server logs |
| `docker compose build --no-cache` | Rebuild the image |
| `docker compose run --rm server cargo test` | Run tests one-off |
| `docker compose run --rm server cargo clippy --features sfu-mediasoup` | clippy inside the container |

## Dockerfile Build Stages

| Stage | Purpose | Base image |
|-------|---------|------------|
| `base` | Rust + system deps + meson | ubuntu:22.04 |
| `dev` | Dev environment (source + cargo fetch) | base |
| `builder` | release build | base |
| `runtime` | Minimal runtime | ubuntu:22.04 |

## Port Mappings

| Port | Purpose |
|------|---------|
| 8000 | HTTP/signaling |
| 40000-40100/udp | mediasoup RTP/RTCP |

> See [SFU mediasoup Integration](../sfu-mediasoup-integration.md)
