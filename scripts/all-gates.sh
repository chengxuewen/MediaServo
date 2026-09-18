#!/usr/bin/env bash
# all-gates — ci.yml 13 job 的本地合并门（09-18 查证：remote 无 Actions 执行者，
# 本脚本 = 门的真实落地形；ci.yml 保留=GitHub 镜像日自动生效，两形语义同源）。
# 用法: pixi run all-gates [--full]
#   默认 quick: 静态/编译/纯测试门；--full 追加重门（bindings 组装/cxx/gui/gstreamer）。
#   mediasoup e2e / weaknet 门 = 9800 端口占用自动 SKIP（PIT-192 活体互斥纪律）。
# 诚实原则: 存量 lint 债（V 批在册）红即红，脚本不降级不白名单——清完自然绿。
set -uo pipefail
cd "$(dirname "$0")/.."
FULL=0; [ "${1:-}" = "--full" ] && FULL=1
# PIT-193: 父 shell 残留 MESON 变量换 build 指纹 → 重建冲突，任务内层清
unset MESON_ARGS MESON 2>/dev/null || true
declare -a RESULTS=() FAILS=0
PASS() { RESULTS+=("✅ $1"); }
FAIL() { RESULTS+=("❌ $1"); FAILS=$((FAILS+1)); }
SKIP() { RESULTS+=("⏭️ $1 (skip: $2)"); }
gate() {  # gate <名> <命令...>（每门独立 mktemp 日志——首跑抓出共享名竞态）
    local name=$1 log; shift
    log=$(mktemp /tmp/all-gate.XXXXXX)
    echo "▶ $name"
    if "$@" >"$log" 2>&1; then PASS "$name"; rm -f "$log"
    else FAIL "$name"; mv "$log" "/tmp/all-gate-fail-$(echo "$name" | tr ' /' '__').log"
         echo "   ↳ $(grep -m1 -E "^error" "/tmp/all-gate-fail-$(echo "$name" | tr ' /' '__').log" 2>/dev/null | cut -c1-80)"; fi
}
port_busy() { ss -tln 2>/dev/null | grep -q ":9800 "; }

# ── fmt job 三碎片 ──
gate "fmt --check (增量)"         bash -c '
  base=$(git rev-parse --abbrev-ref "@{u}" 2>/dev/null || echo HEAD)
  files=$(git diff --name-only "$base" -- "*.rs")
  [ -z "$files" ] && { echo "无改动 .rs（基线 $base）"; exit 0; }
  echo "检查 $(echo "$files" | grep -c .) 个改动文件（基线 $base）"
  rustfmt --edition 2024 --check $files'
gate "changelog markers (F11-X)"  python3 scripts/gate-changelog.py
gate "bindings version parity"    python3 scripts/gate-parity.py
gate "crate literal versions"     bash -c '! git grep -l "^version = \"" -- "crates/*/Cargo.toml" | grep -vE "mediaservo-(host|server|field|client)/Cargo.toml" | grep -q .'

# ── 编译面 ──
gate "check --workspace"          cargo check --workspace
gate "clippy -D --workspace"      cargo clippy --workspace -- -D warnings
gate "server stub --all-targets"  cargo test -p mediaservo-server --no-default-features --all-targets -- --test-threads=1 # g3 并行竞态在册 flake（09-11/09-18 双证），门=确定性判据

# ── 测试面 ──
gate "test --workspace (lib)"     cargo test --workspace --lib
gate "openapi validate"           python3 scripts/gate-openapi.py

# ── 重门（--full）──
if [ "$FULL" = 1 ]; then
    gate "build-c cdylibs"        pixi run build-c
    gate "test-cxx"               bash -c 'LD_LIBRARY_PATH=$PWD/target/debug bash scripts/test-cxx.sh'
    gate "abi drift+client"       bash -c 'bash scripts/check-abi-drift.sh && bash scripts/check-abi-client.sh'
    gate "delivery smoke"         bash scripts/e2e-delivery.sh
    gate "gui example"            bash -c 'MSRTC_RUN_SECS=0 bash mediaservo.sh build example && ctest --test-dir build-examples -R core --output-on-failure 2>/dev/null || bash mediaservo.sh test example'
    gate "gstreamer"              pixi run test-gstreamer
    if port_busy; then SKIP "mediasoup e2e" "9800 活体占用 PIT-192"
    else gate "mediasoup"         cargo test -p mediaservo-server --features sfu-mediasoup; fi
else
    SKIP "重门 7 件" "未加 --full"
fi

echo; echo "════════ all-gates 汇总 ════════"
printf '%s\n' "${RESULTS[@]}"
echo "══════════════════════════════════════"
[ "$FAILS" = 0 ] && echo "ALL-GATES: PASS" || { echo "ALL-GATES: $FAILS 项 FAIL"; exit 1; }
