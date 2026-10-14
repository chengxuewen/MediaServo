# Security Architecture

> Status: pre-Phase 4 design | Consolidates: D116 (STRIDE-Lite) + D-SEC-01 (mTLS+TLS+audit) + D117 (E-Stop) + D130 (AuthProvider) | Created per doc-audit CR5

## Threat Model (STRIDE-Lite, D116)

| Category | Threat | Mitigation |
|----------|--------|------------|
| Spoofing | Peer identity forgery | JWT + mTLS |
| Tampering | Control command tampering | RTCDataChannel HMAC |
| Repudiation | Action denial | Audit logs |
| Info Disclosure | Signaling eavesdropping | TLS 1.3 |
| DoS | Signaling flooding | Rate limiting |
| Elevation | Privilege escalation | D88 RBAC |

## Authentication Architecture

```
Client → JWT (from AuthProvider login) → WebSocket upgrade
       → Token in HTTP header: Authorization: Bearer <jwt>
       → AuthComponent validate(token) → User + Permissions
```

## JWT Token Lifecycle

| Stage | Description | Config |
|-------|-------------|--------|
| Issuance | AuthComponent.login() → JWT | exp: 24h |
| Validation | validate() on every API/WS request | — |
| Refresh | POST /admin/api/auth/refresh → new token | exp: 24h |
| Revocation | DELETE /admin/api/auth/revoke → blacklist (SQLite) | Effective immediately |
| Rotation | JWT_SECRET rotated periodically → all tokens invalidated | 30 days |

## mTLS (Phase 2)

- Phase 1: Server-side TLS (rustls) + HTTP Basic Auth as an alternative
- Phase 2: bidirectional mTLS, encrypted peer-to-peer communication
- Certificates: X.509, ECDSA P-256, 90-day validity

## Certificate Lifecycle Management

Phase 1: self-signed CA + manual rotation. TLS certificates valid 90 days. Private key file mode 0600.
Phase 2: Let's Encrypt (ACME) auto-renewal, CRL revocation lists.
Trust chain: Root CA → Intermediate CA (optional Phase 2) → leaf certificate.

## Key Management

Key rotation policy:

| Key type | Rotation period | Coexistence window | Trigger |
|----------|-----------------|--------------------|---------|
| WS PSK | 30 days | 24h | Manual/config reload |
| JWT signing key | 90 days | 24h (kid header) | Manual |
| TLS private key | 90 days | 0 (instant switch) | Manual/ACME |
| HMAC control-command key | 30 days | 1h (old-frame replay window) | Manual |
## Audit Event Schema

```json
{
  "event_id": "uuid",
  "timestamp": "ISO8601",
  "actor": "user_id | peer_id",
  "action": "login | room.create | peer.connect | admin.config | e-stop",
  "resource": "room_id | config_key",
  "result": "success | denied | error",
  "source_ip": "optional"
}
```

- Phase 1: audit logs → stdout (journald)
- Phase 2: audit logs → SQLite (admin queryable)

## WebSocket PSK Rotation

- Phase 1: static PSK (env var), changed on service restart
- Phase 2: dynamic rotation, negotiating a new PSK every 24h

### PSK Rotation Runbook (doc-audit L5)

> Current state [verified]: signaling.rs:173-177 reads a **single** `MEDIASERVO_PSK` env at connection time; the **dual-key coexistence mechanism is not implemented** (Phase 2 design); rotation today = restart with a new key.

| Step | Action | Verification |
|------|--------|--------------|
| 0. Precondition | Confirm the server log shows AuditEvent (AuthSuccess/AuthFailure) | `grep AuthSuccess` in logs |
| 1. Generate | `openssl rand -hex 32` generates the new PSK | Length 64 hex |
| 2. Switch | Set the new `MEDIASERVO_PSK` → restart the server (Phase 1 semantics) | Old connections drop, new connections AuthSuccess |
| 3. Coexistence window | [Planned] `MEDIASERVO_PSK_OLD` dual-HMAC validation (change point at signaling.rs:174-176), tolerating un-reconnected clients for 24h | No AuthFailure growth within the window |
| 4. Cleanup | After the window remove OLD; production compose drops the hardcoded psk → `env_file`/secret (docker-compose.yml:10) | `grep -rn "mediaservo-dev"` matches only test files |
| 5. Rollback | On failure restore the old env and restart (Phase 1 has no in-window rollback; step 3's OLD mechanism is the key enabler) | Connections recover |

## SFU Media-Plane Security (doc-audit M7)

> Current state [verified]: code located at sfu.rs:150/155-172/215/392-431; docs located at sfu-mediasoup-integration.md:27/120-154/170/250/266.

### Trust Boundary & Threat Model

STRIDE-Lite table extension (media-plane rows):

| Threat | Media-plane analogue | Mitigation |
|--------|----------------------|------------|
| Spoofing | A fake peer joining a room to push | DTLS fingerprint validation (inside mediasoup) + ICE credential ownership |
| Tampering | RTP tampering | SRTP integrity (default AES_CM_128_HMAC_SHA1_80, upgradable) |
| DoS | Port-surface abuse | WebRtcServer single-port convergence + ICE consent timeout (default 20s) |

### Transport/Produce/Consume Authorization Model

**Current state [verified]**: route-level authorization — the peer must be authenticated (signaling.rs:345 uses the session peer_id) + in the room (sfu.rs:302-303/359-360) + transport ownership validated (sfu.rs:414-422). **No role-level authorization**: the RoomJoin role is not stored in the session and the SFU path doesn't check it; any authenticated peer can produce or consume.

**Target [planned]**: inject role into SfuManager (host=produce-only / remote=consume-only); produce validates send-transport ownership, consume validates the producer is in the same room. Scheduled as a Phase 4 security item.

### DTLS Fingerprint Validation

- [Verified] connect_transport passes the client DtlsParameters (with fingerprint) to the mediasoup `transport.connect()` (sfu.rs:424); **fingerprint comparison is performed internally by mediasoup during the DTLS handshake**.
- [Planned] Application-layer policy: log fingerprint-hash audit events; reject new fingerprints after a role change.

### SRTP Suite Policy

- [Verified] Default suite AES_CM_128_HMAC_SHA1_80 (the mediasoup default; `WebRtcTransportOptions::new_with_server()` not configured).
- [Planned] Explicit specification (e.g. AEAD_AES_256_GCM) and downgrade policy.

### Port Exposure & Firewall (B1)

- **⚠️ [Verified] WebRtcServer 20000/udp was previously not published** — docker-compose.yml:6 and dev.yml:12 now have the `20000:20000/udp` mapping (doc-audit B1 fix, 2026-08-03).
- [Verified] 40000-40100/udp is needed only by the PlainTransport path (WebRtcTransport goes through WebRtcServer 20000); the Worker actually uses `WorkerSettings::default()` (sfu.rs:150), which contradicts sfu-mediasoup-integration.md:27's claimed rtcMinPort 40000 — the doc needs alignment.
- [Planned] `announced_address` config to support NAT deployments (currently None at sfu.rs:161).

### Audit Extensions

- [Verified] The existing AuditEvent (audit.rs:32-34) covers the signaling layer.
- [Planned] Audit events for SFU actions (produce/consume/connect).

## Rate Limiting

| Endpoint | Limit | Window |
|----------|-------|--------|
| /admin/api/auth/login | 5 req | 1min |
| /admin/api/* | 100 req | 1min |
| /api/* | unlimited | — |

## Phase Dependencies

- Phase 3 Component framework: AuthComponent (JWT + rate limiting)
- Phase 4 Admin Dashboard: login/logout/session-expiry UX
- Phase 2 mediasoup SFU: signaling-layer mTLS
