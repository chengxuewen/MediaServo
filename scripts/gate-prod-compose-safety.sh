#!/usr/bin/env bash
# gate-prod-compose-safety.sh — production compose safety invariants
# (dev-credentials-hardening D3; user-adjudicated A).
#
# This gate is SECONDARY defense-in-depth. The PRIMARY net is the startup guard
# (accounts.rs::guard_dev_credentials) which panics on dev placeholder accounts
# unless explicitly allowed; the SECOND layer is the image build stripping dev
# accounts (container-build-rework). This gate protects the DELIVERY artifact:
# the production docker-compose.yml must never SET a dev toggle nor mount dev
# credential files — the exact shape of the historical admin123-in-prod incident.
#
# Judgement granularity (D3=A): only CODE lines are checked; comment mentions
# (e.g. a warning comment "never set X here") are allowed. Inline comments are
# stripped before matching.
set -euo pipefail
cd "$(dirname "$0")/.."

compose="docker-compose.yml"
[ -f "$compose" ] || { echo "GATE-PROD-COMPOSE-SAFETY ✗ — $compose missing" >&2; exit 1; }

# Strip full-line and inline comments; keep non-empty code lines.
code_lines=$(sed 's/#.*//' "$compose" | grep -v '^[[:space:]]*$' || true)

fail=0
if echo "$code_lines" | grep -qi 'ALLOW_DEV'; then
    echo "GATE-PROD-COMPOSE-SAFETY ✗ — production compose SETS a dev toggle (ALLOW_DEV*):" >&2
    echo "$code_lines" | grep -in 'ALLOW_DEV' | sed 's/^/  /' >&2
    fail=1
fi
if echo "$code_lines" | grep -qE 'accounts\.yaml|devices\.yaml'; then
    echo "GATE-PROD-COMPOSE-SAFETY ✗ — production compose references dev credential files:" >&2
    echo "$code_lines" | grep -nE 'accounts\.yaml|devices\.yaml' | sed 's/^/  /' >&2
    fail=1
fi

if [ "$fail" = 1 ]; then
    echo "Dev defaults must never ship to production: drop the offending lines from" >&2
    echo "$compose" >&2
    exit 1
fi
echo "GATE-PROD-COMPOSE-SAFETY ✓"