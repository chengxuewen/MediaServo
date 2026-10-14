# Auth & Permissions Reference

> MediaServo provides dual-mode authentication: Local (standalone deployment) and platform mode.

---

## 1. AuthProvider Modes

```rust
#[async_trait]
trait AuthProvider: Send + Sync {
    async fn login(&self, credential: &Credential) -> Result<User, AuthError>;
    async fn validate(&self, token: &str) -> Result<User, AuthError>;
    async fn authorize(&self, user_id: &str, permission: &Permission) -> Result<bool, AuthError>;
}

Two implementations:
- **Local**: SQLite + JWT, used in standalone deployment
- **Platform mode**: gRPC LDAP, used by the Docker platform module

---

## 2. Dual-Mode Comparison

| Dimension | Local (standalone) | Platform mode |
|-----------|--------------------|---------------|
| **User storage** | Embedded SQLite | Delegated to platform RBAC |
| **Authentication** | Local JWT | Host-platform-issued token |
| **Permissions** | Local RBAC table | Host-platform LDAP group mapping |
| **Config** | `auth.mode: "local"` | `auth.mode: "aude"` |
| **Scenario** | Standalone deployment | Docker platform module |

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


## 6. gRPC Auth Contract (D57)

When MediaServo runs in platform-module mode, the AuthProvider calls the host platform via gRPC:

### Proto

```protobuf
syntax = "proto3";
package mediaservo.auth;

service AuthService {
  rpc ValidateToken(ValidateTokenRequest) returns (ValidateTokenResponse);
  rpc CheckPermission(CheckPermissionRequest) returns (CheckPermissionResponse);
}

message ValidateTokenRequest {
  string token = 1;
}

message ValidateTokenResponse {
  bool valid = 1;
  string user_id = 2;
  string user_name = 3;
  repeated string roles = 4;
  repeated string permissions = 5;   // Pre-resolved permission list from the host platform
  int64 expires_at = 6;              // Unix timestamp
  string error = 7;
}

message CheckPermissionRequest {
  string user_id = 1;
  string permission = 2;             // MediaServo permission string
}

message CheckPermissionResponse {
  bool allowed = 1;
  string reason = 2;
}
```

### Call Flow

```
Client request → MediaServo axum receives the JWT
                           │
                           ▼
              AuthProvider.authenticate(token)
                           │
              Platform mode ▼
              gRPC ValidateToken(token) → host platform
                           │
                           ▼
              Returns user_id + permissions[]
                           │
                           ▼
              LicenseManager checks features[]
              Permission::StreamingStart ∈ permissions[]
```

### Caching Policy

- `validateToken`: results cached in memory for 5 minutes (below the JWT expiry)
- `checkPermission`: prefers the permissions[] list returned by `validateToken`
- Permission changes: the host platform must notify MediaServo to clear the cache → Phase 2 channel

### MediaServo Permission Enum

MediaServo maintains its own permission strings; the host platform's RBAC does string→role mapping:

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
