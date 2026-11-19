---
name: design-library
description: Design intelligence library (ported from ui-ux-pro-max, MIT). Searchable styles/palettes/typography/guidelines/industry-rules knowledge base with a BM25 engine -- use for design exploration, style selection, palette candidates, and industry anti-pattern reference when starting UI work.
---

# design-library — the design intelligence library

Ported from nextlevelbuilder/ui-ux-pro-max-skill v2.15.0 (MIT; donor: Next
Level Builder; LICENSE ships alongside). This is the LIBRARY layer of the
four-question design architecture (plan design-multi-style): it answers
"what options and conventions exist" -- it never decides, never writes
design files, never gates acceptance.

## What it holds

- `data/styles.csv` (~90 styles with light/dark suitability columns)
- `data/colors.csv` (194 product-type palettes, 16 semantic roles,
  includes dark-background rows)
- `data/typography.csv` (74 pairings), `data/ux-guidelines.csv` (130),
  `data/ui-reasoning.csv` (195 industry rules incl. Anti_Patterns/Severity)
- `data/charts.csv`, `data/motion.csv`, `data/landing.csv`, and 22 stack
  references under `data/stacks/` (reference only -- implementation rules
  live in this repo's own rules, not here)
- `data/data-provenance.json` (upstream provenance)

## Engine (stdlib only; run under pixi)

    pixi run python .agents/skills/design-library/scripts/search.py "<query>" --json

Domains: styles / palettes / typography / guidelines / reasoning. Outputs
are CANDIDATES for the design-direction step (design-quality.md five lines)
-- never auto-applied.

## Boundaries

Negative (never does): decide -- the direction lines in design-quality.md
rule; write tokens or theme files -- design-system owns those; review --
ui-ux-review owns acceptance; act as authority -- outputs are candidates.

Positive handoffs: token/theme landing -> `design-system`; acceptance ->
`ui-ux-review`; quality floor -> `.agents/rules/web/design-quality.md`.

Write-disk ban: running the engine in-repo FORBIDS `--persist`/`--preview`
(they write MASTER.md / tokens.json / tokens.css / preview.html into the
cwd). Output goes to stdout / `--json`, or into a plan's `evidence/` draft
area only. NEVER point output at `www/packages/ui/` or `docs/`.

Evidence standard: every recommendation cites its CSV row or
data-provenance.json (bottom line #5).

Quality floor for candidates: palette candidates must pass the WCAG
contrast check. The check is EXECUTED by `design-system`'s landing flow
(it owns the gate); this skill only states the floor.

Provenance note: this is a 2B install path (verbatim data port from a
vetted MIT donor, ledger `docs/reference/design-library.md`), not a 2C
mechanism adoption -- mechanism-porting's "never port content" rule does
not apply here; the governance equivalent is the dedicated ledger + the
boundary contract above.
