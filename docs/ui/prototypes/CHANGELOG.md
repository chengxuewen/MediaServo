# Prototypes CHANGELOG

## v1.24 — 2026-11-16 (portal-unified) — monitor-center P3.2: per-channel independent timeline (dual mode)
Completes the last deferred feature of adjudication 5 (global sync-seek default + per-channel solo).
- **Solo toggle** in the ctl-bar (next to the Sync-seek chip). Default off = global sync-seek (all
  cameras one wall-clock time). When armed, each **zoomed** channel keeps its own playhead: `chRatio`
  map saved/restored on zoom change (renderWall), `tlPaint` records the active channel's ratio, and the
  Sync-seek chip reads "Solo". Zooming channel A→B→A restores A's time; turning Solo off returns to the
  shared global time. Honest gate unchanged (recorded seek still recording-recon).
- Verified: node --check + DOM refs clean; headless Solo render; click-through
  `defSync/solo/chA/chB/restore/off` all green.

## v1.23 — 2026-11-16 (portal-unified) — monitor-center P3 elastic heavy tier: drag-line column resize
Extends the P3.1 custom elastic grid from discrete 2×2 spans to continuous track sizing (Synology
"drag the window frame to resize each channel").
- **Draggable column separators** in Custom · elastic mode: N−1 handles positioned at track boundaries;
  dragging rebalances the two adjacent column weights (min-clamped), re-flows the grid live via
  `grid-template-columns: <w>fr …` (no full re-render), and persists weights (`ms_col_weights`).
  Handles highlight on hover/drag; `setPointerCapture` guarded for synthetic pointers.
- Verified: node --check + DOM refs clean; headless custom render shows the dragged column widened with
  live reflow; click-through `handles:3 / resize / persist / fr` all green.

## v1.22 — 2026-11-16 (portal-unified) — monitor-center P3.3 + P3.5: event bus / alarm-to-wall + in-page clip player
- **P3.3 unified event bus (R5) + alarm-to-wall + alarm→intercom linkage**: a runtime alert stream
  (`MON_ALERTS`, seeded from PB_EVENTS) replaces the static list; `emitEvent/onEvent` bus with
  EventType set (motion/alarm/tamper/audio/bookmark/intercom-floor). Consumers: alerts panel (prepend
  + unread badge), cue (alarm=alert tone), **alarm-to-wall** (Auto-wall toggle in the alerts head,
  default off; when armed, new alarm/motion auto-focuses the camera on the wall), and the
  **alarm→intercom** differentiator hook (intercom nav badge pulse + notify toast; deeper floor-grab
  reserved for the real backend). Prototype controls gain a "Simulate event" row (Motion/Alarm/
  Bookmark) as the honest event source. Fixed an `ALERTS` identifier collision with the Overview data
  (renamed monitor stream to MON_ALERTS).
- **P3.5 Recordings in-page clip player (adjudication 8 option B)**: selecting a clip now previews it
  in a right-hand player (16:9 honest "no recorded preview yet" placeholder, metadata, play/export
  honest-gated, bookmark, and an "Open in Monitor" handoff to the single investigation player). The
  clip list keeps search/filter/bookmark; selected row highlighted; responsive single-column ≤900px.
- Verified: node --check + DOM refs clean; headless renders (alarm-to-wall focused VH-02 with Auto-wall
  armed + new alarm prepended; Recordings list + in-page player); click-through P3.3
  (bus/offNoZoom/armed/wall/pulse/sim/read) and P3.5 (pempty/sel/player/playGuard/expGuard/pbm/mon)
  plus intercom regression, all green.

## v1.21 — 2026-11-16 (portal-unified) — monitor-center P3.1: elastic custom grid (per-tile 2×2)
First P3 (advanced) increment — the lightweight elastic tier (per-tile size classes; full drag-line
resize remains future).
- **Custom · elastic layout**: a new layout option using CSS grid `grid-auto-flow:dense` (4 columns);
  tiles keep 16:9 and pack densely. Each tile gains a **size button** (i-resize) toggling **1×1 ↔ 2×2**
  span; enlarged tiles reflow the wall Synology-style. Per-tile spans persist in localStorage
  (`ms_tile_spans`); elastic shows all streams (no pagination); named layouts unchanged.
- **Fix during verification**: the size button initially relied on the tile's delegated onclick +
  `e.target.closest('.size-b')`, which missed clicks landing on the SVG `<use>` shadow target in
  headless dispatch. Bound the size button's onclick directly with `stopPropagation` — robust.
- Verified: node --check + DOM refs clean; Firefox-headless custom render (two tiles enlarged to 2×2,
  dense-packed); click-through `elastic/span2/gridSpan/persist/shrink/back9` all green.

## v1.20 — 2026-11-16 (portal-unified) — monitor-center P2.3–P2.5: Recordings library + carousel (P2 complete)
Completes `docs/plans/monitor-center/PLAN.md` Phase 2.
- **P2.3 Recordings library refactor**: replaced the camera-selection wall with a proper clip library
  — search box (camera/event/time), type filter chips (All/Motion/Alarm/Continuous/Bookmark),
  "Bookmarked" toggle, and a clip list (type-icon disc + camera + time + duration + type tag +
  bookmark star + export). Bookmarks toggle locally (some pre-bookmarked); export per-clip + header
  honest-gated (RECORDING_READY=false). Click a clip → focus its camera on Monitor (zoom; review
  seek gated). Clip list is event-based design samples, clearly labelled in the honesty banner — no
  fake thumbnails (C62). Removed the obsolete Snapshot button + dangling pbSnap refs.
- **P2.4 carousel (R1=C)**: optional sequential page auto-cycle, **default off**; stage-bar Carousel
  toggle + interval spin (3/5/10/30s). Advances pages in order, wraps, pauses when zoomed/single-page/
  off-Monitor; manual pager kept as fallback (big-layout-primary + carousel-optional + manual coexist).
- **P2.5 regression**: Intercom (board 6 + channel PTT), Tower, Devices/admin, Recordings, Monitor all
  render & function unchanged; node --check + DOM refs zero missing + read_lints clean.
- Verified: Firefox-headless Recordings library render (filters/bookmarks/gated export); click-through
  P2.3 (rows10/alarm2/bookmarked2/search4/bookmark-toggle/export-guard/clip-focus) and P2.4
  (default-off/pages2/on+btn/advance/wrap/interval10s/off/manual-pager) all green; P2.5 regression
  (ic6/chan/tower/dev/rec/mon).

## v1.19 — 2026-11-16 (portal-unified) — monitor-center P2.1–P2.2: layout templates + drag-to-orchestrate
- **P2.1 wall layout model + templates (R6)**: replaced the fixed "N per row" combo with a layout
  selector — symmetric **1 / 4 / 9 / 16** + asymmetric featured **1+3 / 1+5 / 1+7** (one big tile
  spans 2×2, 1×3 for 1+3; integer cell spans). Layouts are named CSS grid-template-areas; cameras fill
  ordered slots, unfilled slots render as dashed "Empty slot" drop placeholders. The grid fills the
  wall height (tiles stretch to their slot, big tile genuinely featured); pagination adapts to slot
  count; layout choice persists in localStorage; zoom falls back to single-column.
- **P2.2 drag-to-orchestrate (R8)**: rail stream items + wall tiles are drag sources; drop on a tile
  or empty slot **reorders** the wall; drag to the rail footer **removes** (footer becomes a red
  "Drop here to remove" zone during drag); **Auto-fit layout** button picks a template fitting the
  stream count. Adding new cameras stays with the shuttle picker (drag = orchestrate, picker = batch
  add — the adjudicated division of labor). Drag-over highlights, dragging opacity, grab cursors.
- Verified: node --check + DOM refs clean; Firefox-headless renders of layout 9 (3×3 + empty slot),
  1+5 (big 2×2 + 5 small), 1+3, 16; click-through P2.1 (default9/pageSize9/1+5 tiles6 bigArea/1+3
  pages2/16 empty8) and P2.2 (rail+tile draggable/auto-btn/moveStream/auto-fit/simulated drop reorder/
  footer-drop remove) all green.

## v1.18 — 2026-11-16 (portal-unified) — monitor-center P1.4–P1.6: alerts panel + guards + regression (P1 complete)
Completes `docs/plans/monitor-center/PLAN.md` Phase 1.
- **P1.4 alerts panel**: Monitor is now a 3-column grid (left stream rail | center wall+timeline |
  right alerts). Right panel = event list (PB_EVENTS, now camera-linked) + type filter chips
  (All/Motion/Alarm/Bookmark) + unread badge + "Mark all read". Click an alert → focus that camera on
  the wall (zoom; adds it to the wall if absent) + mark read; time-review seek stays honest-gated.
  Both side panels collapsible with **R7 default-collapsed + localStorage memory**; stage-bar gains an
  "Alerts" toggle with a live unread badge. Explicit grid tracks pinned (PIT-252) for all four
  collapse combinations + mobile/≤900px.
- **P1.5 guards final pass**: confirmed two coherent guard sets — applyLiveGuards (wall: stats/cols/
  pager/sync/Recordings-handoff) and applyPlaybackGuards (timeline/transport/event-nav/calendar +
  Recordings export/snapshot). Live actions (alert-focus, camera select, back-nav) stay enabled;
  recorded review/export/snapshot/event-jump disabled with reasons under RECORDING_READY=false.
- **P1.6 regression**: Intercom (board 6 rows + channel PTT), Tower, Devices/admin render & function
  unchanged; node --check + DOM refs zero missing + read_lints clean.
- **Bug found & fixed during verification**: alert state `const alUnread=new Set(PB_EVENTS…)` was
  placed before the PB_EVENTS declaration → TDZ ReferenceError blanked the whole app at init. Moved
  the state declaration after PB_EVENTS (functions hoist). Caught via a top-injected window.onerror
  overlay (node --check passes syntax but not TDZ order).
- Verified: Firefox-headless fresh-profile default (both panels collapsed, wall+timeline+topbar,
  Alerts badge 5); 3-column expanded; click-through P1.4 (default/badge/rows5/motion3/focus+unread/
  markAll/collapse), P1.5 guards 10/10, P1.6 regression (ic6/chan/tower/dev/mon).

## v1.17 — 2026-11-16 (portal-unified) — monitor-center P1.1–P1.3: nav rename + Monitor merge + timeline semantics
Implementation of `docs/plans/monitor-center/PLAN.md` Phase 1 (first three tasks), after 3 adjudication
rounds + expert-panel review. Synology-style integrated Monitor + Recordings library.
- **P1.1 nav/routing/naming**: menu `Surveillance=[Monitor, Recordings]` (plural collection word);
  page titles "Monitor Center" / "Recordings library"; code/kind `view` (data-kind annotation);
  page ids live→monitor / playback→recordings; render fns renderMonitor/renderRecordings; old
  `#live/#playback` deep-links redirect; Overview stays default; overview quick-link updated.
- **P1.2 Monitor merge**: the global timeline ctl-bar (timeline/transport/speed/window/calendar/
  sync-seek/LIVE) moved OUT of Recordings INTO Monitor — wall + timeline now on ONE page. LIVE badge
  + clock + "⏪ Recordings" handoff in the bar; Recordings reduced to a library (cameras/events/clip
  wall, NO second timeline). Timeline guards now keyed on the Monitor wall's `selected`.
- **P1.3 instant-playback semantics**: drag timeline = in-place review, pull to right edge = back to
  LIVE (`seekTo`); prev/next-event buttons jump across `PB_EVENTS`; all honest-gated by
  `RECORDING_READY=false` (recording-recon) with reason tooltips.
- Verified: node --check + DOM refs clean; Firefox-headless renders (Monitor wall+timeline, Recordings
  library without timeline); click-through assertions P1.1 (nav/route/redirect/kind 8/8), P1.2
  (tlInMon/tlNotRec/ctlInMon/clock/toRec/rewGuard/wallTiles/recClips 8/8), P1.3 (evBtns/evGuard/
  tlGuard/liveBtnGuard 4/4). #live→monitor redirect confirmed by render.
- Remaining P1: P1.4 alerts panel (right collapsible + click-jump + filter + unread), P1.5 guards
  final pass, P1.6 regression + docs. P2 (layouts/drag/Recordings library) and P3 deferred per PLAN.

## v1.16 — 2026-11-16 (portal-unified) — Playback design: Live/Playback coordinated, single owner
User asked to analyse Playback, research mainstream VMS, and make Live & Playback complement (not
conflict). Evidence of the old conflict: Live carried a FULL recorded-playback ctl-bar (timeline +
transport + speed + window + calendar, all guarded) while the Playback page had its OWN second
calendar + a static timeline mock — inverted ownership + duplication (two calendars, two timelines).
- **Research** (Milestone XProtect docs authoritative; Frigate / UniFi Protect / Hikvision-Dahua
  patterns): Playback is a dedicated tab where ALL cameras are tied to ONE wall-clock time
  (sync-seek), a two-tier color-coded timeline (record / motion / event), a playback controller
  (play/pause, frame-step, reverse, speed, shuttle, range), event/bookmark search and export. Live is
  a separate real-time tab. Adopted wholesale.
- **Division of labor**: Live owns the present (wall, columns/pagination, zoom, stats, sync-drift,
  LIVE badge) + a slim bridge "⏪ Review footage" that hands off to Playback carrying the active
  camera. Playback owns the past (camera multi-select, event list click-to-jump, synchronized wall,
  and the SINGLE shared ctl-bar: timeline/transport/speed/window/calendar/sync-seek/LIVE-edge).
- **Single source of truth**: the ctl-bar markup + pb model + tlPaint/transport/calendar moved from
  Live into Playback (one calendar, one timeline, one transport; no duplicate ids). Guards split into
  applyLiveGuards (wall + bridge) and applyPlaybackGuards (recorded review). Honest gate unchanged:
  RECORDING_READY=false keeps recorded controls disabled with the recording-recon reason, now in the
  right page; export/snapshot/events guarded too.
- **Handoff**: Live→Playback (preselects active camera) and Playback→Live; deep-link #playback.
- Verified: node --check; no duplicate static ids; calGrid/syncBtn/ctl-bar each ×1; headless renders of
  Live (slim bridge, no timeline) and Playback (rail + events + synchronized wall + ctl-bar);
  click-through 11/11 (bridge present, no timeline in Live, guards, camera toggle, single calendar,
  back-to-live).

## v1.15 — 2026-11-16 (portal-unified) — Intercom: per-channel mute + full design audit
User approved per-channel mute and asked for a comprehensive intercom design audit. Found & fixed:
- **Per-channel mute + volume** (was one global flag leaking across channels): `chanMutes{}`/`chanVols{}`
  maps keyed by channel id; muting A no longer mutes B; volume remembered per channel. Added a
  "listen off" mic-off tag in the channel header AND on the board row so you can't forget it's muted.
- **Clear, not Mute**: the board's dispatcher action renamed Mute→**Clear** (tooltip: revoke floor +
  empty queue for everyone) to stop confusion with the local "Mute channel (listen off)".
- **Reason-guards everywhere** (v1.6 discipline): Grant/Revoke/Clear/Request/Release switched from
  native `disabled` to `aria-disabled` + hover/click reason tooltips; **Prio** is now a guarded
  placeholder ("not modeled yet") instead of a live no-op button.
- **WS-unknown safety hole closed**: previously PTT/Request/Grant/Clear could act while floor state
  was UNKNOWN, and the channel view falsely read "floor free". Now all floor actions are blocked with
  a reason on unknown, the channel shows "State unknown / connection lost — cached view", and
  drop/reconnect refreshes the open channel view (was board-only).
- **Space vs focused button**: Space on a focused button keeps native activation (no accidental PTT);
  PTT also releases on `pointercancel` (touch).
- Verified: node --check + DOM refs clean; headless renders of board (guards/Clear/mute-tag), muted
  channel (listen-off tag), unknown channel; click-through **11/11** assertions
  (mute/volume isolation, tag count, grant/prio/req guards, unknown view, PTT deny, Space-on-button).

## v1.14 — 2026-11-16 (portal-unified) — Cues wired: WebAudio reference implementation
User approved option A after the layer analysis (web vs app vs SDK): cues = local event tones;
mechanism belongs in the SDK long-term, this is the web-layer reference implementation / acceptance
sample. No external assets — oscillator envelopes only, stays offline.
- **Cue contract** `CUE_DEFS`: floor_permit (two rising blips), queue_ping, floor_yield (falling),
  denied (low buzz), alert (triple). Event hooks: Grant/PTT-open/hybrid-side → permit; PTT-release /
  Revoke / Mute / Release-floor → yield; Request-floor → ping; exclusive-floor-denied → denied;
  WS-drop → alert, reconnect → ping.
- **Policy split**: Intercom `Cues` toggle = local switch (preview ping on enable); topbar 🔔 = global
  master mute; effective audible = local AND master AND unlocked. Tooltips explain master-muted state.
- **Autoplay honesty**: first pointerdown/keydown unlocks the AudioContext; until then the bell title
  reads "Audio locked — click anywhere to enable" and `cueAudible()` gates playback (no silent fake).
- Verified: node --check + DOM refs clean; engine runtime smoke 9/9 (gating + blip counts per cue);
  headless click-through spy logged the exact event→cue sequence
  `floor_permit,floor_yield,denied,queue_ping,alert,queue_ping,queue_ping` with off/mute/locked-gate
  assertions true. Actual sound requires a real browser gesture (headless has no user activation).

## v1.13 — 2026-11-16 (portal-unified) — Intercom: enter-a-channel session (PTT / queue / members)
User review: the Intercom page was only a floor-control group list — no way to enter a channel and
interact (the footer even promised a hold-to-talk that did not exist). Ported the reference
(archive/port-web-shell.html) chan-rail + ptt-stage pattern into the unified dispatcher model:
- **Drill-in session view**: click a board row (or Enter key) to enter the channel; back button / Esc
  returns to the board. Header shows name/id/mode/listening count + live floor timer.
- **PTT stage**: current-transmitter avatar with pulse ring, animated wave, big Hold-to-talk button
  (ON AIR state). Hold-to-talk semantics: idle/queued → take floor while held, release yields;
  exclusive + held by other → denied with reason (Request floor / Grant instead); hybrid → transmit
  alongside. Space works as PTT while a session is open (inputs/combos excluded).
- **Participants**: per-channel member data; you (dispatch.x) marked; speaking row highlighted;
  per-member local mute (mic/mic-off); muted reads as off-glyph.
- **Floor side-card**: holder + queue (+ "you in queue"), Request floor / Release floor / Mute channel
  (listen off), listen-volume slider, honest-state note (simulated audio, dispatcher arbitration).
- Live tick updates timer/holder/queue without re-rendering (slider drags survive).
- Fixes found during verification: back-arrow SVG had no size constraint (rendered huge) → sized;
  `#chanView{display:flex}` overrode the `hidden` attribute (channel stayed visible after Esc) →
  `#chanView[hidden]{display:none}` guard (same class as the v1.5 cal-pop fix).
- Verified: node --check + DOM refs clean; Firefox-headless renders of board / idle channel / ON AIR;
  click-through `hold:true | rel:true | deny:true | q:true | esc:true` (esc asserts computed
  display:none); board Q reflects the queued request.

## v1.12 — 2026-11-13 (portal-unified) — picker: drop fake preview + static res; align checkbox column
User review raised three questions; analysed and resolved:
- **Right-column preview image — not necessary, removed.** The thumbnail was a synthetic gradient
  (`feedBg`), not a real frame; it added width/noise and pushed names into truncation. Selected rows now
  read index + full name + live state + reorder/remove, which is what the shuttle actually needs.
- **Static resolution chip — not necessary, removed.** Resolution switches dynamically at runtime
  (weak-link / stream-mode presets), so a fixed chip on a picker item is stale/misleading. Removed from
  both channel rows and selected rows; live state glyph remains as the only per-item status.
- **Expanded left-column checkboxes misaligned — fixed.** Device header checkbox sat at pad8+twisty16+gap8
  = 32px while channel checkboxes sat at margin16+pad12 = 28px, so expanding shifted the checkbox column.
  Channel rows now use `padding-left:32px` (no margin) putting checkbox AND name on the exact same vertical
  axis as the header; the tree guide runs under the twisty centre (x=16).
- Verified: Firefox-headless expanded render shows device/channel checkboxes and names on one axis and
  clean right rows; click-through test `init8|ch7|dev5|rm4|mv-ok|apply4 closedtrue` still passes; JS/lint clean.

## v1.11 — 2026-11-13 (portal-unified) — select-streams dialog unified (items, sizes, styles)
User review: the select-streams shuttle looked uncoordinated — mixed item sizes/styles. Root causes:
three different row rhythms (device header / channel / selected), native `accent-color` checkboxes,
text glyphs (▸ ⇄ ↑ ↓ −) instead of the icon system, and a width bug (`.modal{width:680px}` overrode
`.pick-modal{width:860px}` at equal specificity, squeezing columns and truncating names to "VH-…").
- **One row rhythm**: device headers, channel rows and selected rows all share a 34px min-height,
  identical padding/radius/hover; consistent typography (mono ids/res, fs13 names, fs11 meta).
- **One checkbox**: palette-drawn custom tri-state `.ck` (checked / indeterminate / focus ring) replaces
  native boxes; selected channels get an accent-tinted row.
- **One icon set**: SVG chevron (expand), swap (shuttle), up/down/minus (reorder/remove), search/grid
  (empty-state discs) replace all text glyphs; 24px icon action buttons match the design system.
- **Richer, aligned items**: device headers show selected/total count + state glyph; channels sit under a
  tree guide with res chip + state glyph; selected rows show index + thumbnail + full name + res + state.
- **Width fix**: `.modal.pick-modal` specificity raises the picker to 860px so names are fully readable;
  ≤760px stacks the two columns vertically with a rotated swap icon.
- Verified: Firefox-headless desktop + narrow renders; real click-through test
  (channel toggle → device select-all → remove → apply) logged `init8|ch7|dev5|rm4|apply4 closedtrue`
  and the wall/rail/badges updated to 4 streams; JS/lint clean.

## v1.10 — 2026-11-13 (portal-unified) — smart prototype-controls popover (never off-screen)
User review: the Prototype controls panel opened/cl closed unintelligently and sometimes rendered beyond
the visible range (bottom-anchored tall panel on short viewports; after dragging, opening overflowed).
- **Viewport-aware placement**: `placeDevtools()` clamps the whole devtools box inside the viewport on
  open, after drag, and on resize; `#dtPanel` gains `max-height:calc(100vh - 24px)` + internal themed
  scroll so even a very short viewport shows a usable, fully-on-screen panel.
- **Smart close**: click-outside closes; Escape closes; on close an un-dragged panel restores its
  bottom-left anchor (dragged panels keep their clamped position). Drag end re-clamps.
- Verified: Firefox-headless at 620px (panel fits + internal scroll), drag-to-edge-then-open (panel
  clamped fully into view), and outside-click (closes, pill returns to anchor); JS/lint clean.

## v1.9 — 2026-11-13 (portal-unified) — version badge demoted to opt-in; meta relocated
User review: the always-on floating "MediaServo …" badge obstructed content and read as debug noise.
Analysis: it is prototype *meta* (version / baseline / not-app-code), not product UI; mainstream practice
(Storybook about/toolbar, Figma dev-mode chrome, build-info in footer/About, `?debug` overlays) keeps such
meta out of the canvas and behind a toggle. Implemented accordingly:
- **Badge hidden by default** (`#proto-bar{display:none}`), revealed only via Prototype controls →
  "Version badge: Hidden / Floating"; when shown it remains a draggable watermark. Reset hides it.
- **Meta relocated to proper homes**: muted sidebar footer ("Prototype · v1.9 · not app code") and the
  controls-panel footer (full "MediaServo unified portal · v1.9 · single iteration baseline · design asset
  only"), plus `document.title`. No permanent overlay over product UI.
- Verified: Firefox-headless default view has a clean bottom-right (canvas-foot unobstructed); panel shows
  the Version badge row + meta footer; toggling Floating shows the draggable watermark.

## v1.8 — 2026-11-13 (portal-unified) — unified palette-driven scrollbars (both engines)
User review: every scrollbar thumb looked wrong. Root causes: the old rules were **WebKit-only**
(Firefox fell back to wide OS-default bars), and the track + thumb inset border were painted with
`--bg` (page canvas), producing a mismatched stripe/outline on every `--surface`/`--raised` container.
- **Cross-browser**: added Firefox `scrollbar-width:thin` + `scrollbar-color` (inherited from `html`),
  alongside the WebKit `::-webkit-scrollbar*` pseudo-elements, so both engines render the same language.
- **Palette-correct**: track and corner are `transparent` (inherit each surface); the thumb is inset with
  a `transparent` border + `background-clip:padding-box` so the gap shows the container's own colour in
  either theme; thumb `--bd2`, hover `--accent`, active `--blue-hi`; rounded pill; 10px gutter.
- **Hover** accent also wired for Firefox via a grouped `:hover{scrollbar-color}` over all scroll regions
  (nav, rail, wall, page body, tables, picker lists, panel bodies, calendar, modal).
- Verified: Firefox-headless renders at short viewport in dark + light show thin rounded palette-matched
  thumbs with transparent tracks on nav / page body / devices table / picker selected-list; no OS default.

## v1.7 — 2026-11-13 (portal-unified) — Prototype controls panel + draggable floating boxes
User request: bring the `port-web-shell.html` "Prototype controls" capability into the unified portal,
and allow dragging the floating boxes / controls.
- **Prototype controls** (`#devtools`, ported pattern): a floating panel with live state toggles —
  Theme, Density, Viewport (desktop/rail/mobile), WebSocket (live/dropped), Content state
  (data/empty/loading/error), Region labels (A1–A7 overlay), Stats overlay, Sync — plus Reset.
  Each toggle applies immediately and the panel's `is-on` states stay synced with the app
  (`syncDevtools`), including changes made elsewhere (topbar theme, settings density, wall toggles).
- **Real state effects**: loading shows a shimmer skeleton grid; empty shows the centred prompt;
  error raises the stale banner + dims tiles; mobile collapses nav + rail to a single column;
  rail folds the app nav; region labels draw dashed outlines + A1–A7 tags.
- **Draggable floats**: generic `makeDraggable` (pointer capture, viewport clamping, drag-vs-click
  discrimination) applied to the controls panel (via its header), its launcher pill, and the
  `#proto-bar` badge — all can be moved and stay on-screen.
- Verified: node --check; DOM refs zero missing; Firefox-headless renders of panel-open, labels,
  loading, mobile (single-column fix), and programmatic drag of panel + badge all correct.

## v1.6 — 2026-11-13 (portal-unified) — Live state-disabled guards with reason tooltips
User request: Live view lacked a state-disabled mechanism (controls disabled in some states with a
reason on hover/click). Added a unified guard system:
- `aria-disabled` + `.is-disabled` + `data-reason` (not native `disabled`, so hover AND click can
  explain); shared `#guardTip` tooltip; capture-phase click interceptor; functional bail-guards.
- Honest capability gate `RECORDING_READY=false` (recording-recon): playback/seek/trick-play disabled;
  live-edge transport disabled; no-streams / zoomed / first-last-page / no-signal reasons per control.
- Empty-state fix: hide only `#tiles` (not `#wallScroll`) so the centred prompt shows; wall-scroll flex column.
- Verified via node --check + Firefox-headless renders (populated/empty/zoomed/stats) with tooltips.

## v1.5 — 2026-11-13 (portal-unified) — Live transport alignment + smart calendar popover
User review: (a) the timeline and rewind/fast-forward buttons looked vertically staggered; (b) the
date/calendar popover was clipped / covered (no smart placement).
- **Transport alignment**: `.ctl-bot` is now a two-row grid — row 1 holds the timeline track **and** the
  play buttons centred on the same line (buttons sized to the 34px track), row 2 holds the time scale
  under the track only. Removed the old `.tl-wrap` flex column that centred buttons against the taller
  track+scale block (the cause of the stagger).
- **Smart calendar**: `.cal-pop` switched to `position:fixed` (escapes the canvas `overflow:hidden` clip)
  with a `placeCal()` that prefers below the date button, **flips above** when there is no room (the bar
  sits at the canvas bottom), and clamps horizontally inside the viewport; repositions on open, month
  nav, day pick, resize and scroll; `max-height` guard for short viewports; clicking inside no longer
  self-closes it.
- Verified: Firefox-headless renders at 900px and 760px show buttons aligned with the track and the
  calendar fully visible above the bar (never clipped); JS `node --check` OK.

## v1.4 — 2026-11-13 (portal-unified) — column-count combobox polish
User review: the "N per row" dropdown displayed poorly. Reworked to reference (port-web-shell) quality:
- **Check only on the selected row** — previously every option rendered a visible ✓; now a fixed
  right-hand check column (`i-check` SVG) is hidden unless `aria-selected`, so labels stay aligned.
- **Open-state trigger** (accent border + surface bg) and **rotating caret**; `aria-expanded` kept in sync.
- **Right-anchored pop** (`right:0`) so it never overflows the canvas; taller aligned rows, selected row
  gets an accent tint; hover + keyboard-active states.
- **Keyboard support**: ArrowUp/Down move the active option, Enter/Space select, Home/End jump, Esc closes;
  focus returns to the trigger after choosing. Outside-click closes and resets `aria-expanded`.
- Verified: Firefox-headless renders of the open list (single check on selected) and of choosing
  "4 per row" (grid reflows to 4 cols, combo closes, toast) both correct; JS `node --check` OK.

## v1.3 — 2026-11-13 (portal-unified) — Live rail-collapse no longer blanks the wall
User review: the `⟨` button at the top-left of the Live grid made the entire multi-grid invisible.
- **Root cause**: `#live-layout` is a 2-column grid with auto-placed children. Collapsing sets
  `.stream-rail{display:none}`, which drops it from auto-placement and slides `.canvas` into column 1 —
  the `0`-width track — blanking the whole wall (same bug class as the v1.2 bottom gap).
- **Fix**: pinned explicit tracks (`.stream-rail{grid-column:1}`, `.canvas{grid-column:2;min-width:0}`),
  so hiding the rail keeps the canvas in the `1fr` track. Also fixes the ≤900px media query that hides the rail.
- Verified: Firefox-headless renders of collapsed (wall full-width, toggle `⟩`) and expanded (rail + wall,
  toggle `⟨`) both correct; JS `node --check` OK.

## v1.2 — 2026-11-13 (portal-unified) — full-viewport fill (bottom blank fix)
User review: the app did not fill the browser; a blank strip remained at the bottom.
- **Root cause**: `#app` is a 3-row grid (`top-h auto 1fr`). `#wsBanner` is `display:none` by default, so
  grid auto-placement skipped it and dropped `#body` into the `auto` track (row 2), leaving the `1fr`
  track (row 3) empty → an unstyled gap below the frame at tall viewports.
- **Fix**: pinned explicit rows (`#topbar:1`, `#wsBanner:2`, `#body:3`) + `min-height:0` on `#body`, and a
  `100dvh` fallback on `#app` for mobile dynamic toolbars. The frame now reaches the viewport bottom on
  every page and height; the Live wall's scroll region absorbs extra height.
- Verified: layout metrics (`#body`/`#sidebar`/`.canvas-foot`/`.sb-foot` all `bottom == innerHeight`) and
  Firefox-headless renders at 760/1100 px on Live + Overview show edge-to-edge fill, no gap, no clipping.

## v1.1 — 2026-11-13 (portal-unified) — Live tile zoom / stats fix (reference: port-web-shell)
User review found two defects in the Live wall; both fixed by adopting the proven approach from
`port-web-shell.html` (workbudy port), keeping the unified P8 token system:
- **Bug 1 — zoom hid the top-right controls.** The old `.tile-stats` was a full-tile opaque overlay
  painted *above* `.tile-actions`, so after zoom the close/shrink buttons disappeared. Now the name bar
  moves to the **top**, actions sit top-right at `z-index:5` (revealed on hover **and while zoomed**,
  larger when zoomed), and the bar yields right padding so they never overlap.
- **Bug 2 — stats broke across grid sizes / zoom.** Stats is now a **compact floating card**
  (`left:8px;top:30px`, `pointer-events:none`) that never covers the video or the controls, with
  **container-query density** on the tile's own width: `<160px` hidden · `160–229px` two compact rows ·
  `≥230px`/zoomed full readout + LIVE head. Fixed-width value columns (`auto 40px auto 40px`) keep numbers
  aligned; bitrate compacted (`2.1 Mbps→2.1M`).
- **Icons**: added an SVG sprite (`i-zoom` expand / `i-zoom-out` contract / `i-close` X / `i-info`),
  matching the reference geometry; the zoom button swaps expand↔contract on toggle.
- Verified: inline JS `node --check` OK; Firefox-headless renders of 1/3/4-col + zoomed (stats on/off)
  confirm controls stay visible and stats adapt without clipping.

## v1.0 — 2026-11-13 (portal-unified) — **THE single iteration baseline**
User directive: the prototypes were fragmented & conflicting (nav / style / page content / operations).
Built **`portal-unified.html`** — one single page integrating everything, the only artifact to iterate on:
- **Unified navigation taxonomy** reconciling the new-design capability groups with workbudy's features:
  Overview / Surveillance(Live,Playback) / Communication(Intercom,Meeting) / Control(Remote,Tower) /
  Administration(Devices,Rooms,Sessions,Streams,Config,Keys,Audit,Settings,Signaling) — **16 pages**.
- **One design system**: P8 tokens, dual theme + density, control-room palette, state grammar (glyph+hue),
  custom combobox, styled scrollbars, toast, WS-drop → floor UNKNOWN + stale banner.
- **Working interactions**: column-driven + paginated Live wall, select-streams picker, zoom, floor
  grant/revoke/mute/prio, device approve/revoke + search/filter/sort, live tables, theme/density, meeting/key CRUD.
- Verified: inline JS node --check OK; 84 ids / 57 refs zero missing; tags balanced; offline; HTTP 200;
  **runtime logic smoke 12/12** (pagination/clamp/counts/floor state machine/re-render).
- **Supersedes for iteration**: `w1-portal-shell`, `w2-live-wall`, `port-web-shell`, `port-admin-console`
  remain as reference/history only — do not iterate on them. Board doc: `boards/portal-unified.md`.

## 2026-11-13 — workbudy H5 prototype port (user: 尽量搬迁/移植)
Ported workbudy's two complete MediaServo consoles into `prototypes/` (they already use our P8 `--ms-`
tokens and MediaServo branding, so the port is faithful, not a rewrite):
- **`port-web-shell.html`** ← workbudy `w1-web-shell.html` (213 KB): the full **14-page operations console**
  (overview / view / intercom / meetings / remote / devices / rooms / sessions / streams / config / audit /
  settings / signaling / keys). Covers the pages our focused boards have not built yet.
- **`port-admin-console.html`** ← workbudy `w2-admin-console.html` (65 KB): the **rooms & fleet admin console**
  (rooms / sessions / devices / streams / fleet + config push / client keys / audit / settings / weaknet).
- Adaptations: `Cockpit → Tower` (8 refs total, web-portal-ui **P5**); titles marked "(workbudy W1/W2 port)"
  to avoid collision with our focused `w1-portal-shell` / `w2-live-wall` boards. `Teleoperation` kept (domain term).
- Both fully offline (no CDN/font/image), inline JS passes `node --check`, HTTP 200. Board docs:
  `boards/port-web-shell.md`, `boards/port-admin-console.md`.
- These are **reference-complete ports**, distinct from the per-board review artifacts (P9 loop).

## v0.2 — 2026-11-13 (w2-live-wall)
Fix (user review): the leftmost **portal navigation** was missing in v0.1 — restored it.
- Layout is now the full three-region order: **portal nav (232px) → selected-streams rail (250px) → canvas**,
  matching W1's grouped nav (Surveillance / Communication / Control / Administration).
- Nav: Live active, collapsible to a 56px icon rail, Live badge = live selected-stream count,
  keyboard-reachable items; other apps toast their target board (per-board loop scope).
- v0.1 archived (`archive/w2-live-wall-v0.1.html`); board sidecar gains region A0 (nav).
- Verified: both scripts node --check; DOM refs zero missing; div/nav/aside tags balanced; HTTP 200.

## v0.1 — 2026-11-13 (w2-live-wall)
New board W2 (Live wall), per web-portal-ui P9 per-board loop. Borrows workbudy Live mechanisms
(provenance in `boards/w2-live-wall.md`) and implements the W2 hard requirement:
- **Column-driven layout + pagination** (NOT a fixed rows×cols matrix): "N per row" combobox (1–4)
  drives `--cols`; page capacity = cols × 2 rows; `‹ 1/2 ›` pager; stream count uncapped, adding
  streams grows pages. Changing columns re-flows + re-paginates.
- **Device→stream selection** modal (device-grouped, search, add) + **selected-streams rail**
  (idx/thumb/name/res/state-glyph/remove).
- **Per-tile stats overlay** (FPS/RES/BIT/JIT/RTT/LOSS/CODEC/DRIFT) via Stats toggle; **Sync** gates
  per-tile drift; tile zoom = full stats card.
- stage-bar (rail toggle + counts + Sync + Stats + columns + pager), state grammar (dot/3-seg bar/
  hollow-ring), stale WS banner, empty state. Dual theme, custom combobox (no native select).
- Verified: both scripts pass node --check; 44 DOM ids, zero missing refs; pagination math tested
  (3 cols → 2 pages [6,2]); HTTP 200.

## v0.2 — 2026-11-12 (w1-portal-shell)
Rework per web-portal-ui **P8** (workbudy transplant: borrow patterns, change boundaries).
Fixes the 4 self-violations the expert panel caught in v0.1, and lands the borrowed design rules:
- **3-layer tokens** (primitive → semantic → component) + **dual theme** (`data-theme` light/dark,
  no-flash resolve, topbar toggle ◐). Admin light + dark both first-class; video letterbox dark in both.
- **State grammar**: no state by hue alone — solid dot (held/online), 3-seg bar + rtt/loss numbers
  (degraded/queued), hollow-ring-slash (offline/signal-lost), triangle (warning/unknown). Added a
  degraded tile (CAM-03, link 2/3) and kept the offline tile (CAM-04).
- **Removed all infinite animation** (v0.1 pulsed live/floor lamps) → static indicators; transitions ≤150ms.
- **Video tiles radius 0** (was 10px); **weights 400/600 only** (was 500/700); system font stack;
  tabular-nums slashed-zero on timers/counters; space/radius from token scale.
- Role badge → neutral chip (no hue); combobox/segmented controls (no native `<select>`).
- Screens unchanged in scope: Live wall / Intercom board / Tower / Settings; WS-drop → floor UNKNOWN demo kept.

## v0.1 — 2026-11-12 (archived: archive/w1-portal-shell-v0.1.html)
First exploration draft: shell + 4 screens, control-room night palette, full interactivity.
Superseded by v0.2 (violated the borrowed rules: infinite animation, tile radius, weight 500/700, dark-only).
