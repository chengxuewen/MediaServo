#!/usr/bin/env bash
# report_2d.sh -- one-click mechanical evidence for the repo-audit 2D
# eight-asset inventory (B12). Runs the gate behind each asset's mechanical
# column and prints the per-asset table. The JUDGMENT column stays a template:
# audits find + judge (repo-audit), this script only collects mechanical proof.
set -u
cd "$(dirname "$0")/../.." || exit 2

declare -A GATE=(
  ["experts (roles)"]="python3 scripts/agents/agent_lint.py"
  ["expert teams + quotas"]="python3 scripts/agents/check_quotas.py"
  ["skills"]="python3 scripts/agents/skill_lint.py"
  ["rules"]="python3 scripts/agents/rules_lint.py"
  ["constraints (gate parity)"]="python3 scripts/agents/rules_lint.py"
  ["config (projection parity)"]="python3 scripts/agents/cli.py check"
  ["MCP pins/quota/same-name"]="python3 scripts/agents/gate_mcp.py"
)

printf '| %-28s | %-38s | %-6s |\n|------------------------------|----------------------------------------|--------|\n' \
  "Asset (2D)" "Mechanical evidence (gate)" "Result"
for asset in "experts (roles)" "expert teams + quotas" "skills" "rules" \
             "constraints (gate parity)" "config (projection parity)" \
             "MCP pins/quota/same-name"; do
  if bash -c "${GATE[$asset]}" >/dev/null 2>&1; then
    printf '| %-28s | %-38s | %-6s |\n' "$asset" "${GATE[$asset]}" "PASS"
  else
    printf '| %-28s | %-38s | %-6s |\n' "$asset" "${GATE[$asset]}" "FAIL"
  fi
done
echo
echo "Judgment column (template): for each PASS row state the per-asset conclusion"
echo "(duplicate? over-cap? bypassable?) -- findings feed the 3D increment table."
echo "Scripts-domain mechanical check (bash -n / ast.parse) runs in agent-tools."