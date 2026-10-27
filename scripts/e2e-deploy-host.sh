#!/usr/bin/env bash
# deploy host e2e smoke — install into a clean dir -> D-H13 layout asserts -> doctor -> idempotent re-deploy
# (identity/signing preserved) -> start/status/stop smoke. C22: runs natively on the host.
# Prereq: the 8 host binaries in target/debug (built automatically if missing).
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
./mediaservo.sh deploy host --prefix "$TMP"

BIN="$TMP/bin"

# ── 1. bin/: 8 process binaries executable + oxmgr packaged ─────
for b in mediaservo-host host-agent host-capturer host-streamer host-recorder \
         host-controller host-emergency host-audio; do
    [ -x "$BIN/$b" ] || { echo "FAIL: bin/$b missing or not executable"; FAIL=1; }
done
echo "OK: bin/ has $(ls "$BIN" | wc -l) executables"
if [ -x "$BIN/oxmgr" ]; then
    echo "OK: oxmgr packaged ($("$BIN/oxmgr" --version 2>/dev/null || echo '?'))"
else
    echo "WARN: oxmgr not packaged (no oxmgr on PATH) — the doctor oxmgr check will fail (expected, documented)"
fi

# ── 2. D-H13 layout asserts ───────────────────────────────
[ -f "$TMP/etc/host.yaml" ] || { echo "FAIL: etc/host.yaml"; FAIL=1; }
for f in signing.pem cam0.token cam0-stream.token recorder.token agent.token; do
    [ -f "$TMP/etc/link/$f" ] || { echo "FAIL: etc/link/$f"; FAIL=1; }
done
[ -d "$TMP/run/logs" ] || { echo "FAIL: run/logs"; FAIL=1; }
[ -d "$TMP/recordings" ] || { echo "FAIL: recordings"; FAIL=1; }
m600() { [ "$(stat -c %a "$1")" = "600" ]; }
m600 "$TMP/etc/link/signing.pem" && echo "OK: signing.pem 0600" || { echo "FAIL: signing.pem permissions"; FAIL=1; }
m600 "$TMP/identity.json" && echo "OK: identity.json 0600" || { echo "FAIL: identity.json permissions"; FAIL=1; }
echo "OK: D-H13 layout complete (etc/ + run/logs + recordings + identity.json)"

# ── 3. <prefix>/host shortcut ─────────────────────────────
if [ -L "$TMP/host" ] || [ -x "$TMP/host" ]; then
    echo "OK: <prefix>/host shortcut exists"
    out=$("$TMP/host" version 2>&1) || { echo "FAIL: <prefix>/host version failed: $out"; FAIL=1; }
    echo "$out" | grep -q "mediaservo-host" || { echo "FAIL: <prefix>/host version output unexpected: $out"; FAIL=1; }
    echo "OK: <prefix>/host version OK"
else
    echo "FAIL: <prefix>/host shortcut missing"; FAIL=1
fi


# ── 4. file/ldd checks ────────────────────────────────
file "$BIN/mediaservo-host" | grep -q ELF || { echo "FAIL: file is not ELF"; FAIL=1; }
if ldd "$BIN/mediaservo-host" 2>/dev/null | grep -q "not found"; then
    echo "FAIL: ldd reports not found"; FAIL=1
fi
echo "OK: file/ldd checks passed"

# ── 5. host doctor (PATH includes bin → packaged oxmgr takes effect) ─────
note "host doctor"
out=$(env PATH="$BIN:$PATH" "$BIN/mediaservo-host" doctor "$TMP" 2>&1 || true)
echo "$out"
echo "$out" | grep -q "all checks passed" || { echo "FAIL: doctor did not pass everything"; FAIL=1; }

# ── 6. idempotent re-deploy: identity/signing/host.yaml hashes unchanged ──
note "re-deploy idempotency"
SUM_BEFORE=$(sha256sum "$TMP/etc/link/signing.pem" "$TMP/identity.json" "$TMP/etc/host.yaml")
./mediaservo.sh deploy host --prefix "$TMP" >/dev/null
SUM_AFTER=$(sha256sum "$TMP/etc/link/signing.pem" "$TMP/identity.json" "$TMP/etc/host.yaml")
if [ "$SUM_BEFORE" = "$SUM_AFTER" ]; then
    echo "OK: hashes of signing.pem/identity.json/host.yaml unchanged after re-deploy (credentials preserved)"
else
    echo "FAIL: re-deploy broke credentials (hash changed)"; FAIL=1
fi

# ── 7. start/status/stop smoke (C25: clear iceoryx2 + leftover processes first — an app
# from the previous run holding 17980 makes this start correctly reject via the C32 conflict guard) ────
# PIT-192-family guard: this leg starts a REAL instance on 17980 — structurally
# mutually exclusive with a live cluster on this host. Skip loudly instead of
# colliding (same shape as the mediasoup gate's 9800-busy skip).
if ss -tln 2>/dev/null | grep -q ':17980 ' || pgrep -x host-agent >/dev/null 2>&1; then
    note "start/status/stop roundtrip — SKIPPED (17980/host-agent busy: a live instance is running on this host; run on a cluster-free box)"
    echo "PASS: deploy host e2e smoke (roundtrip skipped, placement+idempotency green)"
    [ "$FAIL" = 0 ] || { echo "FAIL: $FAIL checks failed"; exit 1; }
    exit 0
fi
note "start/status/stop roundtrip"
rm -rf /tmp/iceoryx2 /dev/shm/iox2_*
env PATH="$BIN:$PATH" "$BIN/mediaservo-host" stop "$TMP" >/dev/null 2>&1
pkill -x host-agent 2>/dev/null; pkill -x host-capturer 2>/dev/null; pkill -x oxmgr 2>/dev/null
sleep 1
env PATH="$BIN:$PATH" "$BIN/mediaservo-host" start "$TMP" >/dev/null 2>&1 || { echo "FAIL: host start"; FAIL=1; }
sleep 8   # daemon cold-start window (oxmgr 30s ready probe + process spawn)
out=$(env PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$TMP" 2>&1 || true)
echo "$out" | grep -q "host-agent" || { echo "FAIL: no host-agent in status: $out"; FAIL=1; }
env PATH="$BIN:$PATH" "$BIN/mediaservo-host" stop "$TMP" >/dev/null 2>&1 || { echo "FAIL: host stop"; FAIL=1; }
out=$(env PATH="$BIN:$PATH" "$BIN/mediaservo-host" status "$TMP" 2>&1 || true)
echo "$out" | grep -q "no managed processes" || { echo "FAIL: processes remain after stop: $out"; FAIL=1; }
echo "OK: start → status (host-agent listed) → stop smoke passed"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: deploy host e2e smoke all green"
else
    echo "FAIL: $FAIL checks failed"; exit 1
fi
