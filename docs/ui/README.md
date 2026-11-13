# docs/ui — UI design assets (structure + prototypes)

> Home of the web-portal-ui design exploration artifacts (parent plan `docs/plans/web-portal-ui/`,
> rulings P1/P2/P8). Design assets follow the product: they live in the submodule so an independent
> MediaServo clone is self-sufficient (P2). English-only per C49; UI copy is English (U2, zh deferred to i18n).

## Layout

| Path | Layer | What |
|---|---|---|
| `prototypes/*.html` | appearance | single-file offline HTML prototypes (P1 exploration carrier). No CDN / webfont / build; open directly in a browser. |
| `prototypes/archive/` | history | superseded versions, kept not deleted (workbudy version discipline). |
| `prototypes/CHANGELOG.md` | history | per-version change log. |
| `boards/*.md` | structure | character/SVG wireframes + region annotations (the "what is on screen, in what order" layer). |

## Rules (borrowed from workbudy via web-portal-ui P8 — "borrow patterns, change boundaries")

- One token set, three orthogonal axes (`data-theme` / `data-density` / `data-surface`); 3-layer tokens
  (primitive → semantic → component), component layer never references primitives.
- Dual theme: admin light + dark both first-class; video letterbox dark in both.
- State grammar: **no state by hue alone** — every state carries a glyph (solid dot / 3-seg bar + numbers /
  hollow-ring-slash / triangle / block).
- No infinite animation; transitions ≤150ms; emergency entry 3 pulses then static.
- Video tiles radius 0; weights 400/600 only; system font stack; tabular-nums for telemetry.
- Prototypes are design assets, **not app code**: never enter `www/**`, never imported by the app,
  retire to `archive/` when the board lands in implementation (P1 guard).

## Design review loop (web-portal-ui P9)

Design exploration runs as an **interactive per-board review loop** — one board at a time:

`spec / PLAN + UI skill → agent draws one H5 board → preview + human review/comments → revise → user approves → write back docs/ui + ledger → next board`

- **One board at a time.** No skipping, no bundling several boards into one file; do not advance until the user approves.
- **Every board carries**: YAML sidecar (`board/surface/theme/density/refs/e2e_hooks/regions[]/open_questions[]`),
  region annotations A1…An → spec clauses, three states (empty/loading/error; session states for safety-critical screens),
  and its own open questions. Sidecar lives in `boards/<board>.md`.
- **Approval ledger** (in the parent plan §3f): per-board status `draft / in-review / approved`; skips/deferrals recorded explicitly.
- Board inventory: W1 shell (in-review) · W2 live · W3 intercom · W4 tower · W5 settings · W6 talk-session · W7 playback (P4-gated).
  W2–W5 are split out of the current W1 file as each enters the loop; W1 then slims to the pure shell.

## Status

Design exploration only. Implementation (`tokens.css`, `?mock=1`, `packages/shell`) lands in B1;
the UI design skill lands in R2 (web-portal-ui P1/P8). Gate closed until user opens B1.
