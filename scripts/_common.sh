#!/usr/bin/env bash
# _common.sh — shared utility for MediaServo scripts
# Source this from other scripts: source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
set -euo pipefail

# Guard against double-sourcing
if [ -n "${_MEDIASERVO_COMMON_SH_LOADED:-}" ]; then
    return 0
fi
_MEDIASERVO_COMMON_SH_LOADED=true

# SCRIPT_DIR detects the CALLER's directory (BASH_SOURCE[1]), not this file
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}")" && pwd -P)"

# PROJECT_ROOT = directory containing pixi.toml
if [ -f "${SCRIPT_DIR}/pixi.toml" ]; then
    PROJECT_ROOT="$SCRIPT_DIR"
else
    PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
fi

# OS detection
IS_MACOS=false
IS_LINUX=false
IS_WIN=false
case "$(uname -s)" in
    Darwin)  IS_MACOS=true ;;
    Linux)   IS_LINUX=true ;;
    MINGW*|MSYS*|CYGWIN*) IS_WIN=true ;;
esac

# Architecture detection
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)  ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
esac

# pixi binary path
PIXI_BIN="${PIXI_BIN:-$HOME/.pixi/bin/pixi}"

# pixi cache directory (keeps downloaded packages between installs)
PIXI_CACHE_DIR="${PIXI_CACHE_DIR:-${PROJECT_ROOT}/.pixi-cache}"
export PIXI_CACHE_DIR

# pixi install directory
PIXI_HOME="${PIXI_HOME:-$HOME/.pixi}"
export PIXI_HOME

# --- cargo artifact-dir single source (round cargo-path-single-source / D-I2) -----
# Asks cargo metadata for .target_directory (= the location cargo itself resolves;
# env CARGO_TARGET_DIR / config build.target-dir are reflected automatically),
# replacing hardcoded target/{debug,release} across scripts. Memoized: one call per
# process. On failure falls back to ${PROJECT_ROOT}/target (= cargo default, matches
# what consumers historically assumed) + stderr WARN; `|| true` avoids tripping
# set -e/pipefail. Callers append the profile (/debug|/release).
_ms_cargo_target_dir_cache=""
ms_cargo_target_dir() {
    [ -n "$_ms_cargo_target_dir_cache" ] && { printf '%s' "$_ms_cargo_target_dir_cache"; return 0; }
    local td=""
    if command -v cargo >/dev/null 2>&1; then
        td="$(cargo metadata --format-version 1 --no-deps \
                --manifest-path "${PROJECT_ROOT}/Cargo.toml" 2>/dev/null \
                | python3 -c 'import sys,json;print(json.load(sys.stdin).get("target_directory",""))' 2>/dev/null || true)"
    fi
    [ -z "$td" ] && { echo "[ms_cargo_target_dir] WARN cargo metadata unavailable; fallback ${PROJECT_ROOT}/target" >&2; td="${PROJECT_ROOT}/target"; }
    _ms_cargo_target_dir_cache="$td"
    printf '%s' "$td"
}
