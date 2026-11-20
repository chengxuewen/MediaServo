# Unified baseline — MediaServo Portal (single page)

> Surface: web · Theme: light+dark · Status: **THE single iteration baseline (v1.0 创建标签；现迭代 v1.97 见 sidecar)**
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
version: v1.97
pages: [admin, accounts, overview, monitor, recordings, intercom, meeting, remote, tower, devices, rooms, sessions, streams, config, keys, audit, settings, signaling]
e2e_hooks: [".nav-item", ".active", ".page", ".active", "#tiles", "#colsTrigger", "#pgPrev", "#pgNext", "#boardList", "#devBody", "#themeSeg", "#meetGrid", "#meetScrim", "#prejoinScrim", "#meetCanvas", "#meetTiles", "#meetRail", "#meetPip", "#muteChip", "#meetBar", "#mkLeave", "#holdRing", "#holdBar", "#meetExitHint", "#meetPanel", "#mpBody", "#mpTabs", "#mpBadge", "#mpChatBadge", "#meetInfo", "#meetTransScrim", "#remGrid", "#remSession", "#remTile", "#remBar", "#remTake", "#remRelease", "#remCtlState", "#remGraceMirror", "#remLive", "#remHealth", "#remEstop", "#remEstopRing", "#remReqChip", "#remHolderChip", "#remForce", "#remResumeScrim", "#remResumeName", "#remStopped", "#remPerimeter", "#remMapPanel", "#remTouch", "#remHud", "#remStateChip", "#remEscHint", "#remTop", "#remZoom", "#remClock", "#remOsdBtn", "#remKeysBtn", "#remInfoBtn", "#remMoreBtn", "#remFs", "#remKeysPop", "#remInfoPop", "#remMorePop", "#remOsd", "#remStage", "#cfgIndex", "#ktVal", "#ktSave", "#ktClear", "#liUser", "#liPass", "#liRole", "#liBtn", "#liOut", "#liErr", "#aboutVer", "#sigResult", "#remLauncher", "#remLauncherEmpty", "#remLauncherTitle", "#lmCode", "#lmCopy", "#lmInfo", "#remDevPop", "#remTiles", ".rem-tile", ".rem-card", "#remViewHint", "#remViewBack"]
regions: [remote-lobby-cards, remote-session-topbar, view-group, semantic-line, gamepad-map-row, feed-tile-letterbox, osd-overlay, bottom-bar-state-grace-take-release, estop-never-hide, health-banner, stopped-banner-resume, req-chip, perimeter-latched, info-popover, settings-login-card, settings-config-index, keys-admin-token, remote-launcher-panel, capability-tiles, power-row-gated, pure-view-safebar-variant]
open_questions: [quality-codec-fps-adjust-backend, B-lease-server-batch, real-pointer-lock-fullscreen-UA-Esc-device, real-gamepad-device, special-keys-real-channel, R6-five-backendization, view-wall-fullscreen-cross-plan, PROGRAM-memorys-parallel-owned, devtools-hidden-in-session, ball-dock-dwell-micro, C-switch-carrier-phase2, launcher-breakpoint, launcher-tile-grid, launcher-code-copy-format, launcher-info-fields]
```

## Unified navigation taxonomy (reconciles new-design groups + workbudy features)
| Group | Items |
|---|---|
| Overview | Overview (fleet dashboard) |
| Surveillance | **Monitor** (integrated wall+timeline), **Recording** (library, honest-gated) — supersedes Live/Playback (see redesign section) |
| Communication | **Intercom** (floor board), Meeting |
| Control | Remote (lease-gated, view-only), **Tower** (multi-panel workbench) |
| Administration | ~~Devices, Rooms, Sessions, Streams, Config push, Client keys, Audit log, Settings, Signaling~~ **v1.64 取代：单 Admin 入口 + hub 四域(Assets/Observability/Security/System)；命名 Config delivery/Keys/Audit** |

Naming follows web-portal-ui W4/P5 (Intercom / Meeting / Remote / **Tower**, not "Cockpit"/"View"); **Live/Playback superseded by Monitor/Recording** (redesign section below).
Workbudy's rich admin pages (sessions/streams/config/keys/audit/signaling) are folded into Administration.

## One design system
- P8 `--ms`-style tokens, 3-layer, dual theme (light + dark), density axis; control-room palette
  (electric-blue accent + acid-green live / amber degraded / red emergency / cyan info / violet meeting).
- State grammar: glyph + hue, never hue alone (dot / 3-seg bar / hollow-ring-slash / triangle).
- Custom combobox (no native `<select>`), styled scrollbars, toast feedback, WS-drop → floor UNKNOWN + stale banner.

## Working interactions (verified) (v1.0 historical snapshot; superseded by v1.17+ redesign)
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
expert-panel elegance re-ruling — see Round 3 below; code/kind stays `view`; **v1.46 superseded: menu/title singular Recording, URL unchanged**).

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
- Whether to make Monitor the default landing (kept Overview in #2) — **closed by K13=A (2026-11-17): keep Overview**.
- Pagination vs big-layout+carousel (raised in #4 impact) — **closed: pagination kept + carousel R1=C (v1.20)**.
- Carousel / patrol (Synology) — **closed R1=C (optional, default off, v1.20)**.
- (Historical open list: elastic grid / independent timeline / alarm-to-wall / in-page player closed in P3 v1.21–24; carousel closed R1=C; default-landing **closed by K13=A**.)
- Nav visual form — kept as-is (collapse + density), not contested.
- Recording smart-search / export — depends on recording backend (`RECORDING_READY=false`).

**Status (historical, Round-1 era): design adjudicated, not yet implemented then.** Superseded —
P1–P5 delivered v1.17–v1.27; see implementation-status sections below.

### Round 2 — pending items walked one by one (2026-11-16)
| # | Topic | Ruling |
|---|---|---|
| R1 | Many-camera navigation | **C coexist**: big layout primary + optional carousel (default off) + keep manual pager |
| R2 | Channel toolbar | **A lean core real** (zoom/stats/remove/fullscreen/audio-listen-mute); snapshot/manual-record/PTZ/stream-switch/dewarp = honest-disabled placeholders |
| R3 | Two "Sync" meanings | **B split**: live "drift-axis" toggle + playback "Sync-seek" fixed chip (default on) |
| R4 | Recording code/URL word | initial A `recording`; **re-ruled by expert panel (elegance): Recordings (plural) nested in view domain `/view/recordings`** (see Round 3; **v1.46 re-revised to singular Recording**) |
| R5 | Event model | **A unified EventType + event bus** consumed by alerts/timeline/cue/Intercom; reserve alarm→intercom→wall hook |
| R6 | Layout templates | **A lean set**: symmetric 1/4/9/16 + asymmetric 1+3/1+5/1+7; integer cell span (2×2/1×1) |
| R7 | Disclosure defaults | **A all collapsed by default + state memory + expand buttons + alert unread badge** (**Revised by Round4 #3 / Round5 K2**: desktop left-open/right-closed, narrow all-collapsed, override memory) |
| R8 | Drag vs picker | **A clear split**: drag = quick placement / picker = batch+search+sort; shared layout/selected model |

Round-2 delivery record + tasks: parent `docs/plans/monitor-center/PLAN.md` (§2b, §7, §8).
Still open after round 2: elastic transition tier (**closed v1.21/23**) (P3 prep), Monitor in-page title wording (**closed v1.46**),
www/apps/view port timing (P4.3); plus the P3-deferred and backend-dependent items above.

### Round 3 — expert-panel review + conflict adjudication (2026-11-16, elegance-first)
A 4-role isolated panel (correctness / evidence / consistency / architecture-boundary) found the plan
conflicted with frozen upstream rulings it had not registered (U6 "monitor retired as menu word",
U8 "view kind + /monitor redirect", U2 table "/view/recordings", U3 view-internal structure).
Conflicts adjudicated one by one, **ignoring sunk cost, elegance-first**:
- **Naming = A**: menu **Monitor + Recording** (v1.46 singular; original plural ruling superseded); page titles "Monitor Center" / "Recording library" (v1.46); **code/kind = `view`**;
  routes **`/view/monitor + /view/recordings`** (one domain, two surfaces). Monitor semantically covers
  live+review (Live cannot); `view` is source-agnostic (RM7); two-layer vocabulary is U2's principle.
- This **supersedes U6's menu-word clause** (Live→Monitor) and **refines R4** (Recording singular →
  Recordings plural, nested in view domain). U8 view kind + redirect escape hatch kept.
- Upstream forward-supersession note added to `web-portal-ui/PLAN.md` (U2 table); internal plan drifts
  (template set, carousel scope, mk('playback') split, route=kind, testable density criterion) fixed.
- Panel findings: 8 (CRITICAL 1 / HIGH 2 / MEDIUM 3 / LOW 2); both conflicts resolved by the naming
  ruling; MEDIUM/LOW fixed in PLAN.md. Still open: elastic transition tier (**closed v1.21/23**), www/apps/view port timing.

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

### Implementation status — Phase 3 delivered (2026-11-16, v1.21–v1.24)
P3.1 elastic custom grid (dense-flow, per-tile 1×1↔2×2, persisted); P3.3 unified event bus (R5) +
alarm-to-wall (Auto-wall toggle, default off) + alarm→intercom linkage hook + devtools simulate-event
source; P3.5 Recordings in-page clip player (honest-gated, Open-in-Monitor handoff); P3 elastic heavy
tier (v1.23): draggable column separators rebalance track weights with live reflow + persistence;
P3.2 (v1.24): per-channel independent timeline — Solo dual-mode, each zoomed channel keeps its own
playhead (saved/restored), global sync-seek default. All P3 features delivered; only the default-landing
revisit (decision, adjudication 2 kept Overview) — **closed by K13=A (2026-11-17)**.

### Responsive / size-adaptation audit (2026-11-16, v1.25) — marked list
User report: both Monitor side panels open squeezed the center wall. Multi-viewport headless audit
(1440/1150/950 + Recordings/Intercom/Tower/Devices/picker) enumerated:
| # | Surface | Defect | Sev | Status |
|---|---|---|---|---|
| M1 | Monitor #live-layout | fixed 248+300 columns squeeze center at 900–1500 | critical | FIXED ≤1500 overlay drawers, wall full-width |
| M2 | Monitor stage/ctl-bar | flex-wrap multi-row stacking when narrow | med | FIXED container-query condense |
| M3 | Monitor timeline scale | labels collide when narrow | med | FIXED container-query thinning |
| M4 | Monitor tile text | name/stats overlap when narrow | med | FIXED hide tile-idle/drift when narrow |
| M5 | Monitor double drawer | both open cover wall at ≤1100 | med | FIXED narrow mutual exclusion |
| R1 | Recordings rec-layout | list narrow at 900–1100 | low | FIXED stack breakpoint →1100 |
| C1 | admin tables | horizontal overflow | low | OK .tbl-wrap overflow:auto |
| C2 | Tower/Intercom/picker | narrow adaptation | low | OK existing breakpoints |

Follow-up (v1.26, user: "current worse than original"): restored 16:9 video tiles + symmetric
cols×2-row pagination with adaptive columns; integrated 3-column kept ≥1200px (mid widths slim panels),
drawers+mutual-exclusion only <1200px. Re-render on resize/panel-toggle.

### Round 4 — responsive/layout re-adjudication (2026-11-16, walked one by one; design; **P5 implemented v1.27**)
User rejected v1.25/v1.26 adaptivity (full-bleed stretch; panels not resident / controls broken). Rulings:
| # | Topic | Ruling |
|---|---|---|
| 1 | Wall model | Synology 3-pane all-adaptive: left\|center(1fr)\|right; center=total−visible panels; grid cols adapt to center width, tiles stay 16:9 (no stretch); panel toggle reflows center |
| 2 | Grid-count rule | "Fixed/Auto-fit" toggle, DEFAULT FIXED; auto=min(chosen,fit) stateless (**Superseded by v1.35 E**: single layout dropdown + built-in minmax, no fit toggle) |
| 3 | Panel default | LEFT resident by default, RIGHT hidden; centralized cluster + in-header close handles + memory (edge strips removed v1.29); desktop ≥1200 docked. **Revises R7** (was all-collapsed) |
| 4 | Narrow <1200 | overlay drawers + expand entry (edge strips removed v1.29) + pin-to-dock + mutual-exclusion (drawer mode only); desktop never drawers |
| 5 | Bottom timeline + unified collapse | left/right/bottom as three isomorphic collapsible regions: centralized top toolbar toggle cluster + in-header close handles (edge strips removed v1.29) + per-region memory; bottom hideable = live-only wall + bottom expand entry (hidden = live mode, stage-bar keeps LIVE/clock); mirrors Synology upper-right Show/hide cluster |
Default disclosure = left open / right collapsed / bottom open. Supersedes v1.25/v1.26 implementation
approach. **P5 implemented v1.27.**

### Monitor control-level UI audit (2026-11-16, render + markup evidence; folded into PLAN §6c / task 5.6)
- Icons: transport ⏪⏸⏩ are color emoji (ignore currentColor) and ⏪ doubles as Recordings handoff →
  replace with SVG sprite (i-play/pause/rew/fwd); text glyphs (⟨⟩ ⓘ ‹› −+ ▾ ■□) mixed with SVG → unify.
- Controls: stage-bar overflows at 1440 leaving "Alerts" orphan-wrapped → fold into the unified panel
  toggle cluster + container condense; control heights inconsistent (34/30/22/28) → one height token,
  touch ≥44, tile-btn ≥28 always visible on touch/focus; Sync-seek static chip looks like a button;
  Carousel interval spin visible while off.
- Layout: dead tile `.tile-idle` (z1) shows through semi-transparent `.deadmsg` (z3) → overlapping text;
  hide idle when dead; idle center text contrast too low (rgba .26).
- A11y: pgPrev/pgNext and icon-only buttons lack aria-label; add focus-visible.

### Round 5 — expert-panel re-review of Phase 5 + conflict rulings (2026-11-16, K1/K2 user-adjudicated)
4-role sequential panel on §2d/6b/6c/Phase5: 8 findings (MEDIUM 5/LOW 3, no CRITICAL/HIGH) + 2 conflicts:
- **K1 fixed/auto default = B**: width-tiered + user-override wins (**Superseded by v1.35 E**: single dropdown + built-in minmax).
- **K2 narrow panel default = B**: <1200 both collapsed + expand entries (edge strips removed v1.29; wall clean on load);
  desktop left-open/right-closed/bottom-open; override memory wins.
- MEDIUM/LOW fixed in PLAN §6b/6c/7: fixed min-tile 230 + horizontal scroll, breakpoint open-state
  carry, bottom-hide = pause + switch live, R7/§2-row3 revision annotated, --ctl-h:30px (coarse 44),
  tile-idle contrast ≥.45, handoff icon = library icon (not i-rew), phase-numbering note.

### Implementation status — Phase 5 delivered (2026-11-16, v1.27)
PLAN Phase 5.1–5.7 implemented: unified left/right/bottom region collapse (edge strips + centralized
[left][right][timeline] cluster + Fit toggle), tiered defaults + override memory + breakpoint carry;
Fixed/Auto fit (min-tile 230 + h-scroll / effCols no-write-back); 3-col adaptive 16:9 grid; narrow
<1200 drawers + pin + mutual-exclusion; bottom-hide = pause+live pure wall; UI polish (emoji→SVG,
--ctl-h, dead-idle hidden, contrast .45, Alerts in cluster, aria/focus). Verified multi-viewport +
click-through + regression green. Supersedes v1.25/v1.26.
- v1.28 fix: left rail head gained `railCollapse` (i-close) isomorphic to alerts `alCollapse` so the
  narrow left drawer can be collapsed from within (previously only the covered toolbar toggle existed).
- v1.29 (adjudicated B, research: Synology/VSCode button-toggle school; IDE edge-tabs are aligned labeled
  tabs, not our cross-row bars): removed standalone edge strips; collapse/expand solely via centralized
  cluster + in-header close + pin. Wall edges clean, footer uncovered.
- v1.30: unified the three region collapse buttons to `region-collapse` (26px ghost) with directional
  chevrons (left ‹ / right › / bottom ⌄), replacing the ×/↓ + size mismatch; pin aligned to 26px.
- v1.31: removed the two bottom-bar Event buttons (evPrev/evNext + jumpEvent + guards); event navigation
  stays in the right alerts panel (click-to-jump); transport bar de-duplicated to a clean single row.
- v1.32: Synology-style toolbar rework — text buttons iconified (Fit/Sync/Carousel/Solo/Sync-seek/
  Recordings + pager/spin SVG), clustered via .tb-group/.tb-sep, stream-count meta moved to footer so
  the stage bar is a single row, and EVERY icon button carries title+aria-label hover tooltips.
- v1.33: real styled hover/focus tooltips (data-tip + #guardTip neutral/warn, sweepTips removes native
  title) + single --ctl-h:30px control-size token across ALL controls (coarse ≥44). Governance: child
  convention C54 (prototype control discipline) added because design-system skill only governs www
  tokens, not this prototype; size/tooltip assertions now part of UI-change verification.
- v1.34 (adjudicated C+D): removed Monitor's persistent bottom message bar; legend→LIVE-badge tooltip,
  showing/page→pager tooltip; health summary became a conditional danger chip (#healthChip) shown only
  when degraded/offline>0, click focuses first problem stream. canvas-foot/.legend CSS kept (Intercom).
- v1.35 (adjudicated E, supersedes K1): single layout dropdown with "Auto · by count" template option
  (live by stream count); removed center Fit:Fixed/Auto toggle + rail Auto-fit button; width response
  built-in via minmax(230px,…)+horizontal scroll (no fit mode). Left rail keeps list/select/drag.
- v1.36: top status cluster (warn-chip / LIVE badge / clock) aligned to --ctl-h 30px + unified type
  scale (badge fs11 compact, clock fs12), fixing the mismatched LIVE sizing (C54).
- v1.37 (adjudicated A): rail list text-first — removed fake black-box thumbnail + index; two lines
  (device·label full + res·state), status dot kept, remove on hover/focus; names no longer truncated
  at 1440/1250. Aligns with C62 (no fake preview) and Synology text+icon tree convention.
- v1.38: unified card-row list language across rail / picker both columns / alerts / recordings /
  intercom (bordered r6 sp8 + hover + selected; resting container signal); picker device headers become
  distinct bands; child convention C55 added. Fixes "white paper" rail + picker left/right mismatch.
- v1.39: layout dropdown popup polish — left-anchored, min-width 220, strong drop shadow, r8, 34px
  inset rows, group separators (Auto | templates | Custom), max-height 60vh scroll-safe (no clip).
- v1.40 (adjudicated A, Synology channel-player model): PlayerTile "little TV" — cell fills grid slot
  (black letterbox, container-type:size); inner .tile-stage keeps 16:9 (min(100cqw,100cqh*16/9)) and
  hosts media+all overlays; per-stage container queries adapt controls; layout/featured rows 1fr fill.
  Fixes resize overlap + aspect break.
- v1.41 (adjudicated A refinement): overlays (bar/actions/stats/live) moved back to the CELL (whole
  player) so letterbox bars are used (title at player top, live at bottom); stage keeps only
  media+scan+idle+deadmsg; per-cell container queries prevent overlap; aspect still via stage.
- v1.42 (adjudicated A): removed stats-header state (st-head) — stats purely numeric; state stays with
  bottom-left LIVE/OFF badge + top-bar dot (de-duplicated).
- v1.43 (adjudicated A): hide the remove (×) button while zoomed (focused inspection ≠ layout edit);
  only exit-zoom remains; removal stays in multi-grid hover × + rail footer drag. Synology-consistent.
- v1.44: layout set expanded to Synology browser tier (+25/36 symmetric, +1+9/1+12 featured; Auto cap
  16) and picker became an icon-thumbnail gallery (mini-grid previews, grouped, keyboard-navigable),
  replacing the text-only dropdown.
- v1.45: sidebar nav icons replaced Unicode glyphs with unique semantic monochrome SVG sprite
  (16 icons, no duplicates, correct meanings; gauge/screens/library-play/mic/users/desktop/headset/
  camera/door/plug/waveform/sliders/key/list-check/gear/exchange); .nav-ico grid-centered SVG container.
- v1.46: renamed Recordings → **Recording** (singular), aligning Synology's "Recording" application
  (management/review workspace, sibling of singular Monitor); nav/title/head/handoff updated, ids kept.

### Intercom optimization — expert-panel review + adjudications (2026-11-17; design adjudicated; Phase 6 delivered v1.47 + v1.48–v1.61 polish/fixes/rulings)
5-role panel vs 南山对讲/PoC 调度(海能达)/Mumble: 17 findings (HIGH 5/MED 9/LOW 3; +queue_max +keyboard-PTT/a11y). Rulings (user, one-by-one):
K1 scope=C layered (core floor correctness first, then missed-speech replay + multi-channel monitor +
emergency force-insert, rest deferred); K2 half-duplex=A exclusive default + hybrid optional with cap;
K3 missed-speech=A short rolling recording buffer + explicit consent + short retention + per-channel off.
Follow-up panel (4-role parallel team) on revised PLAN: 22 findings; objective drifts fixed (naming v1.46,
edge-strip/E supersession propagation, stale labels, anchors, backlog 15->17 +queue_max +keyboard-PTT/a11y,
layer-order 6.3a buffer前置, Verify targets/consent/unknown/carrier-boundary, §11 intercom sources).
Conflicts adjudicated one-by-one: K5=A dual-buffer (replay exit-clear vs event-evidence retained+consent),
K6=A tiered (priority=re-queue, emergency=preempt), K7=B lock-screen PTT = backend-gated deferred + honest
placeholder (in-page cross-page hotkey done now). Full backlog + Phase 6 in parent PLAN §2j/§6d.
- v1.47: **Phase 6 delivered** — 6.1 floor correctness (priority re-queue + auto-grant, hold-timeout 30s
  enforced + remaining hint, hybrid cap 2, queue_max 8, Prio real toggle, queue order transparent, keyboard
  PTT + aria-live); 6.3a consent-gated rolling replay buffer (exit-clear); 6.2 replay player + multi-group
  monitor(cap 4)/scan + emergency insert(two-click, preempted toast, audit); 6.3b event evidence(7d+audit);
  6.4 1:1 call/temp group/VOX-NR-AGC/SD-HD+latency/cross-page PTT/lock-screen honest-disabled; 6.5 messages/
  location/whisper/per-member volume/encrypted tag. Micro-decisions fixed (hybridMax2/queueMax8/MON4/REPLAY20/
  EV7d/VOXoff/HDdefault). Verified assertions + full-page regression green.
- v1.48: channel right column reorganized from a 9-card flat stack into a Material side-sheet with 4 tabs
  (Floor / Recordings / Comms / Diagnostics), research-backed (WAVE PTX region separation, Discord channel
  tabs, Material side-sheet). Diagnostics condensed to compact rows + one simulated footnote; right column
  constant one-screen height; per-tab wiring null-guarded.
- v1.49: participant row harmony — unified control heights to --ctl-h (p-btn 30, p-vol 30×88), actions grouped
  (.p-actions gap sp4) with row gap sp12, volume before action group, state dot breathing, name/role two
  lines, row padding sp10 sp12 + min-height 44 (C54 touch). Fixes tight/disharmonious member rows.
- v1.50: fixed missing `--sp10` spacing token (scale had 8→12 gap). `padding:var(--sp10) …` was an invalid
  declaration → participant rows had 0 left/right padding (flush to edges); also fixed latent `.combo-opt`
  horizontal padding and `.rec-player` gap. Added --sp10:10px; rows now 12px insets aligned with head.
- v1.51: intercom control-size audit — unified secondary controls to --ctl-h:30 (.act 31→30, .ch-tab 28→30,
  .vol 20→30, .msg-in 28→30); primary PTT 46→40 (deliberate hero step); coarse ≥44 extended to
  .ptt/.act/.ch-tab/.p-btn. Two-step scale: secondary 30 / primary 40 / coarse 44.
- v1.52 (corrects v1.51): user flagged VOX/HD/PTT still ununified. v1.51 had imposed PTT=40 (against the
  unify requirement) and audited height only. TRUE unification: all text buttons (.btn/.btn-sm/.chip-btn/
  .ptt/.act/.btn-live) = h --ctl-h:30 + pad 0 sp12 + r6 + fs12; PTT distinguished by color/weight not size.
  Verified h30/r6/pad12 across VOX/HD/PTT/lock/req/emerg. C54: audits cover height+radius+padding, no
  self-imposed scale deviations against explicit unify requirements.
- v1.53: user light-theme screenshot still perceived VOX/HD/PTT different; geometry was already identical
  (h30/r6/pad12 both themes) but FILL family differed (VOX solid sunken / HD outline / PTT faded). Unified
  resting fill: .chip-btn bg sunken→surface (outline family like .btn); on=accent-tint; disabled same box.
  C54: harmony audits cover fill/background family + states, not just geometry.
- v1.54: disabled PTT whole-box opacity .4 made it look fainter/smaller; changed .btn:disabled to same box +
  muted text (opacity 1). Screenshot-verified at 2x isolated: VOX/HD/PTT three identical h30 boxes, only text
  color differs. Stale browser cache ruled out via footer version + hard-refresh guidance.
- v1.55: repeated-fix root cause = three buttons on three cascades (chip-btn / chip-btn.on / btn.btn-sm);
  family-by-family fixes were whack-a-mole. Structural fix: ONE dedicated .diag-ctl class pinned on all three
  (30px min/max height, pad 0 12, r6, border-box, end of cascade) → cascade-independent identical box.
  Screenshot-verified h30 x3. Lesson: unify by shared pinned class, not by patching each family.
- v1.56: widths still auto (label-length) after v1.55 pinned height; .diag-ctl now pins width 56 too →
  VOX/HD/PTT identical 56x30 boxes (screenshot-verified). C54: identical size = both axes pinned.
- v1.57: expert-panel re-review objective fixes — idle/queued auto-grant + no queue-jump; enterChannel cleans
  side/ptt + manual leave stops scan; emergency arm per-channel + survives re-render + no physical press;
  Enter keyup global + focus restore; Comms unknown-guards; autoGrant clears _q; WS double-drop guard; replay
  playback state; pointer capture; chan-head wrap + board track 320; tabs roving/arrow/tabpanel + Esc input
  guard; whisper/call aria-label + cueToggle keyboard; Prio rewrites queue; temp id unique; hybrid seed side;
  floor.hybrid_max config row; r6 alignment; emoji->SVG; board empty state + Space. K8-K11 pending adjudication.
- v1.58 (K8=C): VOX simulated voice source auto-keys PTT; codec simulated weak-net auto-fallback SD +
  recovery HD + turnaround readout; shout/ACL relabelled switch/indicator-level (backend-gated). K9-K11 pending.
- v1.59 (K9=C): replay exit-clear scope — scan switching keeps buffer (continuous listen session); manual
  channel switch / cross-page goto / Back clear it; consent + diag copy state both paths. K10-K11 pending.
- v1.60 (K10=A): chLock + unknown Replay Play use setGuard (aria-disabled + data-reason + is-disabled) instead
  of native disabled, so the disable reason is discoverable on hover/focus (site guard doctrine / C54). K11 pending.
- v1.61 (K11=C): split-path rendering — paintChannelLive/paintBoardLive incremental for PTT + per-second tick
  (nodes not rebuilt -> pointer capture/focus/scroll/range survive); full render only for structural changes.
  All panel conflicts K8-K11 now adjudicated (C/C/A/C).
- Remaining intercom scope adjudicated: K12=B (Phase 7 = group mgmt + cross-page floor notifications +
  evidence privacy completion + assertion harness; history/per-event/loading/shortcuts deferred to PLAN §8);
  K13=A keep Overview default landing (re-review of ruling 2). Phase 7 design-only, NOT implemented.
- Phase 7 mechanisms adjudicated: K14=B (real-Date.now TTL purge + Prototype-controls 'Inject expired
  evidence' simulated trigger), K15=A (cross-page chip+cue suppress toast / in-page toast / badge pure count),
  K16=C (emergency armed double-click; disband/revoke modal confirm). Phase 7 fully defined, NOT implemented.
- v1.62: **Phase 7 delivered** — 7.1 group mgmt (new dialog infra: focus trap/initial focus/stack restore/Esc
  arbitration; create/rename/members/roles dispatcher-gated; disband cascade incl buffers/call/chanOpen;
  member deep-copy; devtools role leg); 7.2 cross-page floorNotif chip (dedup<=3/overflow/expiry per type/
  stale-click revalidate/click-land; K15 gate list; devtools external emergency+queue legs); 7.3 evidence
  epoch + real-time TTL purge + open-tab repaint + revoke modal clears + ev-tag affordance + inject-expired
  leg; 7.4 Marionette harness scripts/ui-harness.mjs (9 cases ALL GREEN, .harness png+results.json, stat>0);
  7.5 regression green (monitor/tower/recordings/overview + node --check + DOM + lint).
- v1.63: board rows aligned in every state — fixed 6 icon action slots (30px, no-wrap, hidden Disband
  placeholder on non-temp rows), checked = color + aria-pressed only (no ✓ label growth), action column
  220 (coarse 284); narrow reflow ≤900 3-row card / ≤600 stacked card; ≤760 topbar hides crumb+role-tag
  (was the 600px page-overflow source); min-height 44. Verified uniform rowH/actH/rightEdge + no overflow.
  Mid-band follow-up (Mon icon outside card at ~901–1100px): desktop tracks → minmax(120,210)/minmax(64,92)/
  minmax(120,1fr)+ellipsis, card breakpoint 900→1000 (coarse ≤1100; coarse 284 col only ≥1101); verified
  inside:true + holder readable + rowSW==CW at 1000/1100/1200.
- harness tooling (no UI bump): board-1100/1000/1200 cases + `--negative` sabotage self-check (5 NEG-OK);
  normal mode 12 cases ALL GREEN; .harness products refreshed (C65/C66 landed).
- v1.64: admin nav IA (B+D, research-backed Synology/Milestone/UniFi) — sidebar Administration collapses to
  ONE Admin entry; #admin hub = 4 domain cards (Assets/Observability/Security/System) with 9 spoke rows;
  spokes keep routes, highlight Admin parent + breadcrumb "Admin / "; naming converged (Config delivery/Keys/
  Audit). C (role disclosure/⌘K/pin) deferred to PLAN Phase 8.4. harness 13 cases ALL GREEN incl admin-hub.
- v1.65: admin spoke back affordance (head back iconbtn + clickable crumb "Admin"; ≤760 crumb single-line
  ellipsis instead of hidden) + narrow tables reflow to label-value cards (data-th via MutationObserver) +
  compact page-head; no horizontal scroll @600. harness 14 cases ALL GREEN incl admin-spoke.
- v1.66: admin control-family normalization (C69) — all spoke interactive controls 30/r6/sp12 (iconbtn 34→30
  incl play-ctl, search 36→30, seg fixed 30, msg-in r4→r6); state=color-only; table action slots min-64;
  set-v min-height 30; duplicate btn-danger removed; ＋/ glyphs → i-plus/i-search SVG. harness 15 cases ALL
  GREEN incl admin-controls (h30/stateStable/actionsUniform).
- v1.67: table action buttons fixed 72px slots (min-width floor left label-driven widths); grpScrim/cfmScrim
  class corrected scrim→modal-scrim (v1.62 dialogs lacked hiding/centering CSS). harness 15 ALL GREEN.
- Device detail adjudicated (design only, NOT implemented): K17=A side-sheet drawer (reuse channel side-sheet;
  upgrade-to-page trigger registered), K18=A staged delivery on offline + reconnect flush + 3-state labels,
  K19=B three-tier role matrix (viewer read / dispatcher identity+config / admin +caps+lifecycle),
  K20=B no fake preview (stats + status timeline + Open in Monitor). PLAN §2m / Phase 9 pending.
- Phase 9 plan v2 (4-role team re-review): dead-ends closed — devtools "device reconnect" leg + flushStaged
  hook (revoked excluded); Reject→rejected / Revoke→offline+revoked+clear-staged / Restore+Re-approve paths;
  failed→Retry/Discard; rollback snapshot at delivered + Undo UI + audit name; Esc priority chain
  modal→drawer→cascade; drawer infra registered NEW (ch-side not reusable); focus trap/restore + a11y into
  Verify; draft kept per-device with row badge; staged row badge + flush toast; Details label; scope resolver
  new (user scope excluded). Cross-doc drifts fixed (§7 order note, §8 ledger, boards head/pages/Admin row).
- v1.68: **Phase 9 delivered** — device drawer (480px / full-width ≤900, 4 tabs, Esc chain modal→drawer→cascade,
  focus trap/restore, tabs roving); config draft→diff→Apply with staged-on-offline + reconnect flush + failed
  Retry/Discard + rollback depth1 + Undo; row staged/draft/failed/revoked badges; role matrix viewer/dispatcher/
  admin (+field→viewer) with devtools legs; state machine rejected/revoked/Restore/Re-approve + confirm+audit;
  history timeline; harness 19 cases ALL GREEN incl device-drawer/narrow/config/role + NEG.
- Design round 2026-11-18 (plan-only, NO UI change; sidecar stays v1.68): web-portal-ui §3h Q1–Q19
  adjudicated (Q18 empty-number logged); §3i B1–B6 board plan v4 approved-pending; gate = plan approval
  before drawing; B1 drawn-without-approval was reverted clean (no accounts/matrix residue).
  B1 scope: Access domain + capability matrix + Accounts (4 roles/Device scope/break-glass/focus contract/
  three-state/guard single-track sweep/default light per Q19); B2 PSK+Keys; B3 creds lifecycle; B4 Fleet
  lens+Telemetry; B5 Monitor real-backend link; B6 audit ring. Safety invariants: fail-closed all channels,
- v1.69 (B1 delivered, PENDING USER REVIEW): Access domain + 9-row capability matrix + Accounts page
  (4 roles/Device scope/break-glass/focus contract/three-state/guard sweep/default light/mode-health
  indicator/degraded banner). Region annotations: U2/W4 naming (Q10), P8 index+spoke, C54/C69 control
  family, C62 honesty (simulated provider/health, gated vision), Q9 derivation, Q11 server-authoritative
  roles, Q12 Device scope, Q13=E′ affordances, Q19 light, PIT-269 dialog contract. Three states: empty/
  loading/error+Retry. Open micro-decisions: scope multi-select form, loading duration, error-leg name,
- v1.70 (B2 delivered, PENDING USER REVIEW): Keys dual partition — PSK card (view audited each reveal /
  rotate confirm + optional custom 8-128 / one-time display) + API keys real fields (id/role/scope/label/
  expires) + create modal + revoke confirm+audit; one-time secret modal with close protection + copy
  fallback; delegated gates + break-glass emergency rotate; three states; legacy fake delegates removed.
  Region annotations: Q2=B, Q13=E′, C62 one-time secrets, C54/C69, PIT-269 (secretScrim body-level after
  cfm for Esc order), Q12 scope. Open micro-decisions: one-time display modal vs inline reveal (chose
  modal), copy-fallback copy, old-secret invalidation copy.
- v1.71 (B3 delivered, PENDING USER REVIEW): Credentials tab (register+approve w/ optional name+provided
  secret / reset-secret confirm / one-time close-protected secrets / revoke=DELETE semantics card /
  Reject-Restore-Reapprove no-endpoint labelled); vision gating Q4=B (VISION_SET + vGuard reason +
  devtools simulate leg); three states; drawer tabs 4→5 regression updated. Region annotations: Q4=B,
  Q12, C62, C54/C69, PIT-269. Open micro-decisions: provided-secret validation rules, reset cooldown,
  semantics-card wording.
- v1.72 (B4 delivered, PENDING USER REVIEW): Fleet lens (radiogroup+roving) + health strip 5 counts
  (unreachable supersede topic/process counts) + row telemetry summary + Telemetry tab (6th, full
  Vehicles.tsx fields) + Overview fleet card deep link + scope cross-link; three states; tabs 5→6
  regression updated. Region annotations: Q3=D', Q12, U2 lens label, C66 breakpoints, C54/C69.
  Open micro-decisions: stale threshold 30s, health-strip narrow wrap, lens default memory.
- v1.73 (B5 delivered, PENDING USER REVIEW): Monitor linkage — picker tree ARIA+keyboard in place,
  Play All (connected-only, skipped counts), WALL_CAP=16 guards (checkbox/bulk/Apply), per-stream
  metrics asserted, #monSimNote honest banner, three states + Retry restores data.
  **Real-backend contract mapping (design)**: stream id `${dev}-${ch}` ↔ SFU producer id
  (GET /sfu/rooms); res/fps/bps ↔ GET /sfu/stats?producerId= (score/bitrate); latency ↔ consumer
  stats roundTripTime; connected ↔ device status + producer paused; wall select ↔ consumer subscribe;
  Play All ↔ batch subscribe capped 16. Simulated until backend wired (C62).
  Open micro-decisions: tree default-expand depth, Play All confirm when >8 streams, metrics refresh rate.
- v1.74 (B6 delivered, PENDING USER REVIEW; ROUND COMPLETE): Audit = security minimal ring (ring filter
  excludes operational events per T2), kind seg radiogroup, source column core/provider/simulated
  (login/logout query face simulated), honest retention label, guarded operational-history link,
  three states. Region annotations: Q8=A, Q6 notes, C62, C54/C69. Open micro-decisions: retention value
  90d, ring page size/export, provider feed contract.
- Design round B1–B6 complete: 26 harness cases ALL GREEN + NEG all red; versions v1.69–v1.74.
- v1.75: nav collapse icon fix — svg.ic has no intrinsic size (scoped rules only); collapse button SVG
  rendered 300x150 and was clipped → invisible. Added #navCollapse .ic size + flex centering; unified
  all state paths through top-level syncNavCollapse() (SVG sprite + title + aria-label); removed ⟨/⟩
  text glyphs (C56). harness 27 cases ALL GREEN + NEG red (nav-collapse case).
- v1.76: drawer collapse icon fix — ddClose was i-arrow-left (back-navigation family, wrong direction
  for right-edge collapse); now region-collapse + i-chevron (right), aria-label/title synced in
  renderDrawer. Precedent: region-collapse chevrons point toward collapse edge; modal dismiss = X.
  harness device-drawer += ddIcon assertion; 27 cases + NEG green.
- v1.97 (Remote N-6 launcher, PENDING REVIEW; §3q round complete): dual-column launcher + meta row +
  3 real/4 gated capability tiles + gated power row + pure-view semantics (capture all-entry guard,
  tile demoted, leaseTick skip, safe-bar view variant, estop kept) + three states + stale guard.
  51 cases + NEG green.
- v1.96 (Remote N-5 window-ization, PENDING REVIEW): safeBar never-hide + auto-hide overlay topbar +
  captured ball (drag/dock/dim/forbidden rebound/ballDockTick/ToDesk menu IA/gated display) +
  full-bleed contain/1920 1:1 scroll+middle-pan + session switch popover + connecting + defects 3/4/5.
  49 cases + NEG green.
- v1.95 (Remote N-4 safety small-fix, PENDING REVIEW): Space-on-chrome/select/range not forwarded to
  asset (native activation kept; tile unaffected); <500px bottom bar yields to E-STOP (400px uncovered
  assertion). 48 cases + NEG green. Defects 3/4/5 scheduled N-5 per D12=C.
- v1.94 (Settings S-2 delivered, PENDING REVIEW): Keys Admin bootstrap token partition (paste/save/
  mask/clear confirm/audits/non-admin guard incl viewer path) + Settings six-row read-only config
  index (server_config shape, simulated values, Open deep-links, Edit gated manifest-pending,
  registration discipline). keys-token verified page-side (Marionette wedge, v1.77 precedent).
  47 cases + NEG green.
- v1.93 (Settings S-1 delivered, PENDING REVIEW): Account Login card (simulated session switch +
  signed-in/TTL/signout/error leg/focus/mask), About card (version+build simulated), Signaling Test
  honest result row (simulated), Access orphan-title fix. harness 46 cases + NEG green.
- v1.92 (Remote N-2 delivered; §3n ROUND COMPLETE pending final review): zero-nav session (grid single
  column fix) + hybrid layout (top bar groups / tile fill contain / shared .session-bar bottom shell
  also on meetBar / tb-pop menus anchored) + six features (fullscreen root + fullscreenchange release
  fallback + headless stub, fit/original zoom, session clock, OSD aria-pressed sim, special-keys same
  lease + audit, Take/Release inherited) + info popover honesty/five-not-built + H exemptions.
  45 cases + NEG green. Open items logged CHANGELOG v1.92.
- v1.91 (Remote N-1 defect batch delivered, PENDING REVIEW): gamepad grace same-lease; conn lost guards
  capture/input/estop + #remHealth (no false STOPPED); resume hold drag/Esc cancel via dlgClose;
  cross-room latch bound id; latch keeps holder (D6, holder-resume reachable); Y-edge pre-early-return +
  remRelease clears estop state; force-latched inert + corrected copy; Back holder gate + full cleanup;
  nav release + body classes + lobby STOPPED chip; Esc single chain v2 (page-owned meet/remote,
  input-guard first-blur); key whitelist v2; tile keyboard + Take/Release bar + seize confirm
  (cfmCancel focus + OK revalidate); --video-ink + WCAG formula assertion; estop tab order/accesskey/600;
  req focus restore; grace #remLive transition-only announcements + bar mirror; dual hint; statusPop
  quote fix; YAML TRULY repaired + 24 remote hooks + regions/open_questions; CORRECTS v1.90 YAML claim
  and v1.88 holder-branch claim; stale copy updated. 43 cases + NEG green.
- v1.90 (Remote R-5 delivered; B-R1 ROUND COMPLETE pending final review): gamepad injection leg +
  gamepadTick same-lease + Y=estop path; in-flow collapsible gamepad map (teleop-only); touch joystick
  gated; setGuard aria-describedby + tap-tip sweep; boards e2e_hooks YAML repaired. 42 cases + NEG
  green. Region annotations: R5=A/R6=A, workbudy 03:163-164 same-lease, C54 guard a11y, DQ tokens.
- v1.89 (Remote R-4 delivered, PENDING REVIEW): holder badge + request-no-seize + persistent
  alertdialog Accept/Deny chip + cue + 30s timeout + accept clears old captured/keys + released frame +
  deny feedback + admin force confirm (four consequence elements, free-for-all not auto-hold) +
  no-holder guard (floor 409 precedent). Region annotations: R4=A, floor force-release precedent,
  C54 guards, PIT-269 confirm. harness 41 cases + NEG green.
- v1.88 (Remote R-3 delivered, PENDING REVIEW): E-STOP hold400 ring + keyboard/touch cancel feedback +
  whole-window 4px perimeter + STOPPED banner + emergency capability gate + latched guards + ttl expiry
  while held + resume hold-then-typed-name asymmetry (holder||admin) + audits. Region annotations:
  workbudy 03 §8/170-174, R2=A, C54 guards, C69/coarse44, PIT-269 modal. harness 40 cases + NEG green.
- v1.87 (Remote R-2 delivered, PENDING REVIEW): three-state chip + capture precondition + ttl lease
  machine (300 renew/1000 grace/<300 warn dual/expiry released frame clears keysHeld/expired→idle) +
  WS-drop immediate release + reconnect stays view-only + HUD five + H exemptions + Esc chain position.
  remote-timing 9-part real-timer leg. Region annotations: W2 numbers, workbudy 03:104-105/158-161,
  R1=A/R3=A, C62 simulated†, C54 guards. harness 39 cases + NEG green.
- v1.86 (Remote R-1 delivered, PENDING REVIEW): lobby desk/ctrl cards (mode/holder/lease/conn/sim†) +
  view-only session (video-bg token/VIEW-ONLY chip/paired-view note for ctrl) + three states + Retry;
  legacy native-disabled Acquire removed. Region annotations: W2 lease numbers, U4 + kind exception
  note (ctrl teleop mode under desk app), workbudy M1 cross-room feed, C62 simulated†, DQ video-bg
  token. harness 36 cases + NEG green.
- v1.85 (M-6 polish, panel-driven): speaker REAL render+rail+off-page indicator; panel lifecycle
  (leave closes/PiP shift/narrow bar z); PiP content linkage; tile conn overlays (dot/3-bar/ring+slash);
  context never-hide row (name/clock/count); guest ended card; Reopen resets+transcript archive;
  speaking scheduled+voice pool; chat unread badge; panel tabs a11y; hold arbitration; #meet-<id> deep
  link. 35 cases + NEG green. Region annotations: workbudy 03 §5.3/§6 + 01 §7 overlays, W03:87 identity
  never-hide, C54/C69, PIT-269, §2i independence note.
- v1.84: fix meet exit perceived-stuck — visible hold progress bar (pointer/Esc/keyboard), cancel
  feedback (pointerleave/pointercancel toast), persistent exit-guidance chip (H-exempt); meet-exit
  10-part real-timer case; harness exit-hook kills firefox (stale accumulation root). 33 cases + NEG.
- v1.83: fix direct-content modal zero padding (.modal.pad on acc/psk/key/cred/meet/prejoin/
  meetTrans/secret); user-reported prejoin edge-to-edge anomaly at 844x682 resolved.
- v1.82 (Meeting M-5 delivered; B-M1 ROUND COMPLETE pending final review): error+Retry/Back with shell
  restore, ended read-only transcript, honesty sweep (host controls simulated / consume-only note),
  regression 32 cases + NEG green. Open items logged in CHANGELOG v1.82.
- v1.81 (Meeting M-4 delivered, PENDING REVIEW): roster/chat side panel, waiting admit/deny + cue +
  badge, mute-all(excl self)/lock/unlock/remove(confirm, excl host/self), non-host guards, hand
  raise/lower, panel focus + Esc-closes-panel contract. harness 31 cases + NEG green.
- v1.80 (Meeting M-3 delivered, PENDING REVIEW): zero-nav live canvas (1/2/4/9+avatar non-black+
  speaking 2px fill-unchanged+self-PiP mirrored narrow-top-right+mute persistent chip+control bar+
  hold-1s leave/Esc chain+H exemptions+coarse44+estop empty). harness 30 cases + NEG green.
- v1.79 (Meeting M-2 delivered, PENDING REVIEW): prejoin (name focus/simulated check/voice cam guard),
  connecting shell-kept, host live/guest waiting + guest screen/cancel, admit/deny logic (UI=M-4),
  locked guards, Esc cancel, interim live card + Leave (canvas=M-3). harness 29 cases + NEG green.
- v1.78 (Meeting M-1 delivered, PENDING REVIEW): lobby type chips/partsCount/host/ended+Reopen;
  create modal (name+Voice/Video radiogroup); invite real copy w/ parameterized fallback hint;
  Join/Start honest interim gate until M-2 prejoin. harness 28 cases + NEG green.
- v1.77: topbar convergence A — right cluster 3 (bell+badge / status chip→statusPop / user menu→
  userPop with role+theme+mute+about+signout); crumb path-only; brand PORTAL removed; popover
  focus/Esc/outside-click contract; narrow converges (name hidden). Verified page-side (overlay
  assertions all true + screenshots) due to unreproducible Marionette wedge on this case;
  Marionette suite 27 cases + NEG green (run() timeout+retry added; accounts-narrow NEG retargeted).
- (ledger fragment, parent entry lost in earlier edit; restored 2026-11-20): placeholder copy, Force
  consequence copy. harness 21 ALL GREEN + NEG red; narrow docSW==600.
- (ledger fragment, parent entry lost in earlier edit; restored 2026-11-20): server-side provider
  enforcement, durable break-glass audit, revocation sync, contract-first ceiling.
- Design rounds (plan-only, pending approval): §3p window-ization D8-D12 (N-4/N-5) + §3q launcher D13-D16 (N-6); evidence/todesk-01..12.
- Architecture diagram set: web-portal-ui PLAN §3j (capability ownership / dual-mode deployment /
  authn-authz-breakglass sequence / prototype IA matrix affordance map), mermaid+ASCII, per Q1–Q19.
