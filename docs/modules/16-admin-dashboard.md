# 16. Admin Dashboard

> Status: Phase 4 design | Related decisions: D87, D136 Phase 1b | Phase label: plan.md Phase 4 (formerly D136 Phase 1b)

## Architecture Overview

React 19 + Ant Design 5 SPA, embedded into the Server binary via rust-embed.

```
┌──────────────────────────────────────┐
│  Server                              │
│  ┌────────────────────────────────┐  │
│  │  Gateway Component (:9800)     │  │
│  │  /admin/api/* → REST → Admin   │  │
│  │  /admin/*      → SPA fallback  │  │
│  │  /health       → Monitor       │  │
│  └────────────────────────────────┘  │
│              │                       │
│  ┌───────────▼────────────────────┐  │
│  │  rust-embed: admin-ui/dist/    │  │
│  └────────────────────────────────┘  │
└──────────────────────────────────────┘
```

## Technology Stack

| Layer | Choice | Notes |
|-------|--------|-------|
| Framework | React 19 | Confirmed by D87 |
| Component library | Ant Design 5 | Enterprise admin-console standard |
| Build | Vite | Fast HMR + tree-shaking |
| State management | React Query (TanStack) | SWR pattern, automatic cache invalidation |
| Routing | React Router v7 | SPA routing |
| Embedding | rust-embed | Compile-time embedding of dist/ |
| Auth | JWT Bearer + Basic Auth fallback | D88 RBAC |

## Route Design

| Path | Component | Permission |
|------|-----------|------------|
| `/` | Dashboard | read |
| `/rooms` | Rooms (session list) | read |
| `/rooms/:id` | RoomDetail (session detail) | read |
| `/sessions` | SessionLog (event log) | read |
| `/settings` | Settings | write |
| `/login` | LoginPage | — |

## Admin API Contract (JSON)

### GET /admin/api/dashboard
```json
{ "active_rooms": 12, "connected_peers": 34, "cpu_pct": 45.2, "mem_mb": 256 }
```

### GET /admin/api/rooms
```json
{ "rooms": [{ "id": "...", "name": "Vehicle-01", "peers": 2, "status": "active" }] }
```

### POST /admin/api/auth/login
```json
{ "username": "admin", "password": "..." }
→ { "token": "eyJ...", "expires_at": "..." }
```

## Auth Guard Flow

```
User → LoginPage → POST /admin/api/auth/login → JWT token
     → Dashboard → React Router guard → validate JWT → render
     → 401 → redirect to /login
```

## rust-embed Integration

- Vite build → `admin-ui/dist/`
- Cargo build: the `rust-embed` macro embeds dist/ into the binary
- Gateway C7: `admin/*` routes → static file fallback (index.html)

## Phase Dependencies

| Dependency | Phase | Notes |
|------------|-------|-------|
| Gateway Component (C7) | Phase 3 | Routing + JWT middleware |
| AuthComponent (C8) | Phase 3 | login/validate API |
| MonitorComponent | Phase 3 | /health + stats |
| SQLite (D89) | Phase 3 | User/session persistence |

## Implementation Plan (plan.md Phase 4)

- A1: Vite + React scaffolding
- A2: Routing + Auth Guard
- A3: Ant Design layout
- A4: Dashboard API
- A5: Rooms page
- A6: Sessions page
- A7: Settings page
