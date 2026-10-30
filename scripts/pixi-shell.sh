#!/usr/bin/env bash
# pixi-shell.sh — Activate pixi environment in current shell
# Source this file: source scripts/pixi-shell.sh
# Do NOT run as a subshell (./pixi-shell.sh) — env vars would be lost.
# Local variant of the supplier activation trio (PolyOrch/scripts/pixi.sh) — kept
# because it carries MediaServo host-policy stages the supplier does not (bashrc
# proxy import, macOS DYLD for GStreamer). Re-diff against the supplier copy on each
# supplier bump (F20 as-built split: bootstrap delegates, activation stays local).

set -eo pipefail

SCRIPT_DIR_PSHELL="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR_PSHELL}/_common.sh"

# TTY guard (review H1): non-interactive/piped contexts (-h capture, script consumption) print no banner — clean help output
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

# libwebrtc link patch (P1-7): a toolchain fact, so it lives on the activation
# face and applies to both S (standalone) and P (submodule) builds.
# The prebuilt .a carries UNDEF GLib symbols from the xdg-portal code; lld
# tolerates link order, and the pixi GLib libs satisfy the symbols.
if [ "$(uname -s)" = "Linux" ] && [ -d "${PROJECT_ROOT}/.pixi/envs/default/lib" ]; then
  export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-fuse-ld=lld -C link-arg=-L${PROJECT_ROOT}/.pixi/envs/default/lib -C link-arg=-lgio-2.0 -C link-arg=-lgobject-2.0 -C link-arg=-lglib-2.0"
fi
