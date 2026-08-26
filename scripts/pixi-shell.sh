#!/usr/bin/env bash
# pixi-shell.sh — Activate pixi environment in current shell
# Source this file: source scripts/pixi-shell.sh
# Do NOT run as a subshell (./pixi-shell.sh) — env vars would be lost.

set -eo pipefail

SCRIPT_DIR_PSHELL="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR_PSHELL}/_common.sh"

# TTY 守卫（评审 H1）：非交互/管道（-h 抓取、脚本消费）不打印横幅——帮助输出干净
if [ -t 1 ]; then
  echo "Activating MediaServo pixi environment..."
fi
# Pre-set vars that conda completion scripts reference without defaults
export ZSH_VERSION="${ZSH_VERSION:-}"
eval "$("${PIXI_BIN}" shell-hook --manifest-path "${PROJECT_ROOT}/pixi.toml" --shell bash)"

if [ -t 1 ]; then
  echo ""
  echo "MediaServo environment active."
  echo "  pixi run build     — cargo build --workspace"
  echo "  pixi run test      — cargo test --workspace"
  echo "  pixi run lint      — cargo clippy + fmt check"
  echo "  pixi run check     — cargo check"
  echo ""
  echo "Deactivate with: exit  (or close this shell)"
fi

# Import proxy from ~/.bashrc if not already set
if [ -z "${http_proxy:-}" ] && [ -f "${HOME}/.bashrc" ]; then
  proxy_line=$(grep -m1 'export http_proxy=' ~/.bashrc 2>/dev/null || true)
  if [ -n "$proxy_line" ]; then
    eval "$proxy_line"
    eval "$(grep -m1 'export https_proxy=' ~/.bashrc 2>/dev/null || true)"
    echo "Proxy loaded from ~/.bashrc"
  fi
fi

# Set DYLD_LIBRARY_PATH for GStreamer runtime linking on macOS
if [ -n "${CONDA_PREFIX:-}" ]; then
  export DYLD_LIBRARY_PATH="${CONDA_PREFIX}/lib:${DYLD_LIBRARY_PATH:-}"
fi
if [ -n "${CONDA_PREFIX:-}" ]; then
  export DYLD_LIBRARY_PATH="${CONDA_PREFIX}/lib:${DYLD_LIBRARY_PATH:-}"
fi
