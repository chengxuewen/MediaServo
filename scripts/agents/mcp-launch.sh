#!/usr/bin/env bash
# mcp-launch.sh — host-neutral MCP launcher (agent toolchain home: scripts/agents).
#
# Resolves node without relying on the GUI host PATH (pixi env first, then PATH),
# then delegates to mcp-serve.mjs. PROJECT_ROOT travels as an env var
# (MEDIASERVO_AGENT_PROJECT_ROOT) so the parent shell can point the server at the
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

exec "$NODE" "$DIR/mcp-serve.mjs" --from "$DIR/mcp-servers.json" --name "$NAME"
