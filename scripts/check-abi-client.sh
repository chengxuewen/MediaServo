#!/usr/bin/env bash
# ABI drift gate (client family, same method as review L2 / D248):
# the mediaservo_client_* functions declared in client.h vs the symbols exported by the cdylib — exact one-to-one check.
# Drift = declared but not exported (missing export / rename) or exported but not declared (missing declaration).
# Prereq: cargo build -p mediaservo-client-c. binutils (readelf) comes from the pixi environment —
# run it via `pixi run bash scripts/check-abi-client.sh` (same discipline as check-abi-drift.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"

SO="target/debug/libmediaservo_client.so"
HDR="bindings/c/mediaservo-client-c/include/mediaservo/client.h"

if ! command -v readelf >/dev/null 2>&1; then
    echo "FAIL: readelf unavailable — run via pixi run bash scripts/check-abi-client.sh"
    exit 1
fi
if [ ! -f "$SO" ]; then
    echo "FAIL: $SO not built (run cargo build -p mediaservo-client-c first)"
    exit 1
fi

# Header declarations: return type mediaservo_err_t at line start (excludes comments/macros/typedefs)
declared=$(grep -oE "^mediaservo_err_t mediaservo_client_[a-z_]+" "$HDR" | awk '{print $2}' | sort -u)
# .so exports: GLOBAL defined symbols (excludes UND)
exported=$(readelf -W --dyn-syms "$SO" | grep " GLOBAL " | grep -v " UND " \
    | grep -oE "mediaservo_client_[a-z_]+" | sort -u)

missing=$(comm -23 <(printf '%s\n' "$declared") <(printf '%s\n' "$exported"))
undeclared=$(comm -13 <(printf '%s\n' "$declared") <(printf '%s\n' "$exported"))

if [ -n "$missing" ] || [ -n "$undeclared" ]; then
    echo "== client DRIFT =="
    [ -n "$missing" ] && echo "  declared-but-not-exported: $(echo "$missing" | tr '\n' ' ')"
    [ -n "$undeclared" ] && echo "  exported-but-not-declared: $(echo "$undeclared" | tr '\n' ' ')"
    echo "ABI CLIENT CHECK FAIL"
    exit 1
fi

echo "client: $(printf '%s\n' "$declared" | wc -l) declared == $(printf '%s\n' "$exported" | wc -l) exported OK"
echo "ABI CLIENT CHECK PASS"
