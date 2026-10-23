#!/usr/bin/env bash
# mediaservo.sh — MediaServo CLI thin shell (Linux/macOS)
# Duties: ① detect pixi (prompt bootstrap if missing) ② activate the environment ③ forward to the CLI
# Usage: ./mediaservo.sh <cmd> [-h]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

# v2: unified pixi detection — prefer command -v, fall back to ~/.pixi/bin (exports PIXI_BIN for pixi-shell)
if command -v pixi >/dev/null 2>&1; then
    export PIXI_BIN="$(command -v pixi)"
elif [ -x "$HOME/.pixi/bin/pixi" ]; then
    export PIXI_BIN="$HOME/.pixi/bin/pixi"
else
    echo "pixi not installed — run first: source bootstrap.sh" >&2
    exit 1
fi

# -h/--help/version: skip pixi activation (review H1: the activation banner pollutes -h output — 8-line banner before usage)
case "${1:-}" in
    -h|--help)
        exec python3 "$ROOT/scripts/mediaservo_cli.py" "$@"   # bare python3 (stdlib-only at the top — no activation noise)
        ;;
    version)
        # SR6 batch-1 (bash-owned): was a python float literal (0.1.0 — C43 drift
        # caught at retirement). Single source = root Cargo.toml [workspace.package].
        sed -n '/\[workspace.package\]/,/^\[/p' "$ROOT/Cargo.toml" | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -1
        exit 0
        ;;
esac
source "$ROOT/scripts/pixi-shell.sh"   # activate (same process, PATH/LIBCLANG injection)

# =============================================================================
# SR6 batch-1 -- compose face + dev, bash-owned (moved out of mediaservo_cli.py
# one-leg-one-owner style; flags/output mirror the retired python exactly).
# Python keeps its read-probe compose uses (status/logs/clean) until those legs'
# own batches retire them. MEDIASERVO_DRY_RUN=1 echoes the final command instead
# of executing (verb-matrix hook; production default off).
# =============================================================================
_msvc_run() { echo "$ $*"; if [ "${MEDIASERVO_DRY_RUN:-0}" = 1 ]; then return 0; fi; "$@"; }
_msvc_compose_file() { if [ "$1" = prod ]; then echo docker-compose.yml; else echo docker-compose.dev.yml; fi; }
_msvc_announced_ips() {
    local out
    out=$(ip -o -4 addr show 2>/dev/null | awk '{print $2, $4}' || true)
    if [ -n "$out" ]; then
        while read -r iface ip; do
            case "$iface" in lo|docker*|br-*|veth*|virbr*) continue ;; esac
            ip=${ip%%/*}
            case "$ip" in 127.*) continue ;; esac
            echo "$ip"
        done <<< "$out"
    else
        hostname -I 2>/dev/null | tr ' ' '\n' | sed '/^$/d'   # coarse fallback (python parity)
    fi | awk '!seen[$0]++'
}
_msvc_compose_env() {  # PIT-79: guarantee MEDIASERVO_SFU_ANNOUNCED_IP has a value
    local announced="${1:-}" ips
    if [ -n "$announced" ]; then
        export MEDIASERVO_SFU_ANNOUNCED_IP="$announced"
    elif [ -z "${MEDIASERVO_SFU_ANNOUNCED_IP:-}" ]; then
        ips=$(_msvc_announced_ips | paste -sd, -)
        if [ -n "$ips" ]; then
            export MEDIASERVO_SFU_ANNOUNCED_IP="$ips"
            echo "MEDIASERVO_SFU_ANNOUNCED_IP auto-detected: $ips"
        fi
    fi
}
_msvc_detect_env() {  # dev|prod via the running container's compose config_files label
    local cfg
    if docker ps --format '{{.Names}}' 2>/dev/null | grep -qx mediaservo-server-1; then
        cfg=$(docker inspect mediaservo-server-1 --format '{{index .Config.Labels "com.docker.compose.project.config_files"}}' 2>/dev/null || true)
        case "$cfg" in
            *docker-compose.yml*) case "$cfg" in *dev*) ;; *) echo prod; return ;; esac ;;
        esac
    fi
    echo dev
}
case "${1:-}" in
    up)
        shift
        svc=server; env=dev; build=0; announced=""; pos=0
        while [ $# -gt 0 ]; do
            case "$1" in
                --env) env="${2:?--env needs a value}"; shift 2 ;;
                --build) build=1; shift ;;
                --announced-ip) announced="${2:?--announced-ip needs a value}"; shift 2 ;;
                -*) echo "up: unknown flag $1" >&2; exit 2 ;;
                *) pos=$((pos+1)); [ $pos -eq 1 ] && svc="$1" || { echo "up: unexpected extra arg: $1" >&2; exit 2; }; shift ;;
            esac
        done
        case "$env" in dev|prod) ;; *) echo "up: --env must be dev|prod (got $env)" >&2; exit 2 ;; esac
        _msvc_compose_env "$announced"
        cmd=(docker compose -f "$(_msvc_compose_file "$env")" up)
        [ "$build" = 1 ] && cmd+=(--build)
        cmd+=(-d "$svc")
        _msvc_run "${cmd[@]}"
        exit $?
        ;;
    down)
        shift
        env=""
        while [ $# -gt 0 ]; do
            case "$1" in
                --env) env="${2:?--env needs a value}"; shift 2 ;;
                *) echo "down: unexpected arg $1" >&2; exit 2 ;;
            esac
        done
        if [ -z "$env" ]; then env=$(_msvc_detect_env); echo "down: detected environment $env"; fi
        case "$env" in dev|prod) ;; *) echo "down: --env must be dev|prod" >&2; exit 2 ;; esac
        _msvc_run docker compose -f "$(_msvc_compose_file "$env")" down
        exit $?
        ;;
    ps)
        shift
        env=dev
        while [ $# -gt 0 ]; do
            case "$1" in
                --env) env="${2:?--env needs a value}"; shift 2 ;;
                *) echo "ps: unexpected arg $1" >&2; exit 2 ;;
            esac
        done
        _msvc_run docker compose -f "$(_msvc_compose_file "$env")" ps || true   # python parity: probe never fails the shell
        exit 0
        ;;
    exec)
        shift
        [ $# -ge 1 ] || { echo "Usage: mediaservo exec <svc> -- <cmd>" >&2; exit 2; }
        svc="$1"; shift
        [ "${1:-}" = "--" ] && shift
        [ $# -ge 1 ] || { echo "Usage: mediaservo exec <svc> -- <cmd>" >&2; exit 2; }
        _msvc_run docker compose -f docker-compose.dev.yml exec -T "$svc" "$@"
        exit $?
        ;;
    data)
        shift
        sub="${1:-}"; [ $# -gt 0 ] && shift
        case "$sub" in
            inspect) _msvc_run docker volume ls --format '{{.Name}} {{.Mountpoint}}' --filter name=mediaservo_ ;;
            backup)
                target="${1:-backup}"
                mkdir -p "$target"; abs=$(cd "$target" && pwd)
                for vol in mediaservo_mediaservo-data mediaservo_mediaservo-recordings; do
                    _msvc_run docker run --rm -v "${vol}:/data:ro" -v "${abs}:/backup" alpine sh -c "tar czf /backup/${vol}.tar.gz -C /data ."
                    echo "backed up ${vol} → ${abs}/${vol}.tar.gz"
                done
                ;;
            reset)
                [ "${1:-}" = "--force" ] || { read -r -p "reset mediaservo_mediaservo-data + mediaservo_mediaservo-recordings (delete all data)? [y/N] " ans
                    [ "$ans" = y ] || [ "$ans" = Y ] || { echo cancelled; exit 0; }; }
                _msvc_run docker compose -f docker-compose.yml down
                _msvc_run docker volume rm mediaservo_mediaservo-data mediaservo_mediaservo-recordings
                echo "volumes deleted — \`mediaservo up --env prod\` re-initializes (keys regenerated)"
                ;;
            *) echo "data: expected backup|reset|inspect (got '${sub:-}')" >&2; exit 2 ;;
        esac
        exit $?
        ;;
    dev)
        shift
        [ "${1:-}" = web ] || { echo "dev: only target is web (got '${1:-}')" >&2; exit 2; }
        shift
        [ "${1:-}" = "--" ] && shift
        command -v pnpm >/dev/null 2>&1 || { echo "Error: missing dependency 'pnpm' — dev web needs it (www is a pnpm workspace, or run manually: cd www && pnpm dev)" >&2; exit 1; }
        cd "$ROOT/www" && _msvc_run pnpm dev "$@"
        exit $?
        ;;
esac

# oxmgr pinned injection (plan Y, aligned with the parent-repo msrtc.sh — the archive layer of a non-mirrorable download site):
# 3rdparty/oxmgr/ stores only the upstream tar.gz + .sha256 (the file name is the coordinate); unpacked artifacts go to
# .cache/oxmgr/<archive name>/ (gitignored, regenerable, versioned directory = bump invalidates naturally).
# sha256 is verified before unpacking; mismatch refuses. Unknown architecture = no injection + WARN
# (PATH self-install as fallback — the deploy side has the _resolve_oxmgr_or_die hard gate).
OXMGR_TRIPLE=""
case "$(uname -m)" in
    x86_64)  OXMGR_TRIPLE="x86_64-unknown-linux-musl" ;;
    aarch64) OXMGR_TRIPLE="aarch64-unknown-linux-musl" ;;
esac
if [ -n "$OXMGR_TRIPLE" ]; then
    OXMGR_PKG="oxmgr-v0.5.0-${OXMGR_TRIPLE}"   # version single source = 3rdparty/oxmgr/ archive file name; edit this line on bumps
    OXMGR_TARBALL="${ROOT}/3rdparty/oxmgr/${OXMGR_PKG}.tar.gz"
    OXMGR_CACHE="${ROOT}/.cache/oxmgr/${OXMGR_PKG}"
    if [ ! -f "$OXMGR_TARBALL" ]; then
        echo "[mediaservo] WARN: oxmgr archive missing (${OXMGR_TARBALL}) — relying on PATH self-install" >&2
    elif [ -f "${OXMGR_TARBALL}.sha256" ] && ! (cd "${ROOT}/3rdparty/oxmgr" && sha256sum -c "${OXMGR_PKG}.tar.gz.sha256" >/dev/null 2>&1); then
        echo "[mediaservo] ERROR: oxmgr archive sha256 verification failed — refusing to unpack (re-pin the archive layer)" >&2
    elif [ ! -x "${OXMGR_CACHE}/oxmgr" ]; then
        mkdir -p "$OXMGR_CACHE" && tar xzf "$OXMGR_TARBALL" -C "$OXMGR_CACHE" 2>/dev/null \
            && chmod +x "${OXMGR_CACHE}/oxmgr"
    fi
    [ -x "${OXMGR_CACHE:-/nonexistent}/oxmgr" ] && export PATH="${OXMGR_CACHE}:${PATH}"
else
    echo "[mediaservo] WARN: uname -m=$(uname -m) has no pinned oxmgr (upstream ships no binaries for this platform) — install it into PATH yourself" >&2
fi

# =============================================================================
# SR6 batch-alpha (P2) -- ops read-probe/guidance family, bash-owned (same
# one-leg-one-owner route as batch-1; flags and output shapes mirror the
# retired python exactly; MEDIASERVO_DRY_RUN honored on command legs).
# =============================================================================
# runtime guidance (was _host_runtime_hint/_server_runtime_hint — the wrappers
# retired at D266/T21; the arms only print the instance-command shape)
_msvc_hint_host() {
    echo "$1 host: CLI wrapper removed — run manually:" >&2
    echo "  deployed instance: systemctl --user start|stop oxmgr-<brand>-host.service" >&2
    echo "            or <prefix>/bin/msrtc-host start|stop <prefix>" >&2
    echo "  development: target/debug/mediaservo-host init . && token issue --all . && start|stop ." >&2
    echo "            clear iceoryx2 residue before running: rm -rf /tmp/iceoryx2 /dev/shm/iox2_* (C25)" >&2
}
_msvc_hint_server() {
    local sb="${MEDIASERVO_BRAND:+${MEDIASERVO_BRAND}-server}"; sb="${sb:-mediaservo-server}"
    echo "$1 server: CLI wrapper removed — run via the instance directory:" >&2
    echo "  deployed instance: <prefix>/bin/$sb start|stop|restart|status <prefix> (not deployed yet: mediaservo deploy server --prefix <X>; bare-metal entry <prefix>/$sb)" >&2
    echo "  development: target/debug/mediaservo-server init . && start . --no-web (frontend dev: dev web = vite:5173 pairs with --no-web; the browser entry itself is cluster-managed)" >&2
    echo "  containers (modes 2/3): mediaservo up / down --env dev|prod unchanged" >&2
    echo "  read-only probes kept: mediaservo status server | logs server" >&2
}

# =============================================================================
# SR6 batch-beta-1 -- deploy host/server shell legs. Helpers were inline in the
# bindings arm (SR6 P2); hoisted to shared scope so every deploy leg rides the
# same exact-path occupancy predicate (PIT-166). Doctrine: the driver only
# orchestrates (guards/brand/stop-first release/cmake invocation/oxmgr vendor/
# managed-binary init/env.sh) — stateful oxfile/Caddyfile merge lives in the
# binary's init [bF3], delivery layout lives in MediaServoAppTree.cmake rows.
# =============================================================================
_ms_pids_using() {
    # exact-path occupancy predicate (was python _pids_using: fd symlinks + exe
    # mmap, no basename/cmdline fuzzy match — PIT-166 multi-instance rule)
    local t p f r
    t=$(realpath -- "$1" 2>/dev/null) || return 0
    if [ -n "$t" ]; then
        for p in /proc/[0-9]*; do
            if [ -d "$p/fd" ]; then
                for f in "$p"/fd/*; do
                    r=$(readlink -- "$f" 2>/dev/null) || r=""
                    if [ "$r" = "$t" ]; then basename "$p"; break; fi
                done
            fi
            r=$(readlink -- "$p/exe" 2>/dev/null) || r=""
            if [ "$r" = "$t" ]; then basename "$p"; fi
        done | sort -u
    fi
    return 0
}
_msvc_kill_using() {
    local pid
    for pid in $(_ms_pids_using "$1"); do
        if kill "$pid" 2>/dev/null; then echo "  killed occupying process $pid"; fi
    done
    sleep 1
    return 0
}
_msvc_copy_with_kill() {
    # busy-retry copy (python _copy_with_kill parity: cp -p keeps mtime/mode;
    # Text file busy -> kill occupier by exact path -> retry; max 3 attempts)
    local src=$1 dst=$2 attempt err
    for attempt in 1 2 3; do
        if err=$(cp -p "$src" "$dst" 2>&1); then return 0; fi
        case "$err" in
            *[Bb]usy*|*Text\ file*) ;;
            *) echo "Error: copy $src -> $dst failed: $err" >&2; return 1 ;;
        esac
        [ "$attempt" = 3 ] && break
        echo "  $(basename "$dst") occupied (Text file busy) — killing the occupier then retrying (${attempt}/3)"
        _msvc_kill_using "$dst"
    done
    echo "Error: failed to copy $dst (still busy after multiple retries)" >&2
    return 1
}
_msvc_oxmgr_resolve() {
    # vendor hunt (D-H13 hard gate, python _resolve_oxmgr_or_die parity):
    # PATH -> ~/.cache/oxmgr reclaim -> fail. Sets MS_OXMGR_SRC. The archive
    # layer/.cache PATH injection above this block is what normally hits PATH.
    local role=${1:-} c
    MS_OXMGR_SRC=$(command -v oxmgr || true)
    if [ -z "$MS_OXMGR_SRC" ]; then
        c=$(ls -1d "$HOME"/.cache/oxmgr/oxmgr-v*/oxmgr 2>/dev/null | sort -r | head -1) || c=""
        if [ -n "$c" ]; then
            echo "  oxmgr: not found in PATH, reclaiming local cache $c"
            MS_OXMGR_SRC=$c
        fi
    fi
    if [ -z "$MS_OXMGR_SRC" ]; then
        echo "Error: oxmgr found in neither PATH nor ~/.cache/oxmgr — deploy ${role} must package the process manager (a hard runtime dependency of start/stop). Install: download the prebuilt Rust binary from GitHub Releases (with sha256/asc verification, https://github.com/Vladimir-Urik/OxMgr/releases),or build oxmgr-src and place it in ~/.local/bin, then re-run deploy" >&2
        exit 2
    fi
}
_ms_derive_brand_server() {
    # first sorted *-server hit minus the upstream name (python _derive_brand_server parity)
    local d=$1 f
    for f in $(ls -1 "$d" 2>/dev/null | sort); do
        case "$f" in
            mediaservo-server) continue ;;
            *-server) echo "${f%"-server"}"; return 0 ;;
        esac
    done
    return 0
}
_msvc_app_install() {
    # app delivery layout via cmake/MediaServoAppTree.cmake install rules (python
    # _cmake_app_install parity). Configure RE-RUNS every invocation — the source-
    # existence hunts are configure-time truth, never frozen by a stale cache
    # (PIT-189). Dedicated binaryDir: app cache vars must not pollute the preset
    # default tree (fresh-clone IDE regression guard). Deploy landing mode = empty
    # MEDIASERVO_APP_STAGING. ETXTBSY survivors die BY EXACT PATH then the
    # idempotent install re-runs (rerun-safe by cmake semantics, SR4 L5 precedent).
    local tree=$1 brand=$2 btype=$3 prefix=$4
    local app_tree="$ROOT/build/app-tree"
    local cfg=(cmake --preset default -B "$app_tree" "-DMEDIASERVO_APP_TREE=$tree" "-DMEDIASERVO_BRAND=$brand" "-DMEDIASERVO_APP_STAGING=" "-DCMAKE_BUILD_TYPE=$btype")
    [ "$tree" = server ] && cfg+=("-DMEDIASERVO_SERVER_OUT_DIR=${MSRTC_OUT_ROOT:-$ROOT/out}/server")
    echo "\$ ${cfg[*]}  (cwd: $ROOT)"
    (cd "$ROOT" && "${cfg[@]}") || exit $?
    local attempt=1 blob rc f
    while :; do
        blob=$( (cd "$ROOT" && cmake --install "$app_tree" --prefix "$prefix") 2>&1 ) && return 0
        rc=$?
        if { printf '%s' "$blob" | grep -qi busy || printf '%s' "$blob" | grep -q 'Text file'; } && [ "$attempt" -lt 3 ]; then
            echo "  install hit busy artifacts — killing occupiers, retrying (${attempt}/3)" >&2
            for f in "$prefix"/bin/*; do [ -f "$f" ] && _msvc_kill_using "$f"; done
            attempt=$((attempt + 1))
        else
            echo "Error: cmake --install $tree app tree into $prefix failed (rc=$rc):" >&2
            printf '%s' "$blob" | tail -c 600 >&2
            exit 1
        fi
    done
}

case "${1:-}" in
    doctor)
        # toolchain version probe (was _cmd_env_diagnose: which + --version, first line)
        for t in pixi cargo docker node; do
            if [ "$t" = pixi ]; then p="${PIXI_BIN:-pixi}"; else p="$t"; fi
            if out=$("$p" --version 2>&1); then
                printf '%-8s %s\n' "$t" "${out%%$'\n'*}"
            else
                printf '%-8s %s\n' "$t" "MISSING (or timeout)"
            fi
        done
        exit 0
        ;;
    list)
        # example roster (was _cmd_list_example; same literal rule as the aggregate
        # root's file(GLOB */CMakeLists.txt) — dir == target == artifact name, G4)
        [ "${2:-}" = example ] || { echo "list: usage: list example" >&2; exit 2; }
        ed="$ROOT/bindings/cxx/examples"
        names=()
        for cml in "$ed"/*/CMakeLists.txt; do
            [ -f "$cml" ] || continue
            names+=("$(basename "$(dirname "$cml")")")
        done
        if [ ${#names[@]} -eq 0 ]; then
            echo "(no examples in bindings/cxx/examples yet)" >&2
            exit 0
        fi
        echo "Available examples (dir = target = artifact name; [lib] = non-executable):"
        for name in "${names[@]}"; do
            if grep -q "add_library(" "$ed/$name/CMakeLists.txt"; then tag="[lib]"
            elif [ -e "$ROOT/target/examples/bin/$name" ]; then tag="[built]"
            else tag="[not built]"; fi
            printf '  %-24s %s\n' "$name" "$tag"
        done
        exit 0
        ;;
    start|stop|restart)
        act="${1}"
        [ $# -le 2 ] || { echo "$act: unrecognized arguments: ${*:3}" >&2; exit 2; }
        if [ "$act" = start ] && [ $# -eq 1 ]; then tgt=server; else tgt="${2:-}"; fi
        case "$tgt" in
            server) _msvc_hint_server "$act" ;;
            host) _msvc_hint_host "$act" ;;
            *) if [ -z "$tgt" ] && [ "$act" != start ]; then echo "usage: $act {host,server}" >&2; else echo "$act: invalid target '${tgt:-}' (choose from host, server)" >&2; fi
               exit 2 ;;
        esac
        exit 2
        ;;
    run)
        # run example stays python (build/run/test example surface = batch-beta);
        # the server/host guidance arms are bash-owned now.
        if [ "${2:-}" = example ]; then
            exec python "$ROOT/scripts/mediaservo_cli.py" "$@"
        fi
        case "${2:-}" in
            server) _msvc_hint_server run ;;
            host) _msvc_hint_host run ;;
            *) echo "run: invalid target '${2:-}' (choose from server, host, example)" >&2; exit 2 ;;
        esac
        exit 2
        ;;
    logs)
        # was _cmd_logs — flags mirror argparse exactly (default target server,
        # --mode/--native/--env mutually exclusive, rc=2 on bad args)
        shift; tgt=server; mode=""; follow=0; seen_mode=0
        while [ $# -gt 0 ]; do
            case "$1" in
                server|host) tgt="$1"; shift ;;
                --mode) [ $seen_mode = 0 ] || { echo "logs: argument --mode: not allowed with the other mode flags" >&2; exit 2; }
                        case "${2:-}" in native|compose) mode="$2";; *) echo "logs: argument --mode: invalid choice '$2' (choose from native, compose)" >&2; exit 2;; esac
                        seen_mode=1; shift 2 ;;
                --native) [ $seen_mode = 0 ] || { echo "logs: argument --native: not allowed with the other mode flags" >&2; exit 2; }
                        mode=native; seen_mode=1; shift ;;
                --env) [ $seen_mode = 0 ] || { echo "logs: argument --env: not allowed with the other mode flags" >&2; exit 2; }
                       case "${2:-}" in dev|prod) mode=compose;; *) echo "logs: argument --env: invalid choice '$2'" >&2; exit 2;; esac
                       seen_mode=1; shift 2 ;;
                -f|--follow) follow=1; shift ;;
                *) echo "logs: unexpected argument $1" >&2; exit 2 ;;
            esac
        done
        if [ "$tgt" = server ]; then
            mode="${mode:-native}"
            if [ "$mode" = native ]; then
                inst="${MSRTC_OUT_ROOT:-$ROOT/out}/server"
                if [ "$follow" = 1 ]; then
                    real=()
                    for f in "$inst"/run/logs/*.log; do [ -e "$f" ] && real+=("$f"); done
                    if [ ${#real[@]} -eq 0 ]; then echo "logs: no cluster logs under $inst/run/logs — start the instance first" >&2; exit 1; fi
                    echo "\$ tail -f ${real[*]}"; exec tail -f "${real[@]}"
                fi
                exe=""
                for c in $(ls "$inst/bin" 2>/dev/null | grep -- '-server$' | sort); do
                    case "$c" in *.exe.bak) ;; *) exe="$inst/bin/$c"; break;; esac
                done
                if [ -z "$exe" ]; then echo "logs: no deployed server instance under <out>/server — deploy first, or --mode compose" >&2; exit 1; fi
                echo "\$ $exe logs"; exec "$exe" logs
            fi
            command -v docker >/dev/null || { echo "Error: missing dependency 'docker' — install docker and start the daemon" >&2; exit 1; }
            cmd=(docker compose -f docker-compose.dev.yml logs)
            [ "$follow" = 1 ] && cmd+=(-f)
            cmd+=(server)
            _msvc_run "${cmd[@]}"
            exit $?
        fi
        if [ -f /tmp/mediaservo-host.log ]; then
            t=(tail -n 100); [ "$follow" = 1 ] && t+=(-f); t+=(/tmp/mediaservo-host.log)
            _msvc_run "${t[@]}" || exit $?
        fi
        echo "host log directory: $HOME/.local/share/oxmgr/logs (instance logs: OXMGR_DATA_DIR/run/logs/msrtc-streamer-*.err.log)" >&2
        exit 0
        ;;
    status)
        # was _cmd_status_runtime + 8 helper fns — health verdict (rc 0 running /
        # 1 not running / 2 bad args or probe undecidable). Native server = the
        # managed binary's own registry view (F31: forward, never re-implement).
        shift; tgt="${1:-}"; [ $# -gt 0 ] && shift
        mode=""; seen_mode=0; env_arg=""
        while [ $# -gt 0 ]; do
            case "$1" in
                --mode) [ $seen_mode = 0 ] || { echo "status: argument --mode: not allowed with the other mode flags" >&2; exit 2; }
                        case "${2:-}" in native|compose) mode="$2";; *) echo "status: argument --mode: invalid choice '$2'" >&2; exit 2;; esac
                        seen_mode=1; shift 2 ;;
                --native) [ $seen_mode = 0 ] || { echo "status: argument --native: not allowed with the other mode flags" >&2; exit 2; }
                        mode=native; seen_mode=1; shift ;;
                --env) [ $seen_mode = 0 ] || { echo "status: argument --env: not allowed with the other mode flags" >&2; exit 2; }
                       case "${2:-}" in dev|prod) env_arg="$2"; mode=compose;; *) echo "status: argument --env: invalid choice '$2'" >&2; exit 2;; esac
                       seen_mode=1; shift 2 ;;
                *) echo "status: unexpected argument $1" >&2; exit 2 ;;
            esac
        done
        if [ -z "$tgt" ] || { [ "$tgt" != server ] && [ "$tgt" != host ]; }; then
            echo "usage: status {server,host} [--mode native|compose] [--env dev|prod]" >&2; exit 2; fi
        if [ "$tgt" = server ] && [ "${mode:-native}" = native ]; then
            inst="${MSRTC_OUT_ROOT:-$ROOT/out}/server"
            exe=""
            for c in $(ls "$inst/bin" 2>/dev/null | grep -- '-server$' | sort); do
                case "$c" in *.exe.bak) ;; *) exe="$inst/bin/$c"; break;; esac
            done
            if [ -z "$exe" ]; then echo "server: no deployed instance under <out>/server — deploy first (mediaservo deploy server --prefix <X>), or --mode compose for containers" >&2; exit 1; fi
            exec "$exe" status "$inst"
        fi
        if [ "$tgt" = server ]; then
            env_now="${env_arg:-$(_msvc_detect_env)}"
            echo "server (compose env=$env_now):"
            command -v docker >/dev/null || { echo "Error: missing dependency 'docker' — install docker and start the daemon" >&2; exit 1; }
            _msvc_run docker compose -f "$(_msvc_compose_file "$env_now")" ps
            code=$(curl --noproxy "*" -s -o /dev/null -w "%{http_code}" --max-time 2 http://127.0.0.1:9800/health 2>/dev/null | tr -d '[:space:]')
            if [ "$code" = 200 ]; then echo "  health 9800: 200 OK"; else echo "  health 9800: ${code:-no response}"; fi
            av="${MEDIASERVO_SFU_ANNOUNCED_IP:-}"
            if [ -z "$av" ]; then
                ips=$(_msvc_announced_ips | paste -sd, -)
                if [ -n "$ips" ]; then av="$ips"; echo "MEDIASERVO_SFU_ANNOUNCED_IP auto-detected: $ips"; fi
            fi
            [ -n "$av" ] && echo "  announced: $av"
            if docker ps --format '{{.Names}}' 2>/dev/null | grep -qx mediaservo-server-1 && [ "$(_msvc_detect_env)" = "$env_now" ]; then exit 0; else exit 1; fi
        fi
        # host: dual brand prefixes (ops#1) + bare oxmgr daemon
        echo "host:"
        any=0; probe_failed=0
        for role in agent streamer capturer oxmgr; do
            pids=""
            if [ "$role" = oxmgr ]; then cands=(oxmgr); else cands=("msrtc-$role" "host-$role"); fi
            for cname in "${cands[@]}"; do
                o=$(pgrep -x "$cname" 2>/dev/null | tr '\n' ' ')
                if [ -n "$o" ]; then pids="${o% }"; break; fi
            done
            if [ -n "$pids" ]; then any=1; echo "  $role: running pid=$pids"; else echo "  $role: not running"; fi
        done
        if out=$(ss -tulnp 2>/dev/null); then
            case "$out" in *":17980 "*) echo "  gateway 17980: listening" ;; *) echo "  gateway 17980: free" ;; esac
        else probe_failed=1; echo "  gateway 17980: undecidable"; fi
        inst="${OXMGR_DATA_DIR:-}"
        if [ -n "$inst" ]; then log_hint="$(dirname "$inst")/run/logs"; else log_hint="${ROOT%/*}/out/host/run/logs"; fi
        echo "  logs: $log_hint/msrtc-streamer-<stream>.err.log (C38 layer-2: grep subscription/acl denied/OpenH264)"
        if [ $any = 0 ]; then
            echo "  → host not running — manual: target/debug/mediaservo-host init . && token issue --all . && start . (development) or systemctl --user start oxmgr-<brand>-host.service (deployed)" >&2
        fi
        if [ $probe_failed = 1 ]; then exit 2; elif [ $any = 1 ]; then exit 0; else exit 1; fi
        ;;
    clean)
        # was _cmd_clean + _rm_path/_rm_tree/_native_runtime_dirs + the mode
        # helpers. Destructive legs mirror the python delete-list exactly;
        # docker lines ride _msvc_run so MEDIASERVO_DRY_RUN=1 can prove the
        # command shape without touching live containers.
        shift; tgt=""; mode=""; seen_mode=0; do_all=0
        while [ $# -gt 0 ]; do
            case "$1" in
                all|server|host|client) [ -z "$tgt" ] || { echo "clean: too many arguments" >&2; exit 2; }; tgt="$1"; shift ;;
                --mode) [ $seen_mode = 0 ] || { echo "clean: argument --mode: not allowed with the other mode flags" >&2; exit 2; }
                        case "${2:-}" in native|compose|both) mode="$2";; *) echo "clean: argument --mode: invalid choice '$2'" >&2; exit 2;; esac
                        seen_mode=1; shift 2 ;;
                --native) [ $seen_mode = 0 ] || { echo "clean: argument --native: not allowed with the other mode flags" >&2; exit 2; }
                        mode=native; seen_mode=1; shift ;;
                --env) [ $seen_mode = 0 ] || { echo "clean: argument --env: not allowed with the other mode flags" >&2; exit 2; }
                       case "${2:-}" in dev|prod) mode=compose;; *) echo "clean: argument --env: invalid choice '$2'" >&2; exit 2;; esac
                       seen_mode=1; shift 2 ;;
                --all) do_all=1; shift ;;
                *) echo "clean: unexpected argument $1" >&2; exit 2 ;;
            esac
        done
        tgt="${tgt:-all}"; mode="${mode:-native}"
        _ms_rm_path() { [ -e "$1" ] || [ -L "$1" ] || return 0; if [ -d "$1" ] && [ ! -L "$1" ]; then rm -rf "$1"; else rm -f "$1"; fi; }
        if [ "$tgt" = all ] || [ "$tgt" = server ]; then
            if [ "$mode" = native ] || [ "$mode" = both ]; then
                if [ -n "${MSRTC_OUT_ROOT:-}" ]; then base="$MSRTC_OUT_ROOT/server"; else base="$ROOT/data"; fi
                mkdir -p "$base/etc" "$base/logs"; etc_d="$base/etc"; logs_d="$base/logs"
                out_server="${MSRTC_OUT_ROOT:-$ROOT/out}/server"
                pid_file="$logs_d/server-native.pid"
                if [ -f "$pid_file" ]; then
                    pid=$(tr -dc '0-9' < "$pid_file" | head -c 12); pid="${pid:--1}"
                    if [ "$pid" -gt 0 ] 2>/dev/null && [ -d "/proc/$pid" ]; then
                        echo "clean server: stopping bare-metal process pid=$pid"
                        kill "$pid" 2>/dev/null || true
                        for _i in 1 2 3 4; do [ -d "/proc/$pid" ] || break; sleep 0.5; done
                        [ -d "/proc/$pid" ] && kill -9 "$pid" 2>/dev/null || true
                    fi
                fi
                for f in "$logs_d/server-native.pid" "$logs_d/server-native.log" \
                         "$out_server/bin/mediaservo-server" "$out_server/bin/mediaservo-weaknet" \
                         "$ROOT/target/debug/mediaservo-weaknet" "$ROOT/target/release/mediaservo-weaknet" \
                         "$out_server/web" "$ROOT/target/debug/mediaservo-server" "$ROOT/target/release/mediaservo-server"; do
                    _ms_rm_path "$f"
                done
                for f in "$out_server"/bin/*-server "$out_server"/bin/*-weaknet; do [ -f "$f" ] && _ms_rm_path "$f"; done
                for f in "$out_server"/*-server; do [ -L "$f" ] && _ms_rm_path "$f"; done
                [ -L "$out_server/server" ] && _ms_rm_path "$out_server/server"
                for d in "$out_server"/run/oxmgr*; do _ms_rm_path "$d"; done
            fi
            if [ "$mode" = compose ] || [ "$mode" = both ]; then
                command -v docker >/dev/null || { echo "Error: missing dependency 'docker' — install docker and start the daemon" >&2; exit 1; }
                cmd=(docker compose -f docker-compose.dev.yml down)
                if [ "$do_all" = 1 ]; then cmd+=(-v); echo "Warning: clean --all deletes the cargo-cache named volume (the next server build recompiles fully, 15-30 minutes)"; fi
                _msvc_run "${cmd[@]}"; rc=$?; [ $rc -ne 0 ] && exit $rc
            fi
        fi
        if [ "$tgt" = all ] || [ "$tgt" = host ] || [ "$tgt" = client ]; then
            if [ -e "$ROOT/target" ]; then
                if rm -rf "$ROOT/target" 2>/dev/null; then echo "deleted: $ROOT/target"; else echo "Warning: cannot delete $ROOT/target (contains container-created root files) — run manually: sudo rm -rf $ROOT/target" >&2; fi
            fi
            if [ -n "${CARGO_TARGET_DIR:-}" ]; then
                echo "Note: CARGO_TARGET_DIR=$CARGO_TARGET_DIR (possibly shared by multiple projects)"
                if [ -e "$CARGO_TARGET_DIR" ]; then
                    if rm -rf "$CARGO_TARGET_DIR" 2>/dev/null; then echo "deleted: $CARGO_TARGET_DIR"; else echo "Warning: cannot delete $CARGO_TARGET_DIR (contains container-created root files) — run manually: sudo rm -rf $CARGO_TARGET_DIR" >&2; fi
                fi
            fi
        fi
        if [ "$do_all" = 1 ] && { [ "$tgt" = all ] || [ "$tgt" = server ]; }; then
            _msvc_run docker builder prune -f; exit $?
        fi
        exit 0
        ;;
    config)
        # was _cmd_config (SR6 batch-beta-1) — dev-track template face.
        # show/set are pure shell; validate rides the mediaservo-gates Rust face
        # (serde_yaml; the pyyaml twin retired with the verb — N4 pattern).
        shift; sub="${1:-}"; [ $# -gt 0 ] && shift
        case "$sub" in
            show)
                for rel in crates/mediaservo-host/config/host.conf config/server.docker.yaml; do
                    echo "--- $rel ---"
                    if [ -f "$ROOT/$rel" ]; then
                        cat "$ROOT/$rel"; echo
                    else
                        echo "(missing: $ROOT/$rel)" >&2
                    fi
                done
                exit 0
                ;;
            set)
                key="${1:-}"
                if [ -z "$key" ] || [ $# -lt 2 ]; then
                    echo "Usage: mediaservo config set <key> <value>" >&2; exit 2
                fi
                value="$2"
                if [ "$key" != signaling.psk ]; then
                    echo "config set: currently supports only signaling.psk (got $key)" >&2; exit 2
                fi
                printf 'signaling.psk = %s\n' "$value" > "$ROOT/config/host.conf"
                echo "wrote config/host.conf (dev track — in prod, use exec to edit the in-volume server.yaml)"
                exit 0
                ;;
            validate)
                if ! (cd "$ROOT" && cargo build -q -p mediaservo-gates); then
                    echo "Error: cannot build mediaservo-gates (config-validate face) — cargo available / pixi activated?" >&2
                    exit 1
                fi
                exec "$ROOT/target/debug/mediaservo-gates" config-validate --root "$ROOT"
                ;;
            *)
                echo "config: expected show|validate|set <key> <value> (got '${sub:-}')" >&2; exit 2
                ;;
        esac
        ;;
    deploy)
        # SR6 P2: bindings leg shell-driven (cmake --install route). SR6 batch-beta-1:
        # the host leg joined; server stays python until its own beta commit. The
        # driver only orchestrates — layout = cmake rows, stateful merge = binary
        # init [bF3], never re-implemented here.
        case "${2:-}" in
        host)
            shift 2; prefix=""; build_type=Debug
            while [ $# -gt 0 ]; do
                case "$1" in
                    --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
                    --release) build_type=Release; shift ;;
                    *) echo "deploy host: unexpected argument $1" >&2; exit 2 ;;
                esac
            done
            [ -n "$prefix" ] || { echo "Error: deploy requires --prefix (no default — prevents polluting out/ stateless delivery; vehicles use /opt/mediaservo-host)" >&2; exit 2; }
            bin_dir="$prefix/bin"
            # brand: env (msrtc.sh injects MEDIASERVO_BRAND) first, falling back to
            # derivation from the installed instance (python _derive_brand parity:
            # first *-agent hit, host-* legacy names skipped; the multi-brand
            # first-match limit is carried knowingly). Explicit default "mediaservo"
            # maps to the legacy host-* layout (empty brand).
            brand_in="${MEDIASERVO_BRAND:-}"
            if [ -z "$brand_in" ]; then
                for p in "$bin_dir"/*-agent; do
                    [ -e "$p" ] || continue
                    bn=$(basename "$p")
                    case "$bn" in host-*) continue ;; esac
                    brand_in=${bn%"-agent"}; break
                done
            fi
            brand=$brand_in; [ "$brand" = mediaservo ] && brand=""
            rust_brand=${brand:-mediaservo}
            mkdir -p "$bin_dir" 2>/dev/null || { echo "Error: cannot create $bin_dir — for deployment use sudo msrtc.sh deploy host --prefix /opt/mediaservo-host" >&2; exit 1; }
            case "$prefix" in
                /opt*) [ "$(id -u)" = 0 ] || { echo "Error: /opt deployment needs root (sudo msrtc.sh deploy host --prefix /opt/mediaservo-host)" >&2; exit 1; } ;;
            esac
            host_cli=${brand:+${brand}-host}; host_cli=${host_cli:-mediaservo-host}
            # triple stop of the revival sources (unit -> daemon -> family), only
            # when a live instance is detectable (oxfile + branded CLI present).
            # Never call the oxmgr CLI here (its IPC auto-spawns the daemon —
            # creating busy instead of clearing it). C50: the unit arm is
            # OXMGR_HOME-scoped — a brand glob never stops another instance.
            if [ -f "$prefix/run/oxfile.toml" ] && [ -f "$bin_dir/$host_cli" ]; then
                echo "Running host instance detected — stopping the process family first (reinstall does not overwrite etc/ credentials)"
                oxmgr_home=$(realpath -m -- "$prefix/run/oxmgr" 2>/dev/null || echo "$prefix/run/oxmgr")
                want_home="Environment=OXMGR_HOME=$oxmgr_home"
                if command -v systemctl >/dev/null 2>&1 && [ -d "$HOME/.config/systemd/user" ]; then
                    for u in "$HOME/.config/systemd/user"/oxmgr-${brand}-*.service; do
                        [ -e "$u" ] || continue
                        un=$(basename "$u")
                        if ! grep -qF -- "$want_home" "$u" 2>/dev/null; then
                            echo "  note: leaving unit $un untouched (belongs to another instance home)"
                            continue
                        fi
                        systemctl --user stop "$un" || true
                        systemctl --user reset-failed "$un" || true
                    done
                fi
                _msvc_kill_using "$bin_dir/oxmgr"
                for base in agent streamer capturer recorder controller emergency audio; do
                    role=${brand:+${brand}-${base}}; role=${role:-host-${base}}
                    pkill -x "$role" || true
                done
                sleep 1
            fi
            _msvc_app_install host "$brand" "$build_type" "$prefix"
            # oxmgr version locked with the deployment (D-H13)
            _msvc_oxmgr_resolve host
            _msvc_copy_with_kill "$MS_OXMGR_SRC" "$bin_dir/oxmgr"
            oxv=$("$MS_OXMGR_SRC" --version 2>/dev/null | head -1) || oxv=""
            [ -n "$oxv" ] || oxv=$("$MS_OXMGR_SRC" --version 2>&1 >/dev/null | head -1) || oxv=""
            echo "  oxmgr packaged: ${oxv:-?}"
            # host init: etc/ + identity.json + tokens — idempotent write-only, never
            # delete (reinstall keeps credentials). init is a separate process: the
            # device prefix brand must match the layout brand (env override).
            if [ -f "$prefix/identity.json" ]; then
                echo "  identity.json exists — write-only, never delete (device identity preserved): $prefix/identity.json"
            fi
            echo "\$ $bin_dir/$host_cli init $prefix"
            MEDIASERVO_BRAND="$rust_brand" "$bin_dir/$host_cli" init "$prefix"
            # env.sh one-shot activation (MED5 — relocatable: derived relative to
            # itself; same bytes as the pack-leg host template, deploy-side owner)
            env_host=${brand:+${brand}-host}; env_host=${env_host:-mediaservo-host}
            cat > "$prefix/env.sh" <<EOF
#!/usr/bin/env bash
# ${env_host} environment activation (generated at deploy) — all commands available after sourcing
# Usage: source env.sh (still valid under any prefix / after relocation, derived relative to this file)
__MSRTC_ENV_DIR="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")" && pwd -P)"
export PATH="\${__MSRTC_ENV_DIR}/bin:\$PATH"
export MEDIASERVO_BRAND="${rust_brand}"   # brand env (D252); the deployment package pins the layout explicitly
BIN_DIR_MSRTC="\${__MSRTC_ENV_DIR}/bin"   # for scripts/aliases to reference
unset __MSRTC_ENV_DIR
echo "[${rust_brand}] ${env_host} environment activated — bin/: \$BIN_DIR_MSRTC"
EOF
            chmod 755 "$prefix/env.sh"
            echo "  env.sh generated: $prefix/env.sh (after sourcing, PATH=bin/ makes all commands available)"
            mkdir -p "$prefix/run/logs" "$prefix/recordings"
            echo "host deployed to $prefix (${build_type,,})"
            echo "  bin/    $(ls -1 "$bin_dir" | sort | paste -sd, - | sed 's/,/, /g')"
            echo "  etc/    host.yaml + link/{signing.pem,*.token} (generated by host init, kept on reinstall)"
            echo "  run/    logs/"
            echo "  recordings/"
            echo "  identity.json (0600, device identity — write-only, do not delete)"
            echo "Usage: export PATH=$bin_dir:\$PATH && $host_cli doctor $prefix && $host_cli start $prefix"
            exit 0
            ;;
        bindings)
            shift 2; prefix=""; release=Debug
            while [ $# -gt 0 ]; do
                case "$1" in
                    --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
                    --release) release=Release; shift ;;
                    *) echo "deploy bindings: unexpected argument $1" >&2; exit 2 ;;
                esac
            done
            src="${MSRTC_OUT_ROOT:-$ROOT/out}/bindings"
            if [ -z "$prefix" ]; then
                prefix="$src"
                echo "[deploy bindings] prefix default → $prefix (out root follows the calling shell; --prefix overrides)"
            fi
            if [ ! -d "$src/lib" ]; then echo "Error: no assembled artifacts in $src — run build bindings first" >&2; exit 1; fi
            if ! mkdir -p "$prefix/lib" 2>/dev/null; then
                echo "Error: cannot create $prefix — for deployment use sudo msrtc.sh deploy bindings --prefix /opt/mediaservo-sdk" >&2; exit 1; fi
            if [ "$(cd "$prefix" && pwd -P)" = "$(cd "$src" && pwd -P)" ]; then
                # same-tree idempotent guard (09-21 SameFileError lesson): the build
                # phase already ran the install rules — a second full pass is churn.
                sos=$(cd "$src/lib" && ls -1 | grep '^libmediaservo_.*\.so' | sort | paste -sd, - | sed 's/,/, /g')
            else
                echo "\$ cmake --preset bindings -DCMAKE_BUILD_TYPE=$release -DMEDIASERVO_BINDINGS_SRC=$src  (cwd: $ROOT)"
                if ! (cd "$ROOT" && cmake --preset bindings -DCMAKE_BUILD_TYPE="$release" "-DMEDIASERVO_BINDINGS_SRC=$src"); then exit 1; fi
                attempt=1
                while :; do
                    if blob=$( (cd "$ROOT" && cmake --install build/presets/bindings --prefix "$prefix" --strip) 2>&1 ); then break; fi
                    rc=$?
                    if { echo "$blob" | grep -qi busy || echo "$blob" | grep -q 'Text file'; } && [ $attempt -lt 3 ]; then
                        echo "  install hit busy artifacts — killing occupiers, retrying ($attempt/3)"
                        for so in $(cd "$prefix/lib" 2>/dev/null && ls -1 | grep '^libmediaservo_.*\.so' | sed "s|^|$prefix/lib/|" | sort); do
                            _msvc_kill_using "$so"
                        done
                        attempt=$((attempt+1))
                    else
                        echo "Error: cmake --install into $prefix failed (rc=$rc):" >&2
                        printf '%s' "$blob" | tail -c 600 >&2
                        exit 1
                    fi
                done
                sos=$(cd "$prefix/lib" && ls -1 | grep '^libmediaservo_.*\.so' | sort | paste -sd, - | sed 's/,/, /g')
            fi
            echo "bindings deployed to $prefix (lib/ $sos + include/mediaservo + pkgconfig + cmake + python + wheel + node + docs/sdk-cxx)"
            echo "  usage: export LD_LIBRARY_PATH=<prefix>/lib; link per the bindings headers; python: pip install <prefix>/wheel/*.whl"
            exit 0
            ;;
        server)
            shift 2; prefix=""
            while [ $# -gt 0 ]; do
                case "$1" in
                    --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
                    --release) shift ;;  # accepted for composite pass-through parity (ignored — server assembly is Debug-pinned, python parity)
                    *) echo "deploy server: unexpected argument $1" >&2; exit 2 ;;
                esac
            done
            [ -n "$prefix" ] || { echo "Error: deploy requires --prefix (no default — prevents polluting out/ stateless delivery; convention: /opt/mediaservo-server)" >&2; exit 2; }
            src_root="${MSRTC_OUT_ROOT:-$ROOT/out}/server"
            src_bin_dir="$src_root/bin"
            bin_dir="$prefix/bin"
            # brand: env first -> deployed-tree derivation -> source-tree derivation
            # (zero-parameter idempotent re-deploy; python deploy-server chain parity)
            brand="${MEDIASERVO_BRAND:-}"
            [ -n "$brand" ] || brand=$(_ms_derive_brand_server "$bin_dir")
            [ -n "$brand" ] || brand=$(_ms_derive_brand_server "$src_bin_dir")
            bin_name=${brand:+${brand}-server}; bin_name=${bin_name:-mediaservo-server}
            if [ -z "$brand" ]; then
                # BLOCKER-2 empty-brand guard: empty brand never deletes/renames branded files
                unexpected=$({ ls -1 "$bin_dir" 2>/dev/null; ls -1 "$src_bin_dir" 2>/dev/null; } | grep -- '-server$' | grep -vx 'mediaservo-server' | sort -u | paste -sd, - | sed 's/,/, /g') || unexpected=""
                [ -z "$unexpected" ] || { echo "Error: $unexpected present but brand resolved empty — export MEDIASERVO_BRAND and retry" >&2; exit 2; }
            fi
            # BLOCKER-1 dual source hunt (ordered): upstream name (fresh build) ->
            # branded (migrated tree — a second deploy needs no rebuild)
            cands=("$src_bin_dir/mediaservo-server")
            if [ -n "$brand" ]; then cands+=("$src_bin_dir/$bin_name"); fi
            src_bin=""; cand_str=""
            for c in "${cands[@]}"; do
                cand_str+="${cand_str:+ / }$c"
                if [ -z "$src_bin" ] && [ -f "$c" ]; then src_bin="$c"; fi
            done
            [ -n "$src_bin" ] || { echo "Error: none of $cand_str exists — run build server first (deploy does not trigger builds — D266)" >&2; exit 2; }
            src_web="$src_root/web"
            [ -f "$src_web/index.html" ] || { echo "Error: no frontend artifacts in $src_web — run build web first (deploy does not trigger builds — D266)" >&2; exit 2; }
            mkdir -p "$bin_dir" 2>/dev/null || { echo "Error: cannot create $bin_dir — for deployment use sudo mediaservo deploy server --prefix /opt/mediaservo-server" >&2; exit 1; }
            case "$prefix" in
                /opt*) [ "$(id -u)" = 0 ] || { echo "Error: /opt deployment needs root (sudo mediaservo deploy server --prefix /opt/mediaservo-server)" >&2; exit 1; } ;;
            esac
            # build:deploy sugar injects --prefix out/<target> — for server the
            # source IS the target: in-place assembly (same-tree copy guard +
            # web rmtree self-destruct guard); init rendering proceeds either way
            inplace=0
            [ "$(realpath -- "$prefix")" = "$(realpath -- "$src_root")" ] && inplace=1
            dst_bin="$bin_dir/$bin_name"
            server_cli="$dst_bin"
            # stop-before-move (PIT-170: graceful stop first — never force through
            # a running binary; the occupier kill would bypass the cluster's own
            # stop. Checks old+new paths (pre-rename vs last-deploy name) and the
            # packaged daemon; the live lifecycle binary owns the stop).
            running=$(_ms_pids_using "$dst_bin")
            if [ -n "$brand" ]; then running="$running $(_ms_pids_using "$bin_dir/mediaservo-server")"; fi
            if [ -n "${running// /}" ] || [ -n "$(_ms_pids_using "$bin_dir/oxmgr")" ]; then
                echo "Running server instance detected — stopping this cluster first (reinstall does not overwrite etc/ credentials)"
                stop_bin="$dst_bin"; [ -f "$dst_bin" ] || stop_bin="$src_bin"
                timeout 30 "$stop_bin" stop "$prefix" >/dev/null 2>&1 && stop_rc=0 || stop_rc=$?
                [ "$stop_rc" = 124 ] && echo "  ⚠ old instance stop timed out (possibly a pre-lifecycle old binary) — continuing reinstall" >&2 || true
            fi
            _msvc_app_install server "$brand" Debug "$prefix"
            # lifecycle smoke: an old pre-lifecycle out/ build fails here (those
            # booted straight through on unknown args — the version probe is the gate)
            ver_out=$(timeout 10 "$server_cli" version 2>/dev/null) && smoke_rc=0 || smoke_rc=$?
            if [ "$smoke_rc" != 0 ]; then
                echo "Error: $server_cli lifecycle smoke failed (version nonzero/timeout — out/ is likely a pre-lifecycle old build) — rebuild server then re-run deploy" >&2
                exit 2
            fi
            ver_line=$(printf '%s\n' "$ver_out" | head -1)
            echo "  binary ready: ${ver_line:-version ok}"
            _msvc_oxmgr_resolve server
            _msvc_copy_with_kill "$MS_OXMGR_SRC" "$bin_dir/oxmgr"
            chmod 755 "$bin_dir/oxmgr"
            oxv=$("$MS_OXMGR_SRC" --version 2>/dev/null | head -1) || oxv=""
            [ -n "$oxv" ] || oxv=$("$MS_OXMGR_SRC" --version 2>&1 >/dev/null | head -1) || oxv=""
            echo "  oxmgr packaged: ${oxv:-?}"
            # the web tree moves wholesale (caddy static root — absolute paths are
            # baked into oxfile/Caddyfile by init below)
            dst_web="$prefix/web"
            if [ "$inplace" = 1 ]; then
                echo "  web/ in place (source = target) — skipping move (rmtree guard)"
            else
                if [ -e "$dst_web" ]; then rm -rf "$dst_web"; fi
                cp -a "$src_web" "$dst_web"
            fi
            mkdir -p "$prefix/run"
            # init renders idempotently: etc/Caddyfile + run/oxfile.toml (weaknet
            # app conditional render + secret bootstrap live in the binary) plus
            # migrate_stale_files [bF3] — the driver never re-implements the merge.
            echo "\$ $server_cli init $prefix"
            "$server_cli" init "$prefix"
            if [ -n "$brand" ]; then entry="$prefix/$bin_name"; else entry="$prefix/server, $prefix/mediaservo-server"; fi
            inplace_note=""; [ "$inplace" = 1 ] && inplace_note=" (assembled in place — rendering complete, start directly)"
            brand_note=""; [ -n "$brand" ] && brand_note=" (brand $brand)"
            echo "server deployed to $prefix${inplace_note}${brand_note}"
            echo "  bin/    $bin_name (physical name branded — D269) + oxmgr (pinned by D-H13)"
            echo "  etc/    server/devices/accounts.yaml + Caddyfile + secret (redeploys keep existing — PIT-160)"
            echo "  web/    frontend deliverables moved as a whole tree (caddy static root)"
            echo "  run/    oxfile.toml + oxmgr/ (OXMGR_DATA_DIR — C32 isolation) + logs/"
            echo "  entrypoints: $entry → bin/$bin_name"
            echo "⚠ accounts.yaml is a dev placeholder template (admin123 etc.) — bare-metal startup will fail-fast (C35 guard):"
            echo '    production: export MEDIASERVO_ADMIN_PASSWORD=*** then delete etc/accounts.yaml and re-run this deploy'
            echo '    integration: add MEDIASERVO_ALLOW_DEV_CREDENTIALS = "1" under the server entry [apps.env] in run/oxfile.toml'
            echo "Next steps (instance-directory commands; this CLI's run branch is retired — C39):"
            echo "  start:   $prefix/$bin_name start $prefix (backend only: add --no-web; frontend transition: mediaservo run web)"
            echo "  probe:   $prefix/$bin_name status $prefix (exit codes 0/1/2) | doctor $prefix"
            echo "  boot anchor (operator step, one-time): $prefix/$bin_name startup on $prefix"
            exit 0
            ;;
        *)
            echo "deploy: invalid target '${2:-}' (choose from bindings, host, server)" >&2
            exit 2
            ;;
        esac
        ;;
esac

# build:deploy composite (2026-09-29: verb surface aligned with the parent-repo msrtc.sh — the submodule shell
# works standalone without returning to the parent repo. Default prefix=<out>/<target> (the _out_root resolution
# lives in the CLI layer — bindings default there; host/server composite forms still pass --prefix out/<target> explicitly for symmetry).
# host/server deploy runtime data is written under out/<target> — same semantics as msrtc.sh.
if [ "${1:-}" = "build:deploy" ] && [ $# -ge 2 ]; then
    TARGET="$2"
    shift 2
    OUT_ROOT="${OUT_ROOT:-$ROOT/out}"   # the submodule shell gets no MSRTC_OUT_ROOT injection (parent-shell duty) — explicit default
    BUILD_FLAGS=(build "$TARGET")
    DEPLOY_FLAGS=(deploy "$TARGET" --prefix "$OUT_ROOT/$TARGET")
    while [ $# -gt 0 ]; do
        case "$1" in
            --prefix) DEPLOY_FLAGS+=(--prefix "$2"); shift 2;;   # explicit override of the default
            --release) BUILD_FLAGS+=(--release); DEPLOY_FLAGS+=(--release); shift;;
            *) BUILD_FLAGS+=("$1"); DEPLOY_FLAGS+=("$1"); shift;;
        esac
    done
    # deploy legs are shell-owned now (bindings SR6 P2, host/server batch-beta-1) —
    # build stays python, deploy re-enters this shell.
    python "$ROOT/scripts/mediaservo_cli.py" "${BUILD_FLAGS[@]}" \
        && "$ROOT/mediaservo.sh" "${DEPLOY_FLAGS[@]}"
    exit $?
fi

exec python "$ROOT/scripts/mediaservo_cli.py" "$@"
