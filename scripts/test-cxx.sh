#!/usr/bin/env bash
# C++ header-only binding tests: compile and run the field/link/deck/client SDK test programs.
# Prereq: the three cdylibs are built (pixi run build-c, incl. .so.0 dev symlink).
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"
export LD_LIBRARY_PATH="$PWD/target/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

for sdk in field link deck client; do
    echo "=== $sdk-cxx ==="
    g++ -std=c++11 -Wall -Wextra \
        -I "bindings/cxx/mediaservo-$sdk-cxx/include" -I bindings/cxx/include \
        -I "bindings/c/mediaservo-$sdk-c/include" -I bindings/c/include \
        "bindings/cxx/mediaservo-$sdk-cxx/tests/test_$sdk.cpp" \
        -L target/debug -lmediaservo_$sdk -o "/tmp/opencode/test_${sdk}_cxx"
    "/tmp/opencode/test_${sdk}_cxx"
    echo "$sdk-cxx tests PASS"
done

echo "=== common-cxx (shared Result contract, C++11) ==="
g++ -std=c++11 -Wall -Wextra -I bindings/cxx/include \
    -I bindings/cxx/mediaservo-field-cxx/include -I bindings/cxx/mediaservo-link-cxx/include -I bindings/cxx/mediaservo-deck-cxx/include \
    -I bindings/c/mediaservo-field-c/include -I bindings/c/mediaservo-link-c/include -I bindings/c/mediaservo-deck-c/include \
    -I bindings/c/include bindings/cxx/tests/test_result_common.cpp \
    -o /tmp/opencode/test_result_common_cxx
"/tmp/opencode/test_result_common_cxx"
echo "common-cxx tests PASS"
