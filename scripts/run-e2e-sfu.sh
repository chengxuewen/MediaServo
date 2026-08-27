#!/usr/bin/env bash
# 浏览器 SFU 拉流 E2E — 前置: server 容器 + Host 进程运行中
# 用法: bash scripts/run-e2e-sfu.sh [--headful] [--legacy]
#   --legacy: 检查旧单进程 host-legacy（默认: 多进程 host-streamer, C4）
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
# 从 server 日志取 admin token (bootstrap token 下一行)
TOKEN=$(docker compose -f docker-compose.dev.yml logs server 2>&1 \
  | grep -A1 "bootstrap token" | tail -1 | tr -d ' ' | sed 's/^server-1|//')
if [ -z "$TOKEN" ]; then
  echo "ERROR: 未找到 admin token — server 是否在运行?" >&2
  exit 1
fi

echo "== SFU 浏览器 E2E (headful=${HEADFUL_FLAG:-0}) =="
echo "  前置检查: server / host / vite"
for port in 9800 5173; do
  curl -s --noproxy "*" -o /dev/null "http://127.0.0.1:${port}/" \
    || { echo "ERROR: 127.0.0.1:${port} 未监听 — 先启动环境" >&2; exit 1; }
done
if [ -n "$LEGACY" ]; then
  pgrep -x host-legacy > /dev/null || { echo "ERROR: Host (legacy) 未运行 — 手动: target/debug/mediaservo-host start .（legacy: cargo run -p mediaservo-host --bin host-legacy）" >&2; exit 1; }
else
  pgrep -x host-streamer > /dev/null || { echo "ERROR: Host (多进程 streamer) 未运行 — 手动: target/debug/mediaservo-host init . && token issue --all . && start ." >&2; exit 1; }
fi

export HEADFUL="$HEADFUL_FLAG"
node scripts/e2e-sfu-consume.cjs "$TOKEN"
