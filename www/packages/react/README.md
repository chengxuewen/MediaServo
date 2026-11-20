# @mediaservo/react

MediaServo base React components (talk-app W0 / ruling D3). Tokens-only, headless-friendly.

## Consume

```tsx
// entry — tokens first, then component styles
import '@mediaservo/ui/tokens.css';
import '@mediaservo/react/react.css';

import { Button, Confirm, Combobox, Modal, StatusGlyph, Table, THead, TBody, TR, TH, TD, Input, Form, Field, Guard } from '@mediaservo/react';
```

## Contract

- **Tokens-only**: components style exclusively via `--ms-*` (see `@mediaservo/ui/TOKENS.md`).
  Theme by overriding tokens on any container scope — no rebuild, no fork.
- **Design discipline baked in** (prototype rules): guards are `aria-disabled` + reason
  (never native `disabled` on actions); state glyphs are hue **and** shape; combobox instead
  of native `<select>`; confirm dialogs instead of `window.confirm`.
- **PIT-269 dialog three-piece**: Modal does initial focus, LIFO focus restore, focus trap,
  and Esc arbitration where **only the top dialog** reacts (module-level stack).
- **headless-friendly (D4)**: behavior is minimal and local; state-heavy logic belongs in
  framework-agnostic stores (e.g. `@mediaservo/client` BoardStore), not here.
- Raw values (hex px etc.) in component CSS are banned; `@mediaservo/ui`'s
  `check-tokens` guard verifies every `var(--ms-*)` reference resolves.

## Components

| Component | Notes |
|---|---|
| `Button` | variants primary/secondary/danger/ghost · sm/md · guarded disable with `disabledReason` |
| `Guard` | wrap any control: blocked activation + reason tooltip, stays focusable |
| `StatusGlyph` | tone(ok/warn/danger/off/unknown) × shape(dot/ring/seg/triangle), requires `label` |
| `Input` | label/hint/error wiring (aria-describedby/invalid), mono + sm |
| `Table/THead/TBody/TR/TH/TD` | styled primitives + `dense` |
| `Form/Field` | preventDefault form + label/hint/error layout |
| `Combobox` | custom listbox, filterable, full keyboard, `aria-activedescendant` |
| `Modal` | portal, scrim, focus trap/restore, top-only Esc |
| `Confirm` | confirm dialog on Modal; message should state the consequence |

## Status

v0.1.0 — foundation delivered with talk-app W0. Consumers next: `packages/shell` (B-shell),
`@mediaservo/talk-ui`, admin page refactors (W-series). Known deferred: `*-ink`/info tokens,
multi-select combobox, table sorting/selection (await real consumers).
