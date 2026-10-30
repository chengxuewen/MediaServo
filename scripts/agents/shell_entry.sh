#!/usr/bin/env bash
# shell_entry.sh — parent-shell command handlers (C3 hybrid).
#
# The release shell keeps help, E1-E7, pixi/brand/cache/deploy policy. These six
# commands only orchestrate parent state, so they live here once and are called
# with SHELL_ROOT from the parent. No parent literals: paths arrive via env.
set -uo pipefail

: "${SHELL_ROOT:?shell_entry: SHELL_ROOT is required}"
: "${SHELL_ENTRY:?shell_entry: SHELL_ENTRY is required}"
: "${SHELL_SUBMODULE:?shell_entry: SHELL_SUBMODULE is required}"
MEDIASERVO_ROOT="${MEDIASERVO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}"
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/shell_entry.sh"
export MEDIASERVO_AGENT_ROOT="${MEDIASERVO_AGENT_ROOT:-$SHELL_ROOT}"

CMD="${1:-}"
[ -n "$CMD" ] || { echo "usage: shell_entry.sh <agents|ci|mcp|verify|doctor|clean> ..." >&2; exit 2; }
shift

case "$CMD" in
    agents)
        exec python3 "${MEDIASERVO_ROOT}/scripts/agents/cli.py" "$@" \
            --source "${MEDIASERVO_ROOT}/.agents" \
            --target-root "${SHELL_ROOT}" \
            --targets "${SHELL_ROOT}/.agents/targets.json"
        ;;
    ci)
        RUN_ALL=false
        case "${1:-}" in
            --all-gates) RUN_ALL=true; shift ;;
            "") ;;
            *) echo "[msrtc] ci: 未知参数 $1（仅支持 --all-gates）" >&2; exit 2 ;;
        esac
        FAIL=0
        echo "[ci] 1/5 agent layers"
        bash "$SELF" agents check || FAIL=1
        echo "[ci] 2/5 python retirement"
        bash "${SHELL_ROOT}/scripts/gate-py-retirement.sh" --final || FAIL=1
        echo "[ci] 3/5 pinned cache"
        bash "${SHELL_ROOT}/scripts/verify-cache.sh" --check || FAIL=1
        echo "[ci] 4/5 shell contract"
        bash "${SHELL_ROOT}/scripts/gate-shell.sh" || FAIL=1
        echo "[ci] 5/5 memory ownership"
        python3 "${MEDIASERVO_ROOT}/scripts/agents/check_memory_ownership.py" --parent-root "${SHELL_ROOT}" || FAIL=1
        if [ "$FAIL" -ne 0 ]; then
            echo "[ci] ✗ parent gates failed" >&2
            exit 1
        fi
        if $RUN_ALL; then
            echo "[ci] delegated all-gates"
            exec "$SHELL_ENTRY" pixi bash scripts/all-gates.sh
        fi
        echo "[ci] ✓ parent gates green (pass --all-gates to run the delegated all-gates)"
        exit 0
        ;;
    mcp)
        case "${1:-status}" in
            status|setup|sync|index)
                exec python3 "${MEDIASERVO_ROOT}/scripts/agents/mcp_cli.py" "$1" ;;
            pins) exec cat "${MEDIASERVO_ROOT}/scripts/agents/mcp-servers.json" ;;
            *) echo "[msrtc] mcp: status|setup|sync|index|pins" >&2; exit 2 ;;
        esac
        ;;
    verify)
        FAIL=0
        echo "[verify] 1/5 submodule file"
        [ -f "${MEDIASERVO_ROOT}/mediaservo.sh" ] || { echo "  missing mediaservo.sh" >&2; FAIL=1; }
        echo "[verify] 2/5 submodule pointer"
        SUB_LINE="$(git -C "${SHELL_ROOT}" submodule status "$SHELL_SUBMODULE" 2>/dev/null | head -1)"
        case "$SUB_LINE" in
            ""|[-+U]*) echo "  pointer not clean: ${SUB_LINE:-not registered}" >&2; FAIL=1 ;;
            *) echo "  $SUB_LINE" ;;
        esac
        echo "[verify] 3/5 toolchain"
        if [ -x "${MEDIASERVO_ROOT}/.pixi/envs/default/bin/cargo" ]; then
            echo "  pixi cargo present"
        else
            echo "  pixi env missing (source bootstrap.sh)" >&2
            FAIL=1
        fi
        echo "[verify] 4/5 agent layers"
        bash "$SELF" agents check || FAIL=1
        echo "[verify] 5/5 pinned cache"
        bash "${SHELL_ROOT}/scripts/verify-cache.sh" --check || FAIL=1
        if [ "$FAIL" -ne 0 ]; then
            echo "[verify] ✗ smoke failed" >&2
            exit 1
        fi
        echo "[verify] ✓ smoke green"
        exit 0
        ;;
    doctor)
        echo "[doctor] workspace: ${SHELL_ROOT}"
        echo "[doctor] submodule: $(git -C "${SHELL_ROOT}" submodule status "$SHELL_SUBMODULE" 2>/dev/null | head -1)"
        echo "[doctor] submodule dirty entries: $(git -C "${MEDIASERVO_ROOT}" status --porcelain 2>/dev/null | wc -l)"
        [ -x "${MEDIASERVO_ROOT}/.pixi/envs/default/bin/cargo" ] \
            && echo "[doctor] pixi cargo: present" || echo "[doctor] pixi cargo: missing"
        ( bash "$SELF" agents status 2>&1 || true ) | sed 's/^/[doctor] /'
        if [ -d "${SHELL_ROOT}/.cache/webrtc" ]; then
            echo "[doctor] cache: $(ls -1 "${SHELL_ROOT}/.cache/webrtc" | head -3 | tr '\n' ' ')"
        else
            echo "[doctor] cache: empty"
        fi
        exit 0
        ;;
    clean)
        echo "[clean] removing ${SHELL_ROOT}/{out,.cache,build}"
        rm -rf "${SHELL_ROOT}/out" "${SHELL_ROOT}/.cache" "${SHELL_ROOT}/build"
        cd "${MEDIASERVO_ROOT}"
        exec "${MEDIASERVO_ROOT}/mediaservo.sh" clean "$@"
        ;;
    *)
        echo "[msrtc] shell_entry: unknown command $CMD" >&2
        exit 2
        ;;
esac
