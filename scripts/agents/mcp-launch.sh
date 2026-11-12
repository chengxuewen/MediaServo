#!/usr/bin/env bash
# mcp-launch.sh — host-neutral MCP launcher (agent toolchain home: scripts/agents).
#
# Resolves node without relying on the GUI host PATH (pixi env first, then PATH),
# then delegates to mcp-serve.mjs. The workspace root travels as
# MEDIASERVO_AGENT_ROOT (older launchers may still send
# MEDIASERVO_AGENT_PROJECT_ROOT) so a parent shell can point the server at the
# workspace root instead of the submodule root.
set -u

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd "$DIR/../.." && pwd -P)"
NAME="${1:-}"
[ -n "$NAME" ] || { echo "usage: mcp-launch.sh <server-name>" >&2; exit 2; }

NODE="${NODE_BIN:-$ROOT/.pixi/envs/default/bin/node}"
if [ ! -x "$NODE" ]; then
    NODE="$(command -v node 2>/dev/null || true)"
fi
[ -n "$NODE" ] || { echo "mcp-launch: node not found (run bootstrap)" >&2; exit 127; }
# The npm server bins (codegraph, playwright-mcp) and npm itself use
# `#!/usr/bin/env node` shebangs, and mcp-serve.mjs spawns them with the
# inherited env: resolving NODE for this script alone is not enough — the
# GUI host PATH has no node, so export the resolved node's dir for every
# descendant (otherwise: "/usr/bin/env: 'node': No such file or directory").
export PATH="$(dirname "$NODE"):$PATH"

PIXI_BIN="${PIXI_BIN:-$(command -v pixi 2>/dev/null || true)}"
if [ -z "$PIXI_BIN" ] && [ -x "$HOME/.pixi/bin/pixi" ]; then
    PIXI_BIN="$HOME/.pixi/bin/pixi"
fi
export PIXI_BIN

exec "$NODE" "$DIR/mcp-serve.mjs" --from "$DIR/mcp-servers.json" --name "$NAME"
