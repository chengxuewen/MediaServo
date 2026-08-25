#!/usr/bin/env bash
# 绑定矩阵 live e2e — 三语言正向链路回归（C field/link/deck + Python field）。
# 验证过的能力固化为可重复门禁（之前仅会话内手动实证）。
# 前置: pixi run build-c + docker 可用；server 未运行会自动拉起。
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"
export LD_LIBRARY_PATH="$PWD/target/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export MEDIASERVO_LIB_DIR="$PWD/target/debug"
export PYTHONPATH="$PWD/bindings/python/mediaservo${PYTHONPATH:+:$PYTHONPATH}"

FAIL=0
note() { echo "== $1"; }

# ── 1. server 前置 ─────────────────────────────────────────
if ! ss -tln 2>/dev/null | grep -q ":9800"; then
    note "server not running, starting..."
    timeout 60 ./mediaservo.sh up --env dev server >/dev/null 2>&1 || true
    for _ in $(seq 1 30); do ss -tln 2>/dev/null | grep -q ":9800" && break; sleep 1; done
fi
ss -tln 2>/dev/null | grep -q ":9800" || { echo "FAIL: server not up on :9800"; exit 1; }
note "server up"

# ── 2. 通用 check helper ────────────────────────────────────
# check <label> <grep-pattern> <cmd...>  — timeout 杀后仍按输出断言
check() {
    local label="$1" pattern="$2"; shift 2
    local out
    out=$(stdbuf -oL timeout 25 "$@" 2>&1 || true)
    if echo "$out" | grep -q "$pattern"; then
        echo "OK: $label"
    else
        echo "FAIL: $label (pattern '$pattern' not found)"; echo "$out" | tail -5; FAIL=1
    fi
}

# ── 3. C field: 真实 server 推流 ────────────────────────────
note "C field push"
gcc bindings/c/mediaservo-field-c/examples/vehicle_push.c \
    -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
    -L target/debug -lmediaservo_field -o /tmp/opencode/e2e_vp
check "C field: connected + published + frames" "frames running" \
    env LD_LIBRARY_PATH="$PWD/target/debug" /tmp/opencode/e2e_vp

# ── 4. C link: 信令 + 事件泵 ────────────────────────────────
note "C link signal"
gcc bindings/c/mediaservo-link-c/examples/vehicle_signal.c \
    -I bindings/c/mediaservo-link-c/include -I bindings/c/include \
    -L target/debug -lmediaservo_link -o /tmp/opencode/e2e_vs
check "C link: connected + event echo" '"type":"message"' \
    env LD_LIBRARY_PATH="$PWD/target/debug" /tmp/opencode/e2e_vs

# ── 5. C deck: 采集→录制→回放闭环 ───────────────────────────
note "C deck record/playback"
gcc bindings/c/mediaservo-deck-c/examples/record_playback.c \
    -I bindings/c/mediaservo-deck-c/include -I bindings/c/include \
    -L target/debug -lmediaservo_deck -o /tmp/opencode/e2e_rp
out=$(stdbuf -oL timeout 40 env LD_LIBRARY_PATH="$PWD/target/debug" /tmp/opencode/e2e_rp 2>&1 || true)
if echo "$out" | grep -q "playback decoded" && echo "$out" | grep -q "recording"; then
    dur=$(pixi run bash -c 'ffprobe -v error -show_entries format=duration -of csv=p=0 /tmp/opencode/deck_test.mp4' 2>/dev/null | tail -1)
    if python3 -c "exit(0 if 2.5 <= float('$dur') <= 3.5 else 1)" 2>/dev/null; then
        echo "OK: C deck closed-loop (duration=${dur}s)"
    else
        echo "FAIL: C deck duration=$dur"; FAIL=1
    fi
else
    echo "FAIL: C deck closed-loop"; echo "$out" | tail -5; FAIL=1
fi

# ── 6. C++ field: 真实 server 推流（header-only RAII）────────
note "C++ field push"
g++ -std=c++11 -Wall -Wextra \
    -I bindings/cxx/mediaservo-field-cxx/include -I bindings/cxx/include \
    -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
    bindings/cxx/mediaservo-field-cxx/examples/vehicle_field.cpp \
    -L target/debug -lmediaservo_field -o /tmp/opencode/e2e_cxx
check "C++ field: published" "published track" \
    env LD_LIBRARY_PATH="$PWD/target/debug" \
        MEDIASERVO_SIGNAL_URL="ws://127.0.0.1:9800/ws" \
        MEDIASERVO_PSK="mediaservo-dev" MEDIASERVO_ROOM="vehicle-cxx-e2e" \
        /tmp/opencode/e2e_cxx < /dev/null

# ── 7. Python field: 真实 server 推流 ───────────────────────
note "Python field push"
check "Python field: connected + published" "frames running" \
    python3 -c "
from mediaservo.field import PushConfig, PushSession
s = PushSession.connect(PushConfig(url='ws://127.0.0.1:9800/ws', psk='mediaservo-dev', room='vehicle-py-e2e'))
print('connected')
t = s.publish_video(); print('published:', t)
s.start_video_frames(); print('frames running')
s.stop_video_frames(); s.close()"

# ── 8. server 侧收帧证据 ────────────────────────────────────
note "server key-frame evidence"
KF=$(docker logs mediaservo-server-1 2>&1 | grep -c "ReceiveRtpPacket.*key frame" || true)
if [ "${KF:-0}" -ge 1 ]; then echo "OK: server received key frames ($KF)"; else echo "FAIL: no key frames in server log"; FAIL=1; fi

# ── 聚合 ────────────────────────────────────────────────────
if [ "$FAIL" -eq 0 ]; then echo "BINDINGS E2E PASS"; else echo "BINDINGS E2E FAIL"; exit 1; fi
