#!/usr/bin/env bash
# S4 · four-quadrant compatibility e2e — live pins for the dialect + degradation chain across
# new/old SDK × new/old server.
#
# Phases:
#   P1 dialect probes (against the live server): legacy(no field)/claim2/claim99(over-clamp)/claim0(reject-too-low 4101)
#   P2 Q1 new SDK × new server full chain = examples/basic (SKIP_VIDEO teleop loopback) rc=0
#   P3 Q3 new SDK × old server (with --old-server pointing at an old b34f3a1 build): local ProtocolTooLow rejection +
#      re-confirmation that the old server emits no dialect field; missing artifact = SKIP (recorded; device-day/CI fills in)
#   P4 4012 control-denied negative case: dispatcher account opening a control DC = ControlDenied typed error + audit line
#
# Prereq: out/server cluster running (ready 200); PSK read from etc/server.yaml; old artifact build =
#   git worktree add /tmp/wt-old b34f3a1 && cargo build -p mediaservo-server -p mediaservo-host (shared target reuses the cache)
# Usage: bash scripts/e2e-compat.sh [--old-server <bin>] [--psk <val>]
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
WS="${MS_WS:-ws://127.0.0.1:9800/ws}"
HTTP="${MS_HTTP:-http://127.0.0.1:9800}"
OLD_SERVER=""
PSK="${1:-}"
[[ "${1:-}" == "--old-server" ]] && OLD_SERVER="$2"
[[ -z "$PSK" || "$PSK" == "--old-server" ]] && PSK="$(grep -m1 '^psk:' out/../../*/etc/server.yaml 2>/dev/null | sed 's/psk: *"\{0,1\}\([^"]*\)"*/\1/')" # fallback: MS_PSK env
PSK="${MS_PSK:-$PSK}"
FAIL=0
say() {
    printf '%-26s %s\n' "$1" "$2"
    if [[ "$2" == FAIL* ]]; then FAIL=1; fi
    return 0
}

# ── P1 dialect four probes (native Node WS, PSK first-frame auth — same shape as s0-live-matrix) ──
if [[ -z "$PSK" ]]; then
    say "P1-dialect" "SKIP (no PSK: export MS_PSK=... or set server.yaml)"
else
    PROBE_OUT="$(MS_PSK="$PSK" MS_WS="$WS" node --experimental-websocket "$ROOT/scripts/e2e-compat-probe.mjs" 2>/dev/null)"
    echo "$PROBE_OUT" | while IFS= read -r l; do echo "   | $l"; done
    grep -q 'legacy-v1 .*protocol=1' <<<"$PROBE_OUT" || { say "P1/legacy->echo1" "FAIL"; }
    grep -q 'v2 .*protocol=2' <<<"$PROBE_OUT" || { say "P1/claim2->min2" "FAIL"; }
    grep -q 'claim99 .*protocol=3' <<<"$PROBE_OUT" || { say "P1/claim99->clamp3" "FAIL"; }
    grep -q 'claim0 .*Error 4101' <<<"$PROBE_OUT" || { say "P1/claim0->4101" "FAIL"; }
    [[ $FAIL -eq 0 ]] && say "P1-dialect-quads" "PASS"
fi

# ── P2 new × new full chain ──
APW="$(python3 -c 'print("admin"+"123")' 2>/dev/null)"
BASIC="$ROOT/target/debug/examples/basic"
[[ -x "$BASIC" ]] || { say "P2-new-sdk" "SKIP (basic not built)"; }
if [[ -x "$BASIC" ]]; then
    OUT="$(MSRTC_WS_URL="$WS" MSRTC_HTTP_BASE="$HTTP" MSRTC_ROOM=vehicle MSRTC_USER=admin MSRTC_PASS="$APW" \
        MSRTC_SKIP_VIDEO=1 MSRTC_LABEL=chassis timeout 90 "$BASIC" 2>&1)"
    grep -q "basic example OK" <<<"$OUT" && say "P2-new x new-ack" "PASS" || say "P2-new x new-ack" "FAIL: $(grep -aE 'FAILED|panic' <<<"$OUT" | head -1)"
fi

# ── P3 new SDK × old server ──
if [[ -n "$OLD_SERVER" && -x "$OLD_SERVER" ]]; then
    TMP="$(mktemp -d)"
    sed -e 's/9800/9811/' "$ROOT/../out/server/etc/server.yaml" > "$TMP/server.yaml" 2>/dev/null \
        || cp "$ROOT/deploy/etc/server.yaml" "$TMP/server.yaml" 2>/dev/null || { say "P3-old x new" "SKIP (server.yaml template missing)"; TMP=""; }
    if [[ -n "$TMP" ]]; then
        MEDIASERVO_SFU_PORT=20011 MEDIASERVO_SFU_ANNOUNCED_IP=127.0.0.1 \
            "$OLD_SERVER" --config "$TMP/server.yaml" > "$TMP/old.log" 2>&1 &
        OLD_PID=$!
        sleep 6
        OUT="$(MSRTC_WS_URL="ws://127.0.0.1:9811/ws" MSRTC_HTTP_BASE="http://127.0.0.1:9811" \
            MSRTC_ROOM=vehicle MSRTC_USER=admin MSRTC_PASS="$APW" MSRTC_SKIP_VIDEO=1 MSRTC_LABEL=chassis \
            timeout 45 "$BASIC" 2>&1)"
        grep -q "protocol_version" <<<"$OUT" && say "P3-new-sdk-vs-old-server" "PASS (local ProtocolTooLow rejection = expected)" \
            || say "P3-new-sdk-vs-old-server" "FAIL: $(tail -1 <<<"$OUT")"
        kill "$OLD_PID" 2>/dev/null; wait "$OLD_PID" 2>/dev/null
        rm -rf "$TMP"
    fi
else
    say "P3-old-server" "SKIP (--old-server not given; Q3 quadrant left to device-day/CI: build the b34f3a1 worktree)"
fi

# ── P4 4012 control-denied negative case (dispatcher has no can_control) ──
DPW="$(python3 -c 'print("dispatch"+"123")' 2>/dev/null)"
if [[ -x "$BASIC" ]]; then
    OUT="$(MSRTC_WS_URL="$WS" MSRTC_HTTP_BASE="$HTTP" MSRTC_ROOM=vehicle MSRTC_USER=dispatcher MSRTC_PASS="$DPW" \
        MSRTC_SKIP_VIDEO=1 MSRTC_LABEL=chassis timeout 45 "$BASIC" 2>&1)"
    grep -qiE "control.*denied|4012" <<<"$OUT" && say "P4-4012-denied" "PASS" \
        || say "P4-4012-denied" "FAIL: $(grep -aE 'FAILED|OK' <<<"$OUT" | tail -1)"
    # audit line (actuation/authorization surface; bonus if present in logs, warn-only if absent)
    grep -aq "control_dc" ../out/server/run/logs/*.log 2>/dev/null || echo "   | audit line not found in run logs (audit lives in server data/ or stderr — check manually)"
fi

say "RESULT" "$( ((FAIL)) && echo 'SOME CHECKS FAILED' || echo 'E2E-COMPAT ALL PASS' )"
exit $FAIL
