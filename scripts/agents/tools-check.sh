#!/usr/bin/env bash
# tools-check.sh — agent toolchain watchdog (ruff + unit tests).
# Run through `pixi run agent-tools` so ruff resolves from the env.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

ruff check scripts/agents
python3 -m unittest discover -s scripts/agents -p 'test_*.py'
