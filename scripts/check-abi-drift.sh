#!/usr/bin/env bash
# ABI 漂移门禁（审核 L2 / D248）: header 声明函数集合 ↔ cdylib 导出符号集合对照。
# 漂移 = header 有而 .so 无（漏导出/改名）或 .so 有而 header 无（漏声明）。
# 前置: 三个 cdylib 已构建（pixi run build-c）。
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.pixi/bin:$PATH"
FAIL=0

for sdk in field link deck client; do
    # client C 面前缀=ms_client_（非 mediaservo_client_，V2 入列时核实）
    case "$sdk" in client) SYM="ms_client_";; *) SYM="mediaservo_${sdk}_";; esac
    HDR="bindings/c/mediaservo-$sdk-c/include/mediaservo/$sdk.h"
    SO="target/debug/libmediaservo_$sdk.so"
    if [ ! -f "$SO" ]; then
        echo "SKIP $sdk: $SO 未构建（先 pixi run build-c）"
        continue
    fi
    # header 声明: 行首为返回类型（排除注释/宏/typedef）
    declared=$(grep -oE "^(mediaservo_err_t|void|int) ${SYM}[a-z_]+" "$HDR" | awk '{print $2}' | sort -u)
    # .so 导出: GLOBAL 定义符号（排除 UND）
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
