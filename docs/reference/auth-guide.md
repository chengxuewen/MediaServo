# 客户端鉴权指南（auth-guide）— 四种凭证方式

> 单点真源：本文。sdk-cxx 四册只留指针；代码示例与判据数字来自
> viewer-auth-matrix 无头矩阵实测（2026-09-29）。

## 1. 选型表（10 秒决策）

| 你是谁 | 用哪种 | 管理员怎么发卡 |
|---|---|---|
| 第三方集成 / 临时给同事看流 | **API-Key** | 管理台「API Keys」页（或 `POST /api/admin/apikeys`）→ secret 一次明文 → 拷给对方 |
| 自家的车 / 相机等边缘设备 | **设备公钥**（D283） | 设备 `init` 自动收录（`ALLOW_DEV_ENROLL=1`）或管理台「待批准」一键批准 |
| 有账号体系的正式用户 | **账号密码** | 管理台「Accounts」页建号 |
| 封闭内网快速联调 | **PSK** | server.yaml `psk:`（⚠ 全能钥匙：全权限、无法按人吊销，勿出内网） |

## 2. 连接时序（前三种同形；PSK/设备路见 §4 变体）

```
┌────────┐  login(账号) 或 exchange(key)   ┌────────┐
│ Client │ ──────────────────────────────→ │ Server │  POST /api/auth/{login,exchange}
│  SDK   │ ←───────── JWT ─────────────── │        │  （角色+车辆白名单+过期，签发即焊死）
│        │                                 │        │
│        │  GET /api/rooms + JWT           │        │  发现：只列在线且白名单内的房
│        │ ──────────────────────────────→ │        │
│        │ ←── [{room_id,kind},...] ────── │        │  kind: video=可拉 / control=遥控
│        │                                 │        │
│        │  WS /ws + JWT（子协议头）        │        │  入房拉流/控制（每个 join 独立鉴权）
│        │ ──────────────────────────────→ │        │
└────────┘                                 └────────┘
```

## 3. 各语言片段

### Rust

```rust
// 换通行证（二选一：账号 or key）
let token = mediaservo_client::exchange("http://server:9800", "my-key", "secret").await?;
// 或 mediaservo_client::login(base, user, pass).await?

// 发现
let rooms = mediaservo_client::list_rooms("http://server:9800", &token.jwt).await?;

// 入房拉流
let cfg = ClientConfig {
    signaling_url: "ws://server:9800/ws".into(),
    room_id: "vehicle_demo_test-stream".into(),
    jwt: Some(token.jwt),
    ..Default::default()
};
let session = RoomSession::connect(&cfg).await?;
```

设备身份（Rust 独有入口）：

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
cfg.jwt = jwt;                    /* 与 psk 二选一 */
/* cfg.identity_dir = "/path/to/instance";  设备路（additive 字段，老代码不写=不启用） */
mediaservo_client_session_create(&cfg, &session);
```

### C++

```cpp
auto tok = mediaservo::client::exchange("http://server:9800", "my-key", "secret");
ms::Config cfg;
cfg.signaling_url = "ws://server:9800/ws";
cfg.room = "vehicle_demo_test-stream";
cfg.jwt = *tok;
cfg.identity_dir = "/path/to/instance";   // 设备路（空=不启用）
auto session = ms::Session::connect(cfg);
auto label = ms::identity_label(session);  // "Device(ms-...)" / "jwt" / "legacy-psk"
```

### imgui_viewer（GUI，测试面板）

登录面板五个 Radio：Account / API-Key / PSK / Device / JWT。选模式→填框→连接；
底部 `identity:` 行即生效身份。无头等价注入：

```bash
MSRTC_AUTH_MODE=apikey MSRTC_KEY_ID=.. MSRTC_KEY_SECRET=.. MSRTC_ROOM=<video房> \
  SDL_VIDEODRIVER=dummy MSRTC_RUN_SECS=20 ./imgui_viewer
# 判据行 [frame] t=..s cb=597/20s ≈ 满帧；坏 token（JWT 模式贴错）→ [tile] join: auth rejected [4013]
```

## 4. 两类变体（跳过发现的直连路）

| 变体 | 为什么跳发现 | 怎么给房间 |
|---|---|---|
| **PSK** | Legacy 身份调 `/api/rooms` = 401（发现接口只认账号角色） | 房名手填（`_` 分隔名按流房处理） |
| **设备身份** | Device 无 REST 发证面（公钥挑战只在 WS join 面） | 同上 |

## 5. 运维要点

- **吊销**：API-Key 删除后**新** exchange 立即 401；已发 JWT 到期自然死（TTL 默认 12h，
  `api_token_ttl_secs` 可调）。设备吊销=管理台删除条目，下次接入 4010。
- **4013 红牌** = token 坏/过期被拒（fail-closed）。**重试无义**——换新凭证。
  客户端 SDK 已把 4013 归终态（不重连风暴）。
- **login vs exchange**（对偶表）：

| | login | exchange |
|---|---|---|
| 凭证 | 账号+密码 | key_id+secret |
| 权限跟谁走 | 账号（管理员改表即变） | 卡（发卡时焊死，与账号无关） |
| 吊销 | 删账号 | 删 key |
| 适用 | 有账号体系的正式用户 | 分发/集成 |

- **PSK 限制**：全权限（矩阵旁路）+ 无审计归属 + 看不到房间列表——只用于封闭内网联调。

## 6. FAQ

**Q: token 过期能自动续吗？**
不能（在册开放项）。重新调 login/exchange 换新的。长会话请评估 TTL 配置。

**Q: JWT 可以和 PSK 一起给吗？**
可以（C ABI 恰一规则：jwt/psk 二选一；叠加语义已从 SDK 面移除，server 端 jwt 验签失败
直接 4013 不会滑向 PSK）。设备身份与任一可叠（server 设备认证优先）。

**Q: 怎么确认我的会话走的哪种鉴权？**
C/C++：`identity_label(session)`；viewer：面板底部行；server 侧：日志 peer 前缀
（`apikey:` / device-id / `legacy-psk`）。
