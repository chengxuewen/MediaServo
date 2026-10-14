#!/usr/bin/env bash
# V5 delivery-surface smoke (the pure-library-consumption lightweight twin of e2e-package): after out/bindings assembly,
# ① component purity (link/field lightweight components must not have FFmpeg DT_NEEDED = V1 sdk-field componentization dependency assertion)
# ② find_package COMPONENTS link+client: compile, link and run against real symbols (V2 drill, full form in CI)
# ③ pkg-config version == workspace actual value (C43 parity, extended to the delivery surface)
# Prereq: python3 scripts/mediaservo_cli.py build bindings (this script runs it itself; zero extra CI steps).
set -euo pipefail
cd "$(dirname "$0")/.."
TMP=$(mktemp -d /tmp/ms-delivery.XXXXXX)
trap 'rm -rf "$TMP"' EXIT
FAIL=0

python3 scripts/mediaservo_cli.py build bindings >/dev/null
OUT="$PWD/out/bindings"
VER=$(python3 -c 'import tomllib;print(tomllib.load(open("Cargo.toml","rb"))["workspace"]["package"]["version"])')

# ① Lightweight components: FFmpeg immunity
for lib in link field; do
    if readelf -d "$OUT/lib/libmediaservo_$lib.so" | grep -qE "NEEDED.*(avformat|avcodec|swresample)"; then
        echo "FAIL: lightweight component libmediaservo_$lib.so has FFmpeg in DT_NEEDED (lightweight surface polluted)"; FAIL=1
    fi
done
[ "$FAIL" = 0 ] && echo "OK: no FFmpeg DT_NEEDED in link/field (deck/sdk surface is exempt)"

# ② find_package three surfaces
mkdir -p "$TMP/pkg"
cat > "$TMP/pkg/CMakeLists.txt" <<'CM'
cmake_minimum_required(VERSION 3.16)
project(delivery_smoke LANGUAGES C)
find_package(mediaservo REQUIRED COMPONENTS link client)
add_executable(smoke main.c)
target_link_libraries(smoke PRIVATE mediaservo::link mediaservo::client)
CM
cat > "$TMP/pkg/main.c" <<'C'
#include <mediaservo/link.h>
#include <mediaservo/client.h>
/* Symbol-existence criterion (proved at link time; not called — avoids network/device dependencies). */
int main(void) {
    void *a = (void *)mediaservo_link_version;
    void *b = (void *)mediaservo_client_login;
    return (a != 0 && b != 0) ? 0 : 1;
}
C
# Generator auto-adapt (pixi has ninja but no make / ubuntu runner the other way)
GEN=""
command -v make >/dev/null 2>&1 || GEN="Ninja"
cmake -S "$TMP/pkg" -B "$TMP/pkg/build" ${GEN:+-G "$GEN"} -DCMAKE_PREFIX_PATH="$OUT/lib/cmake/mediaservo" >/dev/null
cmake --build "$TMP/pkg/build" >/dev/null
LD_LIBRARY_PATH="$OUT/lib" "$TMP/pkg/build/smoke" && echo "OK: find_package(link,client) + real-symbol link & run"

# ③ pkg-config version reconciliation
for pc in mediaservo-link mediaservo-client mediaservo-deck mediaservo-field; do
    got=$(PKG_CONFIG_PATH="$OUT/lib/pkgconfig" pkg-config --modversion "$pc")
    [ "$got" = "$VER" ] || { echo "FAIL: $pc version $got != workspace $VER"; FAIL=1; }
done
[ "$FAIL" = 0 ] && echo "OK: all four pkg-config files == $VER"

# ④ Reuse existing drift checks (zero-cost hardening)
bash scripts/check-abi-drift.sh
exit $FAIL
