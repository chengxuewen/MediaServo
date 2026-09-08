# 18. Codec Crate — mediaservo-codec

> 状态：Phase 2 规划中 | 关联决策：D43, D46, D70, D71, D82, C5

## 定位

`mediaservo-codec` 是 MediaServo 的统一编解码层，提供 `VideoEncoder` / `VideoDecoder` trait，后端通过编译期 feature gate 支持 GStreamer 和 FFmpeg 双后端。遵循 C5 `&[u8]` 字节边界——不依赖 `mediaservo-media` 的类型体系。

```
┌─────────────────────────────────────────────────────┐
│                 mediaservo-codec                      │
│                                                     │
│  CodecFactory::create_encoder()                     │
│    └→ Box<dyn VideoEncoder>                        │
│                                                     │
│  CodecFactory::create_decoder()                     │
│    └→ Box<dyn VideoDecoder>                        │
│                                                     │
│  ┌─────────────────────────────────────────────┐    │
│  │              后端 trait 层                   │    │
│  │  GstEncoder    │    FfmpegEncoder            │    │
│  │  GstDecoder    │    FfmpegDecoder            │    │
│  └──────────────┬──────────────────────────────┘ │
│                 ▼                                  │
│  ┌─────────────────────────────────────────────┐    │
│  │    backend-gstreamer  │  backend-ffmpeg     │   │
│  │   (动态 .so)          │  (静态 .a)          │   │
│  └───────────────────────┴─────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

## 命名规范

| 类型 | 命名 | 说明 |
|------|------|------|
| 工厂 | `CodecFactory` | 编码器/解码器创建入口 |
| 编码 trait | `VideoEncoder` | push-pull 同步接口 |
| 解码 trait | `VideoDecoder` | push-pull 同步接口 |
| 编码配置 | `EncoderConfig` | builder 模式 |
| 解码配置 | `DecoderConfig` | builder 模式 |
| 后端 ID | `BackendId::GStreamer` / `BackendId::FFmpeg` | 运行时后端标识 |
| 编解码 ID | `CodecId::H264` / `CodecId::VP8` / ... | RFC 6381 对齐 |
| 像素格式 | `PixelFormat::Yuv420p` / `PixelFormat::Nv12` / ... | 内部格式，不与 media 耦合 |
| 错误 | `CodecError` | 10 variants |

## Trait 设计

### VideoEncoder

```rust
pub trait VideoEncoder: Send {
    fn configure(&mut self, config: &EncoderConfig) -> Result<(), CodecError>;
    fn push_frame(&mut self, frame: &VideoFrame) -> Result<(), CodecError>;
    fn pull_packet(&mut self) -> Result<Option<EncodedPacket>, CodecError>;
    fn flush(&mut self) -> Result<(), CodecError>;
    fn reset(&mut self) -> Result<(), CodecError>;
}
```

push-pull 模式（FFmpeg `avcodec_send_frame` / `avcodec_receive_packet` 对齐）：
- `push_frame` 送入原始帧 → `pull_packet` 拉取编码包
- B 帧重排：一次 push 可能产出 0-N 个包
- `flush()` 清空缓冲，EOF 信号

### VideoDecoder

```rust
pub trait VideoDecoder: Send {
    fn push_packet(&mut self, packet: &EncodedPacket) -> Result<(), CodecError>;
    fn pull_frame(&mut self) -> Result<Option<VideoFrame>, CodecError>;
    fn flush(&mut self) -> Result<(), CodecError>;
    fn reset(&mut self) -> Result<(), CodecError>;
}
```

编码镜像：`push_packet` → `pull_frame`。

## 后端矩阵

| 后端 | Feature | 链接 | 场景 |
|------|---------|------|------|
| GStreamer | `backend-gstreamer` | 动态 .so | Host 编码默认 |
| FFmpeg | `backend-ffmpeg` | 静态 .a | Remote 解码（str0m 后端，D72）；Host 编码备选（D43 修订） |
| Stub | 无 feature | 无 | 开发/测试/编译检查 |

Host 端双后端可切换：
```toml
# Cargo.toml (mediaservo-host)
mediaservo-codec = { features = ["backend-ffmpeg"] }   # 生产/edge 静态分发
# mediaservo-codec = { features = ["backend-gstreamer"] } # 开发机默认
```

## 编码/解码链路

### Host 编码

```
Capture → VideoFrame(I420) → CodecFactory::create_encoder()
  → VideoEncoder::push_frame() → pull_packet() → EncodedPacket
  → TrackSender::write_frame(packet.data) → WebRTC RTP → Server
```

### Remote 解码

```
TrackReceiver → EncodedPacket(H.264 bytes)
  → CodecFactory::create_decoder()
  → VideoDecoder::push_packet() → pull_frame() → VideoFrame(I420)
  → Renderer
```

## 数据边界 (C5)

codec crate **不依赖** `mediaservo-media`。所有数据交换通过 `&[u8]`：

- `VideoFrame`（codec 自有类型，不同于 media 的 `VideoFrame<T>`）
- `EncodedPacket`（纯 `Vec<u8>` + 元数据）
- 调用方负责 media ↔ codec 类型转换（展平 I420Buffer 或包装 EncodedFragment）

## 工厂入口

```rust
pub struct CodecFactory;

impl CodecFactory {
    pub fn new() -> Self;

    pub fn create_encoder(
        &self, config: EncoderConfig,
        preferred_backend: Option<BackendId>,
    ) -> Result<Box<dyn VideoEncoder>, CodecError>;

    pub fn create_decoder(
        &self, config: DecoderConfig,
        preferred_backend: Option<BackendId>,
    ) -> Result<Box<dyn VideoDecoder>, CodecError>;

    // Capability discovery
    pub fn encoder_capabilities(&self, codec: CodecId) -> Vec<EncoderCapability>;
    pub fn decoder_capabilities(&self, codec: CodecId) -> Vec<DecoderCapability>;
}
```

使用示例：

```rust
use mediaservo_codec::{CodecFactory, EncoderConfig, CodecId, BackendId};

let factory = CodecFactory::new();
let mut encoder = factory.create_encoder(
    EncoderConfig::builder(CodecId::H264, format)
        .bitrate(Bitrate::Vbr { target: 4000, max: 8000 })
        .preset(EncoderPreset::VeryFast)
        .build(),
    Some(BackendId::FFmpeg),  // 强制 FFmpeg 静态后端
)?;

for raw_frame in capture_source {
    encoder.push_frame(&raw_frame)?;
    while let Some(pkt) = encoder.pull_packet()? {
        webrtc_track.write_frame(&pkt.data).await?;
    }
}
encoder.flush()?;
```

## 文件结构

```
crates/mediaservo-codec/
├── Cargo.toml
├── src/
│   ├── lib.rs              re-exports + feature gate guards
│   ├── codec.rs            CodecId, PixelFormat, VideoFormat, FrameRate
│   ├── config.rs           EncoderConfig, DecoderConfig + builders
│   ├── encoder.rs          VideoEncoder trait
│   ├── decoder.rs          VideoDecoder trait
│   ├── factory.rs          CodecFactory + EncoderCapability/DecoderCapability
│   ├── error.rs            CodecError (thiserror)
│   ├── frame.rs            VideoFrame, Plane
│   ├── packet.rs           EncodedPacket
│   └── backend/
│       ├── mod.rs          cfg dispatch + compile_error! guards
│       ├── gstreamer.rs    GstEncoder, GstDecoder (dynamic)
│       ├── ffmpeg.rs       FfmpegEncoder, FfmpegDecoder (static)
│       └── stub.rs         StubEncoder, StubDecoder (dev)
├── tests/
│   ├── integration_roundtrip.rs
│   ├── cross_backend.rs
│   ├── perf_bench.rs
│   └── ...
└── benches/
    └── encode_bench.rs
```
## API 边界与类型映射

codec crate 不依赖 mediaservo-media，通过 &[u8] 交换数据（C5）。

| codec 类型 | media/webrtc 类型 | 转换方向 |
|-----------|-------------------|---------|
| codec::VideoFrame | media::VideoFrame<I420Buffer> | media→codec: 展平 I420Buffer plane |
| codec::EncodedPacket | webrtc::TrackSender::write_frame(&[u8]) | codec→webrtc: 编码包字节 → 传输 |
| codec::EncodedPacket | webrtc::TrackReceiver (原始字节) | webrtc→codec: 接收编码字节 → 解码器 |

**注**: webrtc crate 当前未暴露原始编码包接收 API（Phase 2 预留）。

## 错误处理

CodecError (thiserror)，关键变体：

| 变体 | 说明 |
|------|------|
| InvalidConfig(String) | 无效编码器配置 |
| UnsupportedCodec | 后端不支持该编解码器 |
| NoBackend(CodecId) | 无后端编译（缺少 feature flag） |
| Encoder(String) | 编码运行时错误 |
| Decoder(String) | 解码运行时错误 |
| InvalidState(String) | 非法状态转换 |
| ResourceExhausted(String) | 内部队列满 |
| InvalidInput(String) | 输入数据格式不匹配 |

## 跨平台

| 后端 | macOS x64 | macOS ARM | Linux x64 | Linux ARM | Windows |
|------|:---:|:---:|:---:|:---:|:---:|
| GStreamer | brew | brew | apt | apt | MSYS2 |
| FFmpeg 静态 | 预构建 | 预构建 | 预构建 | 交叉编译 | vcpkg |
| FFmpeg 动态 | brew | brew | apt | apt | vcpkg |

详见 docs/reference/codec/ffmpeg-static-build-strategy.md。


## 测试覆盖 (69 tests, 规划中)

| 类 | 测试数 | 说明 |
|-----|--------|------|
| Unit: encoder config/lifecycle | 15 | configure, push/pull, flush, error |
| Unit: push-pull loop | 10 | 30 frame encode, PTS monotonic |
| Unit: decoder | 11 | decode lifecycle, reset, close |
| Unit: codec selection | 3 | probe, auto-select, fallback |
| Integration: roundtrip | 5 | I420→H.264→I420, PSNR >40dB |
| Integration: pipeline | 2 | as MediaProcessor in PipelineEngine |
| Cross-backend | 4 | GStreamer vs FFmpeg output comparison |
| Performance | 5 | encode latency p99 <33ms, throughput ≥30fps |
| Static build | 4 | nm verify no undefined symbols, size <14MB |
| Fuzz | 6 | random I420 input, random bitstream, concurrent |
| Property | 5 | keyframe interval, PTS monotonic |

## 当前状态

| 能力 | 状态 |
|------|------|
| VideoEncoder trait 定义 | 🔲 规划中 |
| VideoDecoder trait 定义 | 🔲 规划中 |
| CodecFactory | 🔲 规划中 |
| FFmpeg 后端 | 🔲 规划中 |
| GStreamer 后端 | 🔲 规划中 |
| Stub 后端 | 🔲 规划中 |
| TDD 测试 | 🔲 69 tests 规划中 |
| E2E 验收 | 🔲 8 scenarios 规划中 |
| 静态 FFmpeg 构建 | 🔲 策略已设计 |

## 关联文档

- [17. WebRTC Crate](17-webrtc-crate.md) — WebRTC 传输层，编码后输出目标
- [08. 管线模型参考](08-pipeline-model.md) — PipelineEngine 集成
- [决策记录 D43/D46/D70/D71/D82](../.agents/memorys/decisions.md) — 编码架构决策链
- [FFmpeg 静态构建策略](../reference/codec/ffmpeg-static-build-strategy.md) — 构建方案
- [SDD 验收标准](../../docs/plans/mediaservo-codec/acceptance-criteria.md) — 具体阈值
- [验收矩阵 §10](../../docs/plans/mediaservo-codec/acceptance-criteria.md#10-test-matrix) — 20 场景 AC 矩阵

## 交叉引用

以下文档链接回本文档：
- [17. WebRTC Crate](17-webrtc-crate.md) — Phase 2 预留 write_frame / on_encoded_packet API
- [SDD 验收标准](../../docs/plans/mediaservo-codec/acceptance-criteria.md) — 对齐 push-pull trait API
- [FFmpeg 静态构建策略](../reference/codec/ffmpeg-static-build-strategy.md) — 预构建 + CI 集成
- [验收矩阵 §10](../../docs/plans/mediaservo-codec/acceptance-criteria.md#10-test-matrix) — 8+ 场景测试矩阵
