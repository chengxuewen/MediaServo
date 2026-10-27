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
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/e2e-trap-lib.sh
source "$SCRIPT_DIR/e2e-trap-lib.sh"
trap msrtc_e2e_cleanup EXIT

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
MEDIASERVO_BRAND=cp ./mediaservo.sh deploy host --prefix "$TMP" >/dev/null 2>&1
# D269: a branded host deploy lands {brand}-host (bare-brand shortcut is
# unbranded-only, AppTree:399-401) — the `cp` in the old assertion was stale
for link in cp-host; do
    [ -L "$TMP/$link" ] || { echo "FAIL: missing root shortcut $TMP/$link"; FAIL=1; }
done
for app in agent recorder controller emergency audio capturer streamer; do
    # AppTree swap-leg lands regular files (COPY+RENAME), not symlinks —
    # the -L assertion predated SR7 (stale-assert debt, P3b ruling: fix all)
    [ -s "$TMP/bin/cp-$app" ] && [ -x "$TMP/bin/cp-$app" ] || { echo "FAIL: missing bin/cp-$app"; FAIL=1; }
done
python3 -c "import json; d=json.load(open('$TMP/identity.json')); assert d['device_id'].startswith('cp-'), d" \
    || { echo "FAIL: identity device_id should start with cp-"; FAIL=1; }
echo "OK: root shortcuts + bin/cp-* links + identity cp-<12hex>"

# ── 4. branded instance runtime (real daemon + processes) ─────
note "branded instance start/smoke"
# C32 port isolation + PIT-192 host-mutex: a real branded daemon binds the
# gateway port; a live cluster on this host makes that collision-prone, so the
# whole runtime leg skips loudly when any host family process is up (same shape
# as the mediasoup/roundtrip gates). YAML-native signaling override (was
# TOML `[signaling]` split — stale since the C35 yaml migration).
if ss -tln 2>/dev/null | grep -q ':17980 ' || pgrep -x 'host-agent\|cp-agent\|msrtc-agent' >/dev/null 2>&1; then
    echo "  SKIPPED (17980 busy / a host-agent family running — live cluster on this host)"
    echo "OK: runtime leg skipped (placement+identity above still enforced)"
else
    printf '\nsignaling:\n  local_port: 17981\n  room: "cp-veh"\n' >> "$TMP/etc/host.yaml"
    rm -rf /tmp/iceoryx2 /dev/shm/iox2_*   # C25 (guarded above: no live cluster to poison)
    env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp timeout -k 5 40 "$TMP/bin/cp-host" start "$TMP" >/dev/null 2>&1 \
        || { echo "FAIL: brand instance start"; FAIL=1; }
    sleep 8   # daemon cold-start window
    out=$(env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp timeout -k 5 20 "$TMP/bin/cp-host" status "$TMP" 2>&1 || true)
    echo "$out" | grep -q "cp-agent" || { echo "FAIL: no cp-agent in status: ${out:0:180}"; FAIL=1; }
    echo "$out" | grep -q "cp-capturer" || { echo "FAIL: no cp-capturer in status: ${out:0:180}"; FAIL=1; }
    env PATH="$TMP/bin:$PATH" MEDIASERVO_BRAND=cp "$TMP/bin/cp-host" stop "$TMP" >/dev/null 2>&1 || true
    echo "OK: cp-agent/cp-capturer listed (branded process names across the full chain)"
fi

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: brand e2e all green (CLI/freeze gate/layout/identity/runtime)"
else
    echo "FAIL: $FAIL checks failed"; exit 1
fi