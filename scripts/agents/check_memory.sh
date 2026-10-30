#\!/usr/bin/env bash
# C5 batch-close memory disposition gate (manual; not in all-gates).
# Usage: bash scripts/agents/check_memory.sh <base>..<head>
set -uo pipefail
cd "$(dirname "$0")/../.."

RANGE="${1:-}"
if [ -z "$RANGE" ] || \! git rev-list -n1 "$RANGE" >/dev/null 2>&1; then
    echo "usage: scripts/agents/check_memory.sh <base>..HEAD" >&2
    exit 2
fi

DURABLE=(crates contracts bindings scripts book docs .agents/rules .agents/skills
         pixi.toml CMakeLists.txt cmake .github .vscode README.md)
durable=$(git diff --name-only "$RANGE" -- "${DURABLE[@]}")
if [ -z "$durable" ]; then
    echo "MEMORY ✓ no durable paths in $RANGE"
    exit 0
fi
memory=$(git diff --name-only "$RANGE" -- .agents/memorys)
if [ -n "$memory" ]; then
    echo "MEMORY ✓ memory updated ($(printf '%s\n' "$memory" | grep -c .) file(s))"
    exit 0
fi
if git log --format=%B "$RANGE" | grep -Eq '^[ -]*Memory: (parent|child|both|none\s*\(.+\))(\s*\(.+\))?$'; then
    echo "MEMORY ✓ explicit Memory declaration"
    exit 0
fi
echo "MEMORY ✗ durable paths changed but memory untouched (add an entry or '- Memory: none (<reason>)')" >&2
exit 1
