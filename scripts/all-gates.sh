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
declare -a RESULTS=() FAILS=0 SKIPS=0
GATE_FACE="${MEDIASERVO_GATE_FACE:-host-pixi}"
PASS() { RESULTS+=("✅ $1"); }
FAIL() { RESULTS+=("❌ $1"); FAILS=$((FAILS+1)); }
SKIP() { RESULTS+=("⏭️ $1 (skip: $2)"); SKIPS=$((SKIPS+1)); }
gate() {  # gate <name> <command...> (per-gate mktemp log — first run caught a shared-name race)
    local name=$1 log; shift
    log=$(mktemp /tmp/all-gate.XXXXXX)
    echo "▶ $name"
    if "$@" >"$log" 2>&1; then PASS "$name"; rm -f "$log"
    else FAIL "$name"; mv "$log" "/tmp/all-gate-fail-$(echo "$name" | tr ' /' '__').log"
         echo "   ↳ $(grep -m1 -E "^error" "/tmp/all-gate-fail-$(echo "$name" | tr ' /' '__').log" 2>/dev/null | cut -c1-80)"; fi
}
port_busy() { ss -tln 2>/dev/null | grep -q ":9800 "; }

# ── gate checker face (SR6 P3): checker logics live in tools/mediaservo-gates (N4 ruling A′) ──
gate "build mediaservo-gates"      cargo build -q -p mediaservo-gates

# ── fmt job, three fragments ──
gate "fmt --check (incremental)"  bash -c '
  base=$(git rev-parse --abbrev-ref "@{u}" 2>/dev/null || echo HEAD)
  files=$(git diff --name-only "$base" -- "*.rs")
  [ -z "$files" ] && { echo "no changed .rs files (base $base)"; exit 0; }
  echo "checking $(echo "$files" | grep -c .) changed files (base $base)"
  rustfmt --edition 2024 --check $files'
gate "changelog markers (F11-X)"  target/debug/mediaservo-gates changelog
gate "wire changelog pairing (G3)"  bash scripts/gate-wire-changelog.sh
gate "bindings version parity"    target/debug/mediaservo-gates parity
gate "crate literal versions"     bash -c '! git grep -l "^version = \"" -- "crates/*/Cargo.toml" | grep -vE "mediaservo-(host|server|field|client)/Cargo.toml" | grep -q .'
gate "crate lanes reconciliation" target/debug/mediaservo-gates lanes
if [ -f "$PWD/../../scripts/gate-py-retirement.sh" ]; then # SR6 P4: tracked .py minus the allowlist = 0, permanently (checker spans both repos = parent-owned)
    gate "python retirement closure"  bash "$PWD/../../scripts/gate-py-retirement.sh" --final
else
    SKIP "python retirement closure" "parent release shell not colocated (standalone checkout)"
fi

# ── compile surface ──
gate "check --workspace"          cargo check --workspace
gate "clippy -D --workspace"      cargo clippy --workspace -- -D warnings
gate "server stub --all-targets"  cargo test -p mediaservo-server --no-default-features --all-targets -- --test-threads=1 # known g3 parallel-race flake (09-11/09-18 double evidence); gate = deterministic verdict

# ── test surface ──
gate "test --workspace (lib)"     cargo test --workspace --lib
gate "openapi validate"           target/debug/mediaservo-gates openapi

# ── heavy gates (--full) ──
if [ "$FULL" = 1 ]; then
    gate "build-c cdylibs"        pixi run build-c
    gate "test-cxx"               bash -c 'LD_LIBRARY_PATH=$PWD/target/debug bash scripts/test-cxx.sh'
    gate "abi drift+client"       bash -c 'bash scripts/check-abi-drift.sh && bash scripts/check-abi-client.sh'
    gate "delivery smoke"         bash scripts/e2e-delivery.sh
    gate "gui example"            bash -c 'MSRTC_RUN_SECS=0 bash mediaservo.sh build example && ctest --test-dir target/examples/build -R core --output-on-failure'
    gate "gstreamer"              pixi run test-gstreamer
    if port_busy; then SKIP "mediasoup e2e" "live instance on 9800, PIT-192"
    else gate "mediasoup"         cargo test -p mediaservo-server --features sfu-mediasoup; fi
else
    SKIP "7 heavy gates" "--full not given"
fi

echo; echo "════════ all-gates summary ════════"
printf '%s\n' "${RESULTS[@]}"
echo "══════════════════════════════════════"
# census row (C46 extension): pass/skip/fail counts + skip list + execution
# face — skips must never read as green (vacuous-green family), the counts make
# them loud; evidence lines elsewhere name this face.
echo "ALL-GATES: PASS" | { [ "$FAILS" = 0 ] && cat || true; }
[ "$FAILS" = 0 ] || echo "ALL-GATES: $FAILS FAILED"
_np=0; for r in "${RESULTS[@]}"; do case "$r" in ✅*) _np=$(( _np + 1 ));; esac; done
echo "census (face=$GATE_FACE): pass=$_np skip=$SKIPS fail=$FAILS"
for r in "${RESULTS[@]}"; do case "$r" in ⏭️*) echo "  $r";; esac; done
[ "$FAILS" = 0 ] || exit 1
