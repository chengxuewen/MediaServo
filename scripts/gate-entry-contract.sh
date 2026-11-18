#!/usr/bin/env bash
# gate-entry-contract.sh -- mediaservo.sh entry contract E1-E3 + -h golden snapshot (adjudication 7=A+).
# Mirrors the parent scripts/gate-shell.sh assertion style: same expect_rc/ok/bad
# helpers, same --self-test planted-mismatch discipline. The submodule main shell
# had zero entry-contract coverage before this gate (r2 audit: all-gates checked
# crates/tests only) -- B7 verb retirements (run/start/stop/restart) extend this
# file with their four-verbs-to-unknown assertions at deletion time.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p .cache/tmp
OUT="$(mktemp "${PWD}/.cache/tmp/entry.XXXXXX")"
cleanup() { rm -f "$OUT"; }
trap cleanup EXIT

FAIL=0
ok() { echo " [ OK ] $*"; }
bad() { echo " [FAIL] $*" >&2; FAIL=1; }

expect_rc() { # label want got
    if [ "$3" -eq "$2" ]; then ok "$1"; else bad "$1 (want $2 got $3)"; fi
}

# --self-test: a planted mismatch MUST flip FAIL=1 (assertion-honesty check).
if [ "${1:-}" = "--self-test" ]; then
    FAIL=0
    expect_rc "self-test planted mismatch" 2 0
    if [ "$FAIL" -eq 1 ]; then
        echo "GATE-ENTRY-CONTRACT self-test ✓ (mismatch detected)"
        exit 0
    fi
    echo "GATE-ENTRY-CONTRACT self-test ✗ (mismatch missed)" >&2
    exit 1
fi

# E1: -h exits 0, stays compact (measured 52 lines at first shot), no activation banner.
./mediaservo.sh -h >"$OUT" 2>&1; rc=$?
banner=0
grep -q 'environment active' "$OUT" && banner=1
if [ "$rc" -eq 0 ] && [ "$banner" -eq 0 ] && [ "$(wc -l <"$OUT")" -lt 60 ]; then
    ok "E1 compact -h"
else
    bad "E1 -h rc=$rc lines=$(wc -l <"$OUT") banner=$banner"
fi

# E2: no args -> empty-verb unknown pointer + exit 2 (submodule contract:
# the unknown-verb hint doubles as the no-arg surface; unlike the parent
# shell it does not render a full usage page -- measured behavior 2026-11-06).
./mediaservo.sh >"$OUT" 2>&1; rc=$?
expect_rc "E2 no-arg" 2 "$rc"
grep -q "unknown verb ''" "$OUT" && ok "E2 no-arg pointer" || bad "E2 no-arg pointer"

# E3: unknown verb -> explicit pointer + exit 2.
./mediaservo.sh definitely-not-a-verb >"$OUT" 2>&1; rc=$?
expect_rc "E3 unknown verb" 2 "$rc"
grep -q 'unknown verb' "$OUT" && ok "E3 unknown-verb pointer" || bad "E3 unknown-verb pointer"

# B7 (adjudication O-3=D+D1): the run-plane verbs are deleted -- each must
# fall through to the unknown-verb surface (hint + exit 2), never a lane.
for v in run start stop restart; do
    ./mediaservo.sh "$v" >"$OUT" 2>&1; rc=$?
    expect_rc "B7 $v -> unknown" 2 "$rc"
    grep -q "unknown verb '$v'" "$OUT" && ok "B7 $v pointer" || bad "B7 $v pointer"
done

# B4 (container-build-rework R1/R5): the build image / dev verb family faces
# and bad-arg contracts. (The --image deprecation redirect itself builds an
# image, so the gate asserts the static deprecation note in build -h, not a
# live build.)
./mediaservo.sh build image -h >"$OUT" 2>&1; rc=$?
expect_rc "B4 build image -h" 0 "$rc"
grep -q "build image <prod|dev>" "$OUT" && ok "B4 build image -h face" || bad "B4 build image -h face"
./mediaservo.sh dev -h >"$OUT" 2>&1; rc=$?
expect_rc "B4 dev -h" 0 "$rc"
grep -q "dev <up|down|ssh|web>" "$OUT" && ok "B4 dev -h face" || bad "B4 dev -h face"
./mediaservo.sh build image bogus >"$OUT" 2>&1; rc=$?
expect_rc "B4 build image bad face" 2 "$rc"
./mediaservo.sh dev bogus >"$OUT" 2>&1; rc=$?
expect_rc "B4 dev bad sub" 2 "$rc"
./mediaservo.sh build -h >"$OUT" 2>&1
grep -q "DEPRECATED" "$OUT" && ok "B4 --image deprecation note in build -h" || bad "B4 --image deprecation note"

# G1: -h golden snapshot -- help text is contract; a diff means the CLI face
# changed and the fixture must be re-shot deliberately (state what changed).
if [ -f scripts/fixtures/mediaservo-help.golden ]; then
    ./mediaservo.sh -h >"$OUT" 2>&1
    if diff -q "$OUT" scripts/fixtures/mediaservo-help.golden >/dev/null; then
        ok "G1 -h golden snapshot"
    else
        bad "G1 golden drift — re-shot: ./mediaservo.sh -h > scripts/fixtures/mediaservo-help.golden (state what changed)"
    fi
else
    bad "G1 golden fixture missing (first shot: ./mediaservo.sh -h > scripts/fixtures/mediaservo-help.golden)"
fi

[ "$FAIL" -eq 0 ] && echo "GATE-ENTRY-CONTRACT ✓" || echo "GATE-ENTRY-CONTRACT ✗"
exit "$FAIL"
