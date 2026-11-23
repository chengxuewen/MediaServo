#!/usr/bin/env bash
# build-env-hygiene L2 (D2=A fail-only, 2026-11-23). Host preflight for the cmake
# build tree, detecting the two 2026-11-18 incident shapes (a root container
# wrote /workspace-path CMake caches into the host build/):
#   1. CMakeCache.txt whose CMAKE_HOME_DIRECTORY != this repo -> cmake hard-fails mid-build
#   2. build/ top-level entries not owned by the current user  -> cmake --fresh cannot clear them
# Exit 1 with one-paste fixes; NEVER deletes or moves user data.
# Escape hatch: MSRTC_BUILD_HYGIENE_SKIP=1 (printed on every block).
# Test seams: MSRTC_PREFLIGHT_BUILD_ROOT (default <repo>/build),
#             MSRTC_PREFLIGHT_REPO_ROOT  (default script/..).
set -uo pipefail

REPO_ROOT="${MSRTC_PREFLIGHT_REPO_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
BUILD_ROOT="${MSRTC_PREFLIGHT_BUILD_ROOT:-$REPO_ROOT/build}"

if [ "${MSRTC_BUILD_HYGIENE_SKIP:-0}" = "1" ]; then
    echo "[preflight] MSRTC_BUILD_HYGIENE_SKIP=1 — 跳过构建缓存体检（风险自负）" >&2
    exit 0
fi
[ -d "$BUILD_ROOT" ] || exit 0

real_repo="$(cd "$REPO_ROOT" && pwd -P)"

bad_cache=()
while IFS= read -r cache; do
    [ -f "$cache" ] || continue
    home="$(sed -n 's/^CMAKE_HOME_DIRECTORY:INTERNAL=//p' "$cache" | head -1)"
    [ -n "$home" ] || continue
    home_real="$(cd "$home" 2>/dev/null && pwd -P || printf '%s' "$home")"
    if [ "$home_real" != "$real_repo" ]; then
        bad_cache+=("$cache")
    fi
done < <(find "$BUILD_ROOT" -maxdepth 2 -name CMakeCache.txt 2>/dev/null)

bad_owner=()
while IFS= read -r entry; do
    [ -n "$entry" ] && bad_owner+=("$entry")
done < <(find "$BUILD_ROOT" -mindepth 1 -maxdepth 1 ! -user "$(id -u)" 2>/dev/null)

if [ ${#bad_cache[@]} -eq 0 ] && [ ${#bad_owner[@]} -eq 0 ]; then
    exit 0
fi

echo "[preflight] 构建目录体检未通过 —— 防止「别的环境/容器写坏的缓存」把构建带偏" >&2
if [ ${#bad_cache[@]} -gt 0 ]; then
    echo "[preflight] 缓存路径不属于当前仓库（曾在其他环境生成，例如容器的 /workspace）:" >&2
    for c in "${bad_cache[@]}"; do
        echo "    - $c" >&2
    done
    echo "[preflight] 修复（缓存会重建，只丢编译进度）:" >&2
    for c in "${bad_cache[@]}"; do
        echo "    rm -rf \"$(dirname "$c")\"" >&2
    done
fi
if [ ${#bad_owner[@]} -gt 0 ]; then
    echo "[preflight] 以下条目属主不是当前用户（通常是 root 容器写过）:" >&2
    for d in "${bad_owner[@]}"; do
        echo "    - $d" >&2
    done
    printf '[preflight] 修复（脏缓存不救，直接删）: sudo rm -rf' >&2
    for d in "${bad_owner[@]}"; do
        printf ' "%s"' "$d" >&2
    done
    printf '\n' >&2
fi
echo "[preflight] 自知风险仍要继续: MSRTC_BUILD_HYGIENE_SKIP=1" >&2
exit 1
