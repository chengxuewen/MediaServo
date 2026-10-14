#!/usr/bin/env bash
# warm-dev.sh — pre-warm the Docker dev environment in the background (run once in the morning, zero waits all day)
# D208 build optimization: warm the dev image layers + cargo-cache volume
# Usage: scripts/warm-dev.sh
# Note: retired once pre-baked images land (then docker compose up just pulls)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

log() { echo "[warm-dev] $*"; }

# 1. Build the dev image (~1-2 min on layer-cache hits; full 15-30 min on Cargo.lock-change days)
log "Building dev image..."
docker compose -f "$PROJECT_ROOT/docker-compose.dev.yml" build server

# 2. Warm the target volume: compile server deps once (15-28 min first time, seconds-long increments after)
log "Warming cargo-cache volume (first run compiles deps)..."
docker compose -f "$PROJECT_ROOT/docker-compose.dev.yml" run --rm server \
    cargo check --bin mediaservo-server --features sfu-mediasoup,admin-dashboard

log "Done. Dev environment warm."
