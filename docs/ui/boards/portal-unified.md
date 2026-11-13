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
