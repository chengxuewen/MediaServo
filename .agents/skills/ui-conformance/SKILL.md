---
name: ui-conformance
description: Fourth layer of the UI workflow -- pixel-diff the implemented surface against the prototype (screenshot evidence, C1/C2/C3 checks) before handoff. Use after implementing a UI change that has a prototype or design reference.
---

# ui-conformance — design-conformance visual closed loop (the 4th verification layer)

> Structure/computed-style parity and e2e-green are layers 1–3. They do NOT prove the
> UI "looks like the design". This skill adds layer 4: pixel-level conformance against
> the DESIGN AUTHORITY (prototype), machine-measured, with a gap ledger. Born from the
> 2026-11-24/25 lesson: "aligned/done" was claimed on self-defined proxies while the real
> page differed (Goodhart / specification gaming).

## When to use
- Any UI slice that claims alignment to a prototype/design, before saying "done/aligned".
- Every checkpoint: run the loop and attach the gap ledger + dual screenshots.

## The loop (Anthropic vibe-coding / pixel-diff skill pattern)
1. Implement a slice.
2. Screenshot the IMPLEMENTATION and the PROTOTYPE at the SAME viewport/state.
3. Pixel-diff → gap ledger (layout / spacing / component-shape graded).
4. List diffs, fix CRITICAL (layout-level) ones, re-screenshot, re-compare.
5. Only when the ledger is reviewed and CRITICAL diffs closed may you say "conformant".

## Hard constraints (machine-enforced, not self-discipline)
- **C1 dual evidence**: any UI "done/aligned" claim MUST attach prototype screenshot +
  implementation screenshot + diff ledger. No images = NOT done.
- **C2 conformance script**: `node www/scripts/ui-conformance.mjs --page <p> --mode demo|real`
  (pngjs+pixelmatch). Baseline is the PROTOTYPE, never the app-vs-itself. Run per slice.
- **C3 gate**: CRITICAL layout diffs must be fixed; pixel-level residue is recorded as a
  ledger (debt), not silently accepted. The checkpoint report must include mismatchPct.
- **MCP in-loop**: use local-playwright to navigate/operate/screenshot the REAL deployed
  page each UI slice (not curl-only). Install browser if missing (`npx playwright install`).
- **Demo excluded from "aligned"**: demo/simulated mode is a labeled preview; real-mode
  ledger is the honest baseline. Never cite demo richness as real alignment.

## Honest status vocabulary
implemented / reachable / deployed / visually-reviewed / user-signed-off. Only the last
is "done/aligned". A high mismatchPct is a truthful baseline, not a failure to hide.

## First baseline (2026-11-25, monitor @1600x900)
demo 47.5% / real 36.2% pixel mismatch vs prototype — the real gap ledger to burn down.
Prioritize global surfaces (sidebar/topbar/header/table) then per-page chrome.