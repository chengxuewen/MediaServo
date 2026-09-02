#!/usr/bin/env bash
# package e2e — dist/ 双包发布回归（D-H13）: host 包版本顶层目录断言 → 解包 doctor/start/status/stop
# 冒烟 → SDK 包版本顶层目录关键文件断言 → 版本契约文件 → dist/ gitignore。C22: 宿主原生执行。
# 前置: target/debug 8 host 二进制 + 3 SDK .so + node 绑定（缺失自动构建）。
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"

FAIL=0
note() { echo "== $1"; }

# workspace 版本（与 mediaservo_cli.py _workspace_version 同源: Cargo.toml [workspace.package]）
VER=$(sed -n '/\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -1)
[ -n "$VER" ] || { echo "FAIL: 无法解析 workspace version"; exit 1; }
MAJOR=${VER%%.*}

# ── 0. 前置构建（缺啥补啥, 与 e2e-deploy-host.sh 同模式）──────
if [ ! -x target/debug/mediaservo-host ]; then
    note "building mediaservo-host (debug)"
    pixi run cargo build -p mediaservo-host
fi
if [ ! -f target/debug/libmediaservo_field.so ] || [ ! -f target/debug/libmediaservo_link.so ] \
   || [ ! -f target/debug/libmediaservo_deck.so ] || [ ! -f bindings/node/mediaservo.node ]; then
    note "building SDK cdylibs + node binding (debug)"
    pixi run python3 scripts/mediaservo_cli.py build bindings
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; rm -rf /tmp/iceoryx2 /dev/shm/iox2_*' EXIT

HOST_TGZ="dist/mediaservo-host-$VER.tar.gz"
HOST_ROOT="mediaservo-host-$VER"
SDK_TGZ="dist/mediaservo-sdk-$VER.tar.gz"
SDK_ROOT="mediaservo-sdk-$VER"

# ── 1. package host ─────────────────────────────────────────
note "package host (ver=$VER)"
env MEDIASERVO_BRAND=mediaservo pixi run python3 scripts/mediaservo_cli.py package host
[ -f "$HOST_TGZ" ] || { echo "FAIL: 缺少 $HOST_TGZ"; FAIL=1; }

# ── 2. host 包内容断言（版本顶层目录 + D-H13 布局 + host-version.txt）────────
note "host tar contents"
LIST=$(tar tzf "$HOST_TGZ" || true)
[ -n "$LIST" ] || { echo "FAIL: $HOST_TGZ 无法读取"; FAIL=1; }
echo "$LIST" | grep -q "^$HOST_ROOT/$" || { echo "FAIL: 包内缺版本顶层目录 $HOST_ROOT/"; FAIL=1; }
for b in mediaservo-host host-agent host-capturer host-streamer host-recorder \
         host-controller host-emergency host-audio; do
    echo "$LIST" | grep -q "^$HOST_ROOT/bin/$b$" || { echo "FAIL: 包内缺 $HOST_ROOT/bin/$b"; FAIL=1; }
done
# 根快捷（host + mediaservo-host）— bin/ 里是 mediaservo-host, 版本顶层目录根挂快捷
echo "$LIST" | grep -q "^$HOST_ROOT/host$" || { echo "FAIL: 包内缺 $HOST_ROOT/host"; FAIL=1; }
# oxmgr 条件断言（I2 review 修）: 打包时 PATH 有 oxmgr → 包内必须含 $HOST_ROOT/bin/oxmgr;
# 环境缺 oxmgr → install 未打包（cli 明确报错）, 跳过并说明
if command -v oxmgr >/dev/null 2>&1; then
    echo "$LIST" | grep -q "^$HOST_ROOT/bin/oxmgr$" || { echo "FAIL: PATH 有 oxmgr 但包内缺 $HOST_ROOT/bin/oxmgr"; FAIL=1; }
else
    echo "NOTE: PATH 无 oxmgr — 跳过 bin/oxmgr 断言（deploy host 未打包, doctor 预期报错）"
fi
for f in etc/host.yaml etc/link/signing.pem etc/link/cam0.token etc/link/cam0-stream-stream.token \
         etc/link/recorder.token etc/link/agent.token etc/link/ros_bridge.yaml \
         etc/link/issuance.jsonl identity.json host-version.txt; do
    echo "$LIST" | grep -q "^$HOST_ROOT/$f$" || { echo "FAIL: 包内缺 $HOST_ROOT/$f"; FAIL=1; }
done
echo "$LIST" | grep -q "^$HOST_ROOT/run/logs/$" || { echo "FAIL: 包内缺 $HOST_ROOT/run/logs/"; FAIL=1; }
echo "$LIST" | grep -q "^$HOST_ROOT/recordings/$" || { echo "FAIL: 包内缺 $HOST_ROOT/recordings/"; FAIL=1; }
echo "OK: host 包布局完整（版本顶层目录 $HOST_ROOT/ + bin 8 + oxmgr + etc/link 凭据 + run/logs + recordings + identity.json + host-version.txt）"

# ── 3. 解包到干净目录 → doctor + start/status/stop 冒烟 ──────
note "extract + smoke"
mkdir -p "$TMP/extract"
tar xzf "$HOST_TGZ" -C "$TMP/extract"
PKG_DIR="$TMP/extract/$HOST_ROOT"
BIN="$PKG_DIR/bin"
if [ ! -d "$PKG_DIR" ]; then
    echo "FAIL: 解包后缺版本顶层目录 $PKG_DIR"; FAIL=1
elif [ ! -x "$BIN/mediaservo-host" ]; then
    echo "FAIL: 解包后缺可执行 $BIN/mediaservo-host"; FAIL=1
else
    echo "OK: host tar 解包生成版本顶层目录 $HOST_ROOT/"
fi
if command -v oxmgr >/dev/null 2>&1 && [ -x "$BIN/mediaservo-host" ]; then
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" doctor "$PKG_DIR" 2>&1 || true)
    echo "$out" | grep -q "全部通过" || { echo "FAIL: 解包后 doctor 未全过"; echo "$out" | tail -8; FAIL=1; }
    rm -rf /tmp/iceoryx2 /dev/shm/iox2_*   # C25: 清 iceoryx2 残留
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" start "$PKG_DIR" 2>&1 || true)
    if ! echo "$out" | grep -q "已启动\|started\|start"; then
        echo "FAIL: 解包后 host start"; echo "$out" | tail -8; FAIL=1
    fi
    sleep 4
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$PKG_DIR" 2>&1 || true)
    echo "$out" | grep -q "host-agent" || { echo "FAIL: status 无 host-agent"; echo "$out" | tail -8; FAIL=1; }
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" stop "$PKG_DIR" 2>&1 || true)
    if ! echo "$out" | grep -q "已停止\|stopped\|stop"; then
        echo "FAIL: host stop"; echo "$out" | tail -8; FAIL=1
    fi
    out=$(env MEDIASERVO_BRAND=mediaservo PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$PKG_DIR" 2>&1 || true)
    echo "$out" | grep -q "无已管理进程" || { echo "FAIL: stop 后仍有进程"; echo "$out" | tail -8; FAIL=1; }
    echo "OK: 解包后 doctor → start(host-agent) → stop 冒烟通过"
else
    echo "NOTE: PATH 无 oxmgr — 跳过 host doctor/start/status/stop 运行冒烟"
fi

# ── 4. package bindings ─────────────────────────────────────
note "package bindings (ver=$VER)"
env MEDIASERVO_BRAND=mediaservo pixi run python3 scripts/mediaservo_cli.py package bindings
[ -f "$SDK_TGZ" ] || { echo "FAIL: 缺少 $SDK_TGZ"; FAIL=1; }

# ── 5. SDK 包关键文件断言 ────────────────────────────────────
note "sdk tar contents"
LIST=$(tar tzf "$SDK_TGZ" || true)
[ -n "$LIST" ] || { echo "FAIL: $SDK_TGZ 无法读取"; FAIL=1; }
MINOR=$(echo "$VER" | cut -d. -f2); PATCH=$(echo "$VER" | cut -d. -f3)
for f in \
    lib/libmediaservo_field.so.$MAJOR.$MINOR.$PATCH \
    lib/libmediaservo_link.so.$MAJOR.$MINOR.$PATCH \
    lib/libmediaservo_deck.so.$MAJOR.$MINOR.$PATCH \
    lib/libmediaservo_field.so lib/libmediaservo_field.so.$MAJOR \
    lib/libmediaservo_link.so lib/libmediaservo_link.so.$MAJOR \
    lib/libmediaservo_deck.so lib/libmediaservo_deck.so.$MAJOR \
    include/mediaservo/common.h include/mediaservo/field.h \
    include/mediaservo/link.h include/mediaservo/deck.h \
    lib/pkgconfig/mediaservo-field.pc lib/pkgconfig/mediaservo-link.pc lib/pkgconfig/mediaservo-deck.pc \
    lib/cmake/mediaservo/mediaservoConfig.cmake lib/cmake/mediaservo/mediaservoConfigVersion.cmake \
    node/mediaservo/package.json node/mediaservo/mediaservo.node node/mediaservo/lib/index.mjs \
    sdk-version.txt; do
    echo "$LIST" | grep -q "^$SDK_ROOT/$f$" || { echo "FAIL: SDK 包缺 $f"; FAIL=1; }
done
echo "$LIST" | grep -qE "^$SDK_ROOT/lib/python3\.[0-9]+/site-packages/mediaservo/" || { echo "FAIL: SDK 包缺 python 包"; FAIL=1; }
echo "$LIST" | grep -qE "^$SDK_ROOT/wheel/mediaservo-.*\.whl$" || { echo "FAIL: SDK 包缺 wheel"; FAIL=1; }
echo "OK: SDK 包关键文件完整（版本顶层目录 $SDK_ROOT/ + lib 三件套实体+符号链接 + include + python + wheel + node + .pc + cmake + sdk-version.txt）"

# ── 6. 版本契约文件内容（D-H14 最小版: workspace + FrameMeta wire 版本）──
note "version contract file"
mkdir -p "$TMP/sdk"
tar xzf "$SDK_TGZ" -C "$TMP/sdk" "$SDK_ROOT/sdk-version.txt"
grep -q "workspace_version: $VER" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt 缺 workspace_version=$VER"; FAIL=1; }
grep -q "^frame_meta_version: 1$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt 缺 frame_meta_version"; FAIL=1; }
grep -q "^token_schema_version: 1$" "$TMP/sdk/$SDK_ROOT/sdk-version.txt" || { echo "FAIL: sdk-version.txt 缺 token_schema_version"; FAIL=1; }
grep -q "workspace_version: $VER" <(tar xzf "$HOST_TGZ" -O "$HOST_ROOT/host-version.txt") || { echo "FAIL: host-version.txt 缺 workspace_version=$VER"; FAIL=1; }
echo "OK: 版本契约文件（host-version.txt + sdk-version.txt）内容正确"

# ── 7. dist/ gitignore ──────────────────────────────────────
note "dist/ gitignore"
git check-ignore dist/ >/dev/null || { echo "FAIL: dist/ 未被 gitignore"; FAIL=1; }
echo "OK: dist/ 已 gitignore"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: package e2e 全绿（$HOST_TGZ + $SDK_TGZ）"
else
    echo "FAIL: $FAIL 项失败"; exit 1
fi
