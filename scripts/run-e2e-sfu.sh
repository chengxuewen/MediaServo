#!/usr/bin/env bash
# Browser SFU pull E2E — prereq: server container + Host process running
# Usage: bash scripts/run-e2e-sfu.sh [--headful] [--legacy]
#   --legacy: check the old single-process host-legacy (default: multi-process host-streamer, C4)
set -euo pipefail
cd "$(dirname "$0")/.."

HEADFUL_FLAG=""
LEGACY=""
for arg in "$@"; do
  case "$arg" in
    --headful) HEADFUL_FLAG="1" ;;
    --legacy) LEGACY="1" ;;
  esac
done
# Extract admin token from server logs (line after bootstrap token)
TOKEN=$(docker compose -f docker-compose.dev.yml logs server 2>&1 \
  | grep -A1 "bootstrap token" | tail -1 | tr -d ' ' | sed 's/^server-1|//')
if [ -z "$TOKEN" ]; then
  echo "ERROR: admin token not found — is the server running?" >&2
  exit 1
fi

echo "== SFU browser E2E (headful=${HEADFUL_FLAG:-0}) =="
echo "  prereq check: server / host / vite"
for port in 9800 5173; do
  curl -s --noproxy "*" -o /dev/null "http://127.0.0.1:${port}/" \
    || { echo "ERROR: 127.0.0.1:${port} not listening — start the environment first" >&2; exit 1; }
done
if [ -n "$LEGACY" ]; then
  pgrep -x host-legacy > /dev/null || { echo "ERROR: Host (legacy) not running — manually: target/debug/mediaservo-host start . (legacy: cargo run -p mediaservo-host --bin host-legacy)" >&2; exit 1; }
else
  pgrep -x host-streamer > /dev/null || { echo "ERROR: Host (multi-process streamer) not running — manually: target/debug/mediaservo-host init . && token issue --all . && start ." >&2; exit 1; }
fi

export HEADFUL="$HEADFUL_FLAG"
node scripts/e2e-sfu-consume.cjs "$TOKEN"
