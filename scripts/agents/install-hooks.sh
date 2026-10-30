#!/usr/bin/env bash
# install-hooks.sh — copy the repo hook into the real hooks dir.
# Works for normal clones and submodules (the git dir may live under
# .git/modules/<name>, so `git rev-parse --git-path hooks` is the only safe source).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."

hooks="$(git rev-parse --git-path hooks)"
mkdir -p "$hooks"
cp .githooks/pre-commit "$hooks/pre-commit"
chmod +x "$hooks/pre-commit"
echo "installed $hooks/pre-commit"
