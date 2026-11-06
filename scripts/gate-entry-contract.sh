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
