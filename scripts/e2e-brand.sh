#!/usr/bin/env bash
# brand e2e — 应用层品牌化回归（D252/C33）: 布局/命名/identity/运行时全链断言 + bindings 固化门。
# 前置: target/debug/mediaservo-host + oxmgr 在 PATH（缺失自动构建/deploy 提示）。
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"

FAIL=0
note() { echo "$(date +%T) == $1"; }

if [ ! -x target/debug/mediaservo-host ]; then
    note "building mediaservo-host (debug)"
    pixi run cargo build -p mediaservo-host
fi
if ! command -v oxmgr >/dev/null 2>&1; then
    echo "FAIL: PATH 无 oxmgr — 品牌 start 冒烟需它（安装: npm install -g oxmgr）"; FAIL=1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; rm -rf /tmp/iceoryx2 /dev/shm/iox2_*; pkill -x oxmgr 2>/dev/null; pkill -x cp-agent 2>/dev/null || true' EXIT

HOST_BIN="target/debug/mediaservo-host"

# ── 1. CLI 品牌面 ──────────────────────────────────────────
note "CLI brand surface"
out=$($HOST_BIN version)
echo "$out" | grep -q "^mediaservo-host " || { echo "FAIL: 默认 version 应 mediaservo-host: $out"; FAIL=1; }
out=$(env MEDIASERVO_BRAND=cp $HOST_BIN version)
echo "$out" | grep -q "^cp " || { echo "FAIL: brand version 应 cp: $out"; FAIL=1; }
out=$(env MEDIASERVO_BRAND=cp $HOST_BIN -h 2>&1 | head -1)
echo "$out" | grep -q "用法: cp " || { echo "FAIL: brand usage 应 cp: $out"; FAIL=1; }
out=$(env MEDIASERVO_BRAND='Car/' $HOST_BIN version)
echo "$out" | grep -q "^mediaservo-host " || { echo "FAIL: 非法 brand 应回落默认: $out"; FAIL=1; }
echo "OK: version/usage 品牌化 + 非法回落"

# ── 2. bindings 固化门（品牌不得触碰）──────────────
note "bindings freeze gate"
N=$(git diff --stat bindings/ | wc -l)
[ "$N" = 0 ] || { echo "FAIL: bindings/ 有 diff（固化门）: $N 行"; FAIL=1; }
echo "OK: bindings diff 0（固化）"

# ── 3. install --brand cp（真安装——exe_cmd 用 current_exe 目录, brand app 必须与 CLI 同目录）─
note "install --brand cp"
pixi run python3 scripts/mediaservo_cli.py deploy host --brand cp --prefix "$TMP" >/dev/null 2>&1
for link in cp cp-host; do
    [ -L "$TMP/$link" ] || { echo "FAIL: 缺根快捷 $TMP/$link"; FAIL=1; }
done
for app in agent recorder controller emergency audio capturer streamer; do
    [ -L "$TMP/bin/cp-$app" ] || { echo "FAIL: 缺 bin/cp-$app 符号链接"; FAIL=1; }
done
python3 -c "import json; d=json.load(open('$TMP/identity.json')); assert d['device_id'].startswith('cp-'), d" \
    || { echo "FAIL: identity device_id 应为 cp- 前缀"; FAIL=1; }
echo "OK: 根快捷 + bin/cp-* 链接 + identity cp-<12hex>"

# ── 4. 品牌实例运行时（真实 daemon + 进程）──────────────
note "branded instance start/smoke"
# C32: 品牌实例用不同网关端口隔离（默认 17980 可能被开发实例占用）
python3 - <<PY
p = "$TMP/etc/host.toml"
s = open(p).read()
if "local_port" not in s.split("[signaling]")[1].split("\n")[0]:
    s = s.replace("[signaling]", "[signaling]\nlocal_port = 17981\nroom = \"cp-veh\"", 1)
open(p, "w").write(s)
PY
rm -rf /tmp/iceoryx2 /dev/shm/iox2_*   # C25
pkill -x oxmgr 2>/dev/null || true; sleep 1
env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp "$TMP/bin/mediaservo-host" stop "$TMP" >/dev/null 2>&1 || true
env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp timeout -k 5 40 "$TMP/bin/mediaservo-host" start "$TMP" >/dev/null 2>&1 \
    || { echo "FAIL: brand 实例 start"; FAIL=1; }
sleep 8   # daemon cold-start 窗口
# status 消费目录参数（ps/monit/logs 按 cwd 推断——脚本 cwd 非实例则连错 daemon）
out=$(env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp timeout -k 5 20 "$TMP/bin/mediaservo-host" status "$TMP" 2>&1 || true)
echo "$out" | grep -q "cp-agent" || { echo "FAIL: status 无 cp-agent: ${out:0:180}"; FAIL=1; }
echo "$out" | grep -q "cp-capturer" || { echo "FAIL: status 无 cp-capturer: ${out:0:180}"; FAIL=1; }
echo "OK: cp-agent/cp-capturer 在列（品牌进程名全链）"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: brand e2e 全绿（CLI/固化门/布局/identity/运行时）"
else
    echo "FAIL: $FAIL 项失败"; exit 1
fi