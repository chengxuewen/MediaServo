# Ported board — Admin rooms & fleet console (workbudy W2)

> Surface: web (admin) · Theme: light · Status: **ported, reference-complete** (not a per-board review item)
> Carrier: `../prototypes/port-admin-console.html` · Source: `.refinfo/workbudy/docs/ui/prototypes/w2-admin-console.html`

## Sidecar
```yaml
board: port-admin-console
origin: workbudy w2-admin-console.html (65 KB, single-file, offline)
surface: web-admin
theme: [light]
status: ported
views: [overview, rooms, sessions, devices, streams, fleet, config-push, client-keys, audit-log, settings, weaknet]
e2e_hooks: ["[data-action='room-tab']", "[data-action='cfg-tab']", ".ms-table", ".banner"]
adaptations:
  - "Cockpit -> Tower (5 refs incl. browser/Tauri delivery forms, web-portal-ui P5)"
  - "title -> 'rooms & fleet console (workbudy W2 port)' to avoid W1/W2 numbering collision"
```

## What it covers
The admin/operations console: **rooms, clients and devices as first-class objects**, with planned
affordances drawn dashed and hideable via the header toggle. Views: Overview / Rooms (by kind) /
Sessions & clients / Devices / Streams / Fleet, plus Config push, Client keys, Audit log, Settings,
WeakNet. Carries workbudy's honest "why this is being replaced" banners (e.g. the 16-tile Dashboard
violating the ≤4-tile Web cap) — useful governance context for our admin IA (W5+).

## Relationship to the focused boards
- Reference-complete admin surface; complements `port-web-shell.html` (operations) with the
  rooms/fleet/device management depth.
- Our focused W5 (Settings) and admin IA boards will draw from this port.
- Verified: inline JS passes `node --check`; no external/CDN/font/image deps (fully offline); HTTP 200.
