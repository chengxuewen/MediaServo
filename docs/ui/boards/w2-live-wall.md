# Board W2 — Live wall (structure layer)

> Surface: web · Theme: light+dark · Density: comfortable · Refs: web-portal-ui U3/W3/P8/P9; workbudy borrow
> Appearance carrier: `../prototypes/w2-live-wall.html` (v0.1). This file = structure + borrow provenance.

## Sidecar (P9)
```yaml
board: w2-live-wall
surface: web
theme: [light, dark]
density: comfortable
canvas: 1440x900
refs: [web-portal-ui U3, W3, P8, P9]
e2e_hooks: ["#tiles", "#colsTrigger", "#pgPrev", "#pgNext", "#openPicker", "#statsToggle", "#syncToggle", ".stream-item", ".tile"]
status: in-review
version: v0.1
regions:
  - {id: A1, name: topbar,      spec: "brand + conn + theme + mute + user"}
  - {id: A2, name: ws-banner,   spec: "conditional stale slot (WS>5s), cached data shown"}
  - {id: A3, name: stream-rail, spec: "selected-streams list + Select streams (device->stream)"}
  - {id: A4, name: stage-bar,   spec: "rail toggle + counts + Sync + Stats + columns combobox + pager"}
  - {id: A5, name: tiles,       spec: "column-driven grid, paginated; tile stats/drift/state glyphs"}
  - {id: A6, name: canvas-foot, spec: "state legend + showing a-b of N / cols / page"}
open_questions:
  - "rows-per-page fixed at 2 (page capacity = cols x 2); expose as control or keep implicit?"
  - "Sync drift values are seeded; real drift source = playback backend (P4 recon)"
  - "audio-only stream (ENV-04) tile rendering: waveform placeholder vs audio glyph"
```

## Layout (column-driven + paginated — the W2 hard requirement)
```
┌────────────────────────────────────────────────────────────────────────────────────┐
│ ◆ MediaServo  Surveillance / Live view        ● Connected  ◐ 🔔 [DX] dispatch.x   │
├────────────────────────────────────────────────────────────────────────────────────┤
│ (ws-banner, only when WS dropped) Live updates paused — reconnecting… cached·stale │
├───────────────┬────────────────────────────────────────────────────────────────────┤
│ SELECTED      │ ⟨  8 streams·6 live·1 degraded·1 offline   Sync[on] Stats[off]     │
│ STREAMS       │                                     [3 per row ▾]  [‹ 1/2 ›]       │
│ ＋Select      │ ┌────────────┐ ┌────────────┐ ┌────────────┐                       │
│ streams       │ │VH-07·front │ │VH-07·rear  │ │VH-07·cabin │   <- page 1, 3 cols   │
│ 1 ▣ VH-07·fr  │ │ ■LIVE      │ │ ■LIVE      │ │ ■LIVE      │                       │
│ 2 ▣ VH-07·re  │ │ +120ms ●   │ │ -80ms ●    │ │ +40ms ●    │                       │
│ 3 ▣ VH-07·ca  │ └──────────── └────────────┘ ────────────┘                       │
│ 4 ▣ VH-11·fr  │ ┌──────────── ┌────────────┐ ────────────┐                       │
│ 5 ▣ VH-11·re  │ │VH-11·front │ │VH-11·rear  │ │VH-02·front │                       │
│ 6 ▣ VH-02·fr  │ │ ■LIVE      │ │ ◌degraded  │ │ □OFFLINE   │                       │
│ 7 ▣ CAM-01·m  │ └────────────┘ └──────────── └────────────┘                       │
│ 8 ▣ CAM-01·w  │ ┌──────────── ┌────────────┐                                       │
│               │ │CAM-01·main │ │CAM-01·wide │   (page 1 fills 6 = 3col x 2row)      │
│ 8 selected    │ └────────────┘ └────────────┘                                       │
│               │ ● live  ◌ degraded  ○ offline     showing 1–6 of 8 · 3col · pg 1/2 │
└───────────────┴────────────────────────────────────────────────────────────────────┘
```
- **Columns are the layout control** (`1/2/3/4 per row` combobox), NOT a fixed rows×cols matrix.
- **Pagination handles overflow**: page capacity = cols × 2 rows; `‹ 1/2 ›` pager; adding streams grows pages, never the grid. Stream count is uncapped.
- Changing columns re-flows + re-paginates; removing/adding clamps the page.

## Borrowed from workbudy (provenance)
| Mechanism | workbudy anchor | Adaptation here |
|---|---|---|
| "N per row" columns combobox (custom listbox, not native select) | `w1-web-shell.html:1119-1129`, SKILL combobox rule | kept; drives `--cols` |
| column-driven tile grid `data-cols` | `w1-web-shell.html:251-254,1139` | kept; **+ pagination** (user requirement, workbudy has no tile pager) |
| selected-streams rail (idx+thumb+name+sub+glyph+remove) | `w1-web-shell.html:1099-1108` | kept |
| Select streams (device→stream) + empty state | `w1-web-shell.html:1100,1150-1155` | modal shuttle, device-grouped, search |
| stage-bar (rail toggle + counts + stats toggle) | `w1-web-shell.html:1113-1130` | kept; + Sync + pager |
| tile stats overlay (FPS/RES/BIT/JIT/RTT/LOSS/…) | `w1-web-shell.html:1140` tile-stats | kept; + CODEC/DRIFT |
| tile drift (+120ms) for sync | `w1-web-shell.html:1140` tile-drift | kept; gated by Sync |
| state grammar: dot / 3-seg bar+number / hollow-ring | `01-design-tokens.md:190-203` | kept (hue never alone) |
| stale banner (WS>5s, cached shown) + Retry | `w1-web-shell.html:1057-1062,1132-1137`, `02-web-ia.md:211-215` | kept |
| tile zoom / remove actions | `w1-web-shell.html:1140` tile-actions | kept; zoom = full stats card |

## Three states
- **Data**: seeded 8 streams (6 live / 1 degraded / 1 offline), 2 pages at 3 cols.
- **Empty**: "No streams selected" + Select streams (remove all to see).
- **Stale/Error**: WS-drop banner, cached tiles kept, marked stale.
