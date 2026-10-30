#!/usr/bin/env bash
# weaknet.sh — mediaservo-weaknet entry resolver (moved out of the parent shell, P1-7).
#
# Order: WEAKNET_BIN > PATH > <out>/server/bin/{mediaservo-weaknet, <brand>-weaknet}
#        > <out>/server/bin/*-weaknet > <repo>/target/{release,debug}.
# stderr prints the resolved path; a missing binary exits 2.
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
BRAND="${MEDIASERVO_BRAND:-mediaservo}"
OUT_ROOT="${MSRTC_OUT_ROOT:-$ROOT/out}"
PIXI="${PIXI_BIN:-$(command -v pixi 2>/dev/null || true)}"
if [ -z "$PIXI" ] && [ -x "$HOME/.pixi/bin/pixi" ]; then
    PIXI="$HOME/.pixi/bin/pixi"
fi

if [ $# -eq 0 ] || [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
    cat <<'WEOF'
usage: scripts/weaknet.sh <apply|set|scenario|status|clear|serve|up|down|watch> [...]
  apply --rtt 80 --loss 2 --rtp-port 40010[,...] [--duration 300] [--dry-run]
  up smoke            # = apply --profile smoke (default 300s fuse)
  set --rate 4 | status | clear (=down)
  scenario list | run|stop
  build-aarch64       # cross-compile for the vehicle (T13)
entry: WEAKNET_BIN > PATH > out/server/bin/{mediaservo-weaknet,<brand>-weaknet} > target/{release,debug}
channel: WEAKNET_CHANNEL=auto|local|sidecar
exit codes: 0/2/3/4
WEOF
    exit 0
fi

if [ "${1:-}" = "build-aarch64" ]; then
    shift
    [ -n "$PIXI" ] || { echo "weaknet: pixi not found (run bootstrap)" >&2; exit 127; }
    cd "$ROOT"
    # Pure-Rust cross build: clear RUSTFLAGS so the x86 glib/lld link args from
    # the activation do not leak into the aarch64 link.
    if RUSTFLAGS= "$PIXI" run cargo build --release --target aarch64-unknown-linux-gnu \
        -p mediaservo-weaknet "$@"; then
        SRC="target/aarch64-unknown-linux-gnu/release/mediaservo-weaknet"
        OUT="$OUT_ROOT/weaknet-aarch64"
        mkdir -p "$OUT"
        cp "$SRC" "$OUT/${BRAND}-weaknet"
        file "$OUT/${BRAND}-weaknet"
        echo "[weaknet] artifact: $OUT/${BRAND}-weaknet"
    else
        RC=$?
        echo "[weaknet] build-aarch64 failed rc=$RC (cross toolchain missing; see docs/modules/development/weaknet-harness.md)" >&2
        exit "$RC"
    fi
    exit 0
fi

WB="${WEAKNET_BIN:-}"
if [ -z "$WB" ]; then
    WB="$(command -v mediaservo-weaknet 2>/dev/null || true)"
fi
if [ -z "$WB" ]; then
    WN_OUT="$OUT_ROOT/server/bin"
    for candidate in "$WN_OUT/mediaservo-weaknet" "$WN_OUT/${BRAND}-weaknet"; do
        [ -f "$candidate" ] && [ -x "$candidate" ] && WB="$candidate" && break
    done
    if [ -z "$WB" ]; then
        for candidate in "$WN_OUT"/*-weaknet; do
            [ -f "$candidate" ] && [ -x "$candidate" ] && WB="$candidate" && break
        done
    fi
fi
if [ -z "$WB" ]; then
    for candidate in "$ROOT/target/release/mediaservo-weaknet" "$ROOT/target/debug/mediaservo-weaknet"; do
        [ -x "$candidate" ] && WB="$candidate" && break
    done
fi
if [ -z "$WB" ]; then
    echo "[weaknet] binary not found: build it (pixi run cargo build -p mediaservo-weaknet) or set WEAKNET_BIN" >&2
    exit 2
fi
if [ ! -x "$WB" ]; then
    echo "[weaknet] not executable: $WB" >&2
    exit 2
fi
echo "weaknet -> $WB" >&2
exec "$WB" "$@"
