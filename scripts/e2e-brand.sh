#!/usr/bin/env bash
# brand e2e — application-layer branding regression (D252/C33): layout/naming/identity/runtime full-chain asserts + bindings freeze gate.
# Prereq: target/debug/mediaservo-host + oxmgr on PATH (otherwise a build/deploy hint is printed).
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
    echo "FAIL: oxmgr missing from PATH — branded start smoke needs it (install: npm install -g oxmgr)"; FAIL=1
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"; rm -rf /tmp/iceoryx2 /dev/shm/iox2_*; pkill -x oxmgr 2>/dev/null; pkill -x cp-agent 2>/dev/null || true' EXIT

HOST_BIN="target/debug/mediaservo-host"

# ── 1. CLI brand surface ─────────────────────────────────
note "CLI brand surface"
out=$($HOST_BIN version)
echo "$out" | grep -q "^mediaservo-host " || { echo "FAIL: default version should print mediaservo-host: $out"; FAIL=1; }
out=$(env MEDIASERVO_BRAND=cp $HOST_BIN version)
echo "$out" | grep -q "^cp " || { echo "FAIL: branded version should print cp: $out"; FAIL=1; }
out=$(env MEDIASERVO_BRAND=cp $HOST_BIN -h 2>&1 | head -1)
echo "$out" | grep -q "^Usage: cp " || { echo "FAIL: branded usage line should contain cp: $out"; FAIL=1; }
out=$(env MEDIASERVO_BRAND='Car/' $HOST_BIN version)
echo "$out" | grep -q "^mediaservo-host " || { echo "FAIL: invalid brand should fall back to default: $out"; FAIL=1; }
echo "OK: version/usage branded + invalid-brand fallback"

# ── 2. bindings freeze gate (branding must never touch it) ─────
note "bindings freeze gate"
N=$(git diff --stat bindings/ | wc -l)
[ "$N" = 0 ] || { echo "FAIL: bindings/ has a diff (freeze gate): $N lines"; FAIL=1; }
echo "OK: bindings diff 0 (frozen)"

# ── 3. install --brand cp (real install — exe_cmd uses the current_exe dir, brand apps must share the CLI directory) ─
note "install --brand cp"
./mediaservo.sh deploy host --brand cp --prefix "$TMP" >/dev/null 2>&1
for link in cp cp-host; do
    [ -L "$TMP/$link" ] || { echo "FAIL: missing root shortcut $TMP/$link"; FAIL=1; }
done
for app in agent recorder controller emergency audio capturer streamer; do
    [ -L "$TMP/bin/cp-$app" ] || { echo "FAIL: missing bin/cp-$app symlink"; FAIL=1; }
done
python3 -c "import json; d=json.load(open('$TMP/identity.json')); assert d['device_id'].startswith('cp-'), d" \
    || { echo "FAIL: identity device_id should start with cp-"; FAIL=1; }
echo "OK: root shortcuts + bin/cp-* links + identity cp-<12hex>"

# ── 4. branded instance runtime (real daemon + processes) ─────
note "branded instance start/smoke"
# C32: branded instances isolate via a different gateway port (default 17980 may be taken by the dev instance)
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
    || { echo "FAIL: brand instance start"; FAIL=1; }
sleep 8   # daemon cold-start window
# status consumes the directory argument (ps/monit/logs infer by cwd — a script cwd outside the instance connects to the wrong daemon)
out=$(env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp timeout -k 5 20 "$TMP/bin/mediaservo-host" status "$TMP" 2>&1 || true)
echo "$out" | grep -q "cp-agent" || { echo "FAIL: no cp-agent in status: ${out:0:180}"; FAIL=1; }
echo "$out" | grep -q "cp-capturer" || { echo "FAIL: no cp-capturer in status: ${out:0:180}"; FAIL=1; }
echo "OK: cp-agent/cp-capturer listed (branded process names across the full chain)"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: brand e2e all green (CLI/freeze gate/layout/identity/runtime)"
else
    echo "FAIL: $FAIL checks failed"; exit 1
fi