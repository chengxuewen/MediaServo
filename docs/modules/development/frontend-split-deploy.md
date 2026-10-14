# Frontend-Split Deployment & Operations Manual (frontend-process-split end state)

> 2026-08-31 | Related: D262/D268, C24 revision, PIT-163~170 | Plan archive `docs/plans/_archive/frontend-process-split/`

## 1. Topology at a Glance

```
Browser ──HTTPS──▶ caddy (web layer) ──┬── /            static out/server/web (vite dist)
                                       └── /api /ws etc. reverse-proxy 127.0.0.1:9800 → server instance
host-agent ──/ws direct to 9800 (D-2=B phase-one posture, bypasses caddy)
Media plane RTP/UDP direct to server (unaffected by the web layer; not proxyable)
```

Three modes: ① native split (main path) ② single-container all-in-one (embedded, `--image runtime`) ③ compose dev (vite is the frontend).

## 2. Building

```bash
./msrtc.sh build server        # Backend (non-embedded variant) + auto-assembly of out/server/{bin,etc,web}
./msrtc.sh build web           # Refresh only out/server/web (fast path for TS edits, seconds, no cargo)
./msrtc.sh build server --image runtime   # Mode ② image (Dockerfile embeds admin-dashboard explicitly)
```

A one-line TS change → `build web` → browser refresh takes effect (no Rust recompile/restart — C24 narrowed to mode ②).

## 3. Deployment (stateful landing, isomorphic to D266)

```bash
export MEDIASERVO_ADMIN_PASSWORD='***'      # Required on first production start (dev-template accounts fail fast on bare metal, C35 guard)
./msrtc.sh deploy server --prefix /opt/msrtc-server      # /opt needs root; idempotent re-runs preserve etc credentials (PIT-160)
# Landed tree: <prefix>/{bin/<brand>-server,bin/oxmgr,etc/*,web/,run/{oxfile.toml,oxmgr/,logs/},<brand>-server→bin/ (D269 branded physical name + root-level shortcut)}
<prefix>/<brand>-server startup on /opt/msrtc-server  # Boot anchor (systemd user unit → oxmgr daemon)
loginctl enable-linger                                   # Recommended: survives without a login session
<prefix>/<brand>-server status /opt/msrtc-server      # 0=healthy 1=degraded 2=not running (script-consumable)
<prefix>/<brand>-server doctor /opt/msrtc-server      # Exit code = failure count (PATH/ports/yaml/web dist/announced IP)
```

Non-default ports: after changing `etc/server.yaml listen.port`, **manually sync** the `etc/Caddyfile`
`:8080` site and the `127.0.0.1:<port>` upstream, plus `run/oxfile.toml` `[apps.env]` (`MEDIASERVO_SFU_PORT`
etc.) — init rendering is a one-time snapshot (improvement item: `init --port`).

## 4. Day-to-Day Ops (instance command surface = same as msrtc-host)

```bash
<brand>-server start [dir] [--no-web]   # Whole oxmgr cluster / backend only; interactive takeover on port conflict; caddy missing → auto degrade
<brand>-server stop|restart [dir]       # Full cluster convergence (idempotent; operates only on this instance's daemon by scoped address)
<brand>-server status [dir]             # server/web two lines + /ready probe column
<brand>-server logs [server|web|all] [dir]
<brand>-server monit|ps [dir]           # oxmgr TUI / resource columns
```

The Python CLI's `run/start/stop/restart server` are retired → guidance (C39); `run web` remains transitional.

## 5. Troubleshooting

| Symptom | First check | Reference |
|---------|-------------|-----------|
| /ready 503 but /health 200 | mediasoup worker thread dead (a C++ crash = dead process gets restarted by oxmgr; a graceful thread death needs `restart`) | PIT-167/165 |
| Signaling but no picture | sfu/rooms producer count → streamer logs → the C38 three-layer chain; note **growing bytes_sent ≠ healthy link** | PIT-168 |
| Browser 502 | caddy up but backend dead (`status`) | — |
| host cluster missing after deploy re-run | Never hand-run `oxmgr stop`; the deploy guard is inode-precise | PIT-170/166 |
| Account produce not rejected | The psk-unset + JWT-set combination bypass is fixed; regression via e2e_sfu | PIT-163 |

## 6. Rollback

- Web layer: after `stop`, return to the old `run server` posture = run the **embedded variant** binary directly
  (`cargo build -p mediaservo-server --features admin-dashboard`) → the SPA appears at `:9800/admin`;
  keep the root Caddyfile (full passthrough) as-is and it serves as the mode ② ingress.
- Whole track: mode ② images unchanged (this change never touched their semantics; only the build method was fixed, PIT-169).
