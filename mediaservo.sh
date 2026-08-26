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
exec python "$ROOT/scripts/mediaservo_cli.py" "$@"
