#!/usr/bin/env bash
# mediaservo.sh — MediaServo CLI 薄壳（Linux/macOS）
# 职责: ① 检测 pixi（缺失提示 bootstrap）② 激活环境 ③ 转发到 CLI
# 用法: ./mediaservo.sh <cmd> [-h]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"

# v2: pixi 检测统一 — 优先 command -v，回退 ~/.pixi/bin（导出 PIXI_BIN 供 pixi-shell 使用）
if command -v pixi >/dev/null 2>&1; then
    export PIXI_BIN="$(command -v pixi)"
elif [ -x "$HOME/.pixi/bin/pixi" ]; then
    export PIXI_BIN="$HOME/.pixi/bin/pixi"
else
    echo "pixi 未安装 — 先运行: source bootstrap.sh" >&2
    exit 1
fi

# -h/--help/version: 跳过 pixi 激活（评审 H1：激活横幅污染 -h 输出——usage 前 8 行横幅）
case "${1:-}" in
    -h|--help|version)
        exec python3 "$ROOT/scripts/mediaservo_cli.py" "$@"   # 裸 python3（顶部 stdlib-only——免激活噪音）
        ;;
esac
source "$ROOT/scripts/pixi-shell.sh"   # 激活（同进程，PATH/LIBCLANG 注入）

# build:deploy 组合（2026-09-29：与主仓 msrtc.sh 同动词面对齐——子模块壳独立工作
# 时不用回主仓。缺省 prefix=<out>/<target>（_out_root 解析在 CLI 层——bindings
# 缺省即此处；host/server 组合形态仍显式传 --prefix out/<target> 保持对称）。
# host/server 带 deploy 的运行数据写入 out/<target>——与 msrtc.sh 语义一致。
if [ "${1:-}" = "build:deploy" ] && [ $# -ge 2 ]; then
    TARGET="$2"
    shift 2
    OUT_ROOT="${OUT_ROOT:-$ROOT/out}"   # 子模块壳无 MSRTC_OUT_ROOT 注入（主仓壳职责）——显式缺省
    BUILD_FLAGS=(build "$TARGET")
    DEPLOY_FLAGS=(deploy "$TARGET" --prefix "$OUT_ROOT/$TARGET")
    while [ $# -gt 0 ]; do
        case "$1" in
            --prefix) DEPLOY_FLAGS+=(--prefix "$2"); shift 2;;   # 显式覆盖缺省
            --release) BUILD_FLAGS+=(--release); DEPLOY_FLAGS+=(--release); shift;;
            *) BUILD_FLAGS+=("$1"); DEPLOY_FLAGS+=("$1"); shift;;
        esac
    done
    python "$ROOT/scripts/mediaservo_cli.py" "${BUILD_FLAGS[@]}" \
        && python "$ROOT/scripts/mediaservo_cli.py" "${DEPLOY_FLAGS[@]}"
    exit $?
fi

exec python "$ROOT/scripts/mediaservo_cli.py" "$@"
