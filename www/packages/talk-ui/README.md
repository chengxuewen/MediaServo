# @mediaservo/talk-ui

Intercom React components (talk-app T0 scaffold). Renders the monitor-center
K1–K16 intercom design inside the existing admin (L1); reusable by other
React apps / Tauri / H5 (L3).

## Layering

```
@mediaservo/client  (logic: BoardStore, signaling-only multi-room)  — framework-agnostic
@mediaservo/ui      (tokens: --ms-* contract)                        — framework-agnostic
@mediaservo/react   (base primitives)                                — React
@mediaservo/talk-ui (intercom components)                            — React (this package)
apps/admin          (thin shell: nav + routes)                       — not reusable
```

## Consume (once real components land)

```tsx
import '@mediaservo/ui/tokens.css';
import '@mediaservo/react/react.css';
import { FloorBoard } from '@mediaservo/talk-ui';
```

## Status

v0.1.0 — scaffold only. Components arrive with talk-app batches T5/T6/T8/T9/T11.
See `docs/plans/talk-app/PLAN.md` (§4 batches, §5 acceptance incl. A12 latency,
O26 semantics, ui-harness mirror).
