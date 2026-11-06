#!/usr/bin/env bash
# mediaservo.sh — MediaServo CLI thin shell (Linux/macOS)
# Duties: ① detect pixi (prompt bootstrap if missing) ② activate the environment ③ forward to the CLI
# Usage: ./mediaservo.sh <cmd> [-h]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

# v2: unified pixi detection — prefer command -v, fall back to ~/.pixi/bin (exports PIXI_BIN for pixi-shell)
if command -v pixi >/dev/null 2>&1; then
    PIXI_BIN="$(command -v pixi)"
    export PIXI_BIN
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
  build <target> [--release] [--image runtime|dev] [--native] [--profile auto|headless|linux-desktop] [--provider none|system|pixi] [--pixi-env NAME] [--ecosystem vcpkg|conan] [--pkg NAME] [--generate-ecosystem] [NAME...]
        target: all|web|host|server|client|bindings|example|gui-deps (default all;
        server = non-embedded variant + one-step web assembly; web =
        frontend-only fast lane; only --image takes the Docker path;
        build-host/build-server compat aliases kept; gui-deps = the explicit
        SDL3 build/install verb (sr8 P3), --profile/--provider/--pixi-env only
        apply to it -- provider pixi is the sr8 P4 fill, OFF by default;
        --ecosystem/--pkg/--generate-ecosystem are the sr8 P7c dev-only intake
        leg: acquire a library from the vcpkg/conan ecosystem instead of a
        source archive, from a committed recipe + pin island under
        cmake/gui-deps/. Strict-first, so nothing is fetched until
        --generate-ecosystem is given. It never combines with --provider/
        --pixi-env/--profile (those are the build-time/source axis -- a
        different question than where the library comes from), and it is not a
        route for SDL3/imgui, which carry a patch and a branch and stay on
        provider "source")
  build:deploy <target> [--prefix DIR] [--release]   (one-step build + deploy)
  package <target> [--dist DIR] [--brand NAME] [--release]
        target: host|server|sdk-field|sdk-client (bindings = one-cycle alias)
        → <dist>/<brand>-<target>-<ver>-<platform>-<arch>.tar.gz (D-H13/F12,
        versioned top-level dir; --dist defaults to dist/, the MSRTC release
        shell injects out/packages; unpack straight into a prefix with
        tar xzf pkg.tar.gz -C <prefix> --strip-components=1)
  run example <[band/]name> [args…]   test example [<band/]name>   list example
        (bands cxx|rs|py|c, bare name = cxx; foreground dev tool. cxx artifacts land in
        target/examples/bin/ — never in
        out/; the build-c prelude is auto-added. rs runs the cargo track (target/debug
        /examples or the IDE face's .cargo-target), py execs examples/py/<name>.py, c is
        e2e-compiled material; test = ctest roster (cxx: core, rs: example_<name>))
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

# _ms_deploy_via_target <target-suffix> <profile> [VAR=VAL ...] — P2d-2 forwarder:
# re-configures the dedicated deploy binaryDir with the caller's overrides, then
# drives the same custom target the IDE button uses. One implementation for all
# three faces (U5). B4/B5 (user-adjudicated A both): every suffix drives the
# body directly — single-pass, button-identical. Registration configure keeps
# the configure-time AppTree hunt OFF (explicit empty APP_TREE: cache-sticky
# residue from a previous body run self-fires the hunt before artifacts exist,
# PIT-189); the body's own configure re-activates AppTree with the fresh-goods
# artifacts present. The fa48b38 two-phase bin-build lane is retired: it was
# CLI-private choreography (the IDE button never ran it), its isolated
# .cargo-target duplicated what the body's fresh-goods leg builds, and its
# env path missed the MESON/MESON_ARGS unset that every other cargo driver
# carries — conda gcc activation exports MESON_ARGS=-Dbuildtype=release,
# which collides with mediasoup-sys's own --buildtype and panics the worker
# build (pixi.toml build-server-native precedent; measured 2026-11-06, the
# initially suspected memory-pressure theory superseded by this root cause).
# The identity self-check rode that lane and dies with it (button parity; the
# body's guard sequence keeps the single-writer proof).
_ms_deploy_via_target() {
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
        "${gates[@]}" -DMEDIASERVO_APP_TREE= \
        -DMEDIASERVO_DEPLOY_PROFILE="$profile" \
        -DMEDIASERVO_BRAND="${MEDIASERVO_BRAND:-}" "${extra[@]}" >&2 || return $?
    "$cmake_bin" --build "$ROOT/build/deploy-$suffix" \
        --target "mediaservo-deploy-$suffix" || return $?
    # B5 review follow-up (warn-only): the retired two-phase identity self-check
    # was the only detector for a fingerprint-fresh foreign byte-copy at target/
    # (invisible to cargo's incremental rebuild, which the body's fresh-goods leg
    # would then deploy). Visibility restored without resurrecting the lane; the
    # IDE button never had this probe, so CLI-only + warn-only is parity-safe.
    # Shape check, not content check: the version brand resolves as runtime env >
    # compile-time option_env bake > default (brand.rs D252 family), and the two
    # faces print differently — host prints a bare "<brand> <semver>" while
    # server prints "<brand>-server <semver>" (verified on-tree 2026-11-06), so
    # anchoring on the suffix would false-alarm every host deploy. Accept any
    # product token followed by a semantic version; a foreign binary's version
    # output will not coincidentally match that shape.
    if [ "$suffix" != bindings ]; then
        local main_bin
        main_bin="$ROOT/target/$(echo "$profile" | tr 'A-Z' 'a-z')/mediaservo-$suffix"
        if [ -x "$main_bin" ] && ! env -u MEDIASERVO_BRAND "$main_bin" version 2>/dev/null \
            | grep -qE '^[A-Za-z0-9][A-Za-z0-9_-]* [0-9]+\.[0-9]+\.[0-9]+$'; then
            echo "Warning: $main_bin version output anomalous — possible foreign byte-copy riding this deploy" >&2
        fi
    fi
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
        # B3: the caddy vendor copy and the etc seed (server.yaml template +
        # accounts/devices) are app-tree placement now (MediaServoAppTree.cmake
        # build-face legs, first-writer-wins) — the CLI seeding tail is gone.
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
# _cmd_run_example / _cmd_test_example). Directory contract (plan
# directory-artifacts-convergence batch ①, rulings C-D8/C-D9): the user-facing
# name is the directory under examples/<band>/ (default band cxx; a band prefix
# cxx|c|py|rs is accepted and validated); target/artifact carry the brand —
# dir + prefix == target == artifact (G4 upgraded, mediaservo-example-<dir>).
# F25 bridge: the cmake children get the SYSTEM constitution (conda build vars
# dropped, .pixi/envs bins scrubbed from PATH) — the one legit strip site;
# the band root's own probe reads the same signals (keep the lists aligned).
# =============================================================================
_MS_EXAMPLES_BUILD="$ROOT/target/examples/build"
_MS_EXAMPLES_BIN="$ROOT/target/examples/bin"
_ms_example_split() {  # $1 = [band/]name → prints "band<TAB>name", defaults cxx
    local spec=$1 band=cxx name=$1
    if [[ "$spec" == */* ]]; then band=${spec%%/*}; name=${spec#*/}; fi
    case "$band" in
        cxx|py|rs) ;;
        c) echo "[example] band 'c' is gcc material (examples/c/, compiled by scripts/e2e-bindings.sh; no CLI build/run)" >&2; return 2 ;;
        *) echo "[example] unknown band '$band' (cxx|c|py|rs)" >&2; return 2 ;;
    esac
    printf '%s\t%s' "$band" "$name"
}
_ms_example_dir() { printf '%s/examples/%s/%s' "$ROOT" "$1" "$2"; }  # band+name → directory
_MS_STRIP_ENV=(env -u CC -u CXX -u CFLAGS -u CXXFLAGS -u CPPFLAGS -u LDFLAGS -u CPATH -u C_INCLUDE_PATH -u CPLUS_INCLUDE_PATH -u CONDA_PREFIX -u CMAKE_PREFIX_PATH -u PKG_CONFIG_PATH -u PKG_CONFIG_SYSROOT_DIR -u COMPILER_PATH -u LIBRARY_PATH)
_ms_scrubbed_path() {
    printf '%s' "$PATH" | tr ':' '\n' | grep -v '/\.pixi/envs/' | paste -sd: -
}
_ms_bridge_env() {  # prints the PATH=... assignment arg for the _MS_STRIP_ENV prefix
    local sp; sp=$(_ms_scrubbed_path)
    if [ -n "$sp" ]; then echo "PATH=/usr/bin:$sp"; else echo "PATH=/usr/bin"; fi
}
_ms_examples_is_lib() {  # $1=name → prints 0/1; rc1 when the example is unknown (cxx band)
    local cml="$ROOT/examples/cxx/$1/CMakeLists.txt"
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
_ms_ensure_dev_cdylibs() {  # $1=out_dir $2=major
    # CMake configure can leave the dev-tree unversioned cargo outputs absent
    # while the deps/ real artifact remains. Restore exactly what the generated
    # install rows expect: target/{debug,release}/libmediaservo_<sdk>.so and the
    # dev-tree SONAME shim. Delivery layout itself is still cmake-owned.
    local out_dir=$1 major=$2 sdk base dep shim
    for sdk in field link deck client node; do
        base="$out_dir/libmediaservo_${sdk}.so"
        if [ "$sdk" = node ]; then
            shim=""
        else
            shim="$out_dir/libmediaservo_${sdk}.so.$major"
        fi
        if [ ! -e "$base" ]; then
            dep="$out_dir/deps/libmediaservo_${sdk}.so"
            if [ ! -e "$dep" ]; then
                dep="$(find "$out_dir/deps" -maxdepth 1 -name "libmediaservo_${sdk}-*.so" -type f -print -quit 2>/dev/null || true)"
            fi
            if [ -n "$dep" ] && [ -e "$dep" ]; then
                cp -p "$dep" "$base"
            fi
        fi
        if [ -n "$shim" ] && [ -e "$base" ] && [ ! -e "$shim" ]; then
            ln -s "libmediaservo_${sdk}.so" "$shim"
        fi
    done
}

_ms_build_bindings() {  # $1=release(0/1)
    _ms_check cargo "pixi environment not activated? Run first: source bootstrap.sh / pixi.bat"
    local staging="${MSRTC_BUILD_ROOT:-$ROOT/build}/staging/bindings" sdk
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
    _ms_ensure_dev_cdylibs "$out_dir" "$major"
    echo "\$ cmake --install build/presets/bindings --prefix $bind_dst --strip  (cwd: $ROOT)"
    (cd "$ROOT" && cmake --install build/presets/bindings --prefix "$bind_dst" --strip) || exit $?
    _ms_build_pywheel "$out_dir" "$major" "$staging" "$bind_dst" || exit $?
    local n_docs n_lib
    n_docs=$(ls "$bind_dst"/docs/sdk-cxx/*.md 2>/dev/null | wc -l)
    n_lib=$(find "$bind_dst/lib" -maxdepth 1 -name 'libmediaservo_*' 2>/dev/null | wc -l || true)
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
    # mirror of the pip build root — zero writes into the source tree; py examples
    # live repo-side at examples/py/ since batch ③, the rm stays defensive)
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
    # shellcheck disable=SC2100  # false positive: hyphenated literal assignment inside case body, not arithmetic
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
    # shellcheck disable=SC2010  # /proc entries are kernel-controlled numeric names; the ls|grep scan idiom is safe here
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
# sr8 P3 -- GUI dependency face: the SDL3 install prefix is an EXPLICIT-
# VERB artifact (configure never builds, H2). The ImGui wrapper is consumer-
# compiled (fetch_only recipe) and needs no verb. Profile auto = the same
# seven-header probe as cmake/MediaServoGuiDeps.cmake (keep the lists aligned).
_ms_x11_headers_present() {
    local h
    for h in X11/Xlib.h X11/XKBlib.h X11/extensions/Xfixes.h X11/extensions/XInput2.h \
             X11/extensions/Xrandr.h X11/Xcursor/Xcursor.h X11/extensions/Xext.h; do
        [ -f "/usr/include/$h" ] || return 1
    done
    return 0
}
_ms_pixi_present() {
    command -v pixi >/dev/null 2>&1 && return 0
    [ -x "$HOME/.pixi/bin/pixi" ]
}
_ms_build_eco_deps() {  # $1=release(0/1) $2=ecosystem(vcpkg|conan) $3=pkg $4=generate(0/1)
    # sr8 P7c: the dev-only ecosystem intake leg. The ACQUIRE axis (--ecosystem)
    # is a different thing from the BUILD-time axis (--provider), so the two
    # never combine, and the lane has no desktop profile to speak of (SDL3 and
    # imgui stay on provider "source": patch/branch, O-13/M5).
    local release=$1 eco=$2 pkg=$3 gen=$4
    local bt=Debug; [ "$release" = 1 ] && bt=Release
    if [ -z "$pkg" ]; then
        echo "[gui-deps] --ecosystem needs --pkg <name> (the recipe and the pin island are named after it)" >&2; return 2
    fi
    local rec="$ROOT/cmake/gui-deps/recipes/$pkg-$eco.json"
    if [ ! -f "$rec" ]; then
        echo "[gui-deps] no recipe cmake/gui-deps/recipes/$pkg-$eco.json (existing: $(ls "$ROOT/cmake/gui-deps/recipes" 2>/dev/null | sed 's/\.json$//' | tr '\n' ' '))" >&2; return 2
    fi
    local anchor="${MSRTC_BUILD_ROOT:-$ROOT/build}/3rdparty/ecosystem"
    local gargs=(); [ "$gen" = 1 ] && gargs=(-DGENERATE=ON)
    echo "[gui-deps] ecosystem=$eco pkg=$pkg config=$bt generate=$gen anchor=$anchor"
    (cd "$ROOT" && "${_MS_STRIP_ENV[@]}" "$(_ms_bridge_env)" \
        cmake -DECOSYSTEM="$eco" -DPKG="$pkg" -DBUILD_TYPE="$bt" -DWORK_DIR="$anchor/$pkg-$eco" \
              "${gargs[@]}" -P "$ROOT/cmake/gui-deps/ecosystem.cmake") || exit $?
}

_ms_build_gui_deps() {  # $1=release(0/1) $2=profile(auto|headless|linux-desktop) $3=provider(none|system|pixi) $4=pixi-env
    local release=$1 profile=$2 provider=${3:-none} pixi_env=${4:-gui-deps}
    local bt=Debug; [ "$release" = 1 ] && bt=Release
    if [ "$profile" = auto ]; then
        if _ms_x11_headers_present; then profile=linux-desktop; else profile=headless; fi
    fi
    case "$provider" in none|system|pixi) ;; *) echo "[gui-deps] unknown --provider '$provider' (none|system|pixi)" >&2; return 2 ;; esac
    # System-fixed tools (F25 discipline): the SDL child configure runs under
    # the sanitized PATH, so a ninja that only lives in the activated pixi env
    # would vanish there (measured: project() then dies with a misleading
    # "CMAKE_C_COMPILER not set"). Pin absolute paths; ninja only if SYSTEM.
    local gen="" mk=""
    if [ -x /usr/bin/ninja ]; then gen=Ninja; mk=/usr/bin/ninja
    elif [ -x /usr/bin/make ]; then gen="Unix Makefiles"; mk=/usr/bin/make
    else echo "[gui-deps] neither /usr/bin/ninja nor /usr/bin/make found -- the SDL build needs a generator" >&2; return 1; fi
    # F10 dual-track: the shell resolves MSRTC_BUILD_ROOT here; the CMake side
    # pins the same default from the repo root (no env reads there).
    # Provider identity rides the work-dir tag (the cache key already carries
    # it; the paths must not alias either -- P4). Default none keeps the P3
    # layout byte-for-byte (F3=A: pixi is implemented, never the default).
    local anchor="${MSRTC_BUILD_ROOT:-$ROOT/build}/3rdparty"
    local ptag=""
    [ "$provider" != "none" ] && ptag="-$provider"
    local provargs=()
    if [ "$provider" = "pixi" ]; then
        # the explicit-compiler discipline (r4 C1): the system pair, absolute
        if [ ! -x /usr/bin/cc ] || [ ! -x /usr/bin/c++ ]; then
            echo "[gui-deps] provider pixi needs the system compilers at /usr/bin/cc and /usr/bin/c++ (the provider never lets the child discover a compiler)" >&2
            return 1
        fi
        if ! _ms_pixi_present; then
            echo "[gui-deps] provider pixi needs the pixi binary (bootstrap it first: ./bootstrap.sh)" >&2
            return 1
        fi
        provargs=(-DPROVIDER=pixi -DPIXI_ENV="$pixi_env" -DCOMPILER_C=/usr/bin/cc -DCOMPILER_CXX=/usr/bin/c++)
    elif [ "$provider" = "system" ]; then
        provargs=(-DPROVIDER=system)
    fi
    echo "[gui-deps] profile=$profile provider=$provider config=$bt anchor=$anchor generator=$gen"
    (cd "$ROOT" && "${_MS_STRIP_ENV[@]}" "$(_ms_bridge_env)" \
        cmake -DPROFILE="$profile" -DBUILD_TYPE="$bt" -DWORK_DIR="$anchor/$profile$ptag" \
              -DGENERATOR="$gen" -DMAKE_PROGRAM="$mk" -DPARALLEL="${MEDIASERVO_GUI_DEPS_PARALLEL:-}" \
              "${provargs[@]}" \
              -P "$ROOT/cmake/gui-deps/build.cmake") || exit $?
}

_ms_build_example() {  # $1=release(0/1); rest = example names (empty = all)
    local release=$1; shift
    local profile=debug; [ "$release" = 1 ] && profile=release
    # build-c prelude (PLAN G7; auto-added when missing — freshness is cargo's job)
    if [ ! -f "$ROOT/target/$profile/libmediaservo_client.so" ]; then
        echo "[example] libmediaservo_client.so missing → running build-c first" >&2
        (cd "$ROOT" && "${PIXI_BIN:-pixi}" run build-c) || exit $?
    fi
    # GUI dependency prelude (sr8 P3): the SDL3 install prefix is an explicit-
    # verb artifact and configure never builds (H2). Auto-run the verb when the
    # prefix for the effective profile/config is missing; the verb itself is
    # manifest-idempotent, so a present prefix costs two stats.
    local _gdprof=linux-desktop
    _ms_x11_headers_present || _gdprof=headless
    local _gdbt=Debug; [ "$release" = 1 ] && _gdbt=Release
    if [ ! -f "${MSRTC_BUILD_ROOT:-$ROOT/build}/3rdparty/$_gdprof/sdl3/install-$_gdbt/lib/cmake/SDL3/SDL3Config.cmake" ]; then
        echo "[example] SDL3 install prefix missing → running build gui-deps first" >&2
        _ms_build_gui_deps "$release" "$_gdprof" || exit $?
    fi
    local n spec split b
    local _enames=() _rsnames=()
    if [ $# -gt 0 ]; then
        for spec in "$@"; do
            split=$(_ms_example_split "$spec") || exit 2
            b=${split%%$'\t'*}; n=${split#*$'\t'}
            if [ "$b" = py ]; then
                echo "[example] py band needs no build (run: run example py/$n)" >&2
                continue
            fi
            if [ "$b" = rs ]; then
                _rsnames+=("$n")   # batch ④: cargo track, no cmake roundtrip
                continue
            fi
            if ! _ms_examples_is_lib "$n" > /dev/null; then
                echo "[example] unknown example '$n' (see ./mediaservo.sh list example)" >&2
                exit 2
            fi
            _enames+=("$n")
        done
    fi
    if [ ${#_rsnames[@]} -gt 0 ]; then
        local rargs=(); for n in "${_rsnames[@]}"; do rargs+=(--example "$n"); done
        echo "\$ cargo build -p examples ${rargs[*]}  (cwd: $ROOT)"
        (cd "$ROOT" && cargo build -p examples "${rargs[@]}") || exit $?
    fi
    # names given but nothing buildable (all py/rs) → no accidental cxx full build
    if [ $# -gt 0 ] && [ ${#_enames[@]} -eq 0 ]; then
        [ ${#_rsnames[@]} -eq 0 ] && echo "[example] no buildable cxx names in arguments (py runs directly, rs builds via cargo track)"
        return 0
    fi
    local gen=Debug; [ "$release" = 1 ] && gen=Release
    # main-tree bridge (batch ① design fact): the link faces mediaservo::<sdk> are
    # registered by the bindings lanes at the repo root — configuring the band alone
    # cannot resolve them anymore. Artifacts land at the same target/examples/bin.
    # The scrubbed PATH drops cargo (F25), so the supplier's official injection pair
    # points discovery at the pixi toolchain — executables only, PATH stays clean.
    local rustinj=()
    if [ -x "$ROOT/.pixi/envs/default/bin/cargo" ]; then
        rustinj=(-DPolyOrch_RUST_CARGO_EXECUTABLE="$ROOT/.pixi/envs/default/bin/cargo" -DPolyOrch_RUSTC_EXECUTABLE="$ROOT/.pixi/envs/default/bin/rustc")
    fi
    local cfgargs=(-S "$ROOT" -B "$_MS_EXAMPLES_BUILD" "-DCMAKE_BUILD_TYPE=$gen" "-DMEDIASERVO_BUILD_EXAMPLES=ON" "-DMEDIASERVO_BUILD_BINDINGS=ON" "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON" "-DPolyOrch_VSCODE_DEBUG_ALL=ON" "${rustinj[@]}")
    # stale island-style cache (different source dir) = wipe, or cmake hard-errors
    if [ -f "$_MS_EXAMPLES_BUILD/CMakeCache.txt" ] && ! grep -q "^CMAKE_HOME_DIRECTORY:INTERNAL=$ROOT$" "$_MS_EXAMPLES_BUILD/CMakeCache.txt"; then
        rm -rf "$_MS_EXAMPLES_BUILD"
    fi
    # no -G: scrubbed PATH may lack ninja (pixi-only bin); cmake picks its
    # platform default (Makefiles on bare Linux) — same auto-adapt e2e-delivery uses.
    if [ ! -f "$_MS_EXAMPLES_BUILD/CMakeCache.txt" ] || [ "$profile" != debug ]; then
        echo "\$ cmake ${cfgargs[*]}  (cwd: $ROOT)"
        (cd "$ROOT" && "${_MS_STRIP_ENV[@]}" "$(_ms_bridge_env)" cmake "${cfgargs[@]}") || exit $?
    fi
    local cmd=(cmake --build "$_MS_EXAMPLES_BUILD")
    if [ ${#_enames[@]} -gt 0 ]; then
        # C-D9: user names map to branded targets (dir == name, target = mediaservo-example-<name>)
        local _bn targs=()
        for _bn in "${_enames[@]}"; do targs+=("mediaservo-example-${_bn}"); done
        cmd+=(--target "${targs[@]}")
    else
        cmd+=(--parallel)
    fi
    echo "\$ ${cmd[*]}  (cwd: $ROOT)"
    (cd "$ROOT" && "${_MS_STRIP_ENV[@]}" "$(_ms_bridge_env)" "${cmd[@]}") || exit $?
    echo "[example] artifacts → target/examples/bin/"
}
_ms_run_example() {  # $@ = <name> [passthrough…] — foreground dev tool (distinct from the retired host-run C39 semantics)
    if [ $# -eq 0 ]; then
        echo "Usage: run example <name> [args…] (./mediaservo.sh list example)" >&2
        exit 2
    fi
    local spec=$1; shift
    local _split
    _split=$(_ms_example_split "$spec") || exit 2
    local name=${_split#*$'\t'}
    # py band (batch ③): plain-script execution, nothing to build (wheel/dev import
    # is the script's own sys.path contract; export MEDIASERVO_LIB_DIR for dev mode)
    if [ "${_split%%$'\t'*}" = py ]; then
        exec python3 "$ROOT/examples/py/$name.py" "$@"
    fi
    # rs band (batch ④): cargo track straight on the examples package (build+run
    # in one — works from any shell that has cargo; the CMake face adds IDE buttons)
    if [ "${_split%%$'\t'*}" = rs ]; then
        cd "$ROOT" && exec cargo run -p examples --example "$name" -- "$@"
    fi
    local islib
    islib=$(_ms_examples_is_lib "$name") || { echo "[example] unknown example '$name'" >&2; exit 2; }
    _ms_build_example 0 "$name"
    if [ "$islib" = 1 ]; then
        echo "[example] '$name' is a library (no runnable artifact)" >&2
        exit 1
    fi
    local exe="$_MS_EXAMPLES_BIN/mediaservo-example-$name"   # C-D9 branded artifact
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
_ms_test_example() {  # $1 = optional [band/]name (C-D8) | bare ctest regex; missing build dir = build first
    local spec="${1:-}" filter
    if [ -z "$spec" ]; then
        filter='^core$'                     # cxx band roster today (imgui_shell's core test)
    elif [ "$spec" = rs ]; then
        filter='^example_'                  # band-only = the whole rs roster
    elif [ "$spec" = cxx ]; then
        filter='^core$'
    elif [ "$spec" = py ] || [ "$spec" = c ]; then
        echo "[example] band '$spec' has no ctest roster (py: run example py/<name>; c: scripts/e2e-bindings.sh)" >&2
        return 0
    elif [[ "$spec" != */* ]]; then
        filter=$spec                         # bare word = raw ctest -R (pre-band contract kept)
    else
        local split b n
        split=$(_ms_example_split "$spec") || exit 2
        b=${split%%$'\t'*}; n=${split#*$'\t'}
        case "$b" in
            rs)  filter="^example_${n}\$" ;;   # rs test names = example_<registry name>
            cxx) filter="^${n}\$" ;;
            py)  echo "[example] py band has no ctest roster (run: run example py/$n)" >&2; return 0 ;;
        esac
    fi
    if [ ! -f "$_MS_EXAMPLES_BUILD/CTestTestfile.cmake" ]; then
        _ms_build_example 0
    fi
    local cmd=(ctest --test-dir "$_MS_EXAMPLES_BUILD" --output-on-failure -R "$filter")
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
        tgt=""; image=""; native=0; release=0; names=(); seen_tgt=0; gd_profile=""; gd_provider=""; gd_pixi_env=""; gd_eco=""; gd_pkg=""; gd_generate=0
        while [ $# -gt 0 ]; do
            case "$1" in
                --release) release=1; shift ;;
                --native) native=1; shift ;;
                --provider) gd_provider="${2:?build --provider needs a value}"
                            case "$gd_provider" in none|system|pixi) ;; *) echo "build: argument --provider: invalid choice: '$gd_provider' (choose from 'none', 'system', 'pixi')" >&2; exit 2 ;; esac
                            shift 2 ;;
                --pixi-env) gd_pixi_env="${2:?build --pixi-env needs a value}"; shift 2 ;;
                --ecosystem) gd_eco="${2:?build --ecosystem needs a value}"
                             case "$gd_eco" in vcpkg|conan) ;; *) echo "build: argument --ecosystem: invalid choice: '$gd_eco' (choose from 'vcpkg', 'conan')" >&2; exit 2 ;; esac
                             shift 2 ;;
                --pkg) gd_pkg="${2:?build --pkg needs a value}"; shift 2 ;;
                --generate-ecosystem) gd_generate=1; shift ;;
                --profile) gd_profile="${2:?build --profile needs a value}"
                           case "$gd_profile" in auto|headless|linux-desktop) ;; *) echo "build: argument --profile: invalid choice: '$gd_profile' (choose from 'auto', 'headless', 'linux-desktop')" >&2; exit 2 ;; esac
                           shift 2 ;;
                --image) image="${2:?build --image needs a value}"
                         case "$image" in runtime|dev) ;; *) echo "build: argument --image: invalid choice: '$image' (choose from 'runtime', 'dev')" >&2; exit 2 ;; esac
                         shift 2 ;;
                -*) echo "build: unrecognized flag $1" >&2; exit 2 ;;
                *) if [ $seen_tgt = 0 ]; then tgt="$1"; seen_tgt=1; else names+=("$1"); fi; shift ;;
            esac
        done
        tgt="${tgt:-all}"
        case "$tgt" in all|web|host|server|client|bindings|example|gui-deps) ;; *) echo "build: argument target: invalid choice: '$tgt' (choose from 'all', 'web', 'host', 'server', 'client', 'bindings', 'example', 'gui-deps')" >&2; exit 2 ;; esac
        if [ -n "$gd_profile" ] && [ "$tgt" != gui-deps ]; then echo "build --profile only supports build gui-deps (current target=$tgt)" >&2; exit 2; fi
        if [ -n "$gd_provider" ] && [ "$tgt" != gui-deps ]; then echo "build --provider only supports build gui-deps (current target=$tgt)" >&2; exit 2; fi
        if [ -n "$gd_pixi_env" ] && [ "$tgt" != gui-deps ]; then echo "build --pixi-env only supports build gui-deps (current target=$tgt)" >&2; exit 2; fi
        if [ -n "$gd_eco" ] && [ "$tgt" != gui-deps ]; then echo "build --ecosystem only supports build gui-deps (current target=$tgt)" >&2; exit 2; fi
        if [ "$gd_generate" = 1 ] && [ -z "$gd_eco" ]; then echo "build --generate-ecosystem only accompanies --ecosystem (without it nothing is generated, so there is nothing to opt into)" >&2; exit 2; fi
        # two axes, never both (r7 H1): --provider/--pixi-env shape the child env
        # of a source build; --ecosystem lets the tool build the library itself.
        # --profile is the desktop-profile axis of the source lane, refused by
        # the ecosystem lane at the face too -- same reason, stated once here.
        if [ -n "$gd_eco" ]; then
            for _gd_axis in "--provider=$gd_provider" "--pixi-env=$gd_pixi_env" "--profile=$gd_profile"; do
                case "$_gd_axis" in *="") continue ;; esac
                echo "build: $_gd_axis is the build-time/source axis and does not combine with --ecosystem (the library builds inside $gd_eco); drop one of the two" >&2; exit 2
            done
        fi
        if [ -n "$image" ] && [ "$tgt" != server ]; then echo "build --image only supports build server (current target=$tgt)" >&2; exit 2; fi
        if [ "$native" = 1 ] && [ "$tgt" != server ]; then echo "build --native only supports build server (current target=$tgt)" >&2; exit 2; fi
        case "$tgt" in
            bindings) _ms_build_bindings "$release"; exit $? ;;
            example)  _ms_build_example "$release" "${names[@]}"; exit $? ;;
            gui-deps) if [ -n "$gd_eco" ]; then _ms_build_eco_deps "$release" "$gd_eco" "$gd_pkg" "$gd_generate"; exit $?; fi
                      _ms_build_gui_deps "$release" "${gd_profile:-auto}" "${gd_provider:-none}" "${gd_pixi_env:-gui-deps}"; exit $? ;;
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
        # example roster per band (C-D8). cxx = the G4 dirs (dir == CLI arg; target/
        # artifact carry the mediaservo-example- brand). rs = the examples package's
        # own [[example]] table, sed-read so cargo stays the single source of truth
        # (the CMake band registry mirrors the same names). py = examples/py/*.py.
        [ "${2:-}" = example ] || { echo "list: usage: list example" >&2; exit 2; }
        ed="$ROOT/examples/cxx"
        names=()
        for cml in "$ed"/*/CMakeLists.txt; do
            [ -f "$cml" ] || continue
            names+=("$(basename "$(dirname "$cml")")")
        done
        echo "cxx band (dir name = CLI arg; target/artifact carry mediaservo-example- brand; [lib] = non-executable):"
        if [ ${#names[@]} -eq 0 ]; then
            echo "  (no examples in examples/cxx yet)" >&2
        else
            for name in "${names[@]}"; do
                if grep -q "add_library(" "$ed/$name/CMakeLists.txt"; then tag="[lib]"
                elif [ -e "$ROOT/target/examples/bin/mediaservo-example-$name" ]; then tag="[built]"
                else tag="[not built]"; fi
                printf '  %-24s %s\n' "$name" "$tag"
            done
        fi
        echo "rs band (cargo track — run: run example rs/<name>; CMake face: examples/CMakeLists.txt registry)"
        for name in $(sed -n '/^\[\[example\]\]/{n;s/^name *= *"\(.*\)"/\1/p}' "$ROOT/examples/rs/Cargo.toml"); do
            # the CMake face stages into the build tree's isolated .cargo-target; a
            # plain `cargo build -p examples` lands in target/debug/examples
            if [ -e "$_MS_EXAMPLES_BUILD/.cargo-target/debug/examples/$name" ] ||
               [ -e "$ROOT/target/debug/examples/$name" ]; then tag="[built]"
            else tag="[not built]"; fi
            printf '  %-24s %s\n' "$name" "$tag"
        done
        echo "py band (run: run example py/<name> — no build step)"
        for f in "$ROOT"/examples/py/*.py; do
            [ -e "$f" ] || continue
            printf '  %-24s %s\n' "$(basename "$f" .py)" "[script]"
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
                # shellcheck disable=SC2010  # sort-then-first-wins contract over deployed bin/ (incl. .exe.bak skips); glob rewrite deferred until live-verifiable
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
            # shellcheck disable=SC2010  # sort-then-first-wins contract over deployed bin/ (incl. .exe.bak skips); glob rewrite deferred until live-verifiable
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
                mkdir -p "$base/etc" "$base/logs"; logs_d="$base/logs"
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
            case "$prefix" in
                /opt*) [ "$(id -u)" = 0 ] || { echo "Error: /opt deployment needs root (sudo msrtc.sh deploy host --prefix /opt/mediaservo-host)" >&2; exit 1; } ;;
            esac
            host_cli=${brand:+${brand}-host}; host_cli=${host_cli:-mediaservo-host}
            # B4 (cli-cmake-single-source, user-adjudicated A): the staging body —
            # guard TEARDOWN (triple stop), placement, oxmgr vendor, init
            # (identity/tokens), env.sh, run dirs and the PIT-119 strip — lives in
            # the deploy-host target (host-legs.cmake.in 1:1 port; R10-K-a teardown
            # parity). The fa48b38 keep-on-bash reservation is paid off: the
            # two-phase single-writer was proven under the bindings/server forwards.
            # This shim keeps registration-desk duties only: parse (above), the root
            # gate, brand derivation, explicit BRAND hand-off, receipt (below).
            export MEDIASERVO_BRAND="$rust_brand"   # always-explicit (fa48b38); init env-override parity
            _ms_deploy_via_target host "$build_type" "MEDIASERVO_DEPLOY_PREFIX_HOST=$prefix" \
                || { echo "Error: deploy host target failed (rc=$?)" >&2; exit 1; }
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
            # M6: the bindings prefix rides the host-named cmake channel
            # (MEDIASERVO_DEPLOY_PREFIX_HOST) — bindings reuses it by contract
            # (DeployTargets dedicated binaryDir); rename together only.
            _ms_deploy_via_target bindings "$release" "MEDIASERVO_DEPLOY_PREFIX_HOST=$prefix"
            echo "bindings deployed to $prefix (via cmake deploy target)"
            exit 0
            ;;
        server)
            shift 2; prefix=""; build_type=Debug
            while [ $# -gt 0 ]; do
                case "$1" in
                    --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
                    --release) build_type=Release; shift ;;  # B5: server joined the DEPLOY_PROFILE channel (U6) — host/bindings twin
                    *) echo "deploy server: unexpected argument $1" >&2; exit 2 ;;
                esac
            done
            [ -n "$prefix" ] || { echo "Error: deploy requires --prefix (no default — prevents polluting out/ stateless delivery; convention: /opt/mediaservo-server)" >&2; exit 2; }
            # (no pre-flight: the body's fresh-goods leg builds the goods itself —
            # D266's "deploy does not trigger builds" retired with the fresh-goods
            # body, B4/B5 button-identical)
            # P2d-2 (U5): assembly+placement retired into the deploy-server target
            # (APP_STAGING=build ride included; fresh-first + web move inside)
            _ms_deploy_via_target server "$build_type" "MEDIASERVO_DEPLOY_PREFIX_SERVER=$prefix" \
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
# B2 single-source (cli-cmake-single-source adjudication 1=A): the composite is now parent-agnostic —
# OUT_ROOT honors the parent-shell MSRTC_OUT_ROOT injection when colocated, else caller env, else repo out/;
# --brand rides the deploy leg only (the build face has no brand arg — bare pass-through used to make build die
# on the unexpected argument); relative --prefix is anchored to the caller's cwd (parent _abs_path parity, r1-M3);
# missing target now prints usage + exit 2 instead of falling into the unknown-verb echo (r1-M2: verb is known,
# what is missing is the target).
if [ "${1:-}" = "build:deploy" ]; then
    if [ $# -lt 2 ]; then
        echo "usage: mediaservo build:deploy <target> [--prefix DIR] [--brand NAME] [--release] [build-args...]" >&2
        exit 2
    fi
    TARGET="$2"
    shift 2
    OUT_ROOT="${MSRTC_OUT_ROOT:-${OUT_ROOT:-$ROOT/out}}"   # parent-shell injection first, caller env next, repo out/ last
    BUILD_FLAGS=(build "$TARGET")
    DEPLOY_FLAGS=(deploy "$TARGET" --prefix "$OUT_ROOT/$TARGET")
    while [ $# -gt 0 ]; do
        case "$1" in
            --prefix)
                case "$2" in
                    /*) DEPLOY_FLAGS+=(--prefix "$2") ;;      # already absolute
                    *)  DEPLOY_FLAGS+=(--prefix "$PWD/$2") ;;  # caller-cwd anchored
                esac
                shift 2 ;;   # explicit override of the default
            --brand) DEPLOY_FLAGS+=(--brand "$2"); shift 2 ;;   # deploy leg only
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
