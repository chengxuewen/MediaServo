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

## Status

Design exploration only. Implementation (`tokens.css`, `?mock=1`, `packages/shell`) lands in B1;
the UI design skill lands in R2 (web-portal-ui P1/P8). Gate closed until user opens B1.
