# Ported board — Operations console (workbudy W1)

> Surface: web · Theme: light+dark · Status: **ported, reference-complete** (not a per-board review item)
> Carrier: `../prototypes/port-web-shell.html` · Source: `.refinfo/workbudy/docs/ui/prototypes/w1-web-shell.html`

## Sidecar
```yaml
board: port-web-shell
origin: workbudy w1-web-shell.html (213 KB, single-file, offline)
surface: web
theme: [light, dark]
status: ported
pages: [overview, view, intercom, meetings, remote, devices, rooms, sessions, streams, config, audit, settings, signaling, keys]
e2e_hooks: [".nav-item", ".page-view", ".combo", ".tiles", ".ms-table"]
adaptations:
  - "Cockpit -> Tower (3 refs, web-portal-ui P5 naming ruling)"
  - "title -> 'Operations console (workbudy W1 port)' to avoid W1/W2 numbering collision"
```

## Why this port exists
workbudy's prototypes are **already MediaServo-branded and use the same `--ms-` token system** we adopted
in P8 — they are effectively the reference-complete design for this product. This port brings the **full
14-page operations console** into the project in one faithful, runnable artifact, instead of only the
piecemeal mechanism borrows used by the focused W1/W2 boards.

## Page inventory → our app mapping
| workbudy page | Our surface | Focused board |
|---|---|---|
| overview | admin / dashboard | — |
| view | view (Live wall) | W2 (`w2-live-wall.html`, adds column+pagination) |
| intercom | talk | W3 (pending) |
| meetings | meet | — |
| remote | desk | — |
| devices / rooms / sessions / streams | admin IA | W5+ |
| config / audit / keys / signaling / settings | admin | W5 (`settings`) + admin IA |

## Relationship to the focused boards
- `w1-portal-shell.html` / `w2-live-wall.html` = our **per-board review** artifacts (P9 loop), adapted to
  current rulings (tower naming, floor capability, column-driven + paginated wall).
- `port-web-shell.html` = the **reference-complete** workbudy console, ported faithfully; use it to review
  pages we have not yet built as focused boards (intercom / meetings / remote / admin IA).
- Verified: inline JS passes `node --check`; no external/CDN/font/image deps (fully offline); HTTP 200.
