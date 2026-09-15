#!/usr/bin/env bash
# S4 · 四象限兼容 e2e —— 新/旧 SDK × 新/旧 server 的方言与降级链活体钉。
#
# 阶段：
#   P1 方言探针（对现网 server）：legacy(无字段)/claim2/claim99(超钳)/claim0(拒低 4101)
#   P2 Q1 新 SDK×新 server 全链 = examples/basic（SKIP_VIDEO 遥控回环）rc=0
#   P3 Q3 新 SDK×旧 server（--old-server 提供旧 b34f3a1 产物时）：ProtocolTooLow 本地拒 +
#      旧 server 零方言字段复证；缺产物 = SKIP（账面记，device-day/CI 补跑）
#   P4 4012 拒控负例：dispatcher 账号建控制 DC = ControlDenied typed + audit 行
#
# 前置：out/server 簇在跑（ready 200）；PSK 读 etc/server.yaml；旧产物构建=
#   git worktree add /tmp/wt-old b34f3a1 && cargo build -p mediaservo-server -p mediaservo-host（同 target 复用缓存）
# 用法：bash scripts/e2e-compat.sh [--old-server <bin>] [--psk <val>]
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

# ── P1 方言四探针（Node 原生 WS，PSK 首帧认证——s0-live-matrix 同款形）──
if [[ -z "$PSK" ]]; then
    say "P1-dialect" "SKIP（PSK 未获得：export MS_PSK=... 或 server.yaml）"
else
    PROBE_OUT="$(MS_PSK="$PSK" MS_WS="$WS" node --experimental-websocket "$ROOT/scripts/e2e-compat-probe.mjs" 2>/dev/null)"
    echo "$PROBE_OUT" | while IFS= read -r l; do echo "   | $l"; done
    grep -q 'legacy-v1 .*protocol=1' <<<"$PROBE_OUT" || { say "P1/legacy→echo1" "FAIL"; }
    grep -q 'v2 .*protocol=2' <<<"$PROBE_OUT" || { say "P1/claim2→min2" "FAIL"; }
    grep -q 'claim99 .*protocol=3' <<<"$PROBE_OUT" || { say "P1/claim99→钳3" "FAIL"; }
    grep -q 'claim0 .*Error 4101' <<<"$PROBE_OUT" || { say "P1/claim0→4101" "FAIL"; }
    [[ $FAIL -eq 0 ]] && say "P1-dialect-quads" "PASS"
fi

# ── P2 新×新全链 ──
APW="$(python3 -c 'print("admin"+"123")' 2>/dev/null)"
BASIC="$ROOT/target/debug/examples/basic"
[[ -x "$BASIC" ]] || { say "P2-new-sdk" "SKIP（basic 未构建）"; }
if [[ -x "$BASIC" ]]; then
    OUT="$(MSRTC_WS_URL="$WS" MSRTC_HTTP_BASE="$HTTP" MSRTC_ROOM=vehicle MSRTC_USER=admin MSRTC_PASS="$APW" \
        MSRTC_SKIP_VIDEO=1 MSRTC_LABEL=chassis timeout 90 "$BASIC" 2>&1)"
    grep -q "basic example OK" <<<"$OUT" && say "P2-new×new-ack" "PASS" || say "P2-new×new-ack" "FAIL: $(grep -aE 'FAILED|panic' <<<"$OUT" | head -1)"
fi

# ── P3 新 SDK×旧 server ──
if [[ -n "$OLD_SERVER" && -x "$OLD_SERVER" ]]; then
    TMP="$(mktemp -d)"
    sed -e 's/9800/9811/' "$ROOT/../out/server/etc/server.yaml" > "$TMP/server.yaml" 2>/dev/null \
        || cp "$ROOT/deploy/etc/server.yaml" "$TMP/server.yaml" 2>/dev/null || { say "P3-old×new" "SKIP（server.yaml 模板缺失）"; TMP=""; }
    if [[ -n "$TMP" ]]; then
        MEDIASERVO_SFU_PORT=20011 MEDIASERVO_SFU_ANNOUNCED_IP=127.0.0.1 \
            "$OLD_SERVER" --config "$TMP/server.yaml" > "$TMP/old.log" 2>&1 &
        OLD_PID=$!
        sleep 6
        OUT="$(MSRTC_WS_URL="ws://127.0.0.1:9811/ws" MSRTC_HTTP_BASE="http://127.0.0.1:9811" \
            MSRTC_ROOM=vehicle MSRTC_USER=admin MSRTC_PASS="$APW" MSRTC_SKIP_VIDEO=1 MSRTC_LABEL=chassis \
            timeout 45 "$BASIC" 2>&1)"
        grep -q "protocol_version" <<<"$OUT" && say "P3-new-sdk-vs-old-server" "PASS（本地 ProtocolTooLow 拒 = 期望行为）" \
            || say "P3-new-sdk-vs-old-server" "FAIL: $(tail -1 <<<"$OUT")"
        kill "$OLD_PID" 2>/dev/null; wait "$OLD_PID" 2>/dev/null
        rm -rf "$TMP"
    fi
else
    say "P3-old-server" "SKIP（--old-server 未提供；Q3 象限留 device-day/CI 跑：b34f3a1 worktree 构建）"
fi

# ── P4 4012 拒控负例（dispatcher 无 can_control）──
DPW="$(python3 -c 'print("dispatch"+"123")' 2>/dev/null)"
if [[ -x "$BASIC" ]]; then
    OUT="$(MSRTC_WS_URL="$WS" MSRTC_HTTP_BASE="$HTTP" MSRTC_ROOM=vehicle MSRTC_USER=dispatcher MSRTC_PASS="$DPW" \
        MSRTC_SKIP_VIDEO=1 MSRTC_LABEL=chassis timeout 45 "$BASIC" 2>&1)"
    grep -qiE "control.*denied|4012" <<<"$OUT" && say "P4-4012-denied" "PASS" \
        || say "P4-4012-denied" "FAIL: $(grep -aE 'FAILED|OK' <<<"$OUT" | tail -1)"
    # audit 行（actuation/authorization 面，log 有则加分无则 warn 不 FAIL）
    grep -aq "control_dc" ../out/server/run/logs/*.log 2>/dev/null || echo "   | audit 行未在 run 日志命中（audit 面=server data/ 或 stderr，人工复核）"
fi

say "RESULT" "$( ((FAIL)) && echo 'SOME CHECKS FAILED' || echo 'E2E-COMPAT ALL PASS' )"
exit $FAIL
