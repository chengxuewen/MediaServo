#!/usr/bin/env bash
# sync-official-refs.sh — fetch/update the official mediasoup client reference sources (for cross-platform/cross-host development)
#
# Purpose: a stable official-source baseline for AI analysis reference (config dir .refinfo/, git-ignored).
#       Avoids /tmp scratch dirs (lost across hosts/reboots); shallow clone (--depth 1), sources only, no history.
#
# Repos: only the 3 official client baselines (the sole authoritative comparison for the negotiation flow).
#       - mediasoup-client (JS)     — Chrome74.ts send(): standard produce-negotiation baseline
#       - libmediasoupclient (C++)   — Handler.cpp Send(): official C++ client baseline
#       - mediasoup-demo            — RoomClient.js: official demo end-to-end flow
#       Deliberately excluded: mediasoup (server — consumed as-is, never modified) and mediasoup-client-android (third-party fork, irrelevant).
#
# Network: GitHub interference in mainland China (PIT-14) → prefer native git, fall back to --http1.1 / proxy on failure.
#
# Idempotent: skips existing directories (never overwrites); use --force to re-fetch.
# Usage: bash scripts/sync-official-refs.sh [--force]

set -euo pipefail

REFS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.refinfo"
FORCE=0
[[ "${1:-}" == "--force" ]] && FORCE=1

# url|dir  (git clone <url> <refs_dir>/<dir>)
REPOS=(
  "https://github.com/versatica/mediasoup-client    mediasoup-client"
  "https://github.com/versatica/libmediasoupclient  libmediasoupclient"
  "https://github.com/versatica/mediasoup-demo      mediasoup-demo"
  "https://github.com/eclipse-iceoryx/iceoryx2      iceoryx2"  # PIT-14: analysis only, not an official dependency, optional
  "https://github.com/livekit/rust-sdks             livekit-sdk-rust"  # PIT-14: analysis only, not an official dependency, optional
  "https://github.com/livekit/client-sdk-cpp        livekit-sdk-cpp"  # PIT-14: analysis only, not an official dependency, optional
  "https://github.com/livekit/node-sdks             livekit-sdk-node"  # PIT-14: analysis only, not an official dependency, optional
  "https://github.com/livekit/python-sdks           livekit-sdk-python"  # PIT-14: analysis only, not an official dependency, optional
)

mkdir -p "$REFS_DIR"

clone_with_fallback() {
  local url="$1" dir="$2"
  local target="$REFS_DIR/$dir"
  if [[ -d "$target/.git" && $FORCE -eq 0 ]]; then
    echo "✓ $dir — already present, skipping (--force to re-fetch)"
    return 0
  fi
  if [[ -d "$target/.git" ]]; then
    echo "→ $dir — forced re-fetch"
    rm -rf "$target"
  fi
  echo "→ fetching $dir ..."
  # Try native git first, fall back to --http1.1 on failure (PIT-14: HTTP/2 gets interfered)
  if git clone --depth 1 --single-branch "$url" "$target" 2>/dev/null; then
    echo "  ✓ $dir done"
  elif git clone --depth 1 --single-branch --config http.version=HTTP/1.1 "$url" "$target" 2>/dev/null; then
    echo "  ✓ $dir done (http1.1 fallback)"
  else
    echo "  ✗ $dir fetch failed (network issue — see PIT-14: set HTTPS_PROXY or retry)"
    return 1
  fi
}

fail=0
for entry in "${REPOS[@]}"; do
  url="${entry%% *}"
  dir="${entry##* }"
  clone_with_fallback "$url" "$dir" || fail=1
done

echo ""
echo "── reference source directory ─────────────────"
echo "  $REFS_DIR"
for entry in "${REPOS[@]}"; do
  dir="${entry##* }"
  if [[ -d "$REFS_DIR/$dir/.git" ]]; then
    commit="$(git -C "$REFS_DIR/$dir" rev-parse --short HEAD 2>/dev/null || echo '?')"
    echo "  • $dir @ $commit"
  fi
done
  echo "  navigation: docs/reference/webrtc/mediasoup-refs.md"
  echo "  cleanup: rm -rf $REFS_DIR"
echo "──────────────────────────────────────────────"

[[ $fail -eq 0 ]] || echo "warning: some repositories failed to fetch — check the network and retry"
exit $fail