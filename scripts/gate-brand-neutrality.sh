#!/usr/bin/env bash
# gate-brand-neutrality.sh — the MediaServo submodule is the brand-NEUTRAL upstream
# product: it must not carry the customer brand "msrtc" as a branding field.
# Branding is injected by the parent workspace (MEDIASERVO_BRAND env / build-args);
# the neutral product default is "mediaservo".
#
# Scope (user-adjudicated 2026-11-19, brand-neutrality ruling):
#   FORBIDDEN: msrtc anywhere in tracked product files, EXCEPT the allowlist below.
#   ALLOWLIST = functional brand-RECOGNITION (the product must still recognize
#   msrtc-branded deployments on a host — PIT-155/D252), brand regression TESTS
#   (msrtc is the test subject), parent-shell INTEGRATION scripts, historical
#   narrative (changelog/archive/research), agent assets, and the msrtc-authored
#   site DELIVERY manuals (34/35; the in-container OS user stays mediaservo).
#   ALSO allowlisted: the manual-render SOURCE-BRAND constant (msrtc) that lives
#   in mediaservo.sh + docker/server/Dockerfile — functional, because the delivery
#   manuals are msrtc-authored; the product brand DEFAULT there is mediaservo
#   (_ms_image_brand / ARG BRAND), verified separately. And the parent-shell
#   integration / process-family recognition scripts (all-gates/e2e-package/
#   e2e-trap-lib) which must name the real parent tool / recognize msrtc procs.
#
# This gate is submodule-only; the parent msrtc workspace legitimately uses msrtc.
# The gate excludes ITSELF (it must name the forbidden token to grep for it).
set -euo pipefail
cd "$(dirname "$0")/.."

# Allowlisted zones (functional / historical / delivery — see ruling).
hits=$(git grep -I -l 'msrtc' -- . \
    ':!docs/archive' \
    ':!docs/reference/research' \
    ':!docs/ui/prototypes' \
    ':!docs/modules/34-site-deployment.md' \
    ':!docs/modules/35-container-ops-manual.md' \
    ':!.agents/memorys' \
    ':!.agents/skills' \
    ':!CHANGELOG.md' \
    ':!docs/governance/language-convention.md' \
    ':!crates/mediaservo-host' \
    ':!crates/mediaservo-client' \
    ':!crates/mediaservo-common/src/lib.rs' \
    ':!crates/mediaservo-server/src/lifecycle/inspect.rs' \
    ':!crates/mediaservo-server/src/lifecycle/mod.rs' \
    ':!crates/mediaservo-server/src/lifecycle/startup.rs' \
    ':!crates/mediaservo-server/src/lifecycle/reap.rs' \
    ':!crates/mediaservo-server/src/lifecycle/templates.rs' \
    ':!crates/mediaservo-weaknet/tests' \
    ':!scripts/e2e-brand.sh' \
    ':!scripts/tests/e2e-brand-trap.bash' \
    ':!scripts/r18-load-harness' \
    ':!scripts/agents' \
    ':!scripts/all-gates.sh' \
    ':!scripts/e2e-package.sh' \
    ':!scripts/e2e-trap-lib.sh' \
    ':!mediaservo.sh' \
    ':!docker/server/Dockerfile' \
    ':!scripts/gate-brand-neutrality.sh' \
    ':!bindings' \
    ':!vendor' \
    ':!examples' \
    ':!www' \
    || true)

if [ -n "$hits" ]; then
    echo "GATE-BRAND-NEUTRALITY ✗ — customer brand 'msrtc' found in non-allowlisted product files:" >&2
    echo "$hits" | sed 's/^/  /' >&2
    echo "Neutralize to 'mediaservo' (product default) or inject via the parent workspace;" >&2
    echo "functional recognition / tests / history belong in the allowlist (see header)." >&2
    exit 1
fi
echo "GATE-BRAND-NEUTRALITY ✓"
