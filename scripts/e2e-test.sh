#!/usr/bin/env bash
# E2E verification: Host -> Docker Server -> Client video flow
# 用法: bash scripts/e2e-test.sh [--legacy]
#   --legacy: 回退旧单进程 host-legacy（P2P 信令断言）; 默认多进程 host（C4: capturer+streamer, oxmgr）
set -u

HOST_PID=""
CLIENT_PID=""
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
PASS=0; FAIL=0
TMPDIR="/tmp/mediaservo-e2e-$$"
LEGACY="${1:-}"

pass() { echo -e "${GREEN}[PASS]${NC} $1"; ((PASS++)); }
fail() { echo -e "${RED}[FAIL]${NC} $1"; ((FAIL++)); }
info() { echo -e "${YELLOW}[INFO]${NC} $1"; }

cleanup() {
    info "Stopping Host and Client..."
    kill $HOST_PID 2>/dev/null || true
    kill $CLIENT_PID 2>/dev/null || true
    if [ -z "$HOST_PID" ] && [ -z "$LEGACY" ]; then
        # 多进程 host: oxmgr 管理——手动停止（CLI stop host 已移除 D266）
        target/debug/mediaservo-host stop . 2>/dev/null || true
    fi
    echo ""
    echo "═══════════════════════════════════════"
    echo "  Results: ${GREEN}$PASS passed${NC}  ${RED}$FAIL failed${NC}"
    echo "═══════════════════════════════════════"
    echo "Logs: $TMPDIR"
    exit $FAIL
}
trap cleanup EXIT

mkdir -p "$TMPDIR"
info "Logs: $TMPDIR"

# 1. Server health check
info "1. Checking server..."
if curl -sf http://localhost:9800/health | grep -q OK; then
    pass "Server healthy"
else
    fail "Server not running. Start: docker compose up -d"
    exit 1
fi

# 2. Build
info "2. Building..."
cargo build -p mediaservo-host -p mediaservo-client 2>/dev/null && \
    pass "Build" || { fail "Build"; exit 1; }

# 3. Start Host
info "3. Starting Host..."
if [ -n "$LEGACY" ]; then
    cargo run -p mediaservo-host --bin host-legacy > "$TMPDIR/host.log" 2>&1 &
    HOST_PID=$!
    info "Host PID=$HOST_PID"
    for i in 1 2 3 4; do
        sleep 5
        if grep -qi "room_join\|RoomJoin\|WebRTC\|PeerConnection" "$TMPDIR/host.log" 2>/dev/null; then
            pass "Host (legacy) connected"
            break
        fi
        [ $i -eq 4 ] && { fail "Host connect timeout"; head -10 "$TMPDIR/host.log"; }
    done
else
    # 多进程: 内联四步（CLI start host 已移除 D266）——清残留 → init → token → start（oxmgr; 日志 ~/.local/share/oxmgr/logs）
    rm -rf /tmp/iceoryx2
    rm -f /dev/shm/iox2_* 2>/dev/null || true
    target/debug/mediaservo-host init . >/dev/null 2>&1 || true
    target/debug/mediaservo-host token issue --all . >/dev/null 2>&1 || true
    target/debug/mediaservo-host start .
    OXLOG="${OXMGR_HOME:-$HOME/.local/share/oxmgr}/logs/host-streamer.out.log"
    info "streamer log: $OXLOG"
    for i in 1 2 3 4 5 6; do
        sleep 5
        # SFU produce 证据（D4 模式）: bytes_sent>0 = server 已收帧（capturer→FrameBus→streamer 全链路）
        if grep -q "streamer stats: bytes_sent=[1-9]" "$OXLOG" 2>/dev/null; then
            pass "Host producing (capturer+streamer, bytes_sent>0)"
            grep "streamer stats:" "$OXLOG" | tail -1
            break
        fi
        [ $i -eq 6 ] && { fail "Host produce timeout"; tail -10 "$OXLOG" 2>/dev/null; }
    done
fi

# 4. Start Client
info "4. Starting Client..."
cargo run -p mediaservo-client --bin mediaservo-client > "$TMPDIR/client.log" 2>&1 &
CLIENT_PID=$!
info "Client PID=$CLIENT_PID"

for i in 1 2 3 4; do
    sleep 5
    if grep -qi "signaling\|Signaling\|room\|RoomJoin" "$TMPDIR/client.log" 2>/dev/null; then
        pass "Client connected"
        break
    fi
    [ $i -eq 4 ] && { fail "Client connect timeout"; head -10 "$TMPDIR/client.log"; }
done

# 5. SDP exchange（多进程 host 无 P2P 信令 — SDP 断言仅 legacy）
info "5. SDP exchange..."
if [ -n "$LEGACY" ]; then
    sleep 5
    grep -qi "Sdp\|SDP\|offer\|answer\|remote_description" "$TMPDIR/host.log" 2>/dev/null && \
        pass "Host SDP" || fail "Host SDP"
fi
grep -qi "Sdp\|SDP\|offer\|answer" "$TMPDIR/client.log" 2>/dev/null && \
    pass "Client SDP" || fail "Client SDP"

# 6. Data channel（多进程 host 无 P2P DC — DC 断言仅 legacy）
info "6. Data channel..."
if [ -n "$LEGACY" ]; then
    sleep 5
    grep -qi "DataChannel\|data.channel\|RTCDataChannel\|on_open\|dc\|channel" "$TMPDIR/host.log" 2>/dev/null && \
        pass "Host DC" || info "Host DC not detected"
fi
grep -qi "DataChannel\|data.channel\|RTCDataChannel\|spool\|on_message" "$TMPDIR/client.log" 2>/dev/null && \
    pass "Client DC" || info "Client DC not detected"

# 7. Check server logs
info "7. Server relay..."
docker compose logs --tail 20 server 2>/dev/null | grep -qi "room\|Room\|relay\|Relay" && \
    pass "Server relay" || info "Server relay not detected"

# 8. Print summary lines
info "8. Key log lines:"
echo "--- Host ---"
if [ -n "$LEGACY" ]; then
    grep -i "INFO\|WARN\|ERROR\|room\|WebRTC\|DC\|SDP\|Peer" "$TMPDIR/host.log" 2>/dev/null | head -10 || echo "(empty)"
else
    grep "streamer stats:" "$OXLOG" 2>/dev/null | tail -3 || echo "(empty)"
fi
echo "--- Client ---"
grep -i "INFO\|WARN\|ERROR\|room\|WebRTC\|DC\|SDP\|Peer" "$TMPDIR/client.log" 2>/dev/null | head -10 || echo "(empty)"
echo "--- Server ---"
docker compose logs --tail 10 server 2>/dev/null | grep -i "INFO\|WARN\|ERROR\|room\|join\|leave\|SDP" | head -5 || echo "(empty)"
