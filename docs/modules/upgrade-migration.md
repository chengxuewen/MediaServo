# Upgrade & Migration

> Status: pre-Phase 3 design | Related decisions: D118 (version compatibility), D-OPS-10 (Host upgrade) | Created per doc-audit H8

## Version Compatibility Matrix (D118)

```
Host/Server: MAJOR.minor.patch
  - Different MAJOR = incompatible, must upgrade in sync
  - minor/patch can roll independently

Remote (Client): MAJOR.minor.patch
  - Backward-compatible with the previous MAJOR (i.e. a v2 Remote can connect to a v1 Server)
  - Recommended to upgrade once the current MAJOR matches the Server
```

## Crate Version Strategy (doc-audit L7)

> The 7-crate workspace is consumed by Rust static-link consumers and napi-binding consumers (third-party platforms / host platforms), requiring a crate-level version strategy independent of the binary matrix.

### Tiered API Surface

| Layer | crates | Change semantics |
|-------|--------|------------------|
| Contract layer | mediaservo-common (protocol/auth/config) | **Any wire-format change = breaking for all consumers** |
| Capability layer | mediaservo-media / mediaservo-webrtc / mediaservo-codec | Feature-gated capabilities (see the three-backend matrix) |
| Application layer | mediaservo-host / mediaservo-client / mediaservo-server | Follows the D118 binary matrix |

### Rules

1. **Independent semver**: 7 crates versioned individually; breaking changes allowed during 0.1.x (minor increments); strict semver after 1.0. `version.workspace = true` applies only to crates synchronized in release cadence (currently media/codec); full-workspace sync is not enforced — mediaservo-common 0.2 need not drag server to 0.2.
2. **mediaservo-webrtc three-backend feature matrix** (C12): stub/webrtc-rs/webrtc-sys are feature-level compatibility; a new backend = minor, a backend behavior change = major.
3. **MSRV**: pinned in rust-toolchain.toml; [planned] CI adds a `cargo-semver-checks` gate (detects public API breaking changes on PRs).
4. **Consumer pinning**: third-party platforms pin `=x.y.z` or a commit reference; host platforms (napi) have their ABI surface determined by the mediaservo-common version, recorded at the same version on release.
5. **Change notification**: breaking changes recorded in both decisions.md + CHANGELOG ([planned] cargo release or per-crate changelog generation).
## Config Migration

host.conf uses a JSON Schema with a `version` field:

```json
{
  "version": 2,
  "video": { "codec": "h265", "bitrate_kbps": 4000 }
}
```

- On startup, read `version` and run the migration chain: v1→v2→v3→...
- Each migration step is a pure function `fn migrate(config: Value) -> Value`
- serde `#[serde(default)]` means new fields need no manual backfill

## Binary Upgrade (Host, D-OPS-10)

systemd service control:

```
1. systemctl stop mediaservo-host
2. Replace /usr/local/bin/mediaservo-host
3. systemctl start mediaservo-host
```

On startup the Host checks the `version` field and runs the schema migration automatically. Remotes receive update packages pushed by `mediaservo-server`.

## Container Image Upgrade (Server, Docker)

> Related: C13 (unified Server Docker build), D208 (build optimization), PIT-39 (smoke gate) | Created per doc-audit M6

The Server is the only Docker-deployed component (C13); images are published by CI with no manual build/upload flow.

### Image tag convention (docker.yml)

| Tag | Meaning | Updated |
|-----|---------|---------|
| `ghcr.io/org/mediaservo-server:latest` | Rolling latest | Every main push |
| `ghcr.io/org/mediaservo-server:sha-<commit>` | Exact version, rollback anchor | Every main push |

CI (`.github/workflows/docker.yml`) flow: push main → build the runtime stage → tag both → **smoke gate** (run the image, `/health` must return 200 within 30s, else the push fails — PIT-39).

### Upgrade Steps

```
1. docker compose pull                       # Pull the new latest
2. docker compose up -d                      # Rolling rebuild of server (proxy unchanged)
3. docker compose ps | grep healthy          # Wait for the healthcheck to pass
4. curl -sf http://localhost:9800/health     # Final confirmation
```

Health-check gates: the compose healthcheck (30s interval / 3s timeout / 3 retries, `docker-compose.yml:18-22`) and the Dockerfile HEALTHCHECK both exist; compose does not auto-rollback an unhealthy container — manual intervention required.

### Rollback

```
docker compose pull                          # Or skip: pin the old tag directly
docker compose up -d ghcr.io/org/mediaservo-server:sha-<old-commit>
docker compose up -d                         # Restore the compose file's default latest
```

Retention policy: `sha-` tags accumulate with main pushes; clean old tags every N releases (manual deletion on ghcr).

### Config & Data Migration

- Config: `./config/server.docker.yaml` mounted read-only (`docker-compose.yml:24`); image upgrades never touch config; new fields are compatible via serde `#[serde(default)]` (see §Config Migration above).
- Database: sqlx migrate runs automatically at startup (see §Database Migration below); back up `mediaservo.db` before upgrading (operations.md §Backup).
- Version compatibility: follow the §Version Compatibility Matrix (D118) — different MAJORs must upgrade Host/Remote in sync.

### Differences vs Host Upgrade

| Dimension | Host (D-OPS-10) | Server (Docker) |
|-----------|-----------------|-----------------|
| Distribution | Server WS pushes a tarball + SHA256 verification | CI pushes a ghcr image |
| Execution | systemctl stop/start | compose pull + up -d |
| Verification | GET /health → 200 | Smoke gate + compose healthcheck |
| Rollback | Restore the old binary | Switch back to the `sha-<commit>` tag |
| Auto rollback | Phase 2 (health check failing for 60s) | Not implemented; manual rollback |
## Database Migration (Server)

sqlx migrate manages the SQLite schema:

```
migrations/
  20260701000001_init.sql
  20260715000002_add_room_config.sql
```

The Server automatically applies unapplied migrations at startup. In production, back up `mediaservo.db` first.

## Feature Flag Migration

New features roll out behind feature flags:

```toml
[features]
default = ["webrtc"]
webrtc = []
h265 = []        # Phase 2
srttransport = []# Phase 3
```

Phase switch policy: flag off by default for 2 releases → beta defaults on → stable defaults on and the flag is removed.

## Runtime State Migration

### Zero-Downtime Deployment

#### Blue-Green Deployment (single instance)

```
Current instance (blue)             New instance (green)
├── Health check: OK                ├── Startup complete
├── Active connections: 50 peers    ├── Health check: OK
│                                   ├── Begins accepting new connections
├── Receives SIGTERM                │
├── Stops accepting new connections ├── Active connections: 12 peers
├── Drain: waits for existing       │
│   connections to end              │
├── Active: 0 → process exits       │
└── blue instance removed           └── green takes over all traffic
```

#### Connection Draining

| Stage | Time | Action |
|-------|------|--------|
| SIGTERM received | 0s | Stop accepting new WS connections; health check returns 503 |
| Draining | 0-30s | Existing connections continue; Hosts renegotiate to the new instance |
| Timeout | 30s | Force-close all connections; process exits |
| Failure rollback | 60s | New instance health check fails → systemd rolls back to the old version |

#### SFU State Migration

| Component | Migration approach |
|-----------|--------------------|
| mediasoup Router | Recreated on the new instance (not migrated) |
| Active transports | Held during drain; Hosts reconnect to the new instance |
| Producers/consumers | Not migrated; Hosts re-produce, browsers re-consume |
| Session state | JSON file persisted; loaded by the new instance |

#### Rolling Update (multi-instance)

```
Instance 1: blue → green (drain 30s)
Instance 2: blue → green (starts after instance 1's green is ready)
Instance 3: blue → green (starts after instance 2's green is ready)
```

- Parallelism: at most 1 instance upgrades at a time (guarantees minimum capacity)
- Health check: a new instance must pass `/health` before the next one upgrades
- Rollback: any instance's health check failing → stop rolling, roll back the current instance

### Graceful Drain
- SIGTERM → stop accepting new connections → wait for existing connections to end naturally (max 30s) → force-close on timeout.

### WebRTC Connections
- Held alive during drain; peer liveness detected via ICE consent freshness.


### Auto Rollback (Phase 2)
- If the health check fails within 60s of the new binary starting → systemd rolls back to the old version.

> Implementation complete (Phase 3)
