#!/usr/bin/env bash
# Cross-language parity check (D227 ⑤ / contract M5):
# the same operation sequence (version / connect error path with an empty config / non-empty last_error)
# runs on C / C++ / Python and the results must match — guards against API drift.
#
# Prereq: the three cdylibs are built (pixi run build-c); server not required (error path only).
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"
export LD_LIBRARY_PATH="$PWD/target/debug${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export MEDIASERVO_LIB_DIR="$PWD/target/debug"
export PYTHONPATH="$PWD/bindings/python/mediaservo${PYTHONPATH:+:$PYTHONPATH}"

# ── 1. C ───────────────────────────────────────────────
cat > /tmp/opencode/parity_c.c <<'EOF'
#include <stdio.h>
#include <string.h>
#include "mediaservo/field.h"
int main(void) {
    char ver[64];
    mediaservo_field_version(ver, sizeof(ver));
    /* empty required fields (url/psk/room all NULL) → INVALID_ARG */
    mediaservo_push_config_t cfg = MEDIASERVO_PUSH_CONFIG_DEFAULT;
    mediaservo_field_push_t* s = NULL;
    int rc = mediaservo_field_push_connect(&cfg, &s);
    char err[256];
    mediaservo_field_last_error(err, sizeof(err));
    printf("C version=%s rc=%d err_len=%zu\n", ver, rc, strlen(err));
    return 0;
}
EOF
gcc /tmp/opencode/parity_c.c -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
    -L target/debug -lmediaservo_field -o /tmp/opencode/parity_c
C_OUT=$(/tmp/opencode/parity_c)
echo "$C_OUT"

# ── 2. C++ (reuses the field-cxx test: version assert + error-path asserts) ──
g++ -std=c++11 -Wall -Wextra \
    -I bindings/cxx/mediaservo-field-cxx/include -I bindings/cxx/include \
    -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
    bindings/cxx/mediaservo-field-cxx/tests/test_field.cpp \
    -L target/debug -lmediaservo_field -o /tmp/opencode/parity_cxx
/tmp/opencode/parity_cxx
echo "CXX tests PASS (version + error path asserts)"

# ── 3. Python ──────────────────────────────────────────
PY_OUT=$(python3 - <<'EOF'
from mediaservo.field import PushConfig, PushSession, version, FieldError
v = version()
try:
    PushSession.connect(PushConfig("", "", ""))
    rc = 0
except FieldError as e:
    rc = e.code
print("PY version=%s rc=%d" % (v, rc))
EOF
)
echo "$PY_OUT"

# ── 4. Assert consistency ───────────────────────────────
C_V=$(echo "$C_OUT" | sed -n 's/.*version=\([^ ]*\).*/\1/p')
PY_V=$(echo "$PY_OUT" | sed -n 's/.*version=\([^ ]*\).*/\1/p')
[ -n "$C_V" ] && [ "$C_V" = "$PY_V" ] || { echo "PARITY FAIL: version mismatch C=$C_V PY=$PY_V"; exit 1; }
echo "$C_OUT" | grep -q "rc=-1" || { echo "PARITY FAIL: C rc != -1"; exit 1; }
echo "$PY_OUT" | grep -q "rc=-1" || { echo "PARITY FAIL: PY rc != -1"; exit 1; }
echo "PARITY OK: version=$C_V rc=-1 consistent across all three languages"
