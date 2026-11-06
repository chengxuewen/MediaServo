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

usage() {
    cat <<'EOF'
usage: shell_entry.sh <command> ...
  agents sync|check|status
  ci [--all-gates [--full]]
  mcp status|setup|sync|index|pins
  verify
  doctor
  clean [--logs] [--dry-run]
  disk-usage
EOF
}

CMD="${1:-}"
case "$CMD" in
    "") usage >&2; exit 2 ;;
    -h|--help) usage; exit 0 ;;
esac
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
        FULL=0
        case "${1:-}" in
            -h|--help) usage; exit 0 ;;
            "") ;;
            --all-gates)
                RUN_ALL=true; shift
                case "${1:-}" in
                    "") ;;
                    --full) FULL=1; shift ;;
                    *) echo "[msrtc] ci: unknown argument $1 (after --all-gates only --full is supported)" >&2; exit 2 ;;
                esac
                [ "$#" -eq 0 ] || { echo "[msrtc] ci: unexpected extra arguments: $*" >&2; exit 2; }
                ;;
            --full) echo "[msrtc] ci: --full requires --all-gates" >&2; exit 2 ;;
            *) echo "[msrtc] ci: unknown argument $1 (only --all-gates [--full])" >&2; exit 2 ;;
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
            if [ "$FULL" -eq 1 ]; then
                echo "[ci] delegated all-gates (full)"
                exec "$SHELL_ENTRY" pixi bash scripts/all-gates.sh --full
            fi
            echo "[ci] delegated all-gates"
            exec "$SHELL_ENTRY" pixi bash scripts/all-gates.sh
        fi
        echo "[ci] ✓ parent gates green (pass --all-gates to run the delegated all-gates)"
        exit 0
        ;;
    mcp)
        MCP_CMD="${1:-status}"
        shift || true
        case "$MCP_CMD" in
            -h|--help) usage; exit 0 ;;
            status|setup|sync|index)
                [ "$#" -eq 0 ] || { echo "[msrtc] mcp: unexpected extra arguments: $*" >&2; exit 2; }
                exec python3 "${MEDIASERVO_ROOT}/scripts/agents/mcp_cli.py" "$MCP_CMD" ;;
            pins)
                [ "$#" -eq 0 ] || { echo "[msrtc] mcp: unexpected extra arguments: $*" >&2; exit 2; }
                exec cat "${MEDIASERVO_ROOT}/scripts/agents/mcp-servers.json" ;;
            *) echo "[msrtc] mcp: status|setup|sync|index|pins" >&2; exit 2 ;;
        esac
        ;;
    verify)
        case "${1:-}" in
            -h|--help) usage; exit 0 ;;
            "") ;;
            *) echo "[msrtc] verify: unknown argument $1" >&2; exit 2 ;;
        esac
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
        case "${1:-}" in
            -h|--help) usage; exit 0 ;;
        esac
        echo "[doctor] workspace: ${SHELL_ROOT}"
        echo "[doctor] submodule: $(git -C "${SHELL_ROOT}" submodule status "$SHELL_SUBMODULE" 2>/dev/null | head -1)"
        echo "[doctor] submodule dirty entries: $(git -C "${MEDIASERVO_ROOT}" status --porcelain 2>/dev/null | wc -l)"
        [ -x "${MEDIASERVO_ROOT}/.pixi/envs/default/bin/cargo" ] \
            && echo "[doctor] pixi cargo: present" || echo "[doctor] pixi cargo: missing"
        # [toolchain] section (B7/M3, adjudication O-2=D): runs the submodule's
        # four-probe doctor (pixi/cargo/docker/node) — same orchestration shape
        # as the `agents status` call below. Section-level verdict: any MISSING
        # line marks the section "!" and prints the prescription row; otherwise
        # "✓". Doctor stays report-only (exit 0 contract untouched).
        tc_out="$( bash "${MEDIASERVO_ROOT}/mediaservo.sh" doctor 2>&1 || true )"
        printf '%s\n' "$tc_out" | sed 's/^/[doctor] [toolchain] /'
        # "!" = either a probe answered MISSING, or the submodule doctor itself
        # failed to run (its output would lack the first probe's line entirely —
        # a silent green here would be worse than no section at all).
        if printf '%s\n' "$tc_out" | grep -q MISSING || ! printf '%s\n' "$tc_out" | grep -q '^pixi'; then
            echo "[doctor] [toolchain] ! missing entries above (or the submodule doctor failed to run — check MEDIASERVO_ROOT points at the submodule) — msrtc.sh build legs auto-activate the pixi shell (run pixi install if the env is absent); docker/node are only needed for the container/web faces"
        else
            echo "[doctor] [toolchain] ✓ all four probes answer"
        fi
        ( bash "$SELF" agents status 2>&1 || true ) | sed 's/^/[doctor] /'
        if [ -d "${SHELL_ROOT}/.cache/webrtc" ]; then
            echo "[doctor] cache: $(ls -1 "${SHELL_ROOT}/.cache/webrtc" | head -3 | tr '\n' ' ')"
        else
            echo "[doctor] cache: empty"
        fi
        exit 0
        ;;
    clean)
        DRY=0
        LOGS=0
        for arg in "$@"; do
            case "$arg" in
                -h|--help) usage; exit 0 ;;
                --dry-run) DRY=1 ;;
                --logs) LOGS=1 ;;
                *) echo "[msrtc] clean: unknown argument $arg (only --logs/--dry-run)" >&2; exit 2 ;;
            esac
        done
        if [ "$LOGS" -eq 1 ]; then
            # Clean only the delivery-tree runtime logs/recordings (run/logs, recordings);
            # keep the skeleton and state files (oxfile.toml/config-state.json/run/oxmgr/
            # etc/identity.json). Guard: absolute root + non-empty + exists (audit T3).
            OUT="${SHELL_ROOT}/out"
            case "$OUT" in
                /*) ;;
                *) echo "[clean --logs] refuse: out root not absolute (${OUT})" >&2; exit 2 ;;
            esac
            if [ ! -d "$OUT" ]; then
                echo "[clean --logs] no out dir, skip (${OUT})"
                exit 0
            fi
            removed=0
            for d in "$OUT"/*/run/logs "$OUT"/*/recordings; do
                [ -d "$d" ] || continue
                if [ "$DRY" -eq 1 ]; then
                    echo "[clean --logs] dry-run: would empty $d/*"
                    continue
                fi
                while IFS= read -r -d '' c; do
                    rm -rf -- "$c" && removed=$((removed+1))
                done < <(find "$d" -mindepth 1 -maxdepth 1 -print0 2>/dev/null)
                echo "[clean --logs] emptied $d"
            done
            [ "$DRY" -eq 1 ] || echo "[clean --logs] processed $removed entries (kept skeleton/state files)"
            exit 0
        fi
        if [ "$DRY" -eq 1 ]; then
            echo "[clean] dry-run: would remove ${SHELL_ROOT}/{out,.cache,build}"
            exit 0
        fi
        echo "[clean] removing ${SHELL_ROOT}/{out,.cache,build}"
        rm -rf "${SHELL_ROOT}/out" "${SHELL_ROOT}/.cache" "${SHELL_ROOT}/build"
        cd "${MEDIASERVO_ROOT}"
        exec "${MEDIASERVO_ROOT}/mediaservo.sh" clean
        ;;
    disk-usage)
        case "${1:-}" in
            -h|--help) usage; exit 0 ;;
            "") ;;
            *) echo "[msrtc] disk-usage: no arguments (read-only inventory)" >&2; exit 2 ;;
        esac
        _durow() {
            if [ -e "$2" ]; then
                printf '  [%-6s] %8s  %s\n' "$1" "$(du -sh -- "$2" 2>/dev/null | cut -f1)" "$2"
            else
                printf '  [%-6s] %8s  %s (missing)\n' "$1" "-" "$2"
            fi
        }
        echo "[disk-usage] build-artifact disk inventory -- parent=${SHELL_ROOT} sub=${MEDIASERVO_ROOT}"
        [ -n "${MSRTC_BUILD_ROOT:-}" ] && echo "[disk-usage] MSRTC_BUILD_ROOT=${MSRTC_BUILD_ROOT}"
        [ -n "${MSRTC_OUT_ROOT:-}" ] && echo "[disk-usage] MSRTC_OUT_ROOT=${MSRTC_OUT_ROOT}"
        echo "-- parent (ms_rtc) --"
        _durow parent "${SHELL_ROOT}/build"; _durow parent "${SHELL_ROOT}/out"; _durow parent "${SHELL_ROOT}/.cache"
        echo "-- delivery runtime hotspots (clean --logs targets) --"
        _durow parent "${SHELL_ROOT}/out/host/run/logs"; _durow parent "${SHELL_ROOT}/out/host/recordings"
        echo "-- submodule (MediaServo) --"
        _durow sub "${MEDIASERVO_ROOT}/target"; _durow sub "${MEDIASERVO_ROOT}/target/debug"; _durow sub "${MEDIASERVO_ROOT}/target/release"
        _durow sub "${MEDIASERVO_ROOT}/build"; _durow sub "${MEDIASERVO_ROOT}/out"; _durow sub "${MEDIASERVO_ROOT}/www"; _durow sub "${MEDIASERVO_ROOT}/.pixi"
        echo "-- pixi download cache (6-prime: new home vs in-tree leftover) --"
        _durow shared "${PIXI_CACHE_DIR:-${HOME:-}/.cache/pixi-mediaservo}"; _durow sub "${MEDIASERVO_ROOT}/.pixi-cache"
        _total=$(du -shc "${SHELL_ROOT}/build" "${SHELL_ROOT}/out" "${SHELL_ROOT}/.cache" "${MEDIASERVO_ROOT}/target" "${MEDIASERVO_ROOT}/build" "${MEDIASERVO_ROOT}/out" "${MEDIASERVO_ROOT}/www" "${MEDIASERVO_ROOT}/.pixi" "${MEDIASERVO_ROOT}/.pixi-cache" 2>/dev/null | tail -1 | cut -f1)
        echo "[disk-usage] total (dedup top-level: parent build+out+.cache and sub target+build+out+www+.pixi+.pixi-cache; excl pixi new home) ~ ${_total:-?}"
        echo "[disk-usage] note: pixi cache goes to \$HOME/.cache/pixi-mediaservo after 6-prime; in-tree .pixi-cache is a pre-switch leftover, reclaimable"
        exit 0
        ;;
    *)
        echo "[msrtc] shell_entry: unknown command $CMD" >&2
        exit 2
        ;;
esac
