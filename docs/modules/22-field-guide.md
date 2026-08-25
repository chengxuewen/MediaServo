# field SDK 使用指引（车端推流）

> **定位**: `mediaservo-field` 是**遥控车端 SDK**——只需**视频推流**（车端摄像头 → 云端/舱端）。
> 拉流（消费）是舱端 `mediaservo-client` 的职责（骨架阶段, 消费 field::PullSession）。
>
> **关联**: [20-sdk-api-contract.md](../modules/20-sdk-api-contract.md) §4、[04-sdk-layers.md](../modules/04-sdk-layers.md)

## 集成方式

| 消费方 | 方式 | 入口 |
|---|---|---|
| Rust（车端主控）| crate 依赖 | `mediaservo-field` (rlib) |
| C/C++（嵌入式/第三方）| cdylib + 头文件 | `bindings/c/mediaservo-field-c` → `libmediaservo_field_c.so` + `mediaservo_field.h` |
| Python | ctypes 加载 cdylib（计划 D227）| 后续 |

## 推流流程（PushSession）

```
1. PushConfig 配置（WS 地址/PSK/房间/分辨率/帧率/码率）
2. PushSession::connect(cfg)      — 信令连接 + 加入房间
3. publish_video(cfg, opts)       — SFU 协商（transport→answer→Connect→Produce）
4. start_video_frames(cfg)        — 帧生成（Squares 彩条 + 时间戳水印）
   [ 车端循环: 采集/控制 ... ]
5. stop_video_frames()            — 停止帧生成（幂等）
6. close()                        — 关闭会话
```

## Rust 示例

```rust
use mediaservo_field::{PublishOptions, PushConfig, PushSession};

let mut cfg = PushConfig::new("ws://host:9800/ws", "psk", "room");
cfg.width = 1280; cfg.height = 720;
cfg.framerate = 30; cfg.bitrate_kbps = 2000;

let (mut session, _events) = PushSession::connect(cfg.clone()).await?;
let track = session.publish_video(&cfg, &PublishOptions::default()).await?;
session.start_video_frames(&cfg)?;
// ... 运行 ...
session.stop_video_frames();
session.close().await?;
```

完整示例: `crates/mediaservo-field/examples/vehicle_push.rs`

## C ABI 示例

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

完整示例: `bindings/c/mediaservo-field-c/examples/vehicle_push.c`

## 运行时前置

- **server**（mediasoup SFU）运行中: `./mediaservo.sh up --env dev`（CLI 自动注入宿主 IP）
- **网络**: 车端可达 server 的 WS (9800) + UDP (20000/40000-40100)
- **PSK**: 与 server 的 `MEDIASERVO_PSK` 一致

## 已验证能力（2026-08-18）

| 能力 | 验证 |
|---|---|
| 信令连接 + 房间加入 | ✅ e2e |
| SFU 协商（produce） | ✅ e2e 3/3（外部 server, C21）|
| 帧发布（bytes_sent/frames_encoded 增长）| ✅ e2e 实证 |
| server 收帧（ReceiveRtpPacket key frame）| ✅ server 日志实证 |
| C ABI 绑定（connect/publish/start/close）| ✅ 单测 + cdylib 构建 |
| 多 announced IP（宿主多网卡动态）| ✅ 实测过滤 docker/VPN |

## 已知限制

- **PullSession 收帧挂起**（2026-08-18 收口）：协商全通（ssrc 注入/mid 对齐/完整 capabilities），
  但 libwebrtc 接收管线不交付帧（RTP 全对, on_frame 不触发）——libwebrtc/webrtc-sys 集成缺陷。
  消费方是舱端 client（骨架阶段），归属其开发时攻关。
- **单视频轨**：MVP 支持一路 video（重复 publish 报 InvalidState）。
- **码流类型**：当前 Squares 测试图案（车端接入真实相机时替换 VideoFrameGenerator 的
  pattern 为相机帧源即可——接口为 `VideoSource::add_or_update_sink`）。
