# field SDK Usage Guide (Vehicle-Side Push)

> **Positioning**: `mediaservo-field` is the **remote-control vehicle-side SDK** — it needs only **video push** (vehicle cameras → cloud/cockpit).
> Pulling (consumption) is the cockpit `mediaservo-client`'s job (skeleton stage; consumes field::PullSession).
>
> **Related**: [20-sdk-api-contract.md](../modules/20-sdk-api-contract.md) §4, [04-sdk-layers.md](../modules/04-sdk-layers.md)

## Integration Methods

| Consumer | Method | Entry |
|----------|--------|-------|
| Rust (vehicle supervisor) | crate dependency | `mediaservo-field` (rlib) |
| C/C++ (embedded/third-party) | cdylib + headers | `bindings/c/mediaservo-field-c` → `libmediaservo_field_c.so` + `mediaservo_field.h` |
| Python | ctypes loads the cdylib (planned D227) | later |

## Push Flow (PushSession)

```
1. PushConfig setup (WS address/PSK/room/resolution/fps/bitrate)
2. PushSession::connect(cfg)      — signaling connect + join the room
3. publish_video(cfg, opts)       — SFU negotiation (transport→answer→Connect→Produce)
4. start_video_frames(cfg)        — frame generation (Squares color bars + timestamp watermark)
   [ vehicle-side loop: capture/control ... ]
5. stop_video_frames()            — stop frame generation (idempotent)
6. close()                        — close the session
```

## Rust Example

```rust
use mediaservo_field::{PublishOptions, PushConfig, PushSession};

let mut cfg = PushConfig::new("ws://host:9800/ws", "psk", "room");
cfg.width = 1280; cfg.height = 720;
cfg.framerate = 30; cfg.bitrate_kbps = 2000;

let (mut session, _events) = PushSession::connect(cfg.clone()).await?;
let track = session.publish_video(&cfg, &PublishOptions::default()).await?;
session.start_video_frames(&cfg)?;
// ... run ...
session.stop_video_frames();
session.close().await?;
```

Full example: `crates/mediaservo-field/examples/vehicle_push.rs`

## C ABI Example

```c
#include "mediaservo_field.h"

mediaservo_push_config_t cfg = MEDIASERVO_PUSH_CONFIG_DEFAULT;
cfg.url = "ws://host:9800/ws"; cfg.psk = "psk"; cfg.room = "room";

mediaservo_field_push_t* s = NULL;
mediaservo_field_push_connect(&cfg, &s);
char track[64];
mediaservo_field_push_publish_video(s, track, sizeof(track));
mediaservo_field_push_start_video_frames(s);
/* ... */
mediaservo_field_push_close(s);
```

Full example: `bindings/c/mediaservo-field-c/examples/vehicle_push.c`

## Runtime Prerequisites

- **server** (mediasoup SFU) running: `./mediaservo.sh up --env dev` (the CLI auto-injects the host IP)
- **Network**: the vehicle can reach the server's WS (9800) + UDP (20000/40000-40100)
- **PSK**: matches the server's `MEDIASERVO_PSK`

## Verified Capabilities (2026-08-18)

| Capability | Verification |
|------------|--------------|
| Signaling connect + room join | ✅ e2e |
| SFU negotiation (produce) | ✅ e2e 3/3 (external server, C21) |
| Frame publishing (bytes_sent/frames_encoded growth) | ✅ e2e evidence |
| Server frame receipt (ReceiveRtpPacket key frame) | ✅ server log evidence |
| C ABI bindings (connect/publish/start/close) | ✅ unit tests + cdylib build |
| Multiple announced IPs (dynamic host NICs) | ✅ measured, filtering docker/VPN |

## Known Limitations

- **PullSession frame receive pending** (closed out 2026-08-18): negotiation fully succeeds (ssrc injection/mid alignment/complete capabilities),
  but the libwebrtc receive pipeline doesn't deliver frames (RTP all aligned, on_frame never fires) — a libwebrtc/webrtc-sys
  integration defect. The consumer is the cockpit client (skeleton stage); this belongs to its development track.
- **Single video track**: MVP supports one video track (a duplicate publish reports InvalidState).
- **Stream content**: currently the Squares test pattern (when a real camera is attached, swap
  VideoFrameGenerator's pattern for a camera frame source — the interface is `VideoSource::add_or_update_sink`).
