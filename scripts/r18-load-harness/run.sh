#!/bin/bash
# R18 Small-tier SFU load test — one-shot orchestrator (container-build-rework).
# Builds the browser load harness, starts a host-networking prod server, drives
# 10 rooms / 20 peers (720p H.264 ~2Mbps) steady 10min + burst 2min, samples
# docker stats every 5s, and computes mem_limit = cgroup peak x1.3 round-up 512MB.
#
# Usage: bash run.sh            # full Small tier (ROOMS=10 PER_ROOM=2 STEADY=600 BURST=120)
#        ROOMS=1 PER_ROOM=2 STEADY=20 BURST=5 bash run.sh   # quick smoke
# Env: IMAGE (default msrtc-server:0.1.3), ROOMS, PER_ROOM, STEADY, BURST, BITRATE, FPS.
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# node toolchain: pixi env first, else PATH
if [ -x "$REPO_ROOT/.pixi/envs/default/bin/node" ]; then
  export PATH="$REPO_ROOT/.pixi/envs/default/bin:$PATH"
fi
command -v node >/dev/null || { echo "FAIL: node not found (pixi env or PATH)"; exit 1; }

# deps
[ -d node_modules ] || npm install --registry=https://registry.npmmirror.com --no-audit --no-fund
# fake 720p30 complex video source (testsrc2 -> ~2Mbps under H.264)
if [ ! -f fake.y4m ]; then
  command -v ffmpeg >/dev/null || { echo "FAIL: ffmpeg not found"; exit 1; }
  ffmpeg -y -f lavfi -i testsrc2=size=1280x720:rate=30 -t 2 -pix_fmt yuv420p -f yuv4mpegpipe fake.y4m
fi
# bundle the browser peer client
./node_modules/.bin/esbuild load-client.js --bundle --format=iife --platform=browser --target=chrome120 --outfile=load.bundle.js

# server (host networking) + driver
IMAGE="${IMAGE:-msrtc-server:0.1.3}" bash setup-server.sh
PSK="$(cat r18-psk.txt)" \
WS_URL="ws://127.0.0.1:19800/ws" \
CONTAINER="${NAME:-msrtc-r18}" \
ROOMS="${ROOMS:-10}" PER_ROOM="${PER_ROOM:-2}" \
STEADY="${STEADY:-600}" BURST="${BURST:-120}" \
BITRATE="${BITRATE:-2000000}" FPS="${FPS:-30}" \
OUT="${OUT:-r18-result.json}" \
node driver.mjs
echo "=== result: $SCRIPT_DIR/${OUT:-r18-result.json}"