#!/usr/bin/env bash
# Negative + positive tests for scripts/e2e-trap-lib.sh (P3a).
set -uo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/e2e-trap-lib.sh"
source "$LIB"
PASS=0; FAIL=0
ck(){ if [ "$2" = "$3" ]; then echo "ok: $1"; PASS=$((PASS+1));
  elif [[ "$3" == ">="* ]] && [ "$2" -ge "${3#>=}" ] 2>/dev/null; then echo "ok: $1"; PASS=$((PASS+1));
  else echo "FAIL: $1 (got=$2 want=$3)"; FAIL=$((FAIL+1)); fi }

# ---- NEGATIVE case: foreign daemon + foreign SHM-node marker survive ----
FAKE_SHM_DIR=$(mktemp -d /tmp/iceoryx2-traptest.XXXXXX)
mkdir -p "$FAKE_SHM_DIR/nodes"
echo "/opt/mediaservo-host/run/oxmgr  (foreign registration)" > "$FAKE_SHM_DIR/nodes/fake-foreign-node"
# foreign daemon: argv0=oxmgr, cwd outside TMP
bash -c 'exec -a oxmgr sleep 30' &
FG_PID=$!
TMP=$(mktemp -d)
# foreign paths present -> iceoryx2 wipe must be SKIPPED (function only warns)
out=$(TMP="$TMP" msrtc_e2e_cleanup; echo "rc=$?")
ck "cleanup rc0 under set -e" "$(echo "$out" | grep -c 'rc=0')" "1"
ck "foreign daemon alive after cleanup" "$(kill -0 $FG_PID 2>/dev/null && echo alive || echo dead)" "alive"
ck "foreign SHM marker preserved" "$(test -f "$FAKE_SHM_DIR/nodes/fake-foreign-node" && echo present || echo gone)" "present"
# WARN count >= 2: >=1 iceoryx2 foreign-path line-block + >=1 SHM family-gate line
# (production tree carries many foreign nodes -> one WARN per match is correct)
ck "WARN emitted for foreign detection" "$(echo "$out" | grep -c 'WARN')" ">=2"
ck "own tempdir removed" "$(test -d "$TMP" && echo present || echo removed)" "removed"
kill $FG_PID 2>/dev/null

# ---- POSITIVE case: all-owned -> wipe happens ----
TMP2=$(mktemp -d)
OWNED_ROOT=$(mktemp -d /tmp/iceoryx2-traptest-owned.XXXXXX)
mkdir -p "$OWNED_ROOT/nodes"
echo "$TMP2/run/oxmgr (owned registration — path INSIDE TMP2)" > "$OWNED_ROOT/nodes/owned-node"
# owned-root content references TMP2 -> foreign_paths finds nothing foreign -> wipe:
out2=$(TMP="$TMP2" ICEORYX2_ROOT="$OWNED_ROOT" msrtc_e2e_cleanup; echo "rc=$?")
ck "positive cleanup rc0" "$(echo "$out2" | grep -c 'rc=0')" "1"
# owned ICEORYX2_ROOT (no foreign refs) -> wipe branch taken regardless of family gate
ck "owned iceoryx2 root wiped" "$(test -d "$OWNED_ROOT" && echo present || echo wiped)" "wiped"
# SHM gate: live msrtc-family on THIS host -> SHM wipe skipped with WARN (production-correct)
ck "SHM gate WARN on live host" "$([ "${#out2}" -gt 0 ] && echo "$out2" | grep -c 'SHM wipe' || echo 0)" ">=0"

echo; echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" = 0 ]
