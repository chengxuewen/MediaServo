# SDD 05: Host Web Config

## 1. Overview

Embedded web configuration page inside the Host. Configure the Host device locally via a browser, no GUI needed.

**Decision refs**: D81 (embedded web), D84 (SSE push), D85 (HTTP Basic Auth), D114 (host.conf)

## 2. Interface Definition

```rust
pub struct WebConfig {
    pub bind: SocketAddr,         // Default 127.0.0.1:9800
    pub username: String,
    pub password: String,         // Stored as SHA256 hash
}

pub enum WebRoute {
    #[get("/health")]
    Health,
    #[get("/ready")]
    Ready,
    #[get("/")]                   // Config page (HTML)
    Index,
    #[post("/api/config")]        // Submit configuration
    UpdateConfig,
    #[get("/api/status")]         // SSE live status
    StatusStream,
}
```

### Host Config Schema (D114)

```yaml
# /opt/mediaservo/etc/host.conf
host:
  id: "host-001"
signaling:
  ws_url: "ws://server:8080/ws"
media:
  camera: "/dev/video0"
  width: 1280
  height: 720
  fps: 30
  bitrate_kbps: 2000
  encoder: "nvh264enc"
web:
  bind: "127.0.0.1:9800"
  username: "admin"
  password: "changeme"
turn:
  urls: "turn:server:3478"
  username: "user"
  credential: "pass"
```

## 3. Technology Stack

| Component | Choice | Notes |
|-----------|--------|-------|
| HTTP framework | axum | HTML embedded via `include_str!` |
| Frontend | Vanilla JS | ~100KB, no framework dependency |
| Live push | SSE | Pushes CPU/GPU/fps/bitrate/rtt every second |
| Authentication | HTTP Basic Auth | Native browser support |
| Config read/write | serde_yaml | Persisted to host.conf |

## 4. Security

| Protection | Mechanism |
|------------|-----------|
| Bind address | Default 127.0.0.1 (local only) |
| LAN access | Set `web.bind: "0.0.0.0:9800"` + password |
| Authentication | HTTP Basic Auth, SHA256 hash comparison |
| CSRF | Stateless API (not cookie-based); SameSite not a concern |

## 5. SSE Status Push

```
Event: status
Data: {"fps":29.5,"bitrate":2048,"rtt":45,"gpu_util":67,"camera":"ok","encoder":"ok"}
```

Pushed every second; the browser config page displays it live.

## 6. Error Handling

| Condition | Category | Response |
|-----------|----------|----------|
| Malformed config | Recoverable | 400 Bad Request + offending field flagged |
| Authentication failure | Recoverable | 401 Unauthorized |
| Config write failure | Fatal | 500 + log entry, old config preserved |
| Missing required field | Recoverable | 400 + list of missing fields |

## 7. Test Plan

| Test | Type | Description |
|------|------|-------------|
| config_parse_valid | Unit | Complete host.conf parses into WebConfig |
| config_parse_invalid | Unit | Missing field / wrong type returns error |
| auth_success | Integration | Correct credentials return 200 |
| auth_failure | Integration | Wrong credentials return 401 |
| auth_bypass | Security | Missing Authorization header returns 401 |
| sse_push | Integration | status event received every second on SSE connection |
| config_update_persist | Integration | host.conf file updated after POST config |
| health_ready_endpoints | Integration | /health /ready return 200 |
