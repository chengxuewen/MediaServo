#!/usr/bin/env bash
# Leak detection (O-2, gitignore-hygiene round): tracked files that match
# ignore rules. Zero matches is the invariant -- anything here is either a
# force-added leak or a rule that should not exist.
set -uo pipefail
LEAKS=$(git ls-files -i -c --exclude-standard)
if [ -n "$LEAKS" ]; then
    echo "GATE-LEAKS ✗ tracked files match ignore rules:"
    echo "$LEAKS"
    exit 1
fi
echo "GATE-LEAKS ✓ 0 tracked files match ignore rules"
