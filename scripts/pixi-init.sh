#!/usr/bin/env bash
# pixi-init.sh — Bootstrap MediaServo pixi development environment
# Installs pixi (via pixi-install.sh) and project dependencies
# Usage: scripts/pixi-init.sh
set -euo pipefail

SCRIPT_DIR_PINIT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR_PINIT}/_common.sh"

echo "=== MediaServo pixi environment setup ==="
echo "Project root: ${PROJECT_ROOT}"

# --- Step 1: Install pixi binary ---
echo "[1/3] Installing pixi..."
bash "${SCRIPT_DIR_PINIT}/pixi-install.sh"

# --- Step 2: Configure cargo mirror (China) ---
echo ""
echo "[2/3] Configuring cargo mirror..."
CARGO_CONFIG="${HOME}/.cargo/config.toml"
mkdir -p "$(dirname "${CARGO_CONFIG}")"
if grep -q "rsproxy" "${CARGO_CONFIG}" 2>/dev/null; then
    echo "Cargo mirror already configured (rsproxy.cn)"
else
    cat >> "${CARGO_CONFIG}" << 'CARGO_EOF'

# MediaServo: crates.io mirror for China (rsproxy.cn by ByteDance)
# D208: the sparse protocol requires the /index/ path (/crates.io-index/ is the git-protocol address, 404 under sparse)
[source.crates-io]
replace-with = 'rsproxy-sparse'

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

# Fallback: if rsproxy is down, switch replace-with to ustc-sparse manually
# [source.ustc-sparse]
# registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"

[registries.rsproxy]
index = "https://rsproxy.cn/crates.io-index"

[net]
git-fetch-with-cli = true
CARGO_EOF
    echo "Cargo mirror configured: rsproxy.cn"
fi

# --- Step 3: Install project dependencies ---
echo ""
echo "[3/3] Installing project dependencies..."
cd "${PROJECT_ROOT}"

# ponytail: single install call, retry on failure is enough
if ! "${PIXI_BIN}" install --manifest-path "${PROJECT_ROOT}/pixi.toml"; then
    echo "pixi install failed. Regenerating lock file and retrying..."
    "${PIXI_BIN}" update --manifest-path "${PROJECT_ROOT}/pixi.toml"
    "${PIXI_BIN}" install --manifest-path "${PROJECT_ROOT}/pixi.toml"
fi

echo ""
echo "=== MediaServo pixi environment ready ==="
echo "Activate with:  source pixi.sh"
echo "Or run tasks:   pixi run build | pixi run test | pixi run lint"
