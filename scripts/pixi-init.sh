#!/usr/bin/env bash
# pixi-init.sh — MediaServo host wrapper over the PolyOrch standard bootstrap (F20).
#
# The supplier script (PolyOrch/scripts/pixi-init.sh) owns the generic stages:
# auditable pixi install (pin default 0.78.0; `PIXI_VERSION=<v>` env overrides —
# existing hosts below the pin otherwise trigger a reinstall into ~/.pixi),
# conda-forge preflight, `pixi install`, and the lock-drift recovery pattern.
# THIS file keeps only MediaServo host policy: the cold-clone submodule
# prerequisite, the China cargo mirror (rsproxy — deliberately NOT the
# supplier's job), and the ready-prompt. Same split as the VisiaEngine twin.
#
# Usage: scripts/pixi-init.sh   (bootstrap.sh calls this; safe to re-run)
set -euo pipefail

SCRIPT_DIR_PINIT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR_PINIT}/_common.sh"

SUPPLIER_INIT="${PROJECT_ROOT}/PolyOrch/scripts/pixi-init.sh"

# --- [0/3] submodule prerequisite (fresh clones have an empty PolyOrch/) ---
if [ ! -f "$SUPPLIER_INIT" ]; then
    echo "[0/3] PolyOrch submodule not populated — initializing ..."
    git -C "${PROJECT_ROOT}" submodule update --init --recursive PolyOrch \
        || { echo "ERROR: git submodule update --init --recursive PolyOrch failed (network? gitee access?)" >&2; exit 1; }
fi
[ -f "$SUPPLIER_INIT" ] || { echo "ERROR: still missing $SUPPLIER_INIT after submodule init" >&2; exit 1; }

# --- [1/3] pixi + environment via the standard bootstrap ---
echo "[1/3] pixi + environment via the PolyOrch standard bootstrap ..."
bash "$SUPPLIER_INIT" --manifest-path "${PROJECT_ROOT}/pixi.toml" || exit 1

# --- [2/3] cargo mirror (host network policy; NOT part of the supplier bootstrap) ---
# crates.io mirror for China (rsproxy.cn by ByteDance). Sparse protocol must use the
# /index/ path (the /crates.io-index/ form is the git-protocol address; 404 under sparse).
# Fallback: switch replace-with to ustc-sparse if rsproxy is down (D208).
echo "[2/3] Configuring cargo mirror ..."
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

# --- [3/3] smoke ---
echo "[3/3] Smoke ..."
PIXI_SMOKE="${PIXI_BIN:-$HOME/.pixi/bin/pixi}"
command -v pixi >/dev/null 2>&1 && PIXI_SMOKE="$(command -v pixi)"
"$PIXI_SMOKE" --version | head -n1
echo ""
echo "=== MediaServo pixi environment ready ==="
echo "Activate with:  source pixi.sh"
echo "Assembly surface (IDE / cmake):  cmake --preset default|bindings|examples|crates|crates-shared"
echo "CLI:            ./mediaservo.sh -h"
