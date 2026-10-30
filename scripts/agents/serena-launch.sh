#!/usr/bin/env bash
# serena-launch.sh — resolve the project root without relying on pixi's shell
# parameter expansion (pixi tasks do not expand ${VAR:-default}).
set -u
ROOT="${MEDIASERVO_AGENT_ROOT:-${PIXI_PROJECT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}}"
exec serena start-mcp-server --transport stdio --context claude-code --language-backend LSP --project "$ROOT"
