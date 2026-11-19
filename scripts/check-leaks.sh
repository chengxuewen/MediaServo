#!/usr/bin/env bash
# Leak detection (O-2, gitignore-hygiene round): tracked files that match
# ignore rules. Zero matches is the invariant -- anything here is either a
# force-added leak or a rule that should not exist.
# Single implementation: check_guards C10 delegates here (audit Full#2).
set -uo pipefail

if [ "${1:-}" = "--self-test" ]; then
    tmp=$(mktemp -d)
    ( cd "$tmp" && git init -q && printf '*.log\n' > .gitignore \
        && : > leak.log && git add -f leak.log \
        && git ls-files -i -c --exclude-standard | grep -q leak.log ) \
        && { echo "GATE-LEAKS ✓ self-test (injected leak detected)"; rm -rf "$tmp"; exit 0; }
    echo "GATE-LEAKS ✗ self-test broken -- the gate cannot detect an injected leak"
    rm -rf "$tmp"
    exit 1
fi

git rev-parse --git-dir >/dev/null 2>&1 || { echo "GATE-LEAKS ✗ not a git work tree"; exit 1; }
if ! LEAKS=$(git ls-files -i -c --exclude-standard); then
    echo "GATE-LEAKS ✗ git ls-files failed"
    exit 1
fi
if [ -n "$LEAKS" ]; then
    echo "GATE-LEAKS ✗ tracked files match ignore rules:"
    echo "$LEAKS"
    exit 1
fi
echo "GATE-LEAKS ✓ 0 tracked files match ignore rules"
