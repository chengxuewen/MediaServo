#!/usr/bin/env bash
# ABI 漂移门禁（client 家族，审核 L2 / D248 同法）：
# client.h 声明的 ms_client_* 函数集合 ↔ cdylib 导出符号集合 一一对账。
# 漂移 = header 有而 .so 无（漏导出/改名）或 .so 有而 header 无（漏声明）。
# 前置: cargo build -p mediaservo-client-c。binutils(readelf) 来自 pixi 环境——
# 推荐经 `pixi run bash scripts/check-abi-client.sh` 执行（同 check-abi-drift.sh 纪律）。
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"

SO="target/debug/libmediaservo_client.so"
HDR="bindings/c/mediaservo-client-c/include/mediaservo/client.h"

if ! command -v readelf >/dev/null 2>&1; then
    echo "FAIL: readelf 不可用——请用 pixi run bash scripts/check-abi-client.sh 执行"
    exit 1
fi
if [ ! -f "$SO" ]; then
    echo "FAIL: $SO 未构建（先 cargo build -p mediaservo-client-c）"
    exit 1
fi

# header 声明: 行首返回类型 mediaservo_err_t（排除注释/宏/typedef）
declared=$(grep -oE "^mediaservo_err_t ms_client_[a-z_]+" "$HDR" | awk '{print $2}' | sort -u)
# .so 导出: GLOBAL 定义符号（排除 UND）
exported=$(readelf -W --dyn-syms "$SO" | grep " GLOBAL " | grep -v " UND " \
    | grep -oE "ms_client_[a-z_]+" | sort -u)

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
