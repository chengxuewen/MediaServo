#!/usr/bin/env bash
# ABI drift gate (review L2 / D248): compare the set of functions declared in headers against the symbols exported by the cdylibs.
# Drift = declared but not exported (missing export / rename) or exported but not declared (missing declaration).
# Prereq: the three cdylibs are built (pixi run build-c).
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"
FAIL=0

for sdk in field link deck client; do
    SYM="mediaservo_${sdk}_"
    HDR="bindings/c/mediaservo-$sdk-c/include/mediaservo/$sdk.h"
    SO="target/debug/libmediaservo_$sdk.so"
    if [ ! -f "$SO" ]; then
        echo "SKIP $sdk: $SO not built (run pixi run build-c first)"
        continue
    fi
    # Header declarations: return type at line start (excludes comments/macros/typedefs)
    declared=$(grep -oE "^(mediaservo_err_t|void|int) ${SYM}[a-z_]+" "$HDR" | awk '{print $2}' | sort -u)
    # .so exports: GLOBAL defined symbols (excludes UND)
    exported=$(readelf -W --dyn-syms "$SO" | grep " GLOBAL " | grep -v " UND " \
        | grep -oE "${SYM}[a-z_]+" | sort -u)
    missing=$(comm -23 <(printf '%s\n' "$declared") <(printf '%s\n' "$exported"))
    undeclared=$(comm -13 <(printf '%s\n' "$declared") <(printf '%s\n' "$exported"))
    if [ -n "$missing" ] || [ -n "$undeclared" ]; then
        FAIL=1
        echo "== $sdk DRIFT =="
        [ -n "$missing" ] && echo "  declared-but-not-exported: $(echo $missing | tr '\n' ' ')"
        [ -n "$undeclared" ] && echo "  exported-but-not-declared: $(echo $undeclared | tr '\n' ' ')"
    else
        echo "$sdk: $(printf '%s\n' "$declared" | wc -l) declared == $(printf '%s\n' "$exported" | wc -l) exported OK"
    fi
done

[ "$FAIL" -eq 0 ] && echo "ABI DRIFT CHECK PASS" || { echo "ABI DRIFT CHECK FAIL"; exit 1; }
