# MediaServo 参考文档索引

> 更新: 2026-08-06 | 组织原则: **Diátaxis 框架**（活参考 / 调研存档分离）
> 活参考按**产品模块**镜像组织（reference mirror product structure）；调研存档独立于 `research/`（历史调研，不碍事）

## 活参考（Reference — 查用，按产品模块）

### 🌐 WebRTC 模块 — `webrtc/`
| 文档 | 内容 |
|------|------|
| `webrtc/mediasoup-refs.md` | **入口** — 官方 mediasoup 源码导航 + 同步脚本用法 |
| `webrtc/mediasoup-client.md` | 官方客户端架构参考（画像/用法/架构/案例） |
| `webrtc/webrtc-w3c-alignment.md` | Host SFU 对齐 W3C API 重构分析（含团队审核修正） |
| `webrtc/keyframe-black-screen-analysis.md` | PIT-65 关键帧黑屏根因分析 |
| `webrtc/gop-control-internal-encoder.md` | **GOP 控制根因与修复路径** — libwebrtc 内部编码器关键帧机制 + 方案对比 + vendor 决策 |
| `webrtc/sdp-negotiation-bitrate-audit.md` | **SDP 协商审计 + 码率链路** — Host 构建方式分析 + BWE 反馈断裂根因 + A/C 方案决策 (2026-08-12) |

### 🧰 Codec / 构建策略 — `codec/`
| 文档 | 内容 |
|------|------|
| `codec/ffmpeg-static-build-strategy.md` | FFmpeg 静态构建策略（codec 三后端） |
| `codec/build-optimization-strategy.md` | Docker 构建优化（分层缓存、国内镜像、lto） |

### 其他活参考（根目录）
| 文档 | 内容 |
|------|------|
| `janus-gateway.md` | Janus Gateway（WebRTC 会议网关）参考 |

## 调研存档（Explanation — 历史调研，不参与活跃工作）

> 一次性竞品/技术选型调研笔记，写完后不被引用。按原领域子目录归档，仅作历史参考。

### `research/`
| 子目录 | 内容 |
|--------|------|
| `research/remote-desktop/` | anydesk / parsec / rustdesk / moonlight-sunshine / teamviewer |
| `research/streaming/` | mediamtx / srs / zlmediakit / nginx-rtmp-module / pion / obs-studio / lvqr / xiu |
| `research/video-conference/` | mediasoup* / openvidu-* / livekit / jitsi / kurento / zoom |
| `research/teleoperation/` | comma-ai-openpilot / tether-rally / tum-teleoperated-driving / vay |
| `research/network-emulation/` | 弱网模拟工具全景（tc/netem 族 / clumsy / ATC·toxiproxy 核证修正 / webrtc test-network·Pion vnet / 选型速查·深坑清单） |

## WebRTC 活参考关联图

```
webrtc/mediasoup-refs.md (源码导航)
   ├── webrtc/mediasoup-client.md (官方架构参考)
   ├── webrtc/webrtc-w3c-alignment.md (对齐重构分析)
   └── webrtc/keyframe-black-screen-analysis.md (PIT-65 根因)
        ├── webrtc/gop-control-internal-encoder.md (GOP 机制 + 修复路径, 2026-08-10)
        └── .agents/memorys/ (C18 官方用法优先, PIT-46/54/56/65)
```

## 官方参考源码（脚本管理）

官方 mediasoup 客户端源码克隆在 `.refinfo/`（git-ignored），由 `scripts/sync-official-refs.sh` 管理。详见 `webrtc/mediasoup-refs.md`。

## 组织规范（Diátaxis）

| 需求 | 归属 | 规范 |
|------|------|------|
| **Reference**（事实/查用） | `webrtc/` `codec/` 或根目录 | 按产品模块镜像；克制、权威、无歧义 |
| **Explanation**（调研/理解） | `research/` | 历史调研存档；不参与活跃工作 |
| **新增活参考** | 对应模块目录 | 在 README 登记 |
| **新增调研** | `research/<领域>/` | 在 README 登记 |

## 说明

- 本目录为**参考/调研**资料，非项目设计文档
- 项目设计文档在 `docs/modules/`，架构总览在 `docs/architecture.md`
- 调研存档（`research/`）不被代码引用，仅作历史参考；必要时可归档/删除
