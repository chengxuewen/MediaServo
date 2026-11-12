# Board W1 — Portal shell (structure layer)

> Surface: web · Theme: light+dark · Density: comfortable · Refs: web-portal-ui O4/U2/U3/W3/T1/P5/U5/O3/P8
> Appearance carrier: `../prototypes/w1-portal-shell.html` (v0.2). This file = structure (what/where/order).
> Symbols: ● held/online ○ idle ◐ queued ▲ warn/unknown ■ solid-badge □ outline-badge ⟨⟩ collapse ↗ deep-link

## Shell (O4 collapsible sidebar)
```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ◆ MediaServo PORTAL            ● Connected·3ms   ◐theme  🔔  [DX] dispatch.x │
├───────────────┬──────────────────────────────────────────────────────────────┤
│ NAVIGATION  ⟨ │                                                              │
│ SURVEILLANCE  │                                                              │
│  ▣ Live     8 │                                                              │
│  ▤ Playback   │            ( active screen content )                         │
│ COMMUNICATION │                                                              │
│  ◉ Intercom 2 │                                                              │
│  ◍ Meeting    │                                                              │
│ CONTROL       │                                                              │
│  ▦ Remote     │                                                              │
│  ▲ Tower    3 │                                                              │
│ ADMIN         │                                                              │
│  ⚙ Settings   │                                                              │
│  ▤ Devices    │                                                              │
│  ▢ Rooms      │                                                              │
│  ◍ Accounts   │                                                              │
└───────────────┴──────────────────────────────────────────────────────────────┘
```
- `⟨/⟩` manual collapse → icon rail; **Tower auto-collapses** (O4). Badges = live/queue/alert counts.

## Live wall (U3 / W3)
```
│ Live view          [2×2|3×2|3×3]                 Sync[on]        [LIVE]      │
│ ⚠ Recording backend not landed — timeline disabled (honesty bar)            │
│ ┌───────────────────────────┐ ┌───────────────────────────┐                │
│ │ CAM-01 · Gate A    ■LIVE  │ │ CAM-03 · Dock   link 2/3  │← degraded tile │
│ │ 1920×1080·25fps           │ │ rtt 84ms · 1.2% loss      │  (3-seg bar)   │
│ │ 12:04:21      link 3/3    │ │                           │                │
│ └───────────────────────────┘ └───────────────────────────┘                │
│ ┌───────────────────────────┐ ┌───────────────────────────┐                │
│ │ CAM-02 · Yard      ■LIVE  │ │ CAM-04 · Perimeter        │                │
│ │                           │ │ ○ SIGNAL LOST (offline)   │                │
│ └───────────────────────────┘ └───────────────────────────┘                │
│ [⏪][⏸][]  -60m───-45m───-30m───-15m───now  ▮playhead   LIVE (disabled)   │
```
- LIVE solid■ / playback outline□; Sync locks common axis; timeline disabled until recording backend.

## Intercom board (T1 / §6 / D7)
```
│ Intercom board · 6 groups                    capability:floor✓   Cues[on]   │
│ ⚠ (on WS drop) floor state UNKNOWN, not idle. Reconnecting…                 │
│ GROUP              MODE     FLOOR            TIMER   QUEUE   ACTIONS        │
│ Alpha · Gate crew  [EXCL]   ● Held field.03  00:42   Q:2    [Grant][Revoke] │
│ Bravo · Yard ops   [EXCL]   ◐ Queued         --:--   Q:3    [Mute][Prio]    │
│ Charlie · Dock     [HYBR]   ● Held field.11  00:17   Q:0                    │
│ Delta · Perimeter  [EXCL]   ○ Idle           --:--   Q:0                    │
│ ●Held ◐Queued ○Idle ▲Unknown(stale)   Hold-to-talk·Space·Esc-long-press exit│
```
- Floor lamp 4 states; **disconnect → UNKNOWN not idle**; actions = real wire set; viewer has none.

## Tower (P5 / O1=C)
```
│ Tower · multi-panel                                  [sidebar auto-fold]    │
│ ┌─────────────────────────────┐ ┌───────────────────────────────────────┐  │
│ │ LIVE WALL · 4-up            │ │ INTERCOM · 2 held                     │  │
│ │ [CAM-01■][CAM-02■]          │ │ ● Alpha 00:42  ◐ Bravo queued         │  │
│ │ [CAM-03■][CAM-04○LOST]      │ │ ● Charlie 00:17  ○ Delta idle         │  │
│ └─────────────────────────────┘ └───────────────────────────────────────┘  │
│ ┌─────────────────────────────────────────────────────────────────────┐    │
│ │ ALERTS & PRESENCE · ▲CAM-04 lost  ◐Bravo queue3  ℹfield.11 joined    │    │
│ └─────────────────────────────────────────────────────────────────────┘    │
```
- Role-gated (dispatcher+admin); panels = shared components; composes view+talk+alerts.

## Settings (U5 / O3=B)
```
│ Settings · read-only index · deep-link to in-place edit                     │
│ [ Search settings across all apps… ]                             11 items   │
│ KEY                   VALUE        SCOPE           OWNER   EDIT             │
│ ─ INTERCOM ─ floor.mode exclusive  room:talk-alpha talk    edit ↗           │
│ ─ SURVEILLANCE ─ view.grid_default 2x2 user         view    edit ↗           │
│ ─ REMOTE ─ desk.lease_renew_ms 300 global           desk    edit ↗           │
│ ─ ACCESS ─ auth.token_ttl_h 24     global           admin   edit ↗           │
```
- Index only; `edit↗` deep-links to in-place edit; single server-side source of truth.

## open_questions
- Sidebar width 232px vs workbudy research floor 220px — within range, confirm at B1.
- Light-theme video-wall chrome: letterbox stays dark; confirm tile HUD legibility on light at B1.
