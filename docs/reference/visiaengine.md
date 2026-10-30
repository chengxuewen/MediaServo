# VisiaEngine -- external reference profile (agent toolchain port)

> Donor: `.refinfo/VisiaEngine` @ `c662a0e` (MIT OR Apache-2.0).
> Ported 2026-10-30 as behavior-1:1 adaptations. Paths, pins, host envelopes and
> the two-root contract differ on purpose; the ledger below is the attribution
> surface required by the donor's MIT option.

| Donor | Destination | Mode | Batch |
|---|---|---|---|
| `scripts/agent_common.py` | `scripts/agents/common.py` | adapt | P1-2 |
| `scripts/agent-links.py` + `skills-links.py` + `memory-aggregate.py` + `constraints-index.py` | `scripts/agents/cli.py` | adapt (merged, two-root) | P1-2 |
| `scripts/gate-mcp.py` | `scripts/agents/gate_mcp.py` | adapt | P1-1a |
| `scripts/mcp-serve.mjs` | `scripts/agents/mcp-serve.mjs` | adapt (install root `scripts/agents`) | P1-1a |
| `scripts/mcp-launch.sh` | `scripts/agents/mcp-launch.sh` | adapt (root `../..`, node resolution) | P1-1a |
| `scripts/agent-targets.json` | `.agents/targets.json` | adapt (two-root schema) | P1-3 |
| `.githooks/pre-commit` | `.githooks/pre-commit` | adapt (pixi + generated_tracked staging) | P1-5 |

Not ported:

- `rules-links.py` / `skills-links.py` mdc+copy modes -- deferred (S2=B).
- `book-build.py` / `book-check.py` / `book-i18n.py` -- P5 (English-only book; H6).
