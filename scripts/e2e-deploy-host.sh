#!/usr/bin/env bash
# deploy host e2e smoke — 干净目录安装 → D-H13 布局断言 → doctor → 幂等重装
# （identity/signing 保留）→ start/status/stop 冒烟。C22: 宿主原生执行。
# 前置: target/debug 8 host 二进制（缺失自动构建）。
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"

FAIL=0
note() { echo "== $1"; }

if [ ! -x target/debug/mediaservo-host ]; then
    note "building mediaservo-host (debug)"
    pixi run cargo build -p mediaservo-host
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; rm -rf /tmp/iceoryx2 /dev/shm/iox2_*' EXIT

note "deploy host --prefix $TMP"
python3 scripts/mediaservo_cli.py deploy host --prefix "$TMP"

BIN="$TMP/bin"

# ── 1. bin/: 8 进程二进制可执行 + oxmgr 打包 ───────────────
for b in mediaservo-host host-agent host-capturer host-streamer host-recorder \
         host-controller host-emergency host-audio; do
    [ -x "$BIN/$b" ] || { echo "FAIL: bin/$b 缺失或不可执行"; FAIL=1; }
done
echo "OK: bin/ $(ls "$BIN" | wc -l) 个可执行文件"
if [ -x "$BIN/oxmgr" ]; then
    echo "OK: oxmgr 已打包 ($("$BIN/oxmgr" --version 2>/dev/null || echo '?'))"
else
    echo "WARN: oxmgr 未打包（PATH 无 oxmgr）— doctor oxmgr 检查将失败（预期，文档化）"
fi

# ── 2. D-H13 布局断言 ──────────────────────────────────────
[ -f "$TMP/etc/host.toml" ] || { echo "FAIL: etc/host.toml"; FAIL=1; }
for f in signing.pem cam0.token cam0-stream.token recorder.token agent.token; do
    [ -f "$TMP/etc/link/$f" ] || { echo "FAIL: etc/link/$f"; FAIL=1; }
done
[ -d "$TMP/run/logs" ] || { echo "FAIL: run/logs"; FAIL=1; }
[ -d "$TMP/recordings" ] || { echo "FAIL: recordings"; FAIL=1; }
m600() { [ "$(stat -c %a "$1")" = "600" ]; }
m600 "$TMP/etc/link/signing.pem" && echo "OK: signing.pem 0600" || { echo "FAIL: signing.pem 权限"; FAIL=1; }
m600 "$TMP/identity.json" && echo "OK: identity.json 0600" || { echo "FAIL: identity.json 权限"; FAIL=1; }
echo "OK: D-H13 布局完整（etc/ + run/logs + recordings + identity.json）"

# ── 3. <prefix>/host 快捷方式 ───────────────────────────────
if [ -L "$TMP/host" ] || [ -x "$TMP/host" ]; then
    echo "OK: <prefix>/host 快捷方式存在"
    out=$("$TMP/host" version 2>&1) || { echo "FAIL: <prefix>/host version 失败: $out"; FAIL=1; }
    echo "$out" | grep -q "mediaservo-host" || { echo "FAIL: <prefix>/host version 输出异常: $out"; FAIL=1; }
    echo "OK: <prefix>/host version 正常"
else
    echo "FAIL: <prefix>/host 快捷方式缺失"; FAIL=1
fi


# ── 4. file/ldd 检查 ───────────────────────────────────────
file "$BIN/mediaservo-host" | grep -q ELF || { echo "FAIL: file 非 ELF"; FAIL=1; }
if ldd "$BIN/mediaservo-host" 2>/dev/null | grep -q "not found"; then
    echo "FAIL: ldd 有 not found"; FAIL=1
fi
echo "OK: file/ldd 检查通过"

# ── 5. host doctor（PATH 含 bin → 打包版 oxmgr 生效）────────
note "host doctor"
out=$(env PATH="$BIN:$PATH" "$BIN/mediaservo-host" doctor "$TMP" 2>&1 || true)
echo "$out"
echo "$out" | grep -q "全部通过" || { echo "FAIL: doctor 未全过"; FAIL=1; }

# ── 6. 幂等重装: identity/signing/host.toml 哈希不变 ───────
note "re-deploy idempotency"
SUM_BEFORE=$(sha256sum "$TMP/etc/link/signing.pem" "$TMP/identity.json" "$TMP/etc/host.toml")
python3 scripts/mediaservo_cli.py deploy host --prefix "$TMP" >/dev/null
SUM_AFTER=$(sha256sum "$TMP/etc/link/signing.pem" "$TMP/identity.json" "$TMP/etc/host.toml")
if [ "$SUM_BEFORE" = "$SUM_AFTER" ]; then
    echo "OK: 重装后 signing.pem/identity.json/host.toml 哈希不变（凭据保留）"
else
    echo "FAIL: 重装破坏凭据（哈希变化）"; FAIL=1
fi

# ── 7. start/status/stop 冒烟（C25: 先清 iceoryx2 + 残留进程——上次运行的
# app 占 17980 会让本次 start 因 C32 竞争防护正确拒绝）────
note "start/status/stop roundtrip"
rm -rf /tmp/iceoryx2 /dev/shm/iox2_*
env PATH="$BIN:$PATH" "$BIN/mediaservo-host" stop "$TMP" >/dev/null 2>&1
pkill -x host-agent 2>/dev/null; pkill -x host-capturer 2>/dev/null; pkill -x oxmgr 2>/dev/null
sleep 1
env PATH="$BIN:$PATH" "$BIN/mediaservo-host" start "$TMP" >/dev/null 2>&1 || { echo "FAIL: host start"; FAIL=1; }
sleep 8   # daemon cold-start 窗口（oxmgr 30s ready 探测 + 进程拉起）
out=$(env PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$TMP" 2>&1 || true)
echo "$out" | grep -q "host-agent" || { echo "FAIL: status 无 host-agent: $out"; FAIL=1; }
env PATH="$BIN:$PATH" "$BIN/mediaservo-host" stop "$TMP" >/dev/null 2>&1 || { echo "FAIL: host stop"; FAIL=1; }
out=$(env PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$TMP" 2>&1 || true)
echo "$out" | grep -q "无已管理进程" || { echo "FAIL: stop 后仍有进程: $out"; FAIL=1; }
echo "OK: start → status(host-agent 在列) → stop 冒烟通过"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: deploy host e2e smoke 全绿"
else
    echo "FAIL: $FAIL 项失败"; exit 1
fi
