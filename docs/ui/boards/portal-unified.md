# Unified baseline — MediaServo Portal (single page)

> Surface: web · Theme: light+dark · Status: **THE single iteration baseline (v1.0)**
> Carrier: `../prototypes/portal-unified.html` · Supersedes the fragmented boards for iteration.

## Why this exists (user directive 11-13)
The earlier prototypes were **fragmented and conflicting** — separate files each with their own nav,
style, page content and operations (`w1-portal-shell`, `w2-live-wall`, `port-web-shell`,
`port-admin-console`). This unified single page reconciles them into **one navigation taxonomy, one
design system, one set of page behaviors**, and is the **only artifact to iterate on going forward**.

## Sidecar
```yaml
board: portal-unified
surface: web
theme: [light, dark]
density: [comfortable, compact]
status: baseline
version: v1.0
pages: [overview, live, playback, intercom, meeting, remote, tower, devices, rooms, sessions, streams, config, keys, audit, settings, signaling]
e2e_hooks: [".nav-item.active", ".page.active", "#tiles", "#colsTrigger", "#pgPrev", "#pgNext", "#boardList", "#devBody", "#themeSeg"]
```

## Unified navigation taxonomy (reconciles new-design groups + workbudy features)
| Group | Items |
|---|---|
| Overview | Overview (fleet dashboard) |
| Surveillance | **Live** (column-driven + paginated wall), Playback (honest disabled until recording backend) |
| Communication | **Intercom** (floor board), Meeting |
| Control | Remote (lease-gated, view-only), **Tower** (multi-panel workbench) |
| Administration | Devices, Rooms, Sessions, Streams, Config push, Client keys, Audit log, Settings, Signaling |

Naming follows web-portal-ui W4/P5 (Live / Intercom / Meeting / Remote / **Tower**, not "Cockpit"/"View").
Workbudy's rich admin pages (sessions/streams/config/keys/audit/signaling) are folded into Administration.

## One design system
- P8 `--ms`-style tokens, 3-layer, dual theme (light + dark), density axis; control-room palette
  (electric-blue accent + acid-green live / amber degraded / red emergency / cyan info / violet meeting).
- State grammar: glyph + hue, never hue alone (dot / 3-seg bar / hollow-ring-slash / triangle).
- Custom combobox (no native `<select>`), styled scrollbars, toast feedback, WS-drop → floor UNKNOWN + stale banner.

## Working interactions (verified)
- Live wall: columns 1–4, pagination, Sync/Stats toggles, stream-rail show/hide, select-streams picker, zoom, remove.
- Intercom: grant / revoke / mute / prio with live floor state + timers; disconnect → UNKNOWN (not idle).
- Devices: search / filter (All/Pending) / sort / approve / revoke; Rooms/Sessions/Streams/Keys/Audit tables live.
- Settings: theme + density switches; Meeting create/join; Keys issue/revoke; Overview refresh.
- **Runtime logic smoke: 12/12** (pagination across cols, page clamp, counts, floor state machine, re-render triggers).
- Static: inline JS `node --check` OK; 84 ids / 57 refs zero missing; tags balanced; fully offline; HTTP 200.

## Iteration rule
All future UI changes land **here**. The older files remain as reference/history only:
`w1-portal-shell.html`, `w2-live-wall.html` (focused precursors) and `port-web-shell.html`,
`port-admin-console.html` (workbudy reference ports) — **do not iterate on those**.

## Monitor Center redesign — user adjudications (2026-11-16, walked one by one)

User manual review found the Live/Playback split uncoordinated and praised **Synology Surveillance
Station Monitor Center**. Research: Synology official help (Monitor Center five panels; timeline in
live view for instant playback; Recordings library) + community/blogger reviews (praised integrated
turnkey GUI; criticisms are licensing/hardware/AI, not the layout). Eight adjudications were walked
one at a time; naming settled as **Monitor** + **Recordings** (plural collection word, after the
expert-panel elegance re-ruling — see Round 3 below; code/kind stays `view`).

| # | Decision | Key note |
|---|---|---|
| 1 | Top-level model | Integrated monitor + separate recordings library, **two entries, groups unchanged**. Monitor = wall + timeline instant-playback + alerts; Recording = library. **Timeline lives only in Monitor.** |
| 2 | Nav groups / landing | `Surveillance = [Monitor, Recording]`; groups unchanged; **Overview stays default**. Operator activity set: Monitor·Intercom·Meeting·Remote·Tower. |
| 3 | Panel disclosure | **B progressive disclosure** — default wall + timeline + topbar; left device/layout tree and right alert panel collapsible. |
| 4 | Grid / layout | **B named layouts + templates** (incl. asymmetric 1-big+N-small); per-cell span data model; **elastic drag-resize deferred**. |
| 5 | Timeline | **A one global timeline + instant playback** (sync-seek default; drag=review, pull-right=back-to-LIVE); per-channel independent deferred. |
| 6 | Events / alerts | **A alert panel + click-to-jump + type filter + prev/next event**; event-triggered layout (alarm-to-wall) deferred; reserve alarm→intercom→wall hook. |
| 7 | Video selection | **B drag-to-wall + shuttle picker coexist** (drag = place/orchestrate; picker = batch/search). |
| 8 | Recording shape | **A library + click-clip→jump-to-Monitor** (search/bookmarks/export; no second timeline); honest-gated until backend. |

### Open / deferred items (explicit, not silently dropped)
- Elastic drag-resize grid (from #4) — future.
- Per-channel independent timeline (from #5) — future.
- Event-triggered layout / alarm-to-wall + alarm→intercom→wall linkage (from #6) — future.
- Recording in-page clip player (option B of #8) — optional enhancement.
- Whether to make Monitor the default landing (kept Overview in #2) — revisit.
- Pagination vs big-layout+carousel (raised in #4 impact) — not separately adjudicated.
- Carousel / patrol (Synology) — not adjudicated.
- Nav visual form — kept as-is (collapse + density), not contested.
- Recording smart-search / export — depends on recording backend (`RECORDING_READY=false`).

**Status: design adjudicated, NOT yet implemented** (explore mode). Implementation is a separate
change proposal; the current `portal-unified.html` (v1.16) remains the working baseline until then.

### Round 2 — pending items walked one by one (2026-11-16)
| # | Topic | Ruling |
|---|---|---|
| R1 | Many-camera navigation | **C coexist**: big layout primary + optional carousel (default off) + keep manual pager |
| R2 | Channel toolbar | **A lean core real** (zoom/stats/remove/fullscreen/audio-listen-mute); snapshot/manual-record/PTZ/stream-switch/dewarp = honest-disabled placeholders |
| R3 | Two "Sync" meanings | **B split**: live "drift-axis" toggle + playback "Sync-seek" fixed chip (default on) |
| R4 | Recording code/URL word | initial A `recording`; **re-ruled by expert panel (elegance): Recordings (plural) nested in view domain `/view/recordings`** (see Round 3) |
| R5 | Event model | **A unified EventType + event bus** consumed by alerts/timeline/cue/Intercom; reserve alarm→intercom→wall hook |
| R6 | Layout templates | **A lean set**: symmetric 1/4/9/16 + asymmetric 1+3/1+5/1+7; integer cell span (2×2/1×1) |
| R7 | Disclosure defaults | **A all collapsed by default + state memory + expand buttons + alert unread badge** |
| R8 | Drag vs picker | **A clear split**: drag = quick placement / picker = batch+search+sort; shared layout/selected model |

Round-2 delivery record + tasks: parent `docs/plans/monitor-center/PLAN.md` (§2b, §7, §8).
Still open after round 2: elastic transition tier (P3 prep), Monitor in-page title wording,
www/apps/view port timing (P4.3); plus the P3-deferred and backend-dependent items above.

### Round 3 — expert-panel review + conflict adjudication (2026-11-16, elegance-first)
A 4-role isolated panel (correctness / evidence / consistency / architecture-boundary) found the plan
conflicted with frozen upstream rulings it had not registered (U6 "monitor retired as menu word",
U8 "view kind + /monitor redirect", U2 table "/view/recordings", U3 view-internal structure).
Conflicts adjudicated one by one, **ignoring sunk cost, elegance-first**:
- **Naming = A**: menu **Monitor + Recordings** (plural collection word, matches Devices/Rooms/
  Sessions/Streams); page titles "Monitor Center" / "Recordings library"; **code/kind = `view`**;
  routes **`/view/monitor + /view/recordings`** (one domain, two surfaces). Monitor semantically covers
  live+review (Live cannot); `view` is source-agnostic (RM7); two-layer vocabulary is U2's principle.
- This **supersedes U6's menu-word clause** (Live→Monitor) and **refines R4** (Recording singular →
  Recordings plural, nested in view domain). U8 view kind + redirect escape hatch kept.
- Upstream forward-supersession note added to `web-portal-ui/PLAN.md` (U2 table); internal plan drifts
  (template set, carousel scope, mk('playback') split, route=kind, testable density criterion) fixed.
- Panel findings: 8 (CRITICAL 1 / HIGH 2 / MEDIUM 3 / LOW 2); both conflicts resolved by the naming
  ruling; MEDIUM/LOW fixed in PLAN.md. Still open: elastic transition tier, www/apps/view port timing.

### Implementation status — Phase 1 delivered (2026-11-16, v1.17–v1.18)
Parent `docs/plans/monitor-center/PLAN.md` P1.1–P1.6 implemented in `portal-unified.html`:
nav `Surveillance=[Monitor, Recordings]` (code/kind view, old #live/#playback redirect); Monitor =
wall + global timeline on one page (instant-playback semantics, event nav, honest-gated); right
alerts panel (3-column grid, collapsible, R7 default-collapsed + memory, click-focus camera, type
filter, unread badge); Recordings = library (no second timeline); two coherent guard sets; Intercom/
Tower/admin regression green.

### Implementation status — Phase 2 delivered (2026-11-16, v1.19–v1.20)
P2.1–P2.5 implemented: wall layout templates (symmetric 1/4/9/16 + asymmetric featured 1+3/1+5/1+7,
named grid-areas, integer spans, empty-slot drop placeholders, localStorage memory); drag-to-
orchestrate (rail/tile drag → slot reorder, footer-drop remove, Auto-fit layout; picker stays for
batch add per R8); Recordings refactored into a searchable/filterable/bookmarkable clip library
(event-based design samples, gated playback/export, click-clip→Monitor focus); optional sequential
carousel (default off, 3/5/10/30s) coexisting with manual pager (R1=C). Intercom/Tower/admin regression
green.

### Implementation status — Phase 3 largely delivered (2026-11-16, v1.21–v1.22)
P3.1 elastic custom grid (dense-flow, per-tile 1×1↔2×2, persisted); P3.3 unified event bus (R5) +
alarm-to-wall (Auto-wall toggle, default off) + alarm→intercom linkage hook + devtools simulate-event
source; P3.5 Recordings in-page clip player (honest-gated, Open-in-Monitor handoff); P3 elastic heavy
tier (v1.23): draggable column separators rebalance track weights with live reflow + persistence;
P3.2 (v1.24): per-channel independent timeline — Solo dual-mode, each zoomed channel keeps its own
playhead (saved/restored), global sync-seek default. All P3 features delivered; only the default-landing
revisit (decision, adjudication 2 kept Overview) remains open.
