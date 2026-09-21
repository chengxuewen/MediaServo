#!/usr/bin/env bash
# V5 交付面冒烟（e2e-package 的"纯库消费"轻量孪生）：out/bindings 组装后
# ① 组件纯度（link/field 轻组件禁 FFmpeg DT_NEEDED = V1 sdk-field 组件化的依赖断言）
# ② find_package COMPONENTS link+client 真符号编译链接运行（V2 演练全形 CI 化）
# ③ pkg-config 版本 == workspace 实值（C43 parity 交付面延伸）
# 前置：python3 scripts/mediaservo_cli.py build bindings（本脚本自跑，CI 零额外步骤）。
set -euo pipefail
cd "$(dirname "$0")/.."
TMP=$(mktemp -d /tmp/ms-delivery.XXXXXX)
trap 'rm -rf "$TMP"' EXIT
FAIL=0

python3 scripts/mediaservo_cli.py build bindings >/dev/null
OUT="$PWD/out/bindings"
VER=$(python3 -c 'import tomllib;print(tomllib.load(open("Cargo.toml","rb"))["workspace"]["package"]["version"])')

# ① 轻组件 FFmpeg 免疫
for lib in link field; do
    if readelf -d "$OUT/lib/libmediaservo_$lib.so" | grep -qE "NEEDED.*(avformat|avcodec|swresample)"; then
        echo "FAIL: 轻组件 libmediaservo_$lib.so DT_NEEDED 含 FFmpeg（轻组件面被污染）"; FAIL=1
    fi
done
[ "$FAIL" = 0 ] && echo "OK: link/field 无 FFmpeg DT_NEEDED（deck/sdk 面不受此限）"

# ② find_package 三面
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
/* 符号存在性判据（链接期即证；不调用——免网络/设备依赖）。 */
int main(void) {
    void *a = (void *)mediaservo_link_version;
    void *b = (void *)mediaservo_client_login;
    return (a != 0 && b != 0) ? 0 : 1;
}
C
# 生成器自适应（pixi 有 ninja 无 make / ubuntu runner 反之）
GEN=""
command -v make >/dev/null 2>&1 || GEN="Ninja"
cmake -S "$TMP/pkg" -B "$TMP/pkg/build" ${GEN:+-G "$GEN"} -DCMAKE_PREFIX_PATH="$OUT/lib/cmake/mediaservo" >/dev/null
cmake --build "$TMP/pkg/build" >/dev/null
LD_LIBRARY_PATH="$OUT/lib" "$TMP/pkg/build/smoke" && echo "OK: find_package(link,client)+真符号链接运行"

# ③ pkg-config 版本对账
for pc in mediaservo-link mediaservo-client mediaservo-deck mediaservo-field; do
    got=$(PKG_CONFIG_PATH="$OUT/lib/pkgconfig" pkg-config --modversion "$pc")
    [ "$got" = "$VER" ] || { echo "FAIL: $pc 版本 $got != workspace $VER"; FAIL=1; }
done
[ "$FAIL" = 0 ] && echo "OK: pkg-config 四件 == $VER"

# ④ 复用既有巡检（漂移零成本加固）
bash scripts/check-abi-drift.sh
exit $FAIL
