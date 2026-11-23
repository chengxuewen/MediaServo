# Auth & Permissions Reference

> MediaServo provides dual-mode authentication: Local (standalone deployment) and platform mode.
>
> **Implementation status**: §1–§6 are design-state (no AuthProvider/OIDC code in crates yet;
> the implemented surface is accounts.yaml + HS256 JWT, see
> `crates/mediaservo-server/src/accounts.rs`). §7 dev switches are implemented.

---

## 1. AuthProvider Modes

```rust
#[async_trait]
trait AuthProvider: Send + Sync {
    async fn login(&self, credential: &Credential) -> Result<User, AuthError>;
    async fn validate(&self, token: &str) -> Result<User, AuthError>;
    async fn authorize(&self, user_id: &str, permission: &Permission) -> Result<bool, AuthError>;
}

Two planned implementations (design-state):
- **Local**: embedded account store + JWT, used in standalone deployment
- **Platform mode**: external OIDC provider (D298, supersedes the D57 gRPC design);
  LDAP/AD stays behind the provider as user federation — MediaServo never talks LDAP directly

---

## 2. Dual-Mode Comparison

| Dimension | Local (standalone) | Platform mode |
|-----------|--------------------|---------------|
| **User storage** | Embedded account store | Delegated to the OIDC provider (PDP) |
| **Authentication** | Local JWT | Provider-issued OIDC tokens, validated locally (JWKS) |
| **Permissions** | Local role matrix | claims → 4 roles + device scope via configurable mapping dictionary (PEP stays in MediaServo) |
| **Config** | `auth.mode: "local"` | `auth.mode: "aude"` |
| **Scenario** | Standalone deployment | Docker platform module |

Config field names/values above are design-state; final naming lands at implementation time.

---

## 3. Permission Model

```typescript
interface Permission {
  // Feature switches
  capabilities: {
    streaming:    { push: boolean; pull: boolean };
    remote:       { control: boolean; controllable: boolean };
    conference:   { host: boolean; join: boolean };
    surveillance: boolean;
    teleop:       { operator: boolean; vehicle: boolean };
  };

  // Quota limits
  quotas: {
    max_streams:    number;        // Max stream count
    max_bitrate:    number;        // Max bitrate (kbps)
    max_resolution: "720p" | "1080p" | "4k";
    max_duration:   number;        // Max session duration (seconds)
  };

  // License
  license: {
    type:        "trial" | "standard" | "enterprise";
    expires_at:  string;          // ISO 8601
    features:    string[];        // Premium feature list
  };
}
```

---

## 4. Permission Flow

```
Client SDK starts
       ↓
Pulls the permission config from the backend
       ↓
Caches locally
       ↓
License Manager validates before each pipeline operation
```

---

## 5. Reference Model

Analogous to Synology DSM: MediaServo installed as a Docker module on a host platform, using the host platform's user/permission system (like Jira installed on a Synology using DSM's LDAP accounts).


## 6. OIDC Provider Contract (D298, supersedes D57)

> Design-state. The former private gRPC contract (D57) is retired by D298: a private
> per-platform protocol internalizes ecosystem adaptation cost (one adapter per platform);
> standard OIDC lets any compliant provider plug in. AccessBase is one provider instance,
> not the contract itself. Adjudication ledger and open items: parent repo
> `docs/plans/oidc-provider-contract/PLAN.md`.

When MediaServo runs in platform-module mode, human authentication is delegated to an
external OIDC Provider. MediaServo remains the **PEP** (policy enforcement point): the
provider decides identity facts (PDP), MediaServo translates claims and enforces every
operation itself.

### Login Flow (hosted redirect)

```
Browser → MediaServo login page → redirect to provider /authorize (PKCE)
        → user authenticates at the provider (MFA/SSO per provider capability)
        → redirect back with code → MediaServo backend exchanges code for tokens
        → backend validates the ID token, maps claims → local session
```

- No provider UI is embedded (no iframe); provider-exclusive consoles are deep links (new tab).
- Token exchange happens backend-to-backend; the browser never holds provider secrets.

### Token Validation & Revocation

- **Mandatory baseline**: short-lived access tokens (~15 min) + local JWKS signature
  verification — zero per-request network cost. If the provider is down, issued tokens
  remain valid until expiry; core enforcement never depends on provider liveness.
- **Optional enhancement**: OIDC Back-Channel Logout. Providers that declare support push
  logout events; MediaServo keeps an expiring revocation list. Lost notifications or
  provider downtime degrade to the baseline — worst case is one TTL window, never a full
  lockout (degraded = break-glass available).

### Claims Mapping (configurable dictionary)

- Provider config declares `claims_mapping`: role claim path + value dictionary +
  device-scope claim path. The recommended default template reads `ms_roles` /
  `ms_device_scope`; native provider claims (e.g. `realm_access.roles`) work equally well.
- Mapping target = the 4 server-authoritative roles (`roles.rs`: viewer/operator/
  dispatcher/admin) + device scope. Unknown values or missing claims = **fail-closed on
  all channels (REST + WS)**. Invalid mapping config refuses to load.
- Expressiveness loss is explicit: policies beyond the 4-role + device-scope model are
  "unsupported here + reason", never silently dropped.

### Health & Capabilities

- `/capabilities` reports provider health (reachable/degraded/unreachable); capability
  matrix status derives from ownership × health. Core built-in capabilities stay active
  regardless of provider health.

### Break-glass & Audit

- Local break-glass accounts persist independently of the provider; break-glass events go
  to a local append-only file (never depend on the provider audit pipeline).
- The audit query surface proxies the provider audit API (no dual storage); the core
  security audit ring stays local.

### Data Plane Unchanged

- Device/media authentication keeps PSK + Ed25519 capability tokens; the OIDC contract
  covers the control plane only.

### Pins (host shell responsibility)

- Image digest (customer-supplied; the provider image is never distributed with the
  product), discovery/OpenAPI contract version, claim schema version.

### MediaServo Permission Enum (design-state vocabulary)

Feature-level vocabulary for future PEP checks. The authoritative model today is the
4-role matrix + device scope (`roles.rs`); the provider never maps strings to roles —
MediaServo maps claims to roles itself (see Claims Mapping above):

| Permission | Description |
|------------|-------------|
| `RemoteDesktop.Connect` | Initiate a remote desktop connection |
| `RemoteDesktop.Host` | Run as the controlled end |
| `Streaming.Start` | Start pushing a stream |
| `Streaming.Stop` | Stop pushing a stream |
| `Teleop.Connect` | Remote-control connection |
| `Teleop.Control` | Send control commands |
| `Conference.Create` | Create a meeting room |
| `Conference.Join` | Join a meeting room |
| `Conference.Record` | Record a meeting |
| `Surveillance.View` | View surveillance feeds |
| `Admin.Manage` | Backend administration permission |


## 7. Dev Switches (dev-only; production MUST leave them unset)

Two explicit dev-convenience switches relax production safety defaults. Both are
**config-first** (`server.yaml` fields survive tree rebuilds) with an **env
override that wins**. Production keeps both absent; the image also strips dev
placeholder accounts at build time and the startup guard refuses to run with
them unless explicitly allowed.

| Switch | Config field | Env (canonical) | OFF (default = production) | ON (dev) |
|---|---|---|---|---|
| Dev credentials | `allow_dev_credentials` | `MEDIASERVO_ALLOW_DEV_CREDENTIALS=1` | Startup guard PANICS on the known placeholder accounts (`admin123`/`dispatch123`/`operator123`) — the instance refuses to boot until they are replaced | Placeholders are accepted; a startup WARN lists them by name (never silent) |
| Device auto-enroll | `allow_dev_enroll` | `MEDIASERVO_ALLOW_DEV_ENROLL=1` | Unknown devices land in the manual approval queue (web UI) | Unknown devices auto-register after public-key verification passes; startup WARN notes the relaxed policy |

Notes:
- `ALLOW_DEV_ENROLL` (unprefixed) is a **deprecated alias** during the migration
  window; using it still works but logs a deprecation WARN. It is removed after
  one CLI-face cycle.
- Defense in depth for the delivery artifact: `gate-prod-compose-safety` fails if
  the production `docker-compose.yml` sets any `ALLOW_DEV*` toggle or references
  dev credential files. The startup guard remains the PRIMARY net (covers every
  launch path, not just compose).
- The placeholder detection matches **password hashes**, not usernames (a real
  account named `admin` with a strong password is not flagged).
