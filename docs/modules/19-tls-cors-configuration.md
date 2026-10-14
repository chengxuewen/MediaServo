# 19. TLS Reverse Proxy & CORS Configuration

> Phase 1 — deployment reference | 2026-07-24
> Related docs: [security-architecture.md](security-architecture.md) §mTLS, [operations.md](operations.md) §TLS, [13-server-architecture.md](13-server-architecture.md) §Deployment
> Related crate: mediaservo-server

## 19.1 Overview

MediaServo Phase 1 uses an **external TLS termination** strategy: the service process itself listens on plain HTTP/WS only; TLS is handled by a fronting reverse proxy (nginx/Caddy/Traefik). This follows operations.md's design decision: "Phase 1: systemd socket activation + external TLS termination (nginx/Caddy)".

This document provides:
- Reverse proxy configuration examples (nginx / Caddy / Traefik)
- Correct CORS middleware configuration in axum
- Notes on WebSocket proxying
- Certificate management (self-signed dev vs Let's Encrypt production)

**Current code status**: `tower-http`'s `cors` feature is declared in Cargo.toml but **not used in any .rs file**. The CORS middleware must be configured explicitly in `main.rs`. The server has no TLS dependency (no rustls/openssl) and listens on plain TCP.

## 19.2 Architecture Overview

```
Client (browser/GUI)
  │
  │ HTTPS (TLS 1.3)  ←── nginx/Caddy/Traefik handles TLS
  ▼
┌─────────────────────────────┐
│  Reverse Proxy              │
│  - TLS termination          │
│  - WebSocket upgrade proxy  │
│  - CORS headers (optional)  │
│  - Rate limiting (optional) │
└───────────┬─────────────────┘
            │ HTTP/1.1 plain (internal)
            ▼
┌─────────────────────────────┐
│  mediaservo-server (port 9800)│
│  - axum HTTP + WS           │
│  - CORS via CorsLayer       │
│  - GovernorLayer (rate lim) │
│  - No TLS built-in          │
└─────────────────────────────┘
```

## 19.3 CORS Configuration

### 19.3.1 Why CORS Is Needed

When the Web UI (React) accesses the Server API from a different origin, the browser sends cross-origin requests. CORS is needed in these scenarios:

| Scenario | Origin A | Origin B | CORS needed |
|----------|----------|----------|-------------|
| Dev environment | `localhost:5173` (Vite dev) | `localhost:9800` (Server) | ✅ yes |
| Production same-domain | `app.example.com` | `app.example.com/api` | ❌ no |
| Production subdomains | `web.example.com` | `api.example.com` | ✅ yes |
| Embedding scenario | `aude.example.com` | `mediaservo.example.com` | ✅ yes |

### 19.3.2 Current Status

`crates/mediaservo-server/Cargo.toml` (line 25):
```toml
tower-http = { version = "0.5", features = ["trace", "cors"] }
```

The `cors` feature is compiled but **unused**. The axum router in `crates/mediaservo-server/src/main.rs` only configures `GovernorLayer`:

```rust
// Current: no CORS
let app = Router::new()
    .nest("/", api_router)
    .layer(GovernorLayer {
        config: Box::leak(governor_config),
    });

// Should become: add CorsLayer
use tower_http::cors::{CorsLayer, AllowOrigin, AllowMethods, AllowHeaders};

let cors = CorsLayer::new()
    .allow_origin(AllowOrigin::exact("http://localhost:5173".parse().unwrap()))
    .allow_methods(AllowMethods::any())
    .allow_headers(vec![
        header::AUTHORIZATION,
        header::CONTENT_TYPE,
    ])
    .allow_credentials(true);

let app = Router::new()
    .nest("/", api_router)
    .layer(cors)
    .layer(GovernorLayer {
        config: Box::leak(governor_config),
    });
```

### 19.3.3 Recommended Configuration

**Dev environment** (permissive):
```rust
use tower_http::cors::{CorsLayer, AllowOrigin};

let cors = CorsLayer::permissive(); // allow all origins
```

**Production** (strict):
```rust
use tower_http::cors::{CorsLayer, AllowOrigin};
use http::header;
use std::str::FromStr;

fn create_cors_layer(allowed_origins: &[&str]) -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            http::Method::GET,
            http::Method::POST,
            http::Method::PUT,
            http::Method::DELETE,
            http::Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
        ])
        .allow_credentials(true)
        .max_age(Duration::from_secs(3600)) // preflight cache
}
```

### 19.3.4 CORS vs the Reverse Proxy

CORS can be handled at **two layers**:

| Layer | Suitability | Recommendation |
|-------|-------------|----------------|
| Reverse proxy (nginx/Caddy) | Simple, external config | Not recommended: WebSocket preflight is complex, cross-domain cookies are painful |
| Application (axum CorsLayer) | Precise, type-safe | **Recommended**: cooperates with JWT auth, handles preflight correctly |

**Recommended strategy**: handle CORS at the application layer (axum); the reverse proxy does only TLS termination + WebSocket proxying, adding no CORS headers. Double CORS headers cause browser errors.

### 19.3.5 Configurable

Add the CORS config to `ServerConfig`:

```rust
// crates/mediaservo-common/src/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    /// Allowed origins (comma-separated)
    /// dev: "*"
    /// production: "https://app.example.com,https://admin.example.com"
    #[serde(default = "default_cors_allowed_origins")]
    pub allowed_origins: Vec<String>,

    /// Whether to allow credentials (cookies, Authorization header)
    #[serde(default = "default_true")]
    pub allow_credentials: bool,
}

fn default_cors_allowed_origins() -> Vec<String> {
    vec!["*".to_string()]
}
```

## 19.4 Reverse Proxy Configuration

### 19.4.1 Port Planning

| Service | Internal port | External port | Protocol | Notes |
|---------|---------------|---------------|----------|-------|
| Server API | `9800` | `443` (HTTPS) | HTTP/1.1 + WS upgrade | TLS terminated by the proxy |
| Host signaling | `9801` | `9801` (direct) | WS | Host-side, usually not browser-facing |
| Client UI | `9101` | not exposed | HTTP | Local GUI |
| TURN/STUN | `3478-3480` | `3478-3480` | UDP+TCP | coturn |
| WebRTC media | `49152-65535` | `49152-65535` | UDP | ephemeral |

### 19.4.2 Nginx

**Minimal configuration** (`/etc/nginx/sites-available/mediaservo`):

```nginx
upstream mediaservo_server {
    server 127.0.0.1:9800;
    keepalive 64;
}

server {
    listen 443 ssl http2;
    server_name mediaservo.example.com;

    # TLS
    ssl_certificate     /etc/letsencrypt/live/mediaservo.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/mediaservo.example.com/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384;
    ssl_prefer_server_ciphers off;
    ssl_session_cache shared:SSL:10m;
    ssl_session_timeout 1d;

    # Security headers
    add_header Strict-Transport-Security "max-age=63072000" always;
    add_header X-Content-Type-Options nosniff;
    add_header X-Frame-Options DENY;

    # API endpoints (no CORS headers — handled by axum)
    location /api/ {
        proxy_pass http://mediaservo_server;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # WebSocket — critical: upgrade headers required
    location /ws {
        proxy_pass http://mediaservo_server;
        proxy_http_version 1.1;

        # WebSocket upgrade
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

        # Long-lived connection settings
        proxy_read_timeout 86400s;
        proxy_send_timeout 86400s;
    }

    # Health check
    location /health {
        proxy_pass http://mediaservo_server;
        proxy_http_version 1.1;
        access_log off;
    }

    # Metrics (internal only in production)
    location /metrics {
        proxy_pass http://mediaservo_server;
        proxy_http_version 1.1;
        # In production, consider an IP allowlist or basic auth:
        # allow 10.0.0.0/8;
        # deny all;
    }
}

# HTTP → HTTPS redirect
server {
    listen 80;
    server_name mediaservo.example.com;
    return 301 https://$server_name$request_uri;
}
```

**WebSocket proxy key points**:
- The `Upgrade` and `Connection` headers must be forwarded
- `proxy_read_timeout` must be long enough (WS is long-lived; the 60s default is insufficient)
- Nginx 1.3+ supports WebSocket proxying natively

### 19.4.3 Caddy

**Minimal configuration** (`/etc/caddy/Caddyfile`):

```caddyfile
mediaservo.example.com {
    # TLS automatically managed by Let's Encrypt

    # API
    handle /api/* {
        reverse_proxy localhost:9800 {
            header_up Host {host}
            header_up X-Forwarded-Proto {scheme}
        }
    }

    # WebSocket
    handle /ws {
        reverse_proxy localhost:9800
    }

    # Health
    handle /health {
        reverse_proxy localhost:9800
    }

    # Metrics (internal-only in production)
    handle /metrics {
        reverse_proxy localhost:9800
    }

    # All other requests
    handle {
        reverse_proxy localhost:9800
    }
}
```

Caddy advantages:
- **Automatic TLS**: no manual certificate configuration; fetches and renews from Let's Encrypt automatically
- **Automatic HTTP→HTTPS redirect**
- **Automatic WebSocket detection**: no special configuration needed
- Concise config; suits small deployments

### 19.4.4 Traefik (Docker Compose)

```yaml
# docker-compose.yml
services:
  traefik:
    image: traefik:v3.0
    command:
      - "--providers.docker=true"
      - "--providers.docker.exposedbydefault=false"
      - "--entrypoints.websecure.address=:443"
      - "--certificatesresolvers.letsencrypt.acme.tlschallenge=true"
      - "--certificatesresolvers.letsencrypt.acme.email=admin@example.com"
      - "--certificatesresolvers.letsencrypt.acme.storage=/letsencrypt/acme.json"
    ports:
      - "443:443"
      - "80:80"
    volumes:
      - "/var/run/docker.sock:/var/run/docker.sock:ro"
      - "./letsencrypt:/letsencrypt"

  mediaservo-server:
    image: mediaservo-server:latest
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.mediaservo.rule=Host(`mediaservo.example.com`)"
      - "traefik.http.routers.mediaservo.entrypoints=websecure"
      - "traefik.http.routers.mediaservo.tls.certresolver=letsencrypt"
      # WebSocket handled automatically by Traefik 2.x+
      - "traefik.http.services.mediaservo.loadbalancer.server.port=9800"
```

## 19.5 Certificate Management

### 19.5.1 Phase 1: Self-Signed Certificates (dev/intranet)

```bash
# Generate a self-signed CA
openssl req -x509 -newkey rsa:4096 -days 365 -nodes \
  -keyout ca-key.pem -out ca-cert.pem \
  -subj "/CN=MediaServo Dev CA"

# Generate the server certificate
openssl req -newkey rsa:4096 -nodes \
  -keyout server-key.pem -out server-req.pem \
  -subj "/CN=mediaservo.example.com"

# Sign with the CA
openssl x509 -req -in server-req.pem -days 90 \
  -CA ca-cert.pem -CAkey ca-key.pem -CAcreateserial \
  -out server-cert.pem
```

Security policy:
- Private key mode `0600`
- Certificate valid 90 days
- In dev, the CA certificate can be imported into the system trust store

### 19.5.2 Phase 2: Let's Encrypt (production)

| Tool | Integration | Suitable for |
|------|-------------|--------------|
| certbot (nginx) | `certbot --nginx` auto-config | Bare-metal nginx |
| Caddy built-in | Automatic fetch + renewal, zero config | Small deployments |
| Traefik ACME | Docker labels | Docker deployments |
| rustls-acme (Phase 3) | Built-in ACME client | Handled by MediaServo itself |

Certificate policy:
- 90-day validity, auto-renewal (triggered with 30 days remaining)
- `EC secp256r1` (ECDSA P-256) keys
- Private key file mode `0600`

## 19.6 Full Docker Compose Example

```yaml
# docker-compose.prod.yml
version: '3.8'

services:
  # === Reverse Proxy (Caddy) ===
  caddy:
    image: caddy:2
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
    restart: unless-stopped

  # === MediaServo Server ===
  server:
    image: mediaservo-server:latest
    environment:
      - MEDIASERVO_SERVER_HOST=0.0.0.0
      - MEDIASERVO_SERVER_PORT=9800
      - MEDIASERVO_JWT_SECRET=${JWT_SECRET}
      - MEDIASERVO_CORS_ORIGINS=https://mediaservo.example.com
      - RUST_LOG=info,mediaservo_server=debug
    expose:
      - "9800"
    volumes:
      - ./data:/var/lib/mediaservo
      - ./config/server.conf:/etc/mediaservo/server.conf:ro
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9800/health"]
      interval: 30s
      timeout: 5s
      retries: 3

  # === Monitoring ===
  prometheus:
    image: prom/prometheus
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml:ro
      - prometheus_data:/prometheus
    expose:
      - "9090"
    restart: unless-stopped

  grafana:
    image: grafana/grafana
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=${GRAFANA_PASSWORD}
    ports:
      - "3000:3000"
    volumes:
      - grafana_data:/var/lib/grafana
    restart: unless-stopped

volumes:
  caddy_data:
  caddy_config:
  prometheus_data:
  grafana_data:
```

### Caddyfile

```caddyfile
mediaservo.example.com {
    # API routes
    handle /api/* {
        reverse_proxy server:9800 {
            header_up Host {host}
            header_up X-Forwarded-Proto https
        }
    }

    # WebSocket
    handle /ws {
        reverse_proxy server:9800
    }

    # Health & metrics
    handle /health {
        reverse_proxy server:9800
    }

    handle /metrics {
        @internal remote_ip 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16
        handle @internal {
            reverse_proxy server:9800
        }
        respond "Forbidden" 403
    }

    # Catch-all
    handle {
        reverse_proxy server:9800
    }
}
```

## 19.7 Phase Evolution

| Phase | TLS strategy | CORS strategy | Certificate management |
|-------|--------------|---------------|------------------------|
| Phase 1 | External termination (nginx/Caddy) | axum CorsLayer | Self-signed / manual Let's Encrypt |
| Phase 2 | External termination + SFU DTLS | axum CorsLayer + config | certbot/Caddy auto-renewal |
| Phase 3 | Built-in rustls (optional) | axum CorsLayer (kept) | rustls-acme integration |
| Phase 4 | mTLS + built-in rustls | Kept | Built-in ACME client |

## 19.8 Checklist

Verify before deployment:

- [ ] TLS certificate within validity (`openssl s_client -connect host:443`)
- [ ] HTTP → HTTPS redirect works
- [ ] WebSocket connection can upgrade (browser DevTools → Network → WS)
- [ ] CORS preflight passes (OPTIONS returns 200)
- [ ] `/health` endpoint reachable behind the proxy
- [ ] `/metrics` endpoint protected in production
- [ ] Security headers set (HSTS, X-Content-Type-Options)
- [ ] Private key file mode is 0600
- [ ] Certificate auto-renewal cron/systemd timer configured

## 19.9 Cross References

- Auth architecture: [security-architecture.md](security-architecture.md)
- Ops design: [operations.md](operations.md)
- Server architecture: [13-server-architecture.md](13-server-architecture.md)
- Deployment modes: [02-deployment-modes.md](02-deployment-modes.md)
- Signaling architecture: [10-signaling-architecture.md]
