# Client Auth Guide (auth-guide) — The Four Credential Types

> Single source of truth: this document. The four sdk-cxx volumes keep only pointers; code examples and
> threshold numbers come from the viewer-auth-matrix headless matrix measurements (2026-09-29).

## 1. Selection Table (10-second decision)

| Who you are | Which to use | How the admin issues it |
|-------------|--------------|--------------------------|
| Third-party integration / showing a stream to a colleague | **API-Key** | The admin console "API Keys" page (or `POST /api/admin/apikeys`) → the secret is shown in plaintext once → hand it over |
| Your own vehicle / camera edge devices | **Device public key** (D283) | The device is auto-enrolled by `init` (`ALLOW_DEV_ENROLL=1`) or approved in one click from the console "Pending" list |
| Formal users with an account system | **Username+password** | Create accounts in the console "Accounts" page |
| Closed-intranet quick bring-up | **PSK** | server.yaml `psk:` (⚠ a master key: full permissions, cannot be revoked per person; never leave the intranet) |

## 2. Connection Sequence (the first three share one shape; PSK/device paths see §4 variants)

```
┌────────┐  login(account) or exchange(key)   ┌────────┐
│ Client │ ──────────────────────────────→    │ Server │  POST /api/auth/{login,exchange}
│  SDK   │ ←───────── JWT ───────────────     │        │  (role + vehicle allowlist + expiry, baked in at issue)
│        │                                    │        │
│        │  GET /api/rooms + JWT              │        │  Discovery: lists only online rooms in the allowlist
│        │ ──────────────────────────────→    │        │
│        │ ←── [{room_id,kind},...] ──────    │        │  kind: video=pullable / control=remote control
│        │                                    │        │
│        │  WS /ws + JWT (subprotocol header) │        │  Room join for pull/control (each join authed separately)
│        │ ──────────────────────────────→    │        │
└────────┘                                    └────────┘
```

## 3. Snippets per Language

### Rust

```rust
// Exchange for a pass (pick one: account or key)
let token = mediaservo_client::exchange("http://server:9800", "my-key", "secret").await?;
// or mediaservo_client::login(base, user, pass).await?

// Discovery
let rooms = mediaservo_client::list_rooms("http://server:9800", &token.jwt).await?;

// Join a room and pull
let cfg = ClientConfig {
    signaling_url: "ws://server:9800/ws".into(),
    room_id: "vehicle_demo_test-stream".into(),
    jwt: Some(token.jwt),
    ..Default::default()
};
let session = RoomSession::connect(&cfg).await?;
```

Device identity (Rust-only entry):

```rust
let cfg = ClientConfig {
    identity_dir: Some("/path/to/instance".into()), // identity.json + etc/link/signing.pem
    ..cfg
};
```

### C

```c
mediaservo_client_exchange("http://server:9800", "my-key", "secret", jwt, sizeof jwt, NULL);
mediaservo_client_list_rooms("http://server:9800", jwt, rooms, sizeof rooms, NULL);
mediaservo_client_config_t cfg = MEDIASERVO_CLIENT_CONFIG_DEFAULT;
cfg.signaling_url = "ws://server:9800/ws";
cfg.room = "vehicle_demo_test-stream";
cfg.jwt = jwt;                    /* or psk — pick one */
/* cfg.identity_dir = "/path/to/instance";  device path (additive field; old code omitting it = not enabled) */
mediaservo_client_session_create(&cfg, &session);
```

### C++

```cpp
auto tok = mediaservo::client::exchange("http://server:9800", "my-key", "secret");
ms::Config cfg;
cfg.signaling_url = "ws://server:9800/ws";
cfg.room = "vehicle_demo_test-stream";
cfg.jwt = *tok;
cfg.identity_dir = "/path/to/instance";   // device path (empty = not enabled)
auto session = ms::Session::connect(cfg);
auto label = ms::identity_label(session);  // "Device(ms-...)" / "jwt" / "legacy-psk"
```

### imgui_viewer (GUI, test panel)

The login panel has five radios: Account / API-Key / PSK / Device / JWT. Pick a mode → fill the boxes → connect;
the bottom `identity:` line shows the effective identity. Headless-equivalent injection:

```bash
MSRTC_AUTH_MODE=apikey MSRTC_KEY_ID=.. MSRTC_KEY_SECRET=.. MSRTC_ROOM=<video room> \
  SDL_VIDEODRIVER=dummy MSRTC_RUN_SECS=20 ./imgui_viewer
# Criteria line [frame] t=..s cb=597/20s ≈ full frame rate; a bad token (JWT mode with the wrong value) → [tile] join: auth rejected [4013]
```

## 4. Variants & Discovery Capability

| Variant | Discovery surface | Notes |
|---------|-------------------|-------|
| **PSK** | ✅ **Full discovery** (`Authorization: Psk <psk>` → psk-discover) — aligned with the join-gate capability (Legacy allows all); the main route for **cross-room pulling**: check as many tree rows as you connect | viewer M3 leaves the room name empty to use discovery; filling a room name = direct-connect override |
| **Device identity** | ❌ Direct connect only (Device has no REST issuance face; the public-key challenge exists only on the WS join face) | Fill the room name manually |

Rust: `list_rooms_psk(base, psk)`; C/C++: `list_rooms_psk(...)`; viewer: PSK mode with an empty room name.

## 4.5 Room Model Semantics (important — avoid misreading)

The `room_id` values returned by `GET /api/rooms` fall into two classes: **whole-vehicle rooms** (`vehicle`,
the remote-control face) and **stream rooms** (`vehicle_generatorN`, one per stream). Stream rooms are the
**SFU's underlying isolation granularity** (per-stream produce/consume/permissions), **not "one room per
vehicle"** — in product semantics they all belong to the same vehicle.

- Consumer UIs should group by the **base room** (`vehicle_*` prefixes group under `vehicle`), with streams as children
- Underlying join/produce still happens at stream-room granularity (this semantics doesn't affect how the API is called)
- Expressing the ownership relationship on the wire (streams embedded/parent field) = an additive evolution on the books

## 5. Ops Notes

- **Revocation**: after an API-Key is deleted, **new** exchanges immediately 401; already-issued JWTs die naturally at expiry (TTL default 12h, tunable via `api_token_ttl_secs`). Device revocation = delete the entry in the console; the next connection gets 4010.
- **4013 red card** = the token is bad/expired and rejected (fail-closed). **Retrying is pointless** — get a new credential.
  The client SDK already treats 4013 as terminal (no reconnect storm).
- **login vs exchange** (duality table):

| | login | exchange |
|---|---|---|
| Credential | account+password | key_id+secret |
| Permissions follow | the account (admin table edits apply immediately) | the key (baked at issuance, independent of accounts) |
| Revocation | delete the account | delete the key |
| Suited for | formal users with an account system | distribution/integration |

- **PSK caution**: full permissions (matrix bypass) + no audit attribution + full visibility (psk-discover) — use only in closed intranets / trusted environments.

## 6. FAQ

**Q: Can an expired token auto-renew?**
No (an open item on the books). Call login/exchange again for a fresh one. For long sessions, evaluate the TTL configuration.

**Q: Can JWT and PSK be provided together?**
Yes (the C ABI has a pick-one rule: jwt/psk are mutually exclusive; the combined semantics were removed from the SDK surface; on the server a jwt verification failure 4013s directly without sliding to PSK). Device identity may combine with either (server device auth takes precedence).

**Q: How do I confirm which auth my session is using?**
C/C++: `identity_label(session)`; viewer: the bottom panel line; server-side: the log peer prefix
(`apikey:` / device-id / `legacy-psk`).
