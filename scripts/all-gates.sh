#!/usr/bin/env bash
# all-gates — local merged gate for the 13 ci.yml jobs (verified 09-18: the remote has no Actions runner,
# this script = the gate's real execution form; ci.yml stays for auto-activation on the GitHub mirror day, both forms share one semantics).
# Usage: pixi run all-gates [--full]
#   Default quick: static/compile/pure-test gates; --full adds heavy gates (bindings assembly/cxx/gui/gstreamer).
#   mediasoup e2e / weaknet gates = auto-SKIP while port 9800 is occupied (PIT-192 live-instance mutual exclusion).
# Honesty principle: outstanding lint debt (logged in the V batch) stays red — no downgrade, no whitelist — it goes green when paid off.
set -uo pipefail
cd "$(dirname "$0")/.."
FULL=0; [ "${1:-}" = "--full" ] && FULL=1
# PIT-193: MESON vars inherited from the parent shell change the build fingerprint → rebuild conflicts; clear inside the task
unset MESON_ARGS MESON 2>/dev/null || true
declare -a RESULTS=() FAILS=0
PASS() { RESULTS+=("✅ $1"); }
FAIL() { RESULTS+=("❌ $1"); FAILS=$((FAILS+1)); }
SKIP() { RESULTS+=("⏭️ $1 (skip: $2)"); }
gate() {  # gate <name> <command...> (per-gate mktemp log — first run caught a shared-name race)
    local name=$1 log; shift
    log=$(mktemp /tmp/all-gate.XXXXXX)
    echo "▶ $name"
    if "$@" >"$log" 2>&1; then PASS "$name"; rm -f "$log"
    else FAIL "$name"; mv "$log" "/tmp/all-gate-fail-$(echo "$name" | tr ' /' '__').log"
         echo "   ↳ $(grep -m1 -E "^error" "/tmp/all-gate-fail-$(echo "$name" | tr ' /' '__').log" 2>/dev/null | cut -c1-80)"; fi
}
port_busy() { ss -tln 2>/dev/null | grep -q ":9800 "; }

# ── fmt job, three fragments ──
gate "fmt --check (incremental)"  bash -c '
  base=$(git rev-parse --abbrev-ref "@{u}" 2>/dev/null || echo HEAD)
  files=$(git diff --name-only "$base" -- "*.rs")
  [ -z "$files" ] && { echo "no changed .rs files (base $base)"; exit 0; }
  echo "checking $(echo "$files" | grep -c .) changed files (base $base)"
  rustfmt --edition 2024 --check $files'
gate "changelog markers (F11-X)"  python3 scripts/gate-changelog.py
gate "bindings version parity"    python3 scripts/gate-parity.py
gate "crate literal versions"     bash -c '! git grep -l "^version = \"" -- "crates/*/Cargo.toml" | grep -vE "mediaservo-(host|server|field|client)/Cargo.toml" | grep -q .'

# ── compile surface ──
gate "check --workspace"          cargo check --workspace
gate "clippy -D --workspace"      cargo clippy --workspace -- -D warnings
gate "server stub --all-targets"  cargo test -p mediaservo-server --no-default-features --all-targets -- --test-threads=1 # known g3 parallel-race flake (09-11/09-18 double evidence); gate = deterministic verdict

# ── test surface ──
gate "test --workspace (lib)"     cargo test --workspace --lib
gate "openapi validate"           python3 scripts/gate-openapi.py

# ── heavy gates (--full) ──
if [ "$FULL" = 1 ]; then
    gate "build-c cdylibs"        pixi run build-c
    gate "test-cxx"               bash -c 'LD_LIBRARY_PATH=$PWD/target/debug bash scripts/test-cxx.sh'
    gate "abi drift+client"       bash -c 'bash scripts/check-abi-drift.sh && bash scripts/check-abi-client.sh'
    gate "delivery smoke"         bash scripts/e2e-delivery.sh
    gate "gui example"            bash -c 'MSRTC_RUN_SECS=0 bash mediaservo.sh build example && ctest --test-dir build-examples -R core --output-on-failure 2>/dev/null || bash mediaservo.sh test example'
    gate "gstreamer"              pixi run test-gstreamer
    if port_busy; then SKIP "mediasoup e2e" "live instance on 9800, PIT-192"
    else gate "mediasoup"         cargo test -p mediaservo-server --features sfu-mediasoup; fi
else
    SKIP "7 heavy gates" "--full not given"
fi

echo; echo "════════ all-gates summary ════════"
printf '%s\n' "${RESULTS[@]}"
echo "══════════════════════════════════════"
[ "$FAILS" = 0 ] && echo "ALL-GATES: PASS" || { echo "ALL-GATES: $FAILS FAILED"; exit 1; }
