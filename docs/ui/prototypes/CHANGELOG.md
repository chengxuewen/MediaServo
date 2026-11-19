# Prototypes CHANGELOG

## v1.85 — 2026-11-19 (portal-unified) — M-6 polish: meeting live-surface gaps closed (4-seat panel + isolated UX self-pass)
Panel verdicts: correctness NOT READY / evidence FAIL / consistency REJECT; UX seat quota-failed →
isolated sequential self-pass substituted (disclosed). Objective fixes (no new product rulings):
- **Speaker layout REAL render** (was variable+toast fake): main speaking tile + right rail (cap 4 +
  "+N more"), off-page speaker indicator in pager; grid/speaker naming de-collided with monitor §2i
  (titles/aria "Grid / Speaker"); §3k note: meet canvas layout family independent of §2i wall layouts.
- **Panel lifecycle**: meetLeave closes panel (cross-room stale-roster closure hazard removed);
  join re-renders open panel; body.meet-panel-open shifts PiP left (wide) / hides PiP + raises control
  bar z-index (narrow) so Leave stays reachable under full-width panel.
- **self-PiP content linkage**: cam-on simulated feed + mirrored name; cam-off avatar initials;
  mute badge; narrow position asserted.
- **Tile overlays per workbudy 03 §6 / 01 §7**: conn ok dot / weak 3-seg bar / lost hollow-ring+slash
  "no signal" (shape+text, not hue alone); OSD mute glyph + hand chip; tiles role=group + aria-label.
- **Meeting context never-hide row**: name+type chip, elapsed clock, in-room count (W03:87 identity).
- **Guest ended perception**: live-phase detects room ended → "Meeting ended by host" card + Back;
  host-leave toast audience fixed (single broadcast, not per-waiting toast on host screen).
- **Reopen resets** parts=[host self] + chat archived to transcript (transcript modal reads archive).
- **Speaking**: speakerTick scheduled (1.5s while live) + pool = unmuted & not-lost (voice meetings
  get speaking indication); sp compared by name (object-compare bug fixed).
- **Chat unread badge** on mkChat when panel closed/other tab; cleared on open-chat.
- **Panel tabs a11y**: role=tablist/tab/aria-selected/aria-controls + ArrowLeft/Right roving.
- **Hold arbitration**: starting Esc-hold cancels pointer-hold and vice versa (single progress owner).
- **Deep link**: #meet-<id> hash consumed on load → lobby + prejoin (invite link now functional).
- Control-bar titles added; narrow bar wrap + top chips wrap.
- Harness: meet-polish (17 assertions) + meet-polish-narrow (leave reachable under panel/PiP hidden/
  no overflow); meet-live layout assertion upgraded to REAL speaker render; NEG targets registered
  renderMeetLive→speakerTick(rotation assertion makes noop red)/openMeetPanel(narrow); 35 cases ALL
  GREEN + NEG all red; node --check + lint clean; stamps x5; screenshots m6-speaker/m6-narrow-panel.
- Process defects disclosed & fixed: M-6 edit span deleted panel five functions (restored);
  mpTab/mpTrigger declarations lost with them (restored); rail null guard; sp object-vs-name;
  mainP object; off-page interval formula; template-literal \d eaten (clock regex); badge refresh.

## v1.84 — 2026-11-19 (portal-unified) — fix: meet exit perceived-stuck (user report "cannot leave room")
Analysis first (evidence): both exit chains verified working under REAL timers (harness multi-part
450ms probes: pointer-hold prog 0.45→0.9→complete→lobby; Esc-hold same). Not a logic dead-end —
four perception gaps fixed:
1. **Esc/keyboard hold had no visible progress** → new #holdBar (top-center "Hold… N%" + fill track)
   shown by holdStart/escHoldStart, updated by holdTick/escHoldTick, hidden on cancel/complete.
2. **Silent cancels** → pointerleave AND new pointercancel handler now toast "Hold cancelled —
   keep pressed 1s" (touch/system-gesture interrupts no longer feel broken).
3. **No persistent exit guidance in zero-nav canvas** → #meetExitHint chip top-right
   "Exit: hold button 1s · hold Esc 1s", added to H-hide exemption list.
4. Keyboard hold shares the same visible bar (unified hold progress UI).
- Harness: probe promoted to `meet-exit` 10-part real-timer case (join/cancelFeedback/guidance/
  barVisible/hold progress booleans/holdCompletes/rejoin-via-Reopen/escBar/esc progress/escCompletes/
  barHidden/hintExempt); NEG holdTick noop red. Rejoin path fixed (host leave → ended → Reopen first).
- Infra: harness process.on('exit') kills spawned firefox (stale-test-firefox accumulation after
  failed runs was the recurring ECONNREFUSED 2828 root; two precise kills approved this session).
- Verified: 33 cases ALL GREEN + NEG all red + zero stale firefox; node --check + lint clean;
  stamps x5; screenshot mx-bar shows Hold…45% bar + exit guidance chip + leave ring.

## v1.83 — 2026-11-19 (portal-unified) — fix: direct-content modals had zero padding (user-reported prejoin anomaly)
User report: Meeting join dialog content edge-to-edge / clipped look at 844x682.
- Root cause: `.modal` base rule carries NO padding (padding lives in `.modal-head/.modal-body`
  wrappers used by older modals); the eight direct-content modals (acc/psk/key/cred/meet/prejoin/
  meetTrans/secret) placed .card-title/.dd-row/preview blocks directly → zero inset, rows touching
  edges, overflow:hidden clipping risk at short viewports.
- Fix: `.modal.pad{padding:var(--sp16);gap:var(--sp8);overflow:auto}` + class applied to the eight
  direct-content modals; wrapper-based modals (grp/picker/cfm) untouched.
- Verified: 844x682 prejoin screenshot shows proper inset + centered + no clipping; 1440 create
  modal re-shot; harness 32 cases ALL GREEN + NEG all red; node --check + lint clean; stamps x5.

## v1.82 — 2026-11-19 (portal-unified) — Meeting round M-5 delivered: B-M1 round COMPLETE (pending final review)
Implements web-portal-ui §3k B-M1 task M-5 (three states + honesty sweep + regression + closure).
- **Error state**: devtools "Meeting conn · Simulate error" leg; live/connecting → error phase card
  "Meeting connection failed (simulated)" + Retry (→connecting→live/waiting) + Back to lobby
  (shell restored + focus to New meeting); connecting timer honors meetError.
- **Ended transcript**: ended cards get Transcript button → read-only modal (chat list or
  "No messages were sent."), dlgKey Esc/focus contract; no input (read-only asserted).
- **Shell/focus restore**: meetLeave and error-Back restore shell + focus New meeting.
- **Honesty sweep**: host controls labelled "(simulated)"; prejoin video note states backend
  consume-only for video meet → self media simulated (implementation alignment open item).
- **Regression**: nav 8 / hub 10 / admin cases unchanged; meet four cases (lobby/prejoin/live/host)
  + meet-state = 32 cases ALL GREEN + NEG all red; W1 zero-nav / mute chip / avatar non-black /
  partsCount assertions retained.
- Verified: node --check + lint clean; stamps x5 (C68); screenshots error/transcript (+ prior
  lobby/create/prejoin/voice/waiting/live/speaker/roster/chat/narrow set).
- **Round B-M1 complete**: M-1 v1.78 / M-2 v1.79 / M-3 v1.80 / M-4 v1.81 / M-5 v1.82.
  Open items (not swallowed): backend meeting-host field, host handover, rec/share/captions/breakout/
  reactions (recording-recon), waiting backendization, real getUserMedia prejoin, consume-only video
  alignment, meet implementation vehicle (admin-embedded vs standalone app), hand auto-lower timeout,
  invite link form, speaker rail cap / >9 paging vs scroll / off-page speaker indicator.

## v1.81 — 2026-11-19 (portal-unified) — Meeting round M-4 delivered (pending review): roster + chat + host controls
Implements web-portal-ui §3k B-M1 task M-4.
- **Side panel** (right, full-width @760): Participants/Chat tabs + close; roster = self row top +
  host badge + mute/cam/hand icons + Remove (host only, excludes host/self rows, confirmDlg
  consequence + audit meet.remove); waiting rows Admit/Deny (audit meet.admit/deny);
  host controls Mute all (excludes self, self can unmute) + Lock/Unlock (lobby Join/Start + prejoin
  Join guarded while locked, audit meet.lock/unlock).
- **Non-host**: all host controls setGuard + reason "Host only." (keyboard discoverable).
- **Waiting arrival**: cue + toast announcer + roster-key red badge count (syncMpBadge).
- **Chat**: local simulated list + empty "No messages yet." + empty-input guard toast + Enter/send +
  focus kept on input after send.
- **Hand**: guest control-bar key raise/lower with toast feedback (auto-lower timeout = open micro).
- **Panel focus/Esc contract**: initial focus (roster first button / chat input), restore trigger on
  close; Esc closes panel NOT meeting (Esc chain: modal → tb-pop → meet panel → hold-Esc → cascade).
- Verified: harness 31 cases ALL GREEN (meet-host 19 assertions: panelOpen/hostBadge/badge/
  waitingAdmit/waitingDeny/muteAllExcludesSelf/lockGuardsJoin/lobbyLocked/unlockOk/removeConfirm/
  removed/noSelfRemove/nonHostGuard/chatEmptyState/chatEmptyInputGuard/chatSend/handRaise/handLower/
  panelEscKeepsMeeting) + NEG all red (renderRoster noop); node --check + lint clean; stamps x5;
  screenshots roster/chat/narrow.

## v1.80 — 2026-11-19 (portal-unified) — Meeting round M-3 delivered (pending review): zero-nav live canvas
Implements web-portal-ui §3k B-M1 task M-3 (workbudy 03-client-ia §5.3 hard requirements).
- **Zero-nav canvas**: body.meet-live hides sidebar/topbar/deg-banner/devtools; #meetCanvas fixed
  full-screen; shell hidden ONLY in live phase (connecting keeps shell).
- **Adaptive tiles 1/2/4/9** (g1/g2/g3 by count, >9 paging + pager); audio-only/cam-off = neutral
  avatar tile + name (never black frame); cam-on = simulated feed + name OSD.
- **Speaking = 2px accent border, fill unchanged**; speakerTick() direct-callable for deterministic
  assertions (simulated rotation).
- **Self-PiP 1/6 width bottom-right mirrored** (label counter-mirrored); narrow @760 moves top-right;
  never covers leave (bottom:64 above bar).
- **Mute persistent chip** (icon+text Muted/Live mic) independent of #toast (single announcer kept).
- **Control bar**: mic / cam (voice guarded) / layout gallery↔speaker (page+panel state kept) /
  roster+chat (honest interim gate → M-4) / hand (guest only, raise/lower toast) / **Hold to leave**
  (1s progress ring, short-press toast, pointerleave cancel, keyboard Space/Enter hold + keyup cancel).
- **Esc chain meet layer**: short Esc = transient "Hold Esc 1s to leave" hint; Esc hold 1s = leave
  (e.repeat ignored, keyup cancels); inserted after modal/tb-pop, before drawer cascade.
- **H hides overlays** with exemption list: mute chip + leave + simulated chip never hide.
- **pointer:coarse → 44px** control-bar registration (no 30px-family exemption).
- **estop slot empty** in meet canvas (workbudy M2 collision-legal precondition).
- Fixes en route: v1.77 repair residue `id="statusPop class=` missing quote + secretScrim missing
  closing </div> nested canvas inside hidden modal (rect 0); chip svg.ic unsized giant-icon recurrence
  (#meetTop .ic 12px); leave label nowrap; PiP label counter-mirror; devtools hidden in live.
- Verified: harness 30 cases ALL GREEN (meet-live 15 assertions incl holdTick/escHoldTick direct-call,
  speaking fill-unchanged re-query, coarse via style-text) + NEG all red (holdTick noop = completion-path sole tick); node --check + lint clean; stamps x5; screenshots live/speaker/narrow.

## v1.79 — 2026-11-19 (portal-unified) — Meeting round M-2 delivered (pending review): prejoin + waiting guest side
Implements web-portal-ui §3k B-M1 task M-2 (plan v2; state machine v2 no-dead-end).
- **Prejoin modal**: name (default CUR_USER, explicit initial focus), simulated device check
  (mic ok / camera ok or n/a·voice / network warn·simulated, non-blocking), mic+cam preview blocks;
  voice rooms replace cam preview with "Voice conference — no camera" + honesty note (M2=A).
- **Join path**: locked → Join guarded both lobby and prejoin ("Meeting locked by host");
  join → connecting 300ms simulated WITH SHELL KEPT → host direct live / guest waiting;
  waiting guest screen "Waiting for host to admit you…" + Cancel (removes self from queue);
  admitWaiting/denyWaiting logic functions (host UI wiring = M-4); deny → guest back to lobby +
  feedback toast; audits meet.join / meet.join.request / meet.admit / meet.deny / meet.leave.
- **Interim live surface**: phase card "In meeting (simulated). Full canvas lands in M-3" + Leave
  (host leave → ended + endReason + waiting cleared; guest leave → parts removal); shell hide +
  canvas = M-3 per approved sequence (explicit interim, not fake).
- Esc = cancel prejoin (dlgKey); focus restore to trigger (dlgClose stack); detached-trigger safe.
- Verified: harness 29 cases ALL GREEN (meet-prejoin 5-part 11 assertions: nameFocus/voiceCamGuard/
  cancelRestores/escCancel/lockedGuard/lockedJoinGuard/connectingShellKept/hostLive/
  waitingGuestScreen/guestAdmit/denyFeedback) + NEG all red; meet-lobby joinGate assertion upgraded
  to joinOpensPrejoin (M-1 interim gate retired); floor NEG retargeted autoGrant (state-prep target
  was flaky: floorTick queue simulation refilled cleared queue); node --check + lint clean;
  stamps x5; screenshots prejoin/voice/waiting/narrow.
- Harness flake notes: PIT-265 focus-before-click for cancelRestores; stale test firefox reaped
  (approved precise kill) to free Marionette 2828; EADDRINUSE from detached watch-run resolved by
  waiting for completion.

## v1.78 — 2026-11-19 (portal-unified) — Meeting round M-1 delivered (pending review): lobby + create
Implements web-portal-ui §3k B-M1 task M-1 (M1=B/M2=A/M3=A/M4=B adjudicated; plan v2 post 4-seat review).
- **Lobby cards**: type chip Voice/Video (backend video flag surfaced per M2), partsCount, state,
  host line; ended rooms show endReason; host sees Reopen (→idle + audit meet.reopen), guests see
  Ended chip; empty state "No meetings yet — create one."
- **Create modal**: name required + Voice/Video radiogroup (roving tabindex + arrows) + honesty hint;
  create → room {video, host=CUR_USER, parts[self], waiting[], locked, chat[], state idle} + audit
  meet.create; focus initial=name, restore=newMeet after repaint.
- **Invite = real copy**: copyWithFallback(v,hintEl,onOk) with done-dedupe; fallbackCopy hint target
  parameterized (fixes hardcoded #secCopyHint + double-toast defect); lobby #meetCopyHint shows
  copied/manual-select feedback; secret flow reuses same helper.
- **partsCount()** unifies meet object-parts vs talk/view numeric parts (Rooms page no [object Object]).
- **Join/Start honest interim gate**: setGuard + reason "Prejoin surface lands in M-2" (no fake toast);
  prejoin/waiting/canvas land in M-2/M-3 per approved sequence.
- Seeds: meet-standup (video, live, host dispatch.x, 5 parts), meet-review (voice, ended, host admin.x,
  endReason "Host left the meeting", chat transcript seed).
- Verified: harness 28 cases ALL GREEN (meet-lobby 12 assertions) + NEG all red (renderMeeting noop);
  node --check + lint clean; stamps x5 (C68: About toast carries version since v1.77); screenshots
  lobby/create/narrow.

## v1.77 — 2026-11-18 (portal-unified) — topbar convergence (option A, user-adjudicated after research)
Research: B-end topbar doctrine = left global info / right global actions; excess elements collapse
into menus/popovers (DSM taskbar status cluster; ENVY mobile hides account name into menu).
Measured clutter: right cluster 5 items/512px (conn 148 + theme 30 + mute 30 + mode 101 + user 203);
crumb duplicated page-head title+subtitle verbatim; brand PORTAL tag redundant.
- **Right cluster = 3**: bell (badge count, was warn-chip text), status chip (dot + SA/PL) opening
  #statusPop (connection sim chip + provider mode chip + break-glass shortcut when degraded),
  user menu (avatar+name+chevron) opening #userPop (role chip / Theme toggle with current state /
  Cues mute with state / About / Sign out simulated).
- theme/mute buttons MOVED into user menu (ids+handlers preserved); conn/mode chips MOVED into
  status pop; crumb = path only (subtitle removed, page head owns it); brand PORTAL removed.
- Popover contract: aria-haspopup/expanded/controls, initial focus, Esc closes + restores trigger
  (global Esc chain: modal → tb-pop → drawer → cascade), outside-click closes, Tab loop inside.
- syncModeInd drives status chip dot/label + pop break-glass shortcut; setWsState drives dot;
  setTheme/syncCueUI label their menu items; bell aria-live REMOVED (single announcer = toast).
- Narrow @600: user name hidden (avatar only), cluster intact, docSW==600 (converge, not hide).
- **Verification limitation (honest)**: Marionette headless session wedges unreproducibly on the
  topbar interaction case (reads-only also wedged in some sessions; solo runs passed) — topbar is
  therefore verified PAGE-SIDE (injected assertion overlay: cluster/crumb/brand/statusPop/statusEsc/
  userMenu/themeMenu/muteMenu/userEsc/degDot/bgShortcut/bgGoto ALL true + screenshots 1440/600),
  NOT in the Marionette suite. Suite keeps 27 cases ALL GREEN + NEG all red; harness run() gained
  25s ExecuteScript timeout + single re-navigate retry; accounts-narrow NEG retargeted
  (renderAccounts noop) because converged cluster no longer overflows with wide gap.
- Impl bugs fixed en route: popover insertion anchored mid-tag hijacked #toast (two quote/attr
  repairs); syncModeInd extension used block-scoped `del` (del2); helper scope hoisted to top level.

## v1.76 — 2026-11-18 (portal-unified) — fix: device drawer collapse button icon family/direction
User report: drawer top-right collapse button icon wrong.
- Root cause: ddClose used #i-arrow-left (a back-NAVIGATION arrow, pointing left) on a RIGHT-anchored
  overlay side-sheet whose button COLLAPSES the panel toward the right edge. Wrong icon family
  (back-arrow vs region-collapse chevron) and wrong direction; aria-label also diverged
  ("Back to Devices"/"Close details") from the static icon.
- Precedent check: region-collapse family = chevron pointing toward the collapse edge
  (railCollapse i-chevron-left / alCollapse i-chevron / tlCollapse i-chevron-down); modal dismiss = X.
  ch-side is an in-page panel (no dismiss precedent). Sprite audit: i-arrow-left exists and draws
  correctly — the defect was semantic, not a missing symbol.
- Fix: ddClose → class region-collapse + #i-chevron (right) + aria-hidden svg; aria-label/title synced
  in renderDrawer ("Back to Devices (collapse panel)" on devices / "Collapse device details" else).
  Other i-arrow-left usages (spoke back buttons) are legitimate back-navigation and untouched.
- Verified: harness 27 cases ALL GREEN (device-drawer += ddIcon: use href #i-chevron + region-collapse
  class) + NEG all red; node --check + lint clean; stamps x4 (C68); drawer-head screenshot shows
  right chevron (transition disabled for final-frame capture).

## v1.75 — 2026-11-18 (portal-unified) — fix: nav collapse button lost its icon when collapsed
User report: after collapsing the sidebar, the collapse/expand button showed no icon.
- Root cause: `svg.ic` base rule carries NO intrinsic size (sizes come only from scoped rules like
  `.nav-ico .ic` / `.search .ic`); the SVG inserted by the collapse handler had no scoped size rule →
  rendered at the inline-SVG default 300x150, overflowed the 28px button and was clipped by the
  sidebar → invisible icon (measured rect 300x150 pre-fix).
- Secondary debt: initial markup and three state paths (goto tower / view leg / resetProto) used text
  glyphs ⟨/⟩ (C56 violation) and diverged from the click path's SVG.
- Fix: `#navCollapse .ic{width:14px;height:14px}` + flex centering; initial markup → SVG sprite
  (i-chevron-left) + aria-label; single top-level `syncNavCollapse()` helper drives icon
  (i-chevron collapsed / i-chevron-left expanded) + title + aria-label from ALL paths (click, tower,
  view leg, reset). Helper hoisted to top level (first attempt landed inside wire() → applyProto
  ReferenceError, caught by harness).
- Verified: harness 27 cases ALL GREEN incl new nav-collapse (initSvg/collapsedIcon rect<=28/
  collapsedState/expandedIcon/railIcon/resetIcon/aria) + NEG all red (syncNavCollapse noop);
  node --check + lint clean; stamps x4 (C68); collapsed-rail screenshot shows chevron.

## v1.74 — 2026-11-18 (portal-unified) — B6 delivered (pending user review): Audit = core security minimal ring
Implements web-portal-ui §3i B6 (Q8=A / Q6 forward notes / C62). Completes the B1–B6 design round.
- **Ring filter**: Audit page shows ONLY security-ring events (auth./account./psk./key./device./access.);
  operational floor.grant/room.join etc. excluded (T2: not rebuilt here).
- **Ring kind seg** (radiogroup+roving): All/Identity/Credentials/Devices/Break-glass; break-glass kind
  matched BEFORE identity (account.breakglass belongs to bg, not identity).
- **Source column**: core (local ring) / provider (delegated feed, simulated) / simulated (login/logout:
  backend audits auth events but Web query face unproven → honest simulated tag + tooltip).
- **Retention label**: "Retention 90 d · simulated in-memory ring (256 entries) · core security store
  unproven → simulated" (honest, no fake compliance claim).
- **Operational history link**: guarded button + keyboard-discoverable reason "talk app /talk/history
  (T2): separate surface, not rebuilt in this portal" (no fake navigation).
- **Three states**: empty (No security events match) / loading (300ms simulated) / error (audit store
  unreachable banner + Retry; devtools "Audit store · Simulate error" leg).
- Verified: harness 26 cases ALL GREEN (audit-ring 11 assertions: ringRows/retentionLabel/historyLink/
  loginQuerySimulated/logoutSimulated/kindFilter/bgRing/sourceProvider/emptyState/errState/loadingState)
  + NEG all red (audit-ring: renderAudit noop); node --check + lint clean; stamps x4 (C68).
- Screenshots: b6-ring/bg/error/narrow.
- Round status: B1 v1.69 / B2 v1.70 / B3 v1.71 / B4 v1.72 / B5 v1.73 / B6 v1.74 all delivered,
  pending user review per P9 loop; 26 harness cases + NEG self-check green.

## v1.73 — 2026-11-18 (portal-unified) — B5 delivered (pending user review): Monitor real-wall linkage design
Implements web-portal-ui §3i B5 (reuse Monitor, no redesign; §2i layouts/cap honoured, not re-adjudicated).
- **Device-tree picker upgraded in place** (grouping NOT rebuilt): #pickAvail role=tree; device rows
  role=treeitem aria-level=1 aria-expanded; twisty span→button tabindex=0 aria-label/expanded; stream
  rows role=treeitem aria-level=2; delegated tree keyboard (ArrowUp/Down move, ArrowRight/Left
  expand/collapse, Enter/Space toggle, Home/End).
- **Play All**: selects connected streams only (offline/pending/revoked skipped, honest toast counts),
  capped at WALL_CAP=16; guarded when no connected streams.
- **16-wall cap (§2i Auto)**: 17th selection guarded at stream checkbox, device bulk checkbox and Apply
  (setGuard + reason + revert); cap constant WALL_CAP=16 separate from intercom MON_CAP=4.
- **Per-stream metrics**: existing tile-stats overlay (FPS/RES/BIT/…) asserted on-wall with stats ON.
- **Honest banner**: #monSimNote "Simulated video — prototype feeds, not real streams" + real-backend
  contract pointer (/sfu/rooms + /sfu/stats); mapping table recorded in boards B5 note.
- **Three states**: empty (content=empty) / loading (#wallLoading) / error (#liveStale via
  body[data-content=error]) + Retry now restores content=data (was toast-only dead end).
- Verified: harness 25 cases ALL GREEN (monitor-link 13 assertions: treeRole/twistyFocusable/
  treeKeyboard/treeExpand/playAllGuard/cap16Guard17th/bulkGuard/perStreamMetrics/honestBanner/
  emptyState/loadingState/errState/retryWorks) + NEG all red (monitor-link: renderAvail noop);
  node --check + lint clean; stamps x4 (C68). Screenshots: b5-picker/cap/wall/error/narrow.
- Process notes: duplicate case blocks from re-run scripts deduped (keep-newest rule corrected);
  harness DOM-attribute reads replaced with equivalent class/state reads where Marionette injection
  diverged from plain headless (assertions semantics unchanged).

## v1.72 — 2026-11-18 (portal-unified) — B4 delivered (pending user review): fleet split-fused into Devices
Implements web-portal-ui §3i B4 (Q3=D' fleet = vehicle-kind devices + agent telemetry; no new spoke/domain).
- **Fleet lens**: Devices page segmented All devices / Fleet = role=radiogroup + role=radio +
  aria-checked + roving tabindex + arrow keys; Fleet filters kind=vehicle; lens state survives goto
  deep link (goto(id,lens)).
- **Fleet health strip (5 counts)**: Vehicles N / Signal lost x / Agent unreachable y / Stalled topics z /
  Down processes w. Unreachable = no /status report OR report ts stale > 30s (simulated); unreachable
  vehicles supersede topic/process counts (their feed is not trustworthy) — semantics documented.
- **Row telemetry summary** in Fleet lens: signal ok/lost + uptime, or "agent unreachable".
- **Telemetry tab (6th drawer tab)**: signal badge + uptime + processes running/down/unexpected +
  topics fps/bps/stalled + streams connected/resolution/fps (full Vehicles.tsx field set);
  unreachable empty state with stale-threshold reason; loading + error(+Retry) states.
- **Overview fleet card**: counts summary + "Open fleet lens" deep link → devices?lens=fleet.
- **Scope cross-link**: Accounts/Keys Device-scope chips source = same vehicle set (Q12); identity
  source note: real AdminDevice has no kind field → /status reports room_id (simulated seed; backend
  field = open item).
- **Three states**: empty (no vehicles match filter) / loading (300ms simulated) / error (telemetry feed
  unreachable banner + Retry; devtools "Fleet feed · Simulate error" leg).
- Regression registrations honoured: device-drawer tabs 5→6.
- Verified: harness 24 cases ALL GREEN (fleet 13 assertions: lensFilter/lensAria/healthStrip/rowSummary/
  deepLink/telemetrySignalUptime/procThreeState/topicStalled/unreachable/scopeCrossLink/errState/
  loadingState/emptyState) + NEG all red (fleet: renderFleetStrip noop); node --check + lint clean;
  stamps x4 (C68). Screenshots: b4-fleet/telemetry/unreachable/overview/narrow.

## v1.71 — 2026-11-18 (portal-unified) — B3 delivered (pending user review): device credential lifecycle + vision gating
Implements web-portal-ui §3i B3 (Q4=B vision gate / Q12 / real devices endpoints / PIT-269).
- **Credentials tab (5th drawer tab)**: state line (registered·secret issued / pending·no credential yet);
  Register+approve modal (optional rename-on-approve + optional provided secret, else generated) →
  pending→online + audit device.register + device.approve + one-time close-protected secret;
  Reset secret = confirmDlg consequence (old invalidated) → 300ms simulated → one-time secret +
  audit device.secret.reset; revoke semantics card surfaces real backend revoke=DELETE vs prototype
  revoked-flag+Restore vision (marked simulated); Reject/Restore/Re-approve labelled no-real-endpoint.
- **Vision gating (Q4=B)**: can() split canBase + VISION_SET(config/caps/rename/zone/reject/restore/
  reapprove) gated unless VISION_ON; vGuard() gives keyboard-discoverable reason "Backend unproven —
  vision gated. Enable Prototype controls > Device vision (simulated)"; devtools "Device vision · Toggle
  simulate" leg flips VISION_ON and repaints devices+drawer; real-endpoint actions (approve/revoke/
  devops register/reset) stay ungated.
- **Three states**: empty (pending no credential) / loading (Resetting credential… 300ms) / error
  (credential store unreachable banner + Retry; devtools "Cred store · Simulate error" leg).
- **Guards**: register/reset require operator|admin (canBase devops) + not delegated(creds) + state
  (pending no reset / revoked no reset); viewer/operator/dispatcher read-only with reasons.
- Regression registrations honoured: device-drawer tabs 4→5; device-config/device-role cases enable
  vision leg first; duplicate psk case/NEG (from a re-run script) deduped.
- Verified: harness 23 cases ALL GREEN (creds 4-part 18 assertions: visionGated/emptyState/simulateLeg/
  focusInitial/loading/registerOnce/closeGuard/registered/resetConfirm/resetLoading/resetOnce/
  revokeSemantics/focusRestore/escOrder/guardKeyboard/errState/retryLoading) + NEG all red
  (creds: resetSecret noop); node --check + lint clean; stamps x4 (C68).
- Screenshots: b3-gated/vision/register/config-gated/narrow (5 tabs, semantics card, register modal).

## v1.70 — 2026-11-18 (portal-unified) — B2 delivered (pending user review): Keys = PSK + API keys dual partition
Implements web-portal-ui §3i B2 (Q2=B / Q13=E′ / C62 one-time secrets / PIT-269 dialog contract).
- **PSK card**: View reveals full PSK and audits psk.view EACH reveal (real get_psk semantics preserved,
  not narrowed to hint); Rotate = confirm modal with OPTIONAL custom new value (8-128 validation, real
  RotatePskRequest.password semantics) else generated; old value invalidated; new value shown ONCE.
- **One-time secret modal** (PSK rotate + API key create): "shown once" seal + consequence copy;
  close protection — Esc/overlay/Done without copy routes to confirmDlg ("cannot be shown again");
  copied → direct close; Copy = synchronous execCommand fallback + parallel clipboard attempt;
  clipboard unavailable → fully-selectable secret + manual-copy hint (no dead end).
- **API keys table real fields**: id/prefix/role/device scope/label/expires (was label/prefix/created/
  used/expires); create modal (id+label required, role combobox ARIA, Device scope chips with non-vehicle
  gated + empty=zero-devices hint, admin/dispatcher scope ignored); revoke = confirmDlg consequence +
  audit key.revoke (was no-confirm no-audit debt); create audits key.create.
- **Gates**: admin manages (can('keys')); dispatcher read; viewer/operator gated "No access" (server 401);
  delegated(creds/psk) → management guarded + reason; degraded → break-glass emergency PSK rotation
  (confirm + audit psk.breakglass + seal).
- **Three states**: empty (No keys match / No access) / loading (300ms simulated on rotate/create/retry) /
  error (key store unreachable banner + Retry; devtools "Key store · Simulate error" leg).
- Removed legacy fake delegates: #newKey toast-only "Key issued" (no secret) and #idAccess toast fake
  external link (B1 real anchor already in place).
- Impl bugs found & fixed by assertions: secretScrim spliced INSIDE cfmScrim modal (first </div></div>
  anchor) → moved to body level before toast (Esc DOM order: cfm wins over secret guard); legacy #newKey
  delegate pushed old-shape keys (k.vehicles undefined crash); clipboard promise hang → synchronous
  fallback + parallel attempt.
- Verified: harness 22 cases ALL GREEN (psk 5-part: viewAudited/rotateFocus/rotateValidate/loading/
  onceDisplay/closeGuard/closeGuardKeeps/copyHandled/safeClose/rotated/createFocus/nonVehGated/
  createLoading/keySecret/created/revokeConfirm/revoked/delegatedGuard/bgPsk/bgAudited/errState/
  retryLoading/emptyState) + NEG all red (psk: doRotate noop); node --check + lint clean; stamps x4.
- Screenshots: b2-keys/secret/create/delegated/bg/narrow (narrow cards fit, no horizontal overflow).

## v1.69 — 2026-11-18 (portal-unified) — B1 delivered (pending user review): Accounts + Access domain + capability matrix
Implements web-portal-ui §3i B1 (plan v4, post 4-role review; Q1/Q5/Q9/Q10/Q11/Q12/Q13/Q19).
- **Access domain**: Security renamed Access (menu single word, Q10); hub card title "Identity & Access";
  spokes Accounts/Keys(PSK & API keys)/Audit(core minimal ring) → 10 spokes (admin-hub regression updated).
- **Capability matrix (9 rows incl Q15 ldap)**: provider from config seed × health from simulated endpoint
  (Q9 full derivation: built-in×any=active; delegated×reachable=delegated; delegated×degraded/unreachable=
  degraded(break-glass); none=absent; vision override hook gated for B3). authz row dual-label
  delegated(PDP)+active(PEP) (Q14). Affordances: Open(A) / Open in AccessBase ↗ (real anchor new-tab
  rel=noopener + initiated audit, delegated listener) / Force break-glass (delegated+reachable, confirm+
  audit+seal) / Break-glass (degraded) / focusable placeholder button + reason (absent/gated).
- **Accounts page**: real 4-role model (viewer/operator/dispatcher/admin; operator leg added, field→viewer);
  Device scope (Q12): vehicle chips selectable, non-vehicle kinds gated "enforcement pending backend
  extension"; empty scope = zero devices hint (viewer/operator); admin/dispatcher scope ignored = all devices;
  create/edit modal (password required create / optional edit; generalized combobox with role=combobox/
  listbox/option + aria-activedescendant + arrow/Enter/Esc); focus contract (initial focus first input,
  restore trigger after repaint, accScrim registered dlgKey(sc,e) + inserted before cfmScrim for Esc order);
  delete confirmDlg + self-delete guard; role gates (admin manage / dispatcher read / viewer+operator
  gated "No access" aligned to server 401); delegated gate + Force entry; break-glass card (emergency
  account create + credential reset, confirm + audit account.breakglass[.reset] + seal tag).
- **Three states**: empty (No accounts match) / loading (300ms simulated skeleton on Retry) / error
  (store unreachable banner + Retry); audit feedback persistent via AUDIT rows (not toast-only).
- **Global**: topbar mode/health indicator (two-part, narrow hides tail, tooltip full); degraded flow
  banner (wsBanner precedent) + break-glass shortcut; Settings access row conditional (standalone hidden);
  userchip role syncs with devtools role; audit actor = effective role; guard single-track sweep
  (native disabled+data-reason → setGuard focusable); content-state devtools leg labelled Monitor-only;
  **default light** (Q19: init/localStorage fallback/Reset/seg markup).
- **Narrow @600**: card reflow fits; header min-content overflow fixed (modeInd tail hidden + #topbar
  gap/padding narrow + #crumb 24vw); docSW==600 verified.
- Impl bugs found & fixed by assertions: cfmScrim closing quote eaten by insertion (confirmDlg dead →
  groups/evidence/device-drawer regressions); dlgKey registered with wrong signature; matrix force/bg
  wiring spliced into wrong function; accRetry wired after error early-return; focus restore lost on
  repaint; banner fixed-overlay covering topbar + unbounded icon.
- Verified: harness 21 cases ALL GREEN (accounts 25 assertions + accounts-narrow) + NEG all red
  (accounts: renderAccounts noop; accounts-narrow: wide-gap style injection); node --check + lint clean;
  stamps x4 (C68). Screenshots: matrix/accounts/delegated/break-glass/narrow.

## v1.68 — 2026-11-17 (portal-unified) — Phase 9 delivered: device detail drawer (K17=A/K18=A/K19=B/K20=B)
Implements PLAN Phase 9 v2 (post 4-role team review; dead-ends closed before implementation).
- **9.1 drawer**: NEW overlay side-sheet #devDrawer (480px, full-width ≤900, visible back/close) — ch-side was
  an in-page panel, not reusable (honest registration). 4 tabs Overview/Config/Identity/Events with roving
  tabindex + arrow keys; Esc priority chain modal-scrim → drawer → legacy cascade; drawer input Esc cancels
  that draft edit; focus trap + initial focus close + restore trigger; row primary-cell click / Details opens
  (action buttons stopPropagation); Overview = health (fps/bitrate/latency from DEV_STREAMS) + lease/fw/zone +
  history timeline + Open in Monitor (offline/revoked guarded, not-on-wall honest toast); NO fake preview (K20).
- **9.2 config**: resolveConfig (global default → device override; room layer talk-only, user scope excluded);
  draft → live diff (rows == edits) → Apply → audit device.config.apply + visible Undo (rollback depth 1,
  snapshot taken at delivery); offline Apply = **staged · offline** + row badge; devtools "Reconnect offline"
  leg flushes staged (revoked excluded) → applied + audit delivered + toast + aria-live; degraded Apply =
  failed with Retry/Discard; staged revoke; draft kept per-device on close + row draft badge; cancel button.
- **9.3 roles**: viewer (field maps to viewer) read-only / dispatcher rename+zone+config / admin + caps +
  Approve/Reject/Revoke/Restore/Re-approve; setGuard + reasons; devtools role legs admin/viewer added; role
  switch repaints devices + open drawer; drawer head shows effective role.
- **9.4 state machine + honesty**: Details replaces toast-only View; Reject → rejected (admin Re-approve →
  pending); Revoke → offline + revoked + clears staged (admin Restore → pending); all destructive ops modal
  confirm with consequence copy + AUDIT device.*; history appended on every transition; pending/offline/
  degraded guards.
- **9.5 harness**: device-drawer / device-drawer-narrow / device-config / device-role cases + NEG sabotage
  (openDevDrawer/flushStaged/setGuard noop); harness null-result guard added (sabotaged mid-throw = red).
  19 cases ALL GREEN; negative self-check ALL RED AS EXPECTED; node --check + lint clean; stamps x4 (C68).
- Impl bugs found & fixed by assertions: Apply onclick captured stale empty drafts (now live); duplicate
  renderRooms declaration from splice anchor; missing template backtick after diff-container refactor.

## v1.67 — 2026-11-17 (portal-unified) — table action slots fixed-width + dialog scrim class fix
User screenshot: Devices action buttons still unequal (View 64 / Revoke ~72 / Approve ~76) because v1.66 used
min-width:64 (a floor, not a slot). Also found while verifying: v1.62's new dialogs used class="scrim" but the
stylesheet defines `.modal-scrim` → grpScrim/cfmScrim had NO hiding/centering CSS (visible in-flow on short
pages, e.g. filtered tables).
- `.tbl .actions .btn` → **fixed slot** width/min/max:72px, padding:0 → View/Revoke/Approve/Reject all equal
  across every admin table and state; harness admin-controls now asserts all action widths identical ==72.
- grpScrim/cfmScrim class corrected to `modal-scrim` → proper fixed overlay + backdrop + centering + hidden
  when closed (dialog infra from v1.62 now visually correct).
- Verified: harness 15 cases ALL GREEN (admin-controls actionsUniform=fixed72; groups end-state shows no
  stray scrims); node --check + lint clean; version stamps exactly 4 (C68).

## v1.66 — 2026-11-17 (portal-unified) — admin control-family normalization (sizes / state geometry / slots)
User: admin spoke pages' buttons/controls had inconsistent sizes and state-dependent layout. Audit found:
`.iconbtn` 34 (base + play-ctl override) vs --ctl-h 30; `.search` 36; `.seg` container height font-driven
(~32 outer) vs btn 30; `.msg-in` r4/pad8 vs family r6/sp12; `.btn-danger` defined twice (transparent vs tint);
table action buttons width varied by label; `.set-v` rows mixed seg/toggle heights; text glyphs ＋/⌕ as icons.
- **One control family (C69)**: all interactive controls in admin spokes = outer height --ctl-h(30), r6,
  horizontal padding sp12, fs12; `.seg` fixed 30 with buttons height:100%; `.search` 30; `.iconbtn` 30
  (both definitions); `.msg-in` r6/sp12; duplicate `.btn-danger` removed (single transparent+hover-tint).
- **State geometry stable**: on/hover/disabled change color/weight only; table action buttons min-width:64
  (uniform slots); `.set-v{min-height:30}` keeps settings rows aligned (switch stays its own family).
- **Glyphs → SVG**: ＋ New key → i-plus sprite; ⌕ search → i-search sprite.
- Verified: harness 15 cases ALL GREEN incl new admin-controls (every spoke control h==30; seg toggle keeps
  30; action slots >=64); node --check + lint clean; version stamps exactly 4 (C68).

## v1.65 — 2026-11-17 (portal-unified) — admin spoke: explicit back + clickable crumb + narrow card reflow (A+B+C)
User: admin hub spokes had no back control and poor narrow adaptation. Measured @600: backBtns=0, crumb is a
non-clickable SPAN hidden ≤760 (v1.63 side-effect) so hub context vanished; tables scrolled horizontally
(655/498) with hidden columns and no sticky/hint; page-head cramped.
- **A back affordance**: every Admin spoke page-head gets a back iconbtn (aria-label/data-tip → goto admin);
  breadcrumb "Admin" segment becomes a clickable crumb-link; ≤760 crumb no longer hidden — single-line
  ellipsis "Admin / X" (sub hidden, conn text hidden to keep topbar fitting).
- **B narrow tables**: ≤760 `.tbl` reflows to label-value cards (thead hidden, tr=card, td::before=column
  label via data-th injected by a MutationObserver enhancer so re-renders keep labels); no horizontal
  scroll, all columns visible; primary cell = name+sub block.
- **C compact head**: ≤760 page-head wraps, title single-line ellipsis, sub hidden, toolbar full-width,
  search flex/min0.
- Verified @600: back/backWorks/crumbBack true, tblSW==CW(490), card display flex, crumb visible, docSW==600;
  harness 14 cases ALL GREEN incl new admin-spoke; node --check + lint clean.

## v1.64 — 2026-11-17 (portal-unified) — admin nav IA: single Admin hub + domain grouping + naming convergence (B+D)
User: Administration menu too cluttered (9 flat items mixing assets / observability / security / system).
Research: Synology SS separates management domains (Device/Recording/Notification/User+privilege profiles/
System) from operations; Milestone Management Client uses a hierarchy tree (Servers→Devices→Rules→Users&Roles→
Security→Settings) separate from Smart Client; UniFi Protect collapses config into a Settings hub; IA guidance
= group by user mental model, 5–7 per group, hub-spoke for settings, role-based disclosure. Flat 9-item
heterogeneous admin list in the operations sidebar is an anti-pattern.
- **B hub-spoke**: sidebar Administration collapses to ONE entry `Admin`; new #admin hub groups the 9 spokes
  into 4 domain cards — Assets(Devices, Rooms) / Observability(Sessions, Streams, Signaling) /
  Security(Keys, Audit) / System(Settings, Config delivery); spoke rows = card-row language with name+id+desc;
  clicking opens the existing page (routes unchanged); spoke pages highlight the Admin parent in sidebar and
  breadcrumb prefixes "Admin / ".
- **D naming/semantics**: Config push → **Config delivery** (Settings domain); Client keys → **Keys**;
  Audit log → **Audit**; Rooms described as logical grouping of Devices; Streams/Sessions/Signaling unified
  under Observability.
- **C progressive (deferred, recorded)**: role-based disclosure (dispatcher sees operations + read-only
  Devices), ⌘K command palette, pin favorites — next increment, not in v1.64.
- Verified: harness 13 cases ALL GREEN incl new admin-hub (4 domains / 9 spokes / spoke navigation /
  parent-active / breadcrumb) and nav count 8 unique SVG; hub + spoke renders clean; node --check + lint clean.

## harness tooling — 2026-11-17 (scripts/ui-harness.mjs, no UI version bump) — board-align cases + negative self-check
- Added `board-1100 / board-1000 / board-1200` cases (C66): uniform row height, single-line action cluster,
  unique right edge, Mon icon inside card, holder readable, row scrollWidth==clientWidth per width band;
  harness sets window rect per case (SetWindowRect).
- Added `--negative` self-check mode (C65/PIT-265): per-case sabotage (NEG map: clear queue / raise queueMax /
  force action wrap / noop evPurge / noop renderNotif) MUST turn the case red; 5 sabotaged cases NEG-OK,
  others NEG-SKIP; exit non-zero on any NEG-BAD. Normal mode now 12 cases ALL GREEN; .harness products refreshed.

## v1.63 — 2026-11-17 (portal-unified) — intercom board rows: fixed icon action slots + narrow reflow (A+D+E)
User: board (channel list) right-side buttons misalign vertically when checked/other states, and narrow
screens adapt poorly. Root causes: variable button count (temp rows 6 vs 5) + checked labels growing
("Prio ✓"/"Mon ✓") pushed the flex-wrap action cluster over the 320px column → some rows wrapped to two
lines while others stayed one (vertical offset exactly in checked states); no fixed slot widths → ragged
right edges; board-row had NO responsive rule (fixed columns ≈842px min) → narrow overflow.
- **A fixed slots**: `.row-actions` = grid `repeat(6,30px)` no-wrap; `.act` icon-only 30×30 (aria-label +
  data-tip; new i-trash/i-star sprites); checked state = color + aria-pressed ONLY (✓ text removed);
  non-temp rows render a hidden `.act-ph` placeholder in the Disband slot → identical geometry in every
  state; action column 320→220 (coarse 44px slots → 284 column).
- **D narrow reflow**: ≤1000 board-row → 3-row card (`name mode / state timer / q actions`); ≤600 → stacked
  card with full-width action row; ≤760 topbar hides crumb + role-tag (topbar min-content 712px was the
  page-level overflow source at 600). **Mid-band fix (user screenshot: Mon icon outside the card at
  ~901–1100px)**: desktop fixed tracks could not shrink, pushing the 200px action track past the card edge;
  tracks now `minmax(120px,210px) minmax(64px,92px) minmax(120px,1fr) 76px 60px 200px` with ellipsis on
  name/mode/holder/sub (+ `.floor-state>div{min-width:0}`), card breakpoint raised 900→1000 (coarse ≤1100,
  coarse 284px action column only ≥1101) so no width band overflows.
- **E**: `.board-row{min-height:44px}` touch target.
- Verified: rowH uniform 54 across all rows incl temp+checked; actH uniform 30 (single line); rightEdge
  unique=1; @900/@600 renders no horizontal overflow (docSW=600 at 600vw, topbar 540); node --check + lint clean.

## v1.62 — 2026-11-17 (portal-unified) — Phase 7 delivered: group mgmt + cross-page notif + evidence privacy + harness
Implements PLAN Phase 7 (7.1–7.5) after the second 4-role panel re-review; mechanisms per K14=B/K15=A/K16=C.
- **7.1 Group management**: NEW dialog infrastructure (the existing picker modal had no trap/focus restore,
  so it could not be reused): focus trap + initial focus + stack-based focus restore (nested confirm keeps
  the outer restore) + Esc arbitration (modal-first, stopPropagation, never cascades leaveChannel; Esc in a
  modal input cancels the modal). "Manage groups" modal: create / inline rename / member add-remove / role
  edit (dispatcher-gated via setGuard); disband = modal confirm + full cascade (recBuf/evBuf/consents/
  chanMutes/chanVols/chanMsgs/mutedParts/callWith hangup/chanOpen/monitorSet + tower/badges + audit);
  temp groups deep-copy members (no reference bleed); Clear = modal confirm (K16 criterion); enterChannel
  id-guard (stale id no longer crashes); devtools role-switch leg; board empty-state points to Manage groups.
- **7.2 Cross-page floor notifications (K15=A)**: #floorNotif chip before conn chip; per-group+type dedup,
  cap 3 drop-oldest + aggregate count; aria-live + keyboard; expiry per type (held=hold_timeout /
  queued=granted-other|cleared|60s / preempted=ack|10s); stale-click revalidate (toast+clear, no crash);
  valid click = goto intercom + enterChannel + clear; entering intercom hides chip (in-page toast only);
  other floor toasts suppressed cross-page; badge stays pure held count; devtools external emergency +
  external queue-to-you legs; unknown never fakes a notification.
- **7.3 Evidence privacy (K14=B)**: evBuf.at = real Date.now; per-second tick purges entries older than
  EV_RETAIN_DAYS with an audit row; purge repaints an open Recordings tab; display formatted via fmtEvAt
  (storage/display split); revoke consent = modal confirm + clears evBuf + audit; ev-tag becomes an
  affordance opening the Recordings tab; devtools "Inject expired evidence" (simulated) triggers purge;
  AUDIT evidence rows use the real time base.
- **7.4 Assertion harness in-repo**: `scripts/ui-harness.mjs` (Marionette WebDriver:ExecuteScript read-back
  via injected page-script + TakeScreenshot; no OCR / no exit-code trust). 9 cases (floor/queue/replay/
  emergency/tabs-size/nav/notif/groups/evidence) ALL GREEN; products `docs/ui/prototypes/.harness/*.png`
  (stat>0 enforced, C61/PIT-256) + results.json.
- **7.5 Regression**: monitor/tower/recordings/overview renders + node --check + DOM zero-missing + lint clean.
- Verified assertions: grpCreate/grpDisbandCascade/memberCopy/roleGuard/focusRestore/escModal/trapWrap;
  notif xpage/clickLand/inpageHide/dedup/cap3/stalePurge; ev injected/purged/repaint/revokeClears/evTagRec.

## v1.61 — 2026-11-17 (portal-unified) — K11=C: split-path incremental rendering (capture/focus survive)
Adjudicated K11=C: high-frequency paths (PTT down/up, per-second tick, yieldFloor) now use incremental
paintChannelLive()/paintBoardLive() that update text/classes/guards WITHOUT rebuilding nodes, so pointer
capture, focus, scroll and range values survive; structural changes (tab/enter/queue/consent/emergency)
still use full render + focus restore.
- paintChannelLive: timer/name/sub/q-order/avatar/wave/PTT onair/emerg-arm label/rec-count/link readout/
  chReq+chRel guards. paintBoardLive: per-row holder/sub/timer/qdepth+order tooltip/grants/revoke/mute/prio
  guards + holding/stale classes.
- Verified: pttNodeKept / boardNodeKept / pttOnAir / pttReleased assertions green (same DOM node across PTT
  and tick); node --check + lint clean.

## v1.60 — 2026-11-17 (portal-unified) — K10=A: guarded disabled (aria-disabled) replaces native disabled
Adjudicated K10=A: native `disabled` suppresses pointer/click/focus so the guard-reason tooltip could never
appear ("backend-gated" honesty was dead text). Switched #chLock and unknown-state Replay Play to the site
guard doctrine: setGuard -> aria-disabled + data-reason + is-disabled; reason now discoverable on hover/focus.
- Verified: lockAria/lockTip (reason tooltip shows)/playGuard assertions green; node --check + lint clean.

## v1.59 — 2026-11-17 (portal-unified) — K9=C: replay exit-clear scope (scan keeps, manual/cross-page clears)
Adjudicated K9=C: scan cycling is a continuous listening session (buffer kept); manually switching channel,
cross-page goto, or Back counts as exit and clears that channel's replay buffer (K5 privacy default upheld
for real exits). Consent chip + diagnostics copy now state both paths explicitly.
- enterChannel(id,viaScan): recClear(old) only when !viaScan; goto() clears chanOpen buffer when leaving
  intercom; leaveChannel unchanged (clears).
- Verified: scanKeeps / manualClears / xpageClears assertions green; node --check + lint clean.

## v1.58 — 2026-11-17 (portal-unified) — K8=C: simulated behaviors for VOX/codec; shout/ACL relabelled
Adjudicated K8=C (mixed landing standard): high-value sub-features get simulated BEHAVIOR + assertions;
low-value ones stay switch/indicator-level and are relabelled backend-gated (claims aligned with code).
- VOX: when ON, a simulated voice source auto-keys PTT (pttDown/pttUp ~2s utterance every 8s) only when the
  floor is free; button data-tip states the simulated source.
- Codec: simulated weak-net cycle auto-falls back HD->SD (turnaround 260ms) and recovers SD->HD (120ms);
  link readout now shows turnaround (`ta N ms`), making 6.4d's degrade/turnaround claims testable.
- Shout (cross-group) + ACL enforcement (listen/talk/manage): relabelled as backend-gated; Comms "Directed"
  card footnote states prototype shows roles only; backlog #14/#15 marked partial (switch/indicator level).
- Verified: voxT armed / degrade(liveSec20)->SD+260 / recover(liveSec30)->HD+120 / ta readout; node --check
  + lint clean.

## v1.57 — 2026-11-17 (portal-unified) — intercom expert-panel re-review: objective defect fixes
4-role parallel panel on implemented intercom (v1.56) found 24 findings; objective ones fixed here
(conflicts K8-K11 handed to adjudication, NOT decided):
- Floor: idle/queued requests now auto-grant (qPush/floorTick grant head when floor free); bare PTT no
  longer jumps a non-empty queue; autoGrant clears your _q; Prio toggle rewrites existing queue items.
- Session: enterChannel cleans previous channel side-talkers + releases held PTT; manual leave/enter stops
  scan (scan-driven switch keeps scanning); replay-buffer-on-scan semantics left to K9.
- Emergency: arm state per-channel + reflected across re-renders (no single-click degrade); insert grants
  floor without a fake physical press (no pointerleave/keyup accidental release); removes your queue item.
- Input/a11y: global Enter keyup releases PTT; focus restored after re-render; tabs roving tabindex + arrow
  keys + tabpanel; Esc ignored while typing; whisper/call aria-labels; cueToggle role=switch + keyboard;
  board rows respond to Space; board empty state.
- Honesty/robustness: Comms (message/whisper/call/location) guarded under unknown; WS double-drop no longer
  strands groups in permanent unknown; replay playback wave persists via replayUntil; PTT setPointerCapture.
- Layout/visual: chan-head wraps; board actions track 320px + wrap; .act/.ch-tab r6; emoji (bell/pin) -> SVG;
  syncCueUI innerHTML (was textContent escaping SVG); navCollapse chevrons SVG; temp-group id unique;
  hybrid seed side-talker makes cap reachable; floor.hybrid_max config row added.
- Docs freshness: sidecar v1.57; "NOT implemented" title -> delivered; PLAN/README summaries v1.48-57;
  §8 elastic-tier marked delivered; §6d micro-decision values + anchor-snapshot notes; HTML comment version
  attribution restored per feature.
- Verified: idleGrant/noJump/scanSideClean/armPersist/emergNoPress/xpageStill assertions green; node --check
  + lint clean.

## v1.56 — 2026-11-17 (portal-unified) — pin WIDTH too: VOX/HD/PTT identical 56x30 boxes
User: heights now equal but WIDTHS still differ (auto width follows label length: VOX/PTT 3 chars vs HD 2).
v1.55 pinned height only; width was left auto — the remaining axis.
- `.diag-ctl` now pins **width too**: `width/min-width/max-width:56px; padding:0; justify-content:center`
  (height already 30). All three diag controls are now identical 56x30 boxes regardless of label length.
- Screenshot-verified (2x isolated, light): chVox 56.0x30.0 / chCodec 56.0x30.0 / chLock 56.0x30.0.
  node --check + lint clean.
- Lesson folded into C54: "identical size" means BOTH axes pinned (width+height), not height alone.

## v1.55 — 2026-11-17 (portal-unified) — pin VOX/HD/PTT to ONE dedicated class (cascade-independent sizing)
User: several rounds and VOX/HD/PTT STILL different in their browser. Root-cause chain of the repeated
failures (honest accounting): v1.51 imposed PTT=40 (wrong judgment); v1.52 unified geometry but audited
height only; v1.53 unified fill family; v1.54 fixed disabled opacity — yet the three buttons still lived on
THREE different cascades (`.chip-btn`, `.chip-btn.on`, `.btn.btn-sm`), so any environment/override/cache
difference could re-diverge them. Fixing families one at a time was whack-a-mole.
- **Structural fix**: added ONE dedicated class `.diag-ctl` applied to all three (#chVox/#chCodec/#chLock):
  `height/min-height/max-height:30px; padding:0 12px; radius:6px; fs12; line-height:1; inline-flex centered;
  border-box`, placed at the end of the cascade. Identical box regardless of chip/btn family or overrides.
- Screenshot-verified (2x isolated, light): chVox h30 w49.7 / chCodec h30 w43.7 / chLock h30 w48.8 — three
  identical-height boxes; node --check + lint clean.
- If a browser still differs after hard-refresh to v1.55, it is stale cache/environment: check footer version
  and report it; the pinned class makes divergence impossible from this stylesheet.

## v1.54 — 2026-11-17 (portal-unified) — disabled-state visual weight unified (screenshot-verified at 2x)
User: "still different in real test; did you screenshot-confirm?" Produced an ISOLATED 2x-zoom screenshot
of the Diagnostics card: VOX/HD/PTT heights identical (30 each) and same outline family, BUT the disabled
PTT used whole-box `opacity:.4`, making it look fainter/smaller than VOX/HD.
- `.btn:disabled` changed from `opacity:.4` to **same box + muted text** (`opacity:1; color:tx3;
  border:bd; background:surface; cursor:not-allowed`) so disabled keeps identical visual weight; state is
  conveyed by text color + cursor + guard tooltip, not by fading the whole control.
- Screenshot-verified (2x isolated, light): chVox h30 op1 / chCodec h30 op1 / chLock h30 op1 — three
  identical boxes, only text color differs (dark / accent / muted). node --check + lint clean.
- Note: if a browser still shows old sizes it is a stale cache — check the footer version (v1.54) and
  hard-refresh; the live file and headless renders are unified.

## v1.53 — 2026-11-17 (portal-unified) — unify control FILL family (VOX/HD/PTT perceived-size fix, light theme)
User screenshot (light theme) still showed VOX/HD/PTT "different sizes". Re-measured live v1.52 in BOTH
themes: geometry already identical (h30/r6/pad12) — not a lie. But reproduced the user's perception in
light theme: same-size boxes looked different because **fill states differed** (VOX = solid sunken grey,
HD = outline-only on-state, PTT = faded disabled). Visual weight, not geometry, was the remaining gap.
- Unified the resting fill family: `.chip-btn` background sunken → **surface** (same outline family as
  `.btn`); on-state keeps accent-tint + accent border; disabled keeps faded text but identical box.
- Verified light theme: chVox h30 bg white / chCodec h30 bg accent-tint / chLock h30 bg white — same geometry
  AND same outline family; render shows harmonious VOX/HD/PTT; node --check + lint clean.
- Governance: C54 extended — size/harmony audits must also cover **fill/background family and state
  (resting/on/disabled)**, not just geometry.

## v1.52 — 2026-11-17 (portal-unified) — TRUE control-size unification (corrects v1.51's imposed PTT=40 deviation)
User challenged: VOX/HD/PTT still not unified; was the audit incomplete? Honest accounting: v1.51's
*measurements* were correct, but I **imposed a "primary PTT=40" two-step scale** that contradicted the
user's unify requirement, and the audit only checked HEIGHT (not width/radius/padding family). Both were
judgment/coverage errors, now corrected.
- Full re-audit (height+width+border-radius+padding) found: PTT h40/r999/pad20 vs others h30/r6; and
  chip-btn pad12 vs btn-sm pad8 (inconsistent horizontal padding family).
- **Single token family for ALL text buttons**: `.btn/.btn-sm/.chip-btn/.ptt/.act/.btn-live` =
  height `--ctl-h`(30) + padding `0 sp12` + radius `r6` + fs12. PTT keeps accent color + weight 600 to signal
  primary (distinction by color, NOT size). Icon buttons (.p-btn/.iconbtn) remain 30×30 square family.
- Verified: chVox/chCodec/ptt/chLock/chReq/chEmerg all h30 r6 pad12; node --check + lint clean.
- Governance: C54 extended — size audits must cover height+radius+padding (not height alone) and must NOT
  introduce self-imposed scale deviations against an explicit unify requirement.

## v1.51 — 2026-11-17 (portal-unified) — intercom control-size audit: unify to --ctl-h (VOX/HD/PTT/tabs/etc)
User: VOX / HD / PTT and other intercom controls not size-harmonious. Full computed-height audit of the
intercom page (board + all channel tabs) vs --ctl-h:30 found mismatches: `.act` 31, `.ptt` 46, `.ch-tab` 28,
`.vol` 20, `.msg-in` 28 (everything else already 30).
- Unified secondary controls to `--ctl-h`(30): `.act` (board Grant/Revoke/Clear/Prio/Mon, now inline-flex
  centered, padding 0 sp8), `.ch-tab`, `.vol` (listen-volume range), `.msg-in`.
- Primary PTT set to a deliberate **40px** step (hero hold-to-talk target, distinct from 30px secondary),
  and added `.ptt/.act/.ch-tab/.p-btn` to the coarse-pointer ≥44 rule.
- Verified: act 30 / scan 30 / ptt 40 / tab 30 / vox 30 / hd 30 / lock 30 / req 30 / vol 30 / msgin 30 /
  send 30; node --check + lint clean. Two-step scale now: secondary 30, primary 40, coarse ≥44.

## v1.50 — 2026-11-17 (portal-unified) — fix missing --sp10 token (participant row left/right padding was 0)
User: participant list item left/right padding wrong. Measured: `.part-row` computed padding-left/right = **0**
while `.part-head` = 12 → rows flush to the list edges. Root cause: the spacing scale defined
sp2/4/6/8/12/16/20/24 but **no --sp10**; `padding:var(--sp10) var(--sp12)` is an invalid declaration → whole
padding dropped to 0. Two more latent victims of the same missing token: `.combo-opt{padding:0 var(--sp10)}`
(layout options left/right 0) and `.rec-player{gap:var(--sp10)}` (gap 0).
- **Added `--sp10:10px`** to the spacing scale (fills the 8→12 gap), fixing all three invalid declarations at
  once: participant rows now 12px left/right (aligned with the head), combo options regain horizontal
  padding, recordings player regains column gap.
- Verified: part-row padL/padR 12 + leftInset/rightInset 12; combo-opt padding non-zero; rec-player gap 10px;
  node --check + lint clean.

## v1.49 — 2026-11-17 (portal-unified) — participant row spacing/height harmony (C54 token alignment)
User: participant-list buttons/controls/text spacing too tight → disharmonious. Root causes: three
control heights in one row (avatar 30 / buttons 26 / range ~16), a single gap(sp10) for every interval
(no grouping breath), name+role crammed inline (role swallowed by ellipsis), 64px volume squeezed between
buttons, state dot without breathing, tight row padding.
- Unified heights to `--ctl-h`: `.p-btn` 30×30 (icon 14), `.p-vol` 30h × 88w; avatar 30 kept → one row height.
- Grouping: actions (mute/whisper/call) wrapped in `.p-actions{gap:sp4}`; row gap sp12; volume moved before
  the action group; state dot `margin-inline:sp4`; name/role now two lines (b block + role block).
- Row breathing: padding sp10 sp12 + min-height 44 (touch target).
- Verified: rowGap 12 / minH 44 / btnH 30 / volH 30 volW 88 / actGap 4 actN 3 / roleBlock block / mute still
  works; render shows aligned two-line rows + grouped controls; node --check + lint clean.

## v1.48 — 2026-11-17 (portal-unified) — Intercom channel right column: side-sheet + 4 tabs (research-backed)
User: the channel detail right column stacked 9 equal cards (Floor/volume/Replay/Evidence/Mic/Link/
Messages/Lock/Honest) → over-tall, high-frequency features buried, simulated-diagnostic noise.
Research: Motorola WAVE PTX dispatch separates concerns into regions; Discord channel settings uses
TABS (Overview/Permissions/Invites/Integrations); Material side-sheet = supplementary secondary content;
accordions/drawers for low-frequency settings. Flat 9-card stack is an anti-pattern.
- **Reorganized ch-side into a Material side-sheet with 4 tabs**: `Floor` (default; floor control + queue +
  emergency + listen-volume row) / `Recordings` (Replay + Event evidence) / `Comms` (Messages/location +
  directed note) / `Diagnostics` (Mic VOX/NR/AGC + Link SD/HD + Lock-screen condensed into compact rows +
  ONE simulated footnote, replacing 4 cards). Right column now constant one-screen height.
- Tab state `chTab` resets to floor on enter; per-tab wiring null-guarded so switching never throws.
- Verified: tabs=4; floor has chReq; rec has chReplay+chEv and no chReq; diag has chVox+chLock; comms has
  chMsgSend; request-floor works after tab round-trip; four tab renders clean; node --check + lint clean.

## v1.47 — 2026-11-17 (portal-unified) — monitor-center Phase 6: Intercom layered optimization (K1=C/K2=A/K3=A/K5=A/K6=A/K7=B)
Implements PLAN Phase 6 (6.1→6.3a→6.2→6.3b→6.4→6.5→6.6) after the 5-role panel + 6 one-by-one rulings.
- **6.1 floor correctness + honesty + a11y**: ordered priority queue (`g.q`, priority re-queue not preempt)
  with **auto-grant** of queue head on release/revoke/timeout (no dead-end); **hold_timeout=30000 enforced**
  with "Ns left" hint; **hybrid side-talk cap=2** with visible side count; **queue_max=8 enforced**;
  **Prio real toggle** (per-group priority policy, was honest-disabled); queue order/position transparent
  (board tooltip + channel q-order); **keyboard PTT** (Space/Enter on the PTT button) + toast `aria-live`.
- **6.3a replay buffer + consent (P0,前置)**: per-channel rolling buffer (last 20 spurts), consent-gated
  chan-head chip, "● REC buffer / n spurts" indicator, **cleared on exit**; simulated + honest-labelled.
- **6.2 three high-value**: missed-speech **Replay card** (list + Play + empty state, unknown-guarded);
  **multi-group monitor** (Mon toggle, cap 4) + **Scan** (cycle focused channel across monitored, 5s);
  **Emergency insert** = emergency preempt (two-click confirm, preempted-party toast, audit entry).
- **6.3b event evidence**: independent buffer, consent + **7-day retain** + audit link (`floor.evidence`),
  NOT cleared on exit (contrast replay); "event evidence · 7d" indicator.
- **6.4 client increments**: 1:1 duplex **call** (member button + head call bar + End); **temp group** from
  monitored members + Disband; **VOX/NR/AGC/AEC** indicators (simulated, VOX default off); **SD/HD codec** +
  latency/turnaround readout (simulated); **cross-page global Space PTT**; **lock-screen PTT honest-disabled**
  (backend/native-shell gated, K7=B).
- **6.5 supplements**: text **messages** + consent-gated **location** share; **whisper** (directed) +
  **per-member volume**; **encrypted** channel tag (simulated TLS/SRTP).
- Micro-decisions fixed at impl: hybridMax=2, queueMax=8, MON_CAP=4, REPLAY_CAP=20, EV_RETAIN_DAYS=7,
  VOX off, codec HD default, emergency two-click, temp-group from monitored.
- Verified: auto-grant/timeout/hybridCap/queueMax/prio/keyboard/aria-live; replayRow/evBuf/audit/emerg/
  monCap/temp/disband/vox/codec/msg/call/whisper/per-member-vol/lock-disabled/encrypted/cross-page-PTT;
  full-page regression (monitor/recordings/tower/devices/overview/intercom) green; node --check + DOM + lint clean.

## v1.46 — 2026-11-17 (portal-unified) — rename Recordings → Recording (singular, Synology-aligned)
Analysis: Synology names the recording-management/review application **singular "Recording"**
("The Recording application lists … recording files, bookmarks, and their settings"); our page is a
management/review workspace (search/filter/bookmark/export + in-page player), i.e. a singular
workspace like Monitor, not a bare plural file list. User approved removing the "s".
- Renamed user-facing strings: nav label, TITLES, page head ("Recording library"), handoff button
  title/aria ("Open Recording library"), handoff toast. Internal ids/function names (recordings,
  renderRecordings, rec*) unchanged for stability.
- Verified: nav shows "Recording", page/breadcrumb "Recording library"; node --check + lint clean.

## v1.45 — 2026-11-17 (portal-unified) — nav icons: unique semantic monochrome SVG (no glyphs/dupes)
User: sidebar nav icons ugly, style-inconsistent, wrong meaning. Audit: all 16 were Unicode geometric
text glyphs (◈▣▤◉◍▦▲▢≋⚙⚿≣⇄) — mixed fill/outline/line/emoji, platform-variable (⚙ emoji, ⚿ tofu risk),
optically misaligned, ignore currentColor; 3 duplicate pairs (▤ Recordings/Devices, ◍ Meeting/Sessions,
⚙ Config/Settings) and 6+ wrong semantics (Monitor/Intercom/Tower/Remote/Rooms/Overview).
- Replaced all with **unique semantic monochrome SVG sprite** (stroke, currentColor, 18px in 20px box):
  overview=gauge, monitor=2×2 screens, recordings=library-play, intercom=mic, meeting=users,
  remote=desktop, tower=headset, devices=camera, rooms=door, sessions=plug, streams=waveform,
  config=sliders, keys=key, audit=list-check, settings=gear, signaling=exchange.
- `.nav-ico` now a grid-centered SVG container (optical alignment, inherits active/hover color);
  buildNav renders `<svg><use>`; collapsed rail scales cleanly.
- Verified: navSvg/unique(16)/count16/noGlyph assertions green; sidebar render consistent monochrome;
  node --check + lint clean.

## v1.44 — 2026-11-17 (portal-unified) — layout set expanded + icon-thumbnail gallery (Synology-aligned)
User: layout options too few vs Synology and the dropdown used text not icons. Research: Synology
offers 1/4/6/9/10/13/16/25/36/49/64/81/100 (browser cap 36, auto cap 16) via a thumbnail gallery.
- **Expanded templates**: symmetric +25(5×5)/36(6×6); featured +1+9(10ch)/1+12(13ch); kept
  1/4/9/16 + 1+3/1+5/1+7 + Auto + Custom. Auto stays capped at 16 (Synology auto max).
- **Icon-thumbnail gallery**: the layout picker is now a 4-column grid of mini-grid previews
  (featured shows the big cell in accent), grouped Auto | symmetric | featured | Custom with
  separators; selected thumbnail highlighted + check; keyboard nav preserved (options remain
  .combo-opt). Built dynamically from LAYOUTS (renderLayoutPicker/layoutThumb).
- Verified: opts13/thumbs13; 25→5×5, 1+9→8 tiles, 36→6 cols with horizontal scroll; gallery render
  shows grouped icon thumbnails; node --check + lint clean.

## v1.43 — 2026-11-17 (portal-unified) — hide remove button while zoomed (adjudicated A)
User: a zoomed (focused) player should not offer "remove from grid" — removal is a layout-edit action.
- `.tile.zoomed .tile-actions .del{display:none}`; zoomed bar reserve narrowed to 40px (only the
  exit-zoom button remains). Removal stays available in multi-grid (× on hover) and rail footer drag.
- Matches Synology: focused/fullscreen channel offers view actions (exit zoom/PTZ/snapshot), not
  "remove from layout".
- Verified: zoomed del computed display:none, exit-zoom button visible; node --check + lint clean.

## v1.42 — 2026-11-17 (portal-unified) — de-duplicate state: stats panel purely numeric (adjudicated A)
User: the stats header state (LIVE/OFFLINE/DEGRADED) duplicated the bottom-left live badge.
- Removed `.st-head` (state text) from the stats panel and its CSS (base + container + zoomed rules);
  stats now shows only numeric telemetry (FPS/RES/BIT/JIT/LAT/RTT/DEC/LOSS).
- State indication stays with the bottom-left LIVE/OFF badge (primary) + top-bar status dot (glance);
  no triple redundancy.
- Verified: no st-head in source; stats-on render shows numeric-only panel + live badge; node --check
  + lint clean.

## v1.41 — 2026-11-17 (portal-unified) — overlays anchor the whole player, not the inner video (adjudicated A)
User: v1.40 put title/stats/live/actions inside the 16:9 video stage, wasting the letterbox bars and
not putting the title at the player top. Synology anchors OSD/title/controls on the channel FRAME.
- Moved `.tile-bar / .tile-actions / .tile-stats / .tile-live` OUT of `.tile-stage` back to the cell
  (`.tile`): title at player top (over top bar), live at player bottom (over bottom bar), actions
  top-right, stats top-left — letterbox space now used; video stage keeps only media+scan+idle+deadmsg.
- `.tile` gains `container-name:cell`; overlay adaptation uses **per-cell** container queries
  (≤320 hide drift/stats, ≤230 shrink bar/buttons); idle keeps its per-stage query. Aspect still
  guaranteed by the stage (v1.40 core retained).
- Verified: barAtCellTop / liveAtCellBottom / barParentCell / stageRatio 1.78; headless @1440 + featured
  show title/live on the black bars, video letterboxed, no overlap; node --check + lint clean.

## v1.40 — 2026-11-17 (portal-unified) — PlayerTile "little TV" component (adjudicated A; Synology-style)
User: resizing the window made play controls overlap / aspect ratio break. Root cause: overlays were
absolute children of the grid CELL (not a video stage), and cell aspect-ratio is ignored when the grid
constrains both dimensions (featured big span); media filled the cell with no letterbox.
- **Encapsulated PlayerTile** (Synology channel-player model): `.tile` = cell that fills its grid slot
  (any shape, black letterbox, `container-type:size`, place-items:center); inner `.tile-stage` keeps
  **16:9** via `width:min(100cqw, 100cqh*16/9)` and hosts media + ALL overlays (bar/actions/live/stats/
  idle/deadmsg), so resize never distorts the picture or misplaces controls.
- layout-grid / featured rows now `minmax(0,1fr)` (fill wall height); cell aspect:auto height:100%;
  elastic/zoomed keep cell aspect. Removed the elastic/zoomed hover padding hacks (generic 96px reserve).
- **Per-stage container queries** (`@container tile`): ≤320px hide drift+stats; ≤230px hide idle, shrink
  bar/buttons — controls adapt to the video box, never overlap.
- Verified: stageRatio 1.78 / contain:true / bigRatio 1.78 (featured); headless @1440/1100/900 show
  letterboxed 16:9 stages with overlays anchored inside, no overlap; node --check + lint clean.

## v1.39 — 2026-11-16 (portal-unified) — layout dropdown popup polish
User: the layout dropdown's opened popup looked bad. Analysis: right-anchored (hung left over neighbors),
narrow (min-width=trigger), weak elevation (glow shadow invisible on dark wall), cramped 32px rows,
no grouping (Auto/symmetric/featured/Custom mixed), and clip risk on short viewports (canvas overflow).
- Popup now **left-anchored** to the trigger, **min-width 220 / max 280**, **stronger drop shadow**
  (0 8px 24px), r8, sp6 padding, z80; **max-height min(60vh,420px)+overflow-y:auto** (scroll-safe).
- Options: 34px rows, sp10 padding, r6 inset, 2px gap; **group separators** (Auto | templates | Custom).
- Verified: headless opened popup @1440×900 clean/elevated/grouped; @1440×600 not clipped (scroll-safe);
  node --check + lint clean. Keyboard nav skips separators (they are not .combo-opt).

## v1.38 — 2026-11-16 (portal-unified) — unified card-row list language (rail / picker / alerts / rec / board)
User: rail list lacked row/item feel ("white paper"), and the Select-streams dialog's left column was
flat while its right column was card-styled (inconsistent). Analysis found TWO row families: flat
(stream-item / pick-ch / al-row: transparent border, no bg) vs card (pick-row / rec-row / board-row).
- **Unified to one card-row language**: bordered (1px bd) + r6 + sp8 padding + sp2 gap + hover(bd2/hover)
  + selected(accent-tint + accent border); resting rows now have a container signal. Converted the flat
  family (rail stream-item, picker pick-ch, al-row) to cards; aligned card family radius/padding
  (rec-row r8→r6, board-row r10→r6/sp8/sp2). Row bg adapts to container contrast (sunken on surface
  panels, surface on page). Picker device group headers (pick-dev) became distinct sunken bands; removed
  the tree guide line (cards don't need it).
- Governance: child convention **C55** (unified card-row language + resting container signal + verify).
- Verified: railCard resting border/bg non-transparent; picker left channel cards vs right cards same
  language (selected rows accent); headless picker + rail/alerts renders show consistent bordered rows;
  node --check + lint clean.

## v1.37 — 2026-11-16 (portal-unified) — rail list text-first (adjudicated A): drop fake thumb, fix truncation
User: the left rail's black-box thumbnail is unnecessary and stream names/info were truncated.
Analysis: the fake `feedBg` thumb (34px+gap) plus idx/dot/del fixed widths squeezed the flex name to
~118px (~80px at mid rail), ellipsizing names; the thumb carries zero info in a prototype (no real
video) and misplaces "view footage" (a wall job) into the selection list (Synology's tree is text+icon).
- **A implemented**: removed the fake thumbnail and the index; rail rows are now two text lines —
  `device · label` (full width, tooltip fallback) + `res · state` meta line; status dot kept; remove
  button reveals on hover/focus. Freed ~60px so names display in full at 1440 and 1250.
- Verified: noThumb / fullNames(scrollWidth<=clientWidth) / sub "1080p · live" / delHidden:0; headless
  rail renders at 1440 & 1250 show full names + two-line meta; node --check + lint clean.

## v1.36 — 2026-11-16 (portal-unified) — align top status cluster (LIVE badge / clock) to --ctl-h
User: the LIVE badge at the top of the multi-grid had bad sizing. Analysis: the status cluster mixed
three heights/fonts — warn-chip 30px (--ctl-h), live-badge ~25-27px (padding-driven), live-clock fs14
(larger than the fs12 controls) — violating the C54 single control-size token.
- `.live-badge` and `.live-clock` now `height:var(--ctl-h)` (30px), badge padding `0 sp8` + fs11,
  clock fs12 inline-flex centered. Status cluster (warn-chip / LIVE / clock) is uniform height and
  type scale, compact and consistent with the toolbar.
- Verified: computed chip/badge/clock all 30px; headless render shows aligned compact status cluster;
  node --check + lint clean.

## v1.35 — 2026-11-16 (portal-unified) — single layout control (adjudicated E; supersedes K1 fit toggle)
User asked for a more elegant solution to the layout/fit redundancy (rail "Auto-fit layout" button vs
center "Fit: Fixed/Auto" vs Layout combo). Adjudicated **E**: collapse three controls into ONE dropdown.
- **Layout dropdown gains "Auto · by count"**: a template *option* (not a separate mode) that live-picks
  a template by stream count (1/4/1+5/9/16) and re-evaluates as streams are added/removed. Manual entries
  (incl. featured 1+3/1+5/1+7, Custom) remain fixed and respected.
- **Removed the center Fit:Fixed/Auto toggle** and the rail Auto-fit button. Width responsiveness is now
  **built-in**: grid columns use `minmax(230px, …)` so tiles stay readable and the wall horizontal-scrolls
  when the center is too narrow — no user-managed fit mode, no fitMode localStorage, no column-jump mode.
- Left rail keeps stream list + Select streams + drag (only the redundant button removed).
- Verified: residual fit/autoLayout/displayCols/effCols = none; node --check + DOM clean; click-through
  auto8→9 / auto7→9 / auto8b→9 / feat1+5 respected / noFitBtn / noRailAuto / autoOpt; narrow @1000 render
  readable 3-col; featured render big+small.

## v1.34 — 2026-11-16 (portal-unified) — remove persistent bottom message bar (adjudicated C+D)
User discussed whether the multi-grid bottom message bar (canvas-foot: legend + stream counts +
showing/page info) is still needed; adjudicated **C+D hybrid**.
- **Removed** the persistent canvas-foot row from Monitor (reclaims vertical space; wall taller).
  Legend → tooltip on the LIVE badge; showing/page context → tooltip on the pager label; stream-count
  summary folded into the LIVE-badge tooltip. (canvas-foot/.legend CSS kept: shared with Intercom board.)
- **D conditional warning chip**: `#healthChip` in the stage bar appears ONLY when degraded+offline>0
  (`1 degraded · 1 offline`, danger tint), click focuses the first problem stream; hidden when all live.
  Turns "at-a-glance health" into "at-a-glance problems".
- Verified: noFoot/chipVis/legendTip/pgTip/focusProb/hideHealthy assertions green; Intercom board footer
  regression green (shared CSS restored after an over-broad dead-CSS sweep); node --check + lint clean.

## v1.33 — 2026-11-16 (portal-unified) — real hover tooltips + single control-size token (governance fix)
User: icon buttons had no perceptible hover tooltip (native `title` is slow/OS-styled) and button sizes
were inconsistent (34/30/28/26 mixed). Root cause: the design-system skill governs only
`www/packages/ui/tokens.css` (@mediaservo/ui), NOT this single-file prototype; no prototype control-scale
or tooltip discipline existed; ad-hoc px + native title drifted in.
- **Unified hover/focus tooltip**: `showTip(el,text,neutral)` reuses #guardTip (neutral for data-tip,
  warn for data-reason); wireGuardTips now targets `[data-reason],[data-tip]`; `sweepTips()` at init
  converts static button `title`→`data-tip` and removes native title (no double/slow tooltip); dynamic
  titles (fit/carousel/solo/sync-seek) and tile/rail templates write `data-tip`. Immediate, styled,
  viewport-clamped, flips below near top.
- **Single control-size token**: `--ctl-h:30px` applied to ALL interactive controls (iconbtn,
  region-collapse, pin, tile-btn(+zoomed), pager, date-nav, cal-nav, spin, combo-trigger, chip-btn,
  btn-sm, btn-live); coarse-pointer ≥44. Removed scattered 26/28/34 overrides.
- **Governance**: child convention **C54** added (prototype control discipline: single --ctl-h + unified
  data-tip tooltip + icon-only aria-label + computed-size/tooltip assertions each UI change).
- Verified: tooltip pointerover→show/neutral/text + pointerout→hide; all sampled controls computed 30px
  (incl pager/spin/combo); styled tooltip render; node --check + DOM + lint clean.

## v1.32 — 2026-11-16 (portal-unified) — Synology-style toolbar: iconify + group + universal tooltips
User: text buttons (Recordings/Carousel/Fit/Sync/Solo) waste space; all icon buttons need hover tooltips;
group icons like Synology; current layout too arbitrary. Reworked both Monitor bars into clustered
icon groups (Synology "Layout Display & Controls" school):
- **Iconified** text buttons: Fit→i-resize, Sync(drift)→i-swap, stats ⓘ→i-info, Carousel→i-refresh,
  Solo→i-clock, Sync-seek→i-link, Recordings→i-grid (icon-only); pager ‹›/date ‹›/cal ‹›/spin −+ → SVG
  chevrons/minus/plus. New sprites i-plus/i-refresh/i-clock/i-link.
- **Grouped** with `.tb-group` + `.tb-sep`: stage = [panels+fit] | [layout+pager+carousel] | [LIVE+clock]
  … [drift-sync+stats]; ctl = [time+window+date] … [speed] | [sync-seek+solo+LIVE] | [Recordings+collapse].
  Moved the long stream-count meta from stage-bar to canvas-foot so the stage bar is a SINGLE row.
- **Universal tooltips**: every icon/chip button carries title + aria-label (assert allTip:true);
  dynamic titles for fit/carousel/solo/sync-seek states via syncRegionUI/syncSoloUI (no textContent).
- Verified: allTip/noTextBtn/groups assertions green; single-row stage + ctl renders; node --check +
  DOM zero missing + lint clean.

## v1.31 — 2026-11-16 (portal-unified) — remove the two bottom-bar Event buttons (user request)
Removed `evPrev`/`evNext` ("‹ Event"/"Event ›") from the Monitor ctl-bar, plus their `jumpEvent` wiring
and honest-guards. The bottom bar is now a cleaner single row (time/window/date/speed/sync/solo/LIVE/
Recordings/collapse). Event access remains via the right alerts panel (click-to-jump) — no loss of
event navigation capability, just de-duplicated from the transport bar.
- Verified: no evPrev/evNext/jumpEvent in source; node --check clean; bottom-bar render without Event buttons.

## v1.30 — 2026-11-16 (portal-unified) — unify the three region collapse buttons (style parity)
User report: left/right/bottom hide buttons were inconsistent (left/right = `al-collapse` + × icon;
bottom = plain `iconbtn` + ↓ arrow; sizes differed).
- Unified all three to one ghost class `region-collapse` (26px, 14px icon) with a **directional chevron
  family**: left `i-chevron-left`, right `i-chevron` (right), bottom `i-chevron-down` (new sprites).
  Renamed `al-collapse`→`region-collapse`; pin buttons aligned to 26px. Semantics = "collapse toward
  this edge", coherent across the three regions.
- Verified: all three share class + computed 26px + directional icons (cls/size/icons assertions);
  three-panel render shows ‹ / › / ⌄ consistently. node --check + lint clean.

## v1.29 — 2026-11-16 (portal-unified) — remove edge strips (adjudicated B; Synology/VSCode button-toggle school)
User found the collapsed-edge arrow strips ugly/uncoordinated (they crossed the stage-bar/wall/footer
rows, double-bordered, and the bottom strip covered the footer text). Research: Synology toggles panels
via a "show/hide panel" BUTTON (kb liveview_alert); VS Code uses toolbar/Ctrl+B with NO edge handle;
only desktop IDEs (VS/JetBrains auto-hide) use edge TABS, and those are aligned labeled tabs, not our
cross-row surface bars. Adjudicated **B = remove standalone edge strips**.
- Removed railStrip/alertsStrip/tlStrip (HTML/CSS/wiring). Collapse/expand now solely via the centralized
  cluster `[left][right][timeline]` + in-header close (railCollapse/alCollapse) + pin. Bottom reopen via
  tlToggle. Wall edges clean, footer no longer covered, symmetric margins.
- Verified: no `.edge-strip` in DOM; all-collapsed render clean (footer intact, pure live wall); cluster
  reopen left/timeline works (render + assertions noStrips/reopenL/reopenT). node --check + lint clean.

## v1.28 — 2026-11-16 (portal-unified) — fix: left rail close-button parity (narrow drawer could not collapse)
User report: the left rail lacked the right panel's in-header close button, so on narrow screens the
open left drawer (which covers the top toolbar cluster) could not be collapsed.
- Added `railCollapse` (i-close) to the rail head, isomorphic to the alerts `alCollapse`; rail head now
  a flex row (title + spacer + pin + close). Wired to `setRailOpen(false)`; visible at all widths.
- Verified: railCollapse visible; click closes (no-rail) + left edge-strip appears; strip reopens;
  narrow @1150 drawer head shows pin + close. node --check clean.

## v1.27 — 2026-11-16 (portal-unified) — monitor-center Phase 5: responsive rework + unified collapse + UI polish
Implements `docs/plans/monitor-center/PLAN.md` Phase 5 (5.1–5.7) after the §2d/§2e adjudications
(K1/K2 = width-tiered defaults + user-override) and the 4-role panel re-review. Supersedes the rejected
v1.25/v1.26 responsive approach.
- **5.1 unified region controller**: left/right/bottom are isomorphic collapsible regions
  (`setRailOpen/setAlertsOpen/setTimelineOpen`), each with a persistent edge expand-strip; a centralized
  top toolbar cluster `[left][right][timeline]` + Fit toggle; per-region localStorage memory;
  **tiered defaults** (≥1200 left-open/right-closed/bottom-open; <1200 both-collapsed/bottom-open) with
  **user-override priority** and breakpoint open-state carry (classes persist across resize).
- **5.2 Fixed/Auto fit**: tiered default (≥1200 Fixed, <1200 Auto) + override memory; Fixed honors the
  chosen layout with min-tile 230 + horizontal-scroll fallback (no column downgrade); Auto = effCols
  recompute, never writes back the layout.
- **5.3 three-column adaptive grid**: center = total − visible panels; tiles stay 16:9; reflow on any
  region toggle/resize; ≥1200 docked (1201–1500 slim panels).
- **5.4 narrow <1200 drawers**: overlay + edge strips + pin-to-dock button + mutual exclusion (drawer
  mode only); wall keeps full width.
- **5.5 bottom hide = pure live wall**: hides ctl-bar, pauses + switches to live; bottom expand strip;
  expand does not auto-resume; stage-bar keeps LIVE/clock.
- **5.6 UI polish**: transport ⏪⏸⏩ + handoff emoji → monochrome SVG (i-rew/i-pause/i-fwd/i-grid); new
  sprite i-panel-left/right/i-timeline/i-pin; `--ctl-h:30px` control-height token (coarse ≥44, tile-btn
  28); `.tile.dead .tile-idle{display:none}` fixes dead-tile text overlap; idle contrast raised to .45;
  Alerts moved into the cluster (no orphan wrap); Carousel interval spin hidden when off; Sync-seek
  static chip de-buttoned; aria-labels + focus-visible added.
- Verified: node --check + DOM zero missing; headless @1440 default (left-open/right-closed/bottom-open,
  Fit Fixed, cluster, SVG transport, clean dead tile), @1150 default (both-collapsed + edge strips +
  Fit Auto), @1150 drawer-open (overlay + pin, wall full), bottom-hide (ctl hidden + mode live + no
  auto-resume); click-through (defL/defR/defT/fit/stripR/noTl/mode/ctlHidden/backT/noResume/deadIdle/
  rewSvg) all green; regression ic6/chan/tower/dev/rec/mon green.

## v1.26 — 2026-11-16 (portal-unified) — restore the original video-wall look (user: current worse than original)
User judged the v1.25 Monitor style/layout worse than the original. Two regressions identified & fixed:
- **Stretched tiles**: P2.1's fill-height grid made tiles flat wide banners (aspect auto), not video.
  FIX: restore **16:9 tiles**; symmetric layouts (1/4/9/16) return to the original **cols × 2 rows +
  pagination** rhythm with **adaptive column count** (effCols from center width, min tile ~230px);
  asymmetric templates keep their areas but with 16:9 auto rows. Empty slots also 16:9.
- **Drawers at desktop**: v1.25 turned side panels into overlay drawers below 1500px, losing the
  integrated 3-column look at 1440. FIX: integrated columns kept at **≥1200px** (mid 1201–1500 slims
  panels to 210/260 so both-open never squeezes); drawers + mutual-exclusion only **<1200px**.
- Re-render wall on resize (debounced) and on panel toggle so effCols tracks the center width.
- Verified: node --check + DOM clean; headless @1440 default = 16:9 3×2 video wall + pager 1/2;
  @1440 both-open = integrated 3-column with readable 16:9 tiles; @1150 = drawers. (Earlier "panels
  missing" screenshot was the .18s grid-transition mid-frame, not a defect.)

## v1.25 — 2026-11-16 (portal-unified) — responsive audit + fix: side panels no longer squeeze the wall
User report: with BOTH Monitor side panels open the center multi-grid + controls were compressed and
unreadable. A multi-viewport headless audit (1440/1150/950 + other pages) confirmed and enumerated the
size-adaptation defects; all fixed or verified:
- **M1 (critical) Monitor #live-layout**: fixed 248+300px grid columns squeezed the center at 900–1500px
  (1440→~660, 1150→~360, 950→~200). FIX: below 1500px the rail/alerts become **overlay drawers**
  (absolute, shadowed) so the wall keeps FULL width; above 1500px they stay grid columns.
- **M5 Monitor double-drawer cover**: at ≤1100px opening both drawers covered the wall. FIX: narrow-screen
  **mutual exclusion** (opening one closes the other).
- **M2/M3/M4 Monitor bars/timeline/tile text**: narrow center wrapped bars 2–5 rows, timeline scale labels
  collided, tile name/stats overlapped. FIX: **container queries** on .canvas condense gaps, thin/hide
  alternate scale labels, and drop secondary tile text (tile-idle/drift) when narrow.
- **R1 Recordings**: player column narrowed the list at 900–1100. FIX: stack breakpoint raised to 1100.
- **Checked-OK**: tables (.tbl-wrap overflow:auto), Tower/Intercom channel/picker breakpoints already
  adapt at tested widths — no change needed.
- Verified: node --check + DOM clean; headless re-renders at 1440/1150/950 show full-width readable wall
  with drawers overlay (950 = single drawer via mutual exclusion), scale labels no longer collide.

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
