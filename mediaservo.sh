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
        # SR6 batch-beta-2: -h moved off the retired python argparse (static text
        # before activation — the banner-free -h duty from review H1 survives).
        cat <<'HEOF'
usage: mediaservo [-h] <verb> [...]

MediaServo unified build CLI (terms: native = bare metal, compose = containers)
Entry: ./mediaservo.sh (bash driver; the python CLI retired at SR6 batch-beta-2 —
the build/package/example/compose/ops faces all live in this shell, cmake and the
managed binaries own layout and runtime truth).

build/assembly face:
  build <target> [--release] [--image runtime|dev] [--native] [NAME...]
        target: all|web|host|server|client|bindings|example (default all;
        server = non-embedded variant + one-step web assembly; web =
        frontend-only fast lane; only --image takes the Docker path;
        build-host/build-server compat aliases kept)
  build:deploy <target> [--prefix DIR] [--release]   (one-step build + deploy)
  package <target> [--dist DIR] [--brand NAME] [--release]
        target: host|server|sdk-field|sdk-client (bindings = one-cycle alias)
        → <dist>/<brand>-<target>-<ver>-<platform>-<arch>.tar.gz (D-H13/F12,
        versioned top-level dir; --dist defaults to dist/, the MSRTC release
        shell injects out/packages; unpack straight into a prefix with
        tar xzf pkg.tar.gz -C <prefix> --strip-components=1)
  run example <name> [args…]   test example [<name>]   list example
        (foreground dev tool; artifacts in target/examples/bin/ — never in
        out/; the build-c prelude is auto-added)
  deploy bindings|host|server [--prefix DIR] [--release]   (cmake install rows
        lay the tree; the managed binaries render state [bF3] — driver only
        orchestrates)

Three-mode quick reference (native = bare metal, compose = containers):
  Mode 1 bare-metal delivery: build server (assembles bin/etc/web in out/) → deploy server --prefix <X> (bin/etc layout = cmake install rules) →
             <X>/<brand>-server init/start/stop/status/restart/doctor <X> (startup on = boot anchor; bin physical name branded per D269)
    fast lanes: build web (frontend only → out/server/web) | dev web (vite:5173 hot reload, pair with start --no-web)
  Mode 2 single-container production: build server --image runtime → up --env prod → logs server --env prod
  Mode 3 compose development: up --env dev (hot reload) → logs -f → down --env dev

ops face (this shell): up down exec ps data dev version doctor list
  start stop restart run logs status clean config e2e test deploy
  guidance arms exit 2 (C39 — instance-directory commands; the container face up/down is unchanged)
  exit codes (shell-owned status/logs): 0 = success, 1 = not running/target missing, 2 = bad args / probe undecidable
HEOF
        exit 0
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
        # LC_ALL=C so the "Text file busy" message is English on every host —
        # a zh_CN box reported "text file busy" and the busy pattern missed, so the
        # ETXTBSY retry never fired (real bug: re-deploy failed on this host)
        if err=$(LC_ALL=C cp -p "$src" "$dst" 2>&1); then return 0; fi
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
# _ms_deploy_stop_instance <prefix> <brand> — triple-stop of revival sources
# (unit -> daemon -> family), C50-scoped: the unit arm matches OXMGR_HOME so a
# brand glob never stops another instance; never calls the oxmgr CLI here (its
# IPC auto-spawns the daemon). Byte-equal port of the host leg's inline block
# (pre-P2d-2 mediaservo.sh:1317-1337), shared by build:deploy + deploy host.
_ms_deploy_stop_instance() {
    local prefix="$1" brand="$2"
    local bin_dir="$prefix/bin"
    local host_cli="${brand:+${brand}-host}"; host_cli=${host_cli:-mediaservo-host}
    local oxmgr_home want_home un
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
    # family pkill is CWD-SCOPED (C50/C54 lesson, live-fired 10-27: an unscoped
    # pkill -x killed the PRODUCTION cluster's same-named processes when a /tmp
    # test prefix triggered the stop arm — restart_policy revived them, but that
    # is luck, not safety). Only pids whose cwd resolves inside $prefix die.
    local base role p pcwd
    for base in agent streamer capturer recorder controller emergency audio; do
        role=${brand:+${brand}-${base}}; role=${role:-host-${base}}
        for p in $(pgrep -x "$role" 2>/dev/null || true); do
            pcwd=$(readlink "/proc/$p/cwd" 2>/dev/null) || continue
            case "$pcwd" in
                "$prefix"|"$prefix"/*) pkill -x "$role" 2>/dev/null || true; break ;;
            esac
        done
    done
}

# _ms_deploy_via_target <target-suffix> <profile> [VAR=VAL ...] — P2d-2 forwarder:
# re-configures the dedicated deploy binaryDir with the caller's overrides, then
# drives the same custom target the IDE button uses. One implementation for all
# three faces (U5).
# _ms_deploy_via_target <target-suffix> <profile> [VAR=VAL ...] — P2d-2 forwarder.
# Two-phase: configure binaryDir -> build the bin lane (cargo artifact exists) ->
# reconfigure (AppTree EXISTS pins now see real files) -> drive deploy target.
# Single cargo writer at any moment (concurrent cargo on one target/ corrupted
# the main bin mid-deploy, measured 10-27).
_ms_deploy_via_target() {
    # NOTE: host leg is NOT forwarded yet (P2d-2 reverted — see deploy host body);
    # this helper serves bindings/server only.
    if [ "$1" = host ]; then
        echo "internal error: deploy host must use the bash leg" >&2; return 2
    fi
    local suffix="$1" profile="$2"; shift 2
    local cmake_bin="${CMAKE_BIN:-$ROOT/.pixi/envs/default/bin/cmake}"
    # NO ambient fallback: a system cmake here runs the lanes cargo under the
    # ambient (possibly non-pixi) toolchain and has produced corrupted main-bin
    # copies (measured: e2e shell without pixi on PATH -> cp-branded byte-copy
    # at target/debug/mediaservo-host). Fail with the activation hint instead.
    if [ ! -x "$cmake_bin" ]; then
        echo "Error: deploy target face needs the pixi cmake ($cmake_bin not found — run via msrtc.sh/mediaservo.sh which activate pixi, or export CMAKE_BIN)" >&2
        return 2
    fi
    local -a extra=()
    for kv in "$@"; do extra+=("-D$kv"); done
    local -a gates=(-DMEDIASERVO_BUILD_CRATES_LANES=ON)
    [ "$suffix" = bindings ] && gates=(-DMEDIASERVO_BUILD_BINDINGS=ON)
    "$cmake_bin" -S "$ROOT" -B "$ROOT/build/deploy-$suffix" \
        "${gates[@]}" -DMEDIASERVO_DEPLOY_PROFILE="$profile" \
        -DMEDIASERVO_BRAND="${MEDIASERVO_BRAND:-}" "${extra[@]}" >&2 || return 1
    if [ "$suffix" != bindings ]; then
        "$cmake_bin" --build "$ROOT/build/deploy-$suffix" \
            --target "mediaservo-$suffix-bin-build" || return 1
        # identity self-check: a replaced/foreign byte-copy at target must never
        # ride a deploy (fingerprint-fresh foreign content is invisible to cargo)
        # per-suffix identity: host checks mediaservo-host, server checks
        # mediaservo-server (server's version prints its own name — the host
        # probe on a server tree was a P2d-2 bug, caught on live restore)
        local main_bin="$ROOT/target/$(echo "$profile" | tr 'A-Z' 'a-z')/mediaservo-$suffix"
        local main_name="mediaservo-$suffix"
        # identity check runs BRAND-NEUTRAL (env -u MEDIASERVO_BRAND): D252
        # version prints are runtime-branded, so a caller's msrtc env made the
        # unbranded target bin read "msrtc-server" and the check self-fired
        # (measured on the out/server live restore)
        if [ -x "$main_bin" ] && ! env -u MEDIASERVO_BRAND "$main_bin" version 2>/dev/null | grep -q "^mediaservo-$suffix "; then
            echo "  target main-bin identity broken — forcing rebuild" >&2
            touch "$ROOT/crates/mediaservo-$suffix/src/main.rs" 2>/dev/null || \
                touch "$ROOT/crates/mediaservo-$suffix/src/bin/mediaservo-$suffix.rs" 2>/dev/null || true
            "$cmake_bin" --build "$ROOT/build/deploy-$suffix" \
                --target "mediaservo-$suffix-bin-build" || return 1
            env -u MEDIASERVO_BRAND "$main_bin" version 2>/dev/null | grep -q "^mediaservo-$suffix " || \
                { echo "Error: main-bin identity still broken" >&2; return 1; }
        fi
    fi
    "$cmake_bin" -S "$ROOT" -B "$ROOT/build/deploy-$suffix" \
        "${gates[@]}" -DMEDIASERVO_DEPLOY_PROFILE="$profile" \
        -DMEDIASERVO_BRAND="${MEDIASERVO_BRAND:-}" "${extra[@]}" >&2 || return 1
    "$cmake_bin" --build "$ROOT/build/deploy-$suffix" \
        --target "mediaservo-deploy-$suffix" || return 1
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
    local tree=$1 brand=$2 btype=$3 prefix=$4 staging=${5:-}
    local app_tree="$ROOT/build/app-tree"
    local cfg=(cmake --preset default -B "$app_tree" "-DMEDIASERVO_APP_TREE=$tree" "-DMEDIASERVO_BRAND=$brand" "-DMEDIASERVO_APP_STAGING=$staging" "-DCMAKE_BUILD_TYPE=$btype")
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
_ms_check() {
    # was python _check — dependency probe with a clear error (never silent)
    command -v "$1" >/dev/null 2>&1 || { echo "Error: missing dependency '$1' — $2" >&2; exit 1; }
}

# =============================================================================
# SR6 batch-beta-2 -- build family, bash-owned (moved out of mediaservo_cli.py;
# echo/command shapes mirror the retired python exactly; the python
# _run/_run_or_exit echo convention `$ cmd  (cwd: X)` is reproduced per leg).
# =============================================================================
_ms_build_host() {
    _ms_check cargo "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat"
    echo "\$ cargo build -p mediaservo-host -p mediaservo-client"
    cargo build -p mediaservo-host -p mediaservo-client || exit $?
    echo "✓ host build complete (pure compile to target/ — assembly and branding happen at deploy: mediaservo deploy host --prefix <X>)"
}
_ms_ensure_admin_dist() {
    # incremental by mtime (C24/C13 dual-track parity): dist missing or any src
    # file newer than dist/index.html → pnpm build:admin; zero cost otherwise.
    local dist="$ROOT/www/apps/admin/dist/index.html" src="$ROOT/www/apps/admin/src"
    if [ -f "$dist" ] && [ -d "$src" ] && [ -z "$(find "$src" -type f -newer "$dist" -print -quit 2>/dev/null)" ]; then
        echo "admin dist up to date (www/apps/admin/dist — no src changes) — skipping frontend build, reusing existing dist"
        return 0
    fi
    _ms_check pnpm "pnpm not installed — required for the frontend build (or run manually: cd www && pnpm build:admin)"
    echo "admin dist stale/missing — building frontend (tsc -b && vite build)..."
    echo "\$ pnpm build:admin  (cwd: $ROOT/www)"
    (cd "$ROOT/www" && pnpm build:admin) || exit $?
}
_ms_build_web() {
    # frontend-only fast lane: vite invocation stays here (language-native builder,
    # F30); the placement rides the app-tree cmake face (staging=web).
    _ms_ensure_admin_dist
    _msvc_app_install server "" Debug "${MSRTC_OUT_ROOT:-$ROOT/out}/server" web
}
_ms_build_stage_weaknet() {
    # $1=release flag — ordering contract [bF4 bF5 BA-15]: compiles BEFORE the
    # server build (mediasoup-hung trees never guilt-by-associate it); its own
    # failure only WARNs (degraded polarity carried from the retired python).
    local cmd=(cargo build) rc=0
    [ "$1" = 1 ] && cmd+=(--release)
    cmd+=(-p mediaservo-weaknet)
    if ! command -v cargo >/dev/null 2>&1; then
        echo "WARN: weaknet staging skipped (cargo unavailable: command not found) — not blocking the server build" >&2
        return 0
    fi
    echo "\$ ${cmd[*]}"
    "${cmd[@]}" || rc=$?
    if [ "$rc" != 0 ]; then
        echo "WARN: weaknet compile failed (rc=$rc) — not blocking the server build; the app-tree weaknet row WARNs if nothing stageable is found (re-run build server after troubleshooting)" >&2
    fi
    return 0
}

_ms_build_server() {
    # $1=image (''|runtime|dev) $2=native(0/1) $3=release(0/1)
    # default native (user ruling B); Docker only on explicit --image.
    _ms_build_stage_weaknet "$3"
    local out_server="${MSRTC_OUT_ROOT:-$ROOT/out}/server" sbt=Debug
    [ "$3" = 1 ] && sbt=Release
    if [ "$2" = 1 ] || [ -z "$1" ]; then
        _ms_ensure_admin_dist
        _ms_check cargo "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat"
        if [ -z "${MESON:-}" ]; then
            echo "Error: MESON environment variable not set — invoke via ./mediaservo.sh (source pixi-shell.sh injects the activation env)" >&2
            exit 2
        fi
        local cargs=(cargo build)
        [ "$3" = 1 ] && cargs+=(--release)
        cargs+=(-p mediaservo-server)
        echo "\$ ${cargs[*]}"
        # MESON/MESON_ARGS pop (tasks.py env pitfall, T1 unset semantics — the
        # assertion above only proxies "activated via mediaservo.sh")
        env -u MESON -u MESON_ARGS "${cargs[@]}" || exit $?
        mkdir -p "$out_server/bin"
        local brand="${MEDIASERVO_BRAND:-}"
        [ -n "$brand" ] || brand=$(_ms_derive_brand_server "$out_server/bin")
        # PIT-191 single-point branding: env -> in-tree derivation -> upstream,
        # rides -DMEDIASERVO_BRAND (swap-install rows, staging=build).
        _msvc_app_install server "$brand" "$sbt" "$out_server" build
        echo "server assembly staged: out/server/ (bin/etc/web — app-tree rules, brand ${brand:-mediaservo})"
        # ship caddy (PATH -> in-repo pixi tree -> WARN; never blocking)
        local caddy_src="" c
        caddy_src=$(command -v caddy || true)
        if [ -z "$caddy_src" ]; then
            for c in "$ROOT/.pixi/envs/default/bin/caddy" "$ROOT/../.pixi/envs/default/bin/caddy"; do
                if [ -f "$c" ]; then caddy_src=$c; break; fi
            done
        fi
        if [ -n "$caddy_src" ]; then
            local caddy_dst="$out_server/bin/caddy"
            if [ ! -e "$caddy_dst" ] || [ "$(realpath -- "$caddy_src")" != "$(realpath -- "$caddy_dst")" ]; then
                cp -p "$caddy_src" "$caddy_dst"
                echo "caddy shipped: out/server/bin/caddy (from $caddy_src)"
            fi
        else
            echo "WARN: caddy not found (PATH / in-repo pixi tree) — out/server contains no caddy; at start it must come from PATH, otherwise web silently degrades" >&2
        fi
        # config template seeding (PIT-158/160: never overwrite runtime data —
        # delete the old file first to get a fresh template)
        mkdir -p "$out_server/etc"
        local src_cfg="$ROOT/config/server.docker.yaml" f
        if [ -f "$src_cfg" ] && [ ! -f "$out_server/etc/server.yaml" ]; then
            sed -e 's|/opt/mediaservo/etc/accounts.yaml|accounts.yaml|g' \
                -e 's|/opt/mediaservo/etc/devices.yaml|devices.yaml|g' \
                "$src_cfg" > "$out_server/etc/server.yaml"
        fi
        if [ -f "$src_cfg" ]; then
            for f in accounts.yaml devices.yaml; do
                if [ -f "$ROOT/config/$f" ] && [ ! -f "$out_server/etc/$f" ]; then
                    cp -p "$ROOT/config/$f" "$out_server/etc/$f"
                fi
            done
            echo "server default config staged: $out_server/etc/server.yaml"
        fi
        return 0
    fi
    _ms_check docker "install docker and start the daemon"
    if [ "$1" = runtime ]; then
        echo "\$ docker build --target runtime -t mediaservo-server:latest ."
        docker build --target runtime -t mediaservo-server:latest . || exit $?
    else
        echo "\$ docker compose -f docker-compose.dev.yml build server"
        docker compose -f docker-compose.dev.yml build server || exit $?
    fi
}
_ms_build_client() {
    _ms_check cargo "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat"
    echo "\$ cargo build -p mediaservo-client"
    cargo build -p mediaservo-client || exit $?
}

# =============================================================================
# SR6 batch-beta-2 -- C++ example surface (p3 W0/G7; was _cmd_build_example /
# _cmd_run_example / _cmd_test_example). Directory contract =
# bindings/cxx/examples/<name>/CMakeLists.txt (dir == target == artifact, G4);
# same literal rule as the aggregate root's file(GLOB */CMakeLists.txt).
# F25 bridge: the cmake children get the SYSTEM constitution (conda build vars
# dropped, .pixi/envs bins scrubbed from PATH) — the one legit strip site;
# the island's own probe reads the same signals (keep the lists aligned).
# =============================================================================
_MS_EXAMPLES_BUILD="$ROOT/target/examples/build"
_MS_EXAMPLES_BIN="$ROOT/target/examples/bin"
_MS_STRIP_ENV=(env -u CC -u CXX -u CFLAGS -u CXXFLAGS -u CPPFLAGS -u LDFLAGS -u CPATH -u C_INCLUDE_PATH -u CPLUS_INCLUDE_PATH -u CONDA_PREFIX -u CMAKE_PREFIX_PATH -u PKG_CONFIG_PATH -u PKG_CONFIG_SYSROOT_DIR -u COMPILER_PATH -u LIBRARY_PATH)
_ms_scrubbed_path() {
    printf '%s' "$PATH" | tr ':' '\n' | grep -v '/\.pixi/envs/' | paste -sd: -
}
_ms_bridge_env() {  # prints the PATH=... assignment arg for the _MS_STRIP_ENV prefix
    local sp; sp=$(_ms_scrubbed_path)
    if [ -n "$sp" ]; then echo "PATH=/usr/bin:$sp"; else echo "PATH=/usr/bin"; fi
}
_ms_examples_is_lib() {  # $1=name → prints 0/1; rc1 when the example is unknown
    local cml="$ROOT/bindings/cxx/examples/$1/CMakeLists.txt"
    [ -f "$cml" ] || return 1
    if grep -q "add_library(" "$cml"; then echo 1; else echo 0; fi
}

# =============================================================================
# SR6 batch-beta-2 -- bindings face (was _cmd_build_bindings). The DELIVERY
# layout legs are cmake install rules (SR4 L1-L4 + SR6 batch-5 rows in
# cmake/MediaServoBindings.cmake — one leg one owner); what stays here is the
# cargo invocation pair, the DEV-tree .so.<MAJOR> shim links (ruled-away
# territory: target/ dev tree is not a delivery concern — SR4 P3 audit T0 #2),
# and the pip wheel mechanics (language-native packaging per SR4 §1.3 / N6).
# =============================================================================
_ms_workspace_version() {
    # [workspace.package] version — the C43 single source (no tomllib needed)
    sed -n '/\[workspace.package\]/,/^\[/p' "$ROOT/Cargo.toml" | sed -n 's/^version *= *"\([^"]*\)".*/\1/p' | head -1
}
_ms_build_bindings() {  # $1=release(0/1)
    _ms_check cargo "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat"
    local staging="$ROOT/target/bindings-staging" sdk
    mkdir -p "$staging"
    local cmd=(cargo build)
    [ "$1" = 1 ] && cmd+=(--release)
    # Staging discipline (2026-09-29): node/python build artifacts land under
    # target/ (gitignored) — zero writes into the source bindings/ trees.
    for sdk in field link deck client; do cmd+=("-p" "mediaservo-${sdk}-c"); done
    echo "\$ ${cmd[*]}"
    "${cmd[@]}" || exit $?
    local major ver; ver=$(_ms_workspace_version); major=${ver%%.*}  # shim crates are workspace-inherited (S3)
    local profile=debug; [ "$1" = 1 ] && profile=release
    local out_dir="$ROOT/target/$profile"
    for sdk in field link deck client; do
        # dev .so.<MAJOR> shim (D241 DT_NEEDED resolution) — ruled-away: the
        # target/ DEV tree, never the delivery layout
        rm -f "$out_dir/libmediaservo_${sdk}.so.$major"
        ln -s "libmediaservo_${sdk}.so" "$out_dir/libmediaservo_${sdk}.so.$major"
    done
    # node bindings (napi-rs .node; FFmpeg dynamic-lib linking done via build.rs)
    local ncmd=(cargo build)
    [ "$1" = 1 ] && ncmd+=(--release)
    ncmd+=(-p mediaservo-node)
    echo "\$ ${ncmd[*]}"
    "${ncmd[@]}" || exit $?
    local bind_dst="${MSRTC_OUT_ROOT:-$ROOT/out}/bindings"
    mkdir -p "$bind_dst/lib"
    # lib triple (D241) + N7 strip + headers + .pc/cmake transport + stale
    # sweeps + node/docs/python placement = cmake install rows (SR4 L1-L4,
    # batch-5). --strip is mandatory here: the STRIP CODE legs gate on
    # CMAKE_INSTALL_DO_STRIP — install(FILES) rows are NOT covered by --strip
    # itself, that is precisely why the legs exist. SRC reset is EXPLICIT:
    # deploy may have left MEDIASERVO_BINDINGS_SRC in the cache; the build
    # assembly configures ROWS mode, never inherits mirror mode.
    local _btype=Debug; [ "$1" = 1 ] && _btype=Release
    echo "\$ cmake --preset bindings -DCMAKE_BUILD_TYPE=$_btype -DMEDIASERVO_BINDINGS_SRC=  (cwd: $ROOT)"
    (cd "$ROOT" && cmake --preset bindings "-DCMAKE_BUILD_TYPE=$_btype" "-DMEDIASERVO_BINDINGS_SRC=") || exit $?
    echo "\$ cmake --install build/presets/bindings --prefix $bind_dst --strip  (cwd: $ROOT)"
    (cd "$ROOT" && cmake --install build/presets/bindings --prefix "$bind_dst" --strip) || exit $?
    _ms_build_pywheel "$out_dir" "$major" "$staging" "$bind_dst" || exit $?
    local n_docs n_lib
    n_docs=$(ls "$bind_dst"/docs/sdk-cxx/*.md 2>/dev/null | wc -l)
    n_lib=$(ls -1 "$bind_dst/lib" 2>/dev/null | grep -c '^libmediaservo_' || true)
    echo "bindings build complete: libmediaservo_{field,link,deck,client}.so version-full triple + node + python + .pc + cmake ($([ "$1" = 1 ] && echo release || echo debug))"
    echo "bindings delivery layout staged: out/bindings/ (lib $n_lib files + include/mediaservo + pkgconfig + cmake + python + wheel + node + docs/sdk-cxx $n_docs volumes)"
}
_ms_build_pywheel() {  # $1=out_dir $2=major $3=staging $4=bind_dst — fat wheel
    # (_libs real files, DT_NEEDED self-resolution) + pip --prefix assembly.
    # Ruled-away (SR4 P3 audit): wheel CONTENTS are language-native packaging
    # territory; the _libs copies feed the WHEEL, never the cmake-laid tree.
    local out_dir=$1 major=$2 staging=$3 bind_dst=$4
    local py_src="$staging/python-pkg" rc=0 werr ierr sdk
    rm -rf "$py_src"
    cp -R "$ROOT/bindings/python/mediaservo" "$py_src"
    # ignore= build, __pycache__, _libs, *.egg-info, dist, examples (staging
    # mirror of the pip build root — zero writes into the source tree)
    rm -rf "$py_src/build" "$py_src/__pycache__" "$py_src/_libs" "$py_src/dist" "$py_src/examples" "$py_src"/*.egg-info
    local libs_src="$py_src/mediaservo/_libs"
    mkdir -p "$libs_src"
    local wheel_dir="$bind_dst/wheel"  # pip -w creates it; stale-whl sweep rides the cmake bindings-face legs
    for sdk in field link deck; do  # py domain = the three device-half items (client is the sdk-client package — V1b redundancy guard)
        cp -p "$out_dir/libmediaservo_${sdk}.so" "$libs_src/libmediaservo_${sdk}.so.$major"
        # N7: wheel-embedded _libs are immutable once baked — strip must be front-loaded
        if command -v strip >/dev/null 2>&1; then
            strip --strip-unneeded "$libs_src/libmediaservo_${sdk}.so.$major" 2>/dev/null || echo "  warning: strip libmediaservo_${sdk}.so.$major failed (skipped — delivery size not optimized)"
        fi
        rm -f "$libs_src/libmediaservo_${sdk}.so"
        ln -s "libmediaservo_${sdk}.so.$major" "$libs_src/libmediaservo_${sdk}.so"
    done
    # setuptools build/ cache same-family sweep (09-17 V1b; build/ lives in staging)
    rm -rf "$py_src/build"
    werr=$( (cd "$py_src" && python3 -m pip wheel --no-deps --no-build-isolation -w "$wheel_dir" .) 2>&1 >/dev/null ) || rc=1
    if [ "$rc" != 0 ]; then
        rm -rf "$libs_src"
        echo "Error: wheel build failed — ${werr: -500}" >&2
        exit 1
    fi
    # Force the platform tag (the wheel contains Linux .so — refuse any-tag
    # misinstall across platforms): rewrite the Tag inside WHEEL + rename.
    # H3 interpreter-inline exception (the single surviving python in the build
    # driver): no installed tool face restamps wheel tags (uv/wheel-cli absent —
    # measured 2026-10-23); zipfile logic kept byte-faithful to the retired leg,
    # RECORD untouched by the same design (version stays = pyproject-embedded).
    local tag name wheel fixed
    tag="$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m | tr '-' '_')"
    wheel=$(ls -1 "$wheel_dir"/mediaservo-*-py3-none-any.whl 2>/dev/null | head -1)
    if [ -z "$wheel" ]; then
        rm -rf "$libs_src"
        echo "Error: wheel build produced no py3-none-any wheel in $wheel_dir" >&2
        exit 1
    fi
    name=$(basename "$wheel")
    fixed="$wheel_dir/${name/py3-none-any/py3-none-$tag}"
    python3 - "$wheel" "$fixed" "$tag" <<'PYRESTAMP' || rc=1
import os, sys, zipfile
src, dst, tag = sys.argv[1], sys.argv[2], sys.argv[3]
with zipfile.ZipFile(src, "r") as z:
    items = z.infolist()
    data = {i.filename: z.read(i) for i in items}
with zipfile.ZipFile(dst, "w") as z:
    for i in items:
        content = data[i.filename]
        if i.filename.endswith(".dist-info/WHEEL"):
            content = content.replace(b"Tag: py3-none-any", f"Tag: py3-none-{tag}".encode())
        z.writestr(i, content)
os.unlink(src)
PYRESTAMP
    if [ "$rc" != 0 ]; then
        rm -rf "$libs_src"
        exit 1
    fi
    # pip --prefix leaves old data_files (V1b corpse family) — the pre-install
    # sweep of site-packages mediaservo/ + dist-info runs as a cmake bindings-face
    # leg in the --install pass that precedes this invocation (SR6 batch-5).
    ierr=$( (cd "$ROOT" && python3 -m pip install --prefix "$bind_dst" --no-deps "$fixed") 2>&1 >/dev/null ) || rc=1
    rm -rf "$libs_src"  # clean up temp _libs (gitignore as backstop) — finally-parity
    if [ "$rc" != 0 ]; then
        echo "Error: pip install failed — ${ierr: -500}" >&2
        exit 1
    fi
}

# =============================================================================
# SR6 batch-beta-2 -- package driver (was _cmd_package + _cpack_* + C43(8)
# guard + _git_out + _crate_version/_TARGET_CRATE + _resolve_oxmgr/_derive_
# brand_server python twins). The release tree is described by cmake install
# rules (cmake/MediaServoPack.cmake + pack-legs/*.cmake.in) and packed by
# native CPack TGZ; the driver owns invoke + brand + lifecycle only (F29/F30).
# =============================================================================
_ms_target_crate() {  # F12: delivery target -> version-source crate
    case "$1" in
        host) echo mediaservo-host ;;
        server) echo mediaservo-server ;;
        bindings|sdk-field) echo mediaservo-field ;;
        sdk-client) echo mediaservo-client ;;
    esac
}
_ms_crate_version() {  # $1=crate dir name; [package] version, workspace-inherited falls back
    local f="$ROOT/crates/$1/Cargo.toml" v
    v=$(sed -n '/^\[package\]/,/^\[/{s/^version *= *"\([^"]*\)".*/\1/p;}' "$f" 2>/dev/null | head -1)
    if [ -n "$v" ]; then echo "$v"; else _ms_workspace_version; fi
}
_ms_git_out() {  # read-only git query: any failure (no git/shallow/timeout) -> rc1,
    # never raises — the degraded surface of the release-integrity guard.
    (cd "$ROOT" && timeout 10 git "$@" 2>/dev/null)
}
_ms_cpack_stage_sdk() {  # $1=staging $2=target $3=brand — L1/L2 halves
    local src="${MSRTC_OUT_ROOT:-$ROOT/out}/bindings" pkg bdir
    if [ ! -e "$src" ]; then echo "Error: out/bindings does not exist — run build bindings first" >&2; exit 1; fi
    case "$2" in bindings|sdk-field) pkg=sdk-field ;; *) pkg="$2" ;; esac
    bdir="$ROOT/build/packages/$2"   # alias keeps its own cache dir
    local cfg=(cmake --preset default -B "$bdir" "-DMEDIASERVO_PKG=$pkg" "-DMEDIASERVO_PKG_BRAND=$3" "-DMEDIASERVO_BIND_OUT=$src")
    [ "$2" = bindings ] && cfg+=("-DMEDIASERVO_PKG_ALIAS=sdk")   # one-cycle naming slot (F12)
    echo "\$ ${cfg[*]}  (cwd: $ROOT)"
    (cd "$ROOT" && "${cfg[@]}") || exit $?
    # reconfigure-always at the call site (SR7 discipline): the source-existence
    # globs are configure-time truth (PIT-189 family)
    echo "\$ cmake --install $bdir --prefix $1  (cwd: $ROOT)"
    (cd "$ROOT" && cmake --install "$bdir" --prefix "$1") || exit $?
}
_ms_cpack_stage_app() {  # $1=staging $2=target $3=brand-arg $4=release(0/1) — L3/L4
    local target=$2 src_root="${MSRTC_OUT_ROOT:-$ROOT/out}/server" bdir="$ROOT/build/packages/$2"
    local oxmgr brand bi cfg
    _msvc_oxmgr_resolve "$target"   # hard gate, same fail-loud message as deploy legs
    oxmgr=$MS_OXMGR_SRC
    cfg=(cmake --preset default -B "$bdir" "-DMEDIASERVO_PKG=$target" "-DMEDIASERVO_APP_TREE=$target" "-DMEDIASERVO_OXMGR_BIN=$oxmgr" "-DMEDIASERVO_PKG_BRAND=$3")
    if [ "$target" = server ]; then
        if [ ! -f "$src_root/web/index.html" ]; then
            echo "Error: no frontend artifacts in $src_root/web — run build web first (deploy does not trigger builds — D266)" >&2
            exit 2
        fi
        # deploy server's brand chain minus the always-empty staging arm (a fresh
        # staging dir derives nothing): env -> source-tree hunt
        brand="${MEDIASERVO_BRAND:-}"
        [ -n "$brand" ] || brand=$(_ms_derive_brand_server "$src_root/bin")
        cfg+=("-DMEDIASERVO_BRAND=$brand" "-DMEDIASERVO_SERVER_OUT_DIR=$src_root" "-DCMAKE_BUILD_TYPE=Debug")
    else
        bi="${MEDIASERVO_BRAND:-}"; bi="${bi// /}"  # python .strip() parity (spaces only — single-source env value)
        brand=""
        [ "$bi" = mediaservo ] || brand=$bi
        local sbt=Debug; [ "$4" = 1 ] && sbt=Release
        cfg+=("-DMEDIASERVO_BRAND=$brand" "-DCMAKE_BUILD_TYPE=$sbt")
    fi
    echo "\$ ${cfg[*]}  (cwd: $ROOT)"
    (cd "$ROOT" && "${cfg[@]}") || exit $?
    echo "\$ cmake --install $bdir --prefix $1  (cwd: $ROOT)"
    (cd "$ROOT" && cmake --install "$bdir" --prefix "$1") || exit $?
}
_ms_cpack_pack() {  # $1=staging $2=package_root $3=ver $4=dist — CPack TGZ over
    # the staged tree (design (a)/(c)): wrapper dir + tar name both come from
    # CPACK_PACKAGE_FILE_NAME; symlinks and empty dirs survive (measured). Scratch
    # goes to a temp work dir INSIDE dist (same fs for the move; self-cleans).
    _ms_check cpack "cmake's packaging tool missing — install cmake (>= 3.25)"
    local work rc=0
    work=$(mktemp -d "$4/.cpack-work-XXXXXX")
    cat > "$work/cpack-config.cmake" <<EOF
set(CPACK_GENERATOR "TGZ")
set(CPACK_PACKAGE_NAME "mediaservo")
set(CPACK_PACKAGE_VERSION "$3")
set(CPACK_PACKAGE_DESCRIPTION "MediaServo release package")
set(CPACK_PACKAGE_FILE_NAME "$2")
set(CPACK_INSTALLED_DIRECTORIES "$1;/")
EOF
    echo "\$ cpack --config $work/cpack-config.cmake -B $work  (cwd: $ROOT)"
    (cd "$ROOT" && cpack --config "$work/cpack-config.cmake" -B "$work") || rc=$?
    if [ $rc = 0 ]; then
        mv "$work/$2.tar.gz" "$4/$2.tar.gz" || rc=$?
    fi
    rm -rf "$work"
    [ $rc = 0 ] || exit $rc
}
_ms_reap_dist_daemons() {  # $1=dist $2=staging — staging hygiene: a run against a
    # fresh tree must never leave a daemon whose OXMGR_HOME points into dist
    # (PIT-224 family — auto-spawn during staging init). python finally-parity.
    local pid home comm
    for pid in $(ls -1 /proc 2>/dev/null | grep -E '^[0-9]+$'); do
        [ -r "/proc/$pid/environ" ] || continue
        home=$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null | sed -n 's/^OXMGR_HOME=//p' | head -1)
        case "$home" in "$1"*) ;; *) continue ;; esac
        comm=$(cat "/proc/$pid/comm" 2>/dev/null) || continue
        if [ "$comm" = oxmgr ]; then
            echo "  note: reaping staging-scoped daemon pid=$pid home=$home"
            kill "$pid" 2>/dev/null || true
        fi
    done
    rm -rf "$2"
}
_ms_package() {  # $1=target $2=dist $3=brand $4=release(0/1)
    case "$1" in host|server|bindings|sdk-field|sdk-client) ;; *) echo "package: argument target: invalid choice: '$1' (choose from 'host', 'server', 'bindings', 'sdk-field', 'sdk-client')" >&2; exit 2 ;; esac
    if [ "$1" = bindings ]; then echo 'WARN: target `bindings` renamed to `sdk-field` (F12 dual-half split; this alias stays for one version cycle)' >&2; fi
    local ver; ver=$(_ms_crate_version "$(_ms_target_crate "$1")")
    # C43(8) release-integrity guard: Cargo.toml differs from HEAD while git is
    # available -> the packaged version has no git anchor (no tag to attach, same
    # family as the v0.1.8.2 floating-number incident) — WARN without blocking
    # (debug packages may be dirty).
    if _ms_git_out log -1 --format=%s >/dev/null 2>&1; then
        if ! _ms_git_out diff --quiet HEAD -- Cargo.toml crates/mediaservo-host/Cargo.toml crates/mediaservo-server/Cargo.toml crates/mediaservo-field/Cargo.toml crates/mediaservo-client/Cargo.toml >/dev/null 2>&1; then
            echo "WARN: Cargo.toml has uncommitted changes carrying version $ver — before release: commit the bump + tag v$ver (this package's CHANGES header version has no git anchor)" >&2
        fi
    fi
    local dist="${2:-$ROOT/dist}" package_root="" rc=0
    mkdir -p "$dist"
    local staging; staging=$(mktemp -d "$dist/ms-$1-pkg-XXXXXX")
    if [ "$1" = host ] || [ "$1" = server ]; then
        _ms_cpack_stage_app "$staging" "$1" "$3" "$4" || rc=$?
    else
        _ms_cpack_stage_sdk "$staging" "$1" "$3" || rc=$?
    fi
    if [ $rc = 0 ]; then
        # naming + layout owner = cmake (package-root.txt readback; the
        # platform/arch formula lives in MediaServoPack.cmake)
        package_root=$(cat "$ROOT/build/packages/$1/package-root.txt")
        _ms_cpack_pack "$staging" "$package_root" "$ver" "$dist" || rc=$?
    fi
    if [ $rc = 0 ]; then
        local out="$dist/$package_root.tar.gz"
        echo "✓ packaged: $out ($(( $(stat -c%s "$out") / 1024 )) KiB)"
        echo "  top-level dir: $package_root/"
        echo "  contents: $(cd "$staging" && ls -1 | sort | paste -sd, - | sed 's/,/, /g')"
    fi
    _ms_reap_dist_daemons "$dist" "$staging"
    [ $rc = 0 ] || exit $rc
}
_ms_build_example() {  # $1=release(0/1); rest = example names (empty = all)
    local release=$1; shift
    local profile=debug; [ "$release" = 1 ] && profile=release
    # build-c prelude (PLAN G7; auto-added when missing — freshness is cargo's job)
    if [ ! -f "$ROOT/target/$profile/libmediaservo_client.so" ]; then
        echo "[example] libmediaservo_client.so missing → running build-c first" >&2
        (cd "$ROOT" && "${PIXI_BIN:-pixi}" run build-c) || exit $?
    fi
    local n
    if [ $# -gt 0 ]; then
        for n in "$@"; do
            if ! _ms_examples_is_lib "$n" > /dev/null; then
                echo "[example] unknown example '$n' (see ./mediaservo.sh list example)" >&2
                exit 2
            fi
        done
    fi
    local gen=Debug; [ "$release" = 1 ] && gen=Release
    local cfgargs=(-S "$ROOT/bindings/cxx/examples" -B "$_MS_EXAMPLES_BUILD" "-DCMAKE_BUILD_TYPE=$gen" "-DMEDIASERVO_BUILD_EXAMPLES=ON" "-DMEDIASERVO_SDK_DIR=$ROOT/target/$profile")
    # no -G: scrubbed PATH may lack ninja (pixi-only bin); cmake picks its
    # platform default (Makefiles on bare Linux) — same auto-adapt e2e-delivery uses.
    if [ ! -f "$_MS_EXAMPLES_BUILD/CMakeCache.txt" ] || [ "$profile" != debug ]; then
        echo "\$ cmake ${cfgargs[*]}  (cwd: $ROOT)"
        (cd "$ROOT" && "${_MS_STRIP_ENV[@]}" "$(_ms_bridge_env)" cmake "${cfgargs[@]}") || exit $?
    fi
    local cmd=(cmake --build "$_MS_EXAMPLES_BUILD")
    if [ $# -gt 0 ]; then cmd+=(--target "$@"); else cmd+=(--parallel); fi
    echo "\$ ${cmd[*]}  (cwd: $ROOT)"
    (cd "$ROOT" && "${_MS_STRIP_ENV[@]}" "$(_ms_bridge_env)" "${cmd[@]}") || exit $?
    echo "[example] artifacts → target/examples/bin/"
}
_ms_run_example() {  # $@ = <name> [passthrough…] — foreground dev tool (distinct from the retired host-run C39 semantics)
    if [ $# -eq 0 ]; then
        echo "Usage: run example <name> [args…] (./mediaservo.sh list example)" >&2
        exit 2
    fi
    local name=$1; shift
    local islib
    islib=$(_ms_examples_is_lib "$name") || { echo "[example] unknown example '$name'" >&2; exit 2; }
    _ms_build_example 0 "$name"
    if [ "$islib" = 1 ]; then
        echo "[example] '$name' is a library (no runnable artifact)" >&2
        exit 1
    fi
    local exe="$_MS_EXAMPLES_BIN/$name"
    if [ ! -e "$exe" ]; then
        echo "[example] still no artifact after build: $exe" >&2
        exit 1
    fi
    # Windowing hint: probe + state, never set for the user — display is session
    # semantics; a wrong scripted guess is harder to diagnose than fail-soft.
    if [ -z "${DISPLAY:-}" ] && [ -z "${SDL_VIDEODRIVER:-}" ]; then
        echo "[example] no display env vars (neither DISPLAY nor SDL_VIDEODRIVER set):" >&2
        local xs=""
        if [ -d /tmp/.X11-unix ]; then
            xs=$(cd /tmp/.X11-unix && for f in X*; do [ -e "$f" ] && printf ':%s\n' "${f#X}"; done 2>/dev/null | sort | paste -sd' ' -) || xs=""
        fi
        if [ -n "$xs" ]; then
            echo "  • windowed: local displays available: $xs → export DISPLAY=:N and re-run (if it reports x11 not available = this build has SDL_X11=OFF; reconfigure with cmake -DSDL_X11=ON)" >&2
        fi
        echo "  • headless acceptance: SDL_VIDEODRIVER=dummy MSRTC_RUN_SECS=20 re-run (cb/tex dual-count predicate)" >&2
    fi
    local ld="$ROOT/target/debug"
    [ -n "${LD_LIBRARY_PATH:-}" ] && ld="$ld:$LD_LIBRARY_PATH"
    LD_LIBRARY_PATH="$ld" exec "$exe" "$@"
}
_ms_test_example() {  # $1 = optional ctest -R filter; missing build dir = build first
    if [ ! -f "$_MS_EXAMPLES_BUILD/CTestTestfile.cmake" ]; then
        _ms_build_example 0
    fi
    local cmd=(ctest --test-dir "$_MS_EXAMPLES_BUILD" --output-on-failure)
    [ -n "${1:-}" ] && cmd+=(-R "$1")
    echo "\$ ${cmd[*]}"
    "${cmd[@]}" || exit $?
}

case "${1:-}" in
    build|build-host|build-server)
        # SR6 batch-beta-2: build family bash-owned; bindings/example still forward
        # to python until their own beta-2 commits land (one-leg-one-owner order).
        # build-host/build-server = compat aliases (retired argparse ALIASES).
        _ms_orig=("$@")
        case "${_ms_orig[0]}" in
            build-host) set -- build host "${_ms_orig[@]:1}" ;;
            build-server) set -- build server "${_ms_orig[@]:1}" ;;
        esac
        shift
        tgt=""; image=""; native=0; release=0; names=(); seen_tgt=0
        while [ $# -gt 0 ]; do
            case "$1" in
                --release) release=1; shift ;;
                --native) native=1; shift ;;
                --image) image="${2:?build --image needs a value}"
                         case "$image" in runtime|dev) ;; *) echo "build: argument --image: invalid choice: '$image' (choose from 'runtime', 'dev')" >&2; exit 2 ;; esac
                         shift 2 ;;
                -*) echo "build: unrecognized flag $1" >&2; exit 2 ;;
                *) if [ $seen_tgt = 0 ]; then tgt="$1"; seen_tgt=1; else names+=("$1"); fi; shift ;;
            esac
        done
        tgt="${tgt:-all}"
        case "$tgt" in all|web|host|server|client|bindings|example) ;; *) echo "build: argument target: invalid choice: '$tgt' (choose from 'all', 'web', 'host', 'server', 'client', 'bindings', 'example')" >&2; exit 2 ;; esac
        if [ -n "$image" ] && [ "$tgt" != server ]; then echo "build --image only supports build server (current target=$tgt)" >&2; exit 2; fi
        if [ "$native" = 1 ] && [ "$tgt" != server ]; then echo "build --native only supports build server (current target=$tgt)" >&2; exit 2; fi
        case "$tgt" in
            bindings) _ms_build_bindings "$release"; exit $? ;;
            example)  _ms_build_example "$release" "${names[@]}"; exit $? ;;
            all)    _ms_build_host; _ms_build_server "" 0 0; _ms_build_client ;;
            host)   _ms_build_host ;;
            server) _ms_build_server "$image" "$native" "$release" ;;
            client) _ms_build_client ;;
            web)    _ms_build_web ;;
        esac
        exit 0
        ;;
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
        # SR6 batch-beta-2: run example is bash-owned (server/host guidance
        # arms were batch-alpha).
        if [ "${2:-}" = example ]; then
            shift 2; _ms_run_example "$@"; exit $?
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
    e2e)
        # was _cmd_e2e + _E2E_SUITES (SR6 batch-beta-1). Honesty ledger: the
        # python face has crashed with NameError since c9d4acb (2026-08-27 —
        # ledger residual '_E2E_SUITES NameError'); the suite table is
        # restored from that face's last living shape. 'host' targeted
        # scripts/e2e-install-host.sh then — the file is e2e-deploy-host.sh today.
        # win32 branch dropped (this entry point is the unix shell).
        shift; suite="${1:-}"
        [ -z "$suite" ] && { echo "mediaservo e2e: error: the following arguments are required: suite" >&2; exit 2; }
        [ $# -gt 1 ] && { echo "mediaservo e2e: error: unrecognized arguments: ${*:2}" >&2; exit 2; }
        case "$suite" in
            sfu|push|ui|host|package|brand|bindings|client|smoke) ;;
            *) echo "mediaservo e2e: error: argument suite: invalid choice: '${suite}' (choose from sfu, push, ui, host, package, brand, bindings, client, smoke)" >&2; exit 2 ;;
        esac
        _ms_check cargo "pixi environment not activated?"
        _ms_check docker "the server container requires docker"
        _ms_check bash "e2e scripts require bash"
        case "$suite" in
            sfu)      cmd=(cargo test -p mediaservo-host --test e2e_sfu) ;;
            push)     cmd=(cargo test -p mediaservo-field --test push_e2e) ;;
            ui)       cmd=(bash -c "cd www/apps/admin && npx playwright test e2e") ;;
            host)     cmd=(bash scripts/e2e-deploy-host.sh) ;;
            package)  cmd=(bash scripts/e2e-package.sh) ;;
            brand)    cmd=(bash scripts/e2e-brand.sh) ;;
            bindings) cmd=(bash scripts/e2e-bindings.sh) ;;
            client)   cmd=(bash scripts/e2e-test.sh) ;;
            smoke)    cmd=(bash scripts/e2e-prod-smoke.sh) ;;
        esac
        echo "e2e $suite: ${cmd[*]}"
        _msvc_run "${cmd[@]}"   # MEDIASERVO_DRY_RUN=1 judges the command shape without firing suites
        exit $?
        ;;
    test)
        # bare = workspace tests (was _cmd_test); test example is bash-owned
        # since SR6 batch-beta-2.
        if [ "${2:-}" = example ]; then
            shift 2
            [ $# -le 1 ] || { echo "mediaservo test: error: unrecognized arguments: $*" >&2; exit 2; }
            _ms_test_example "${1:-}"; exit $?
        fi
        if [ $# -gt 1 ]; then
            echo "mediaservo test: error: argument what: invalid choice: '${2}' (choose from example)" >&2
            exit 2
        fi
        _ms_check cargo "pixi environment not activated?"
        echo "\$ cargo test --workspace --exclude mediaservo-server"
        exec cargo test --workspace --exclude mediaservo-server
        ;;
    package)
        # SR6 batch-beta-2: release-tar driver on bash (CPack legs + pack-legs
        # templates stay cmake-owned; wheel/pip mechanics are build bindings'
        # territory). Flags mirror the retired argparse (--dist/--brand/--release).
        shift; tgt="${1:-}"; [ $# -gt 0 ] && shift
        dist=""; brand=""; release=0
        while [ $# -gt 0 ]; do
            case "$1" in
                --dist) dist="${2:?package --dist needs a value}"; shift 2 ;;
                --brand) brand="${2:?package --brand needs a value}"; shift 2 ;;
                --release) release=1; shift ;;
                *) echo "package: unexpected argument $1" >&2; exit 2 ;;
            esac
        done
        [ -n "$tgt" ] || { echo "mediaservo package: error: the following arguments are required: target" >&2; exit 2; }
        _ms_package "$tgt" "$dist" "$brand" "$release"
        exit 0
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
                # W5-H (audit B1/F2): routed through the CWD-scoped helper — the
                # inline copy here was the UNSCOPED twin (live-fired 10-27 on a
                # /tmp prefix); _ms_deploy_stop_instance gates the family kill on
                # /proc/<pid>/cwd resolving inside $prefix.
                _ms_deploy_stop_instance "$prefix" "$brand"
                sleep 1
            fi
            _msvc_app_install host "$brand" "$build_type" "$prefix" ""
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
            # P2d-2 (U5): implementation retired into the cmake deploy target —
            # this shim resolves args and drives the same dedicated-binaryDir
            # choreography the IDE button uses (single implementation, F29/F32).
            src="${MSRTC_OUT_ROOT:-$ROOT/out}/bindings"
            [ -n "$prefix" ] || prefix="$src"
            if [ ! -d "$src/lib" ]; then echo "Error: no assembled artifacts in $src — run build bindings first" >&2; exit 1; fi
            _ms_deploy_via_target bindings "$release" "MEDIASERVO_DEPLOY_PREFIX_HOST=$prefix"
            echo "bindings deployed to $prefix (via cmake deploy target)"
            exit 0
            ;;
        server)
            shift 2; prefix=""
            while [ $# -gt 0 ]; do
                case "$1" in
                    --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
                    --release) shift ;;  # composite pass-through parity (server assembly is Debug-pinned, python parity)
                    *) echo "deploy server: unexpected argument $1" >&2; exit 2 ;;
                esac
            done
            [ -n "$prefix" ] || { echo "Error: deploy requires --prefix (no default — prevents polluting out/ stateless delivery; convention: /opt/mediaservo-server)" >&2; exit 2; }
            # D266 pre-flight (old-leg parity)
            [ -e "target/debug/mediaservo-server" ] || \
                { echo "Error: target/debug/mediaservo-server missing — run build server first (deploy does not trigger builds — D266)" >&2; exit 1; }
            # P2d-2 (U5): assembly+placement retired into the deploy-server target
            # (APP_STAGING=build ride included; fresh-first + web move inside)
            _ms_deploy_via_target server Debug "MEDIASERVO_DEPLOY_PREFIX_SERVER=$prefix" \
                || { echo "Error: deploy server target failed (rc=$?)" >&2; exit 1; }
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
    # build legs are shell-owned too now (SR6 batch-beta-2) — build re-enters
    # this shell, deploy rides the same arms.
    "$ROOT/mediaservo.sh" "${BUILD_FLAGS[@]}" && "$ROOT/mediaservo.sh" "${DEPLOY_FLAGS[@]}"
    exit $?
fi

echo "mediaservo: unknown verb '${1:-}' — ./mediaservo.sh -h lists the surface (the python CLI entry retired at SR6 batch-beta-2)" >&2
exit 2
