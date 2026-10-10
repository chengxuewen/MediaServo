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

# oxmgr 固化注入（方案 Y，对齐主仓 msrtc.sh——不可镜像下载点档案层）：仓内 3rdparty/oxmgr/
# 只存上游 tar.gz+.sha256（文件名即坐标），解包产物进 .cache/oxmgr/<档案名>/（gitignored、
# 可再生、目录含版本=升版自然失效）。解包前 sha256 对账，不符拒解。未知架构不注入+WARN
# （由 PATH 自装兜底——deploy 侧 _resolve_oxmgr_or_die 硬门）。
OXMGR_TRIPLE=""
case "$(uname -m)" in
    x86_64)  OXMGR_TRIPLE="x86_64-unknown-linux-musl" ;;
    aarch64) OXMGR_TRIPLE="aarch64-unknown-linux-musl" ;;
esac
if [ -n "$OXMGR_TRIPLE" ]; then
    OXMGR_PKG="oxmgr-v0.5.0-${OXMGR_TRIPLE}"   # 版本单源=3rdparty/oxmgr/ 档案文件名，bump 时改此行
    OXMGR_TARBALL="${ROOT}/3rdparty/oxmgr/${OXMGR_PKG}.tar.gz"
    OXMGR_CACHE="${ROOT}/.cache/oxmgr/${OXMGR_PKG}"
    if [ ! -f "$OXMGR_TARBALL" ]; then
        echo "[mediaservo] WARN: oxmgr 档案缺位（${OXMGR_TARBALL}）——依赖 PATH 自装" >&2
    elif [ -f "${OXMGR_TARBALL}.sha256" ] && ! (cd "${ROOT}/3rdparty/oxmgr" && sha256sum -c "${OXMGR_PKG}.tar.gz.sha256" >/dev/null 2>&1); then
        echo "[mediaservo] ERROR: oxmgr 档案 sha256 对账失败——拒绝解包（重新固化档案层）" >&2
    elif [ ! -x "${OXMGR_CACHE}/oxmgr" ]; then
        mkdir -p "$OXMGR_CACHE" && tar xzf "$OXMGR_TARBALL" -C "$OXMGR_CACHE" 2>/dev/null \
            && chmod +x "${OXMGR_CACHE}/oxmgr"
    fi
    [ -x "${OXMGR_CACHE:-/nonexistent}/oxmgr" ] && export PATH="${OXMGR_CACHE}:${PATH}"
else
    echo "[mediaservo] WARN: uname -m=$(uname -m) 无固化 oxmgr（上游无此平台发布）——需自装进 PATH" >&2
fi

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
