# MediaServo 架构设计

> Phase 0 — 架构定义 | 2026-07-16 | 最后同步 decisions.md: D205 (2026-07-31)

> **⚠️ MVP v2 架构变更 (D118, 2026-07-17)**: Phase 1 MVP 采用 Host→Server→Remote relay 三组件模式（原为 Host↔Remote P2P）。信令 relay 和媒体 relay 均由 mediaservo-server 承载。完整 P2P/Host↔Remote 直连模式保留为 Phase 2+ 架构目标。

## 1. 概述

MediaServo 是可独立部署的多媒体基础设施，提供远程桌面、视频会议、直播推拉流、监控相机接入等能力。采用微内核 + 插件架构，支持多形态部署。

### 1.1 七个产品能力

| 能力 | Phase | 说明 | 典型场景 |
|------|-------|------|---------|
| 远程桌面 (Phase 1) | Phase 1 | 屏幕捕获、GPU 编码、输入注入 | IT 运维、远程办公 |
| 视频会议 (Phase 2+) | Phase 2+ | 多方音视频、SFU/MCU、屏幕共享 | 团队协作 |
| 推拉流 (Phase 2+) | Phase 1 | RTMP/HLS/SRT 接入与分发 | 直播、内容分发 |
| 监控相机 (Phase 2+) | Phase 2+ | ONVIF/GB28181 发现与流管理 | 安防、巡检 |
| WebRTC 遥操作 (Phase 1) | Phase 1 | 低延迟视频 + RTCDataChannel 控制 | 车辆遥控、机器人 |
| 车端推流 (Phase 1) | Phase 1 | 车辆摄像头推流到云端 | 车联网 |
| 录制与回放 (Phase 2+) | Phase 2+ | 录制管理、存储与回放（D61: 推迟至 Phase 2） | 合规、审计 |

> 注：WebRTC 遥操作、车端推流、舱内拉流三个子能力均属遥操作域（teleop domain），由 field/client 等 SDK 承载（见 §7）。

> 🔄 **D126-D155 增量 (2026-07-19)**: 三层逻辑抽象模型 (D126)、Component 架构 (D127-D134)、WebRTC 架构升级为 webrtc-sys RTP track (D137)、mediasoup SFU (D138)、统一 Gateway (D128)、多后端 feature gate (D139-D140, webrtc-sys/webrtc-rs/stub, str0m Phase 2+)、webrtc-kit 设计借鉴 (D144-D151)、Host 单体架构确认 (D155)。已实施完成（Phase 3）。

> ⚠️ **D15/D118 范围变更**: Phase 1 原为 Host↔Remote P2P 直连模式，现已变更为 Host→Server→Remote relay 三组件模式。详见 [模块文档](modules/03-client-host.md) 和 §2。

### 1.2 与第三方平台的关系

```
工业控制端         ──┐              ┌── 宿主平台 (企业应用)
                     ├── MediaServo ──┤
   引用 native crate │  多媒体核心   │ Docker 模块
                     │              │
                     └──────────────┘
```

- **与第三方平台**：可选嵌入，通过 Rust crate 静态链接，仅使用远程桌面和遥操作能力
- **与宿主平台**：零硬依赖。宿主平台可运行 MediaServo 作为 Docker 模块（类比群晖 Surveillance Station）。此时用户/权限委托给平台 RBAC/LDAP
- **独立部署**：可脱离第三方平台完全独立运行，自带完整后端

### 1.3 设计原则

| 原则 | 说明 | 关联决策 |
|------|------|---------|
| **多后端编译期分发** | WebRTC 多后端 (webrtc-sys/webrtc-rs/stub, str0m Phase 2+) 通过 `#[cfg(feature)]` 编译期 dispatch，无运行时 dyn trait 开销。借鉴 webrtc-kit trait 抽象模式 | D144-D151 |
| **零 WebRTC 依赖内核** | mediaservo-common 不依赖任何 WebRTC crate，MediaTransport trait 为纯抽象接口 | D150 |
| **Component 服务层** | 服务级 Component 与管线级 Plugin 分离，三层抽象模型 | D126 |
## 2. 三层架构

> 📄 详见 [modules/03-client-host.md](/docs/modules/03-client-host.md)

MediaServo 采用控制面与数据面分离的三层架构：后台服务集中管理用户、设备、权限、License 及信令；Client/Host 数据面通过 gRPC/REST 与控制面通信。Client 和 Host 双应用设计源自 D2 决策——Host 需运行在无桌面环境的平台（Linux 服务器、车端边缘设备），双应用可减少 Host 体积。

> 🔄 **D128 Gateway 路由**: 统一端口 :9800 对外暴露，Gateway Component 内部路由分派：`/ws` → SignalingComponent, `/admin/api/*` → AdminComponent, `/admin/*` → SPA 静态文件, `/health` → MonitorComponent。
```
┌──────────────────────────────────────────────────────────────────┐
│                     MediaServo 后台服务                            │
│                                                                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐ │
│  │ 用户管理  │ │ 权限控制  │ │ License  │ │ 设备管理  │ │ 信令   │ │
│  │          │ │          │ │ 管理     │ │          │ │ 服务   │ │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └────────┘ │
│                                                                  │
│  Auth Provider (trait): Local 模式 | 平台模式      | 未来 LDAP   │
│                                                                  │
│  控制面：鉴权 / 授权 / 功能开关 / 流数限制 / 分辨率限制 / 到期     │
└────────────────────────────┬─────────────────────────────────────┘
                             │ gRPC / REST
        ┌────────────────────┼────────────────────┐
        ▼                    ▼                    ▼
┌───────────────────┐ ┌───────────────────┐ ┌───────────────────┐
│ MediaServo Client  │ │ MediaServo Host    │ │ 其他客户端/AI     │
│ (桌面 GUI)        │ │ (Headless)        │ │                   │
│                   │ │                   │ │                   │
│ 全量 SDK          │ │ 仅生产 SDK        │ │ 按需集成          │
│ 生产 + 消费       │ │ capture/encode    │ │                   │
│                   │ │ /push             │ │                   │
│ ┌───────────────┐ │ │                   │ │                   │
│ │ Tauri v2│ │ │ embedded web 配置  │ │                   │
│ └───────────────┘ │ │                   │ │                   │
├───────────────────┤ ├───────────────────┤ │                   │
│ 场景：操作员桌面  │ │ 场景：车端/机房/   │ │                   │
│ 可远程控制他人    │ │ 边缘设备/摄像头    │ │                   │
│ 也可被远程控制    │ │ 仅产出媒体流      │ │                   │
└───────────────────┘ └───────────────────┘ └───────────────────┘

        客户端/SDK 层 — 数据面：采集 / 编码 / 传输 / 解码 / 渲染
```

| 层 | 职责 | 技术 |
|---|---|---|
| **后台服务** | 控制面：用户、设备、权限、License、信令 | Rust (axum/tonic) |
| **Client** | 桌面 GUI，全功能操作端 | Tauri v2 + mediaservo-common |
| **Host** | Headless 远端，纯产出媒体流 | mediaservo-common (无 GUI 依赖) |
| **SDK** | 核心管线 + 领域插件 | Rust crate 体系 |

### 2.1 Client vs Host

| | MediaServo Client | MediaServo Host |
|---|---|---|
| **运行环境** | 桌面操作系统 | 服务器 / 边缘 / 车端 / 无桌面 |
| **GUI** | Tauri v2 | Embedded Web (localhost 配置页) |
| **SDK** | 全量（生产 + 消费） | 仅生产（capture, encode, push） |
| **角色** | 可控制他人，也可被控制 | 仅产出媒体流 |
| **安装** | 桌面安装包 | 单一二进制 `mediaservo-host` |
| **体积** | 大（含 GUI 框架） | 小（无 GUI 依赖） |

双应用而非单应用的决策原因：Host 需要运行在没有桌面环境的平台上（无 GUI 的 Linux 服务器、车端嵌入式设备）。

> Phase 2+ 特性：Headless Host 支持 virtual display（虚拟显示），无需物理显示器即可产出合成画面（测试、CI、云端渲染场景）。

## 3. 账户与权限

> 📄 详见 [modules/05-auth-permissions.md](/docs/modules/05-auth-permissions.md)

Auth Provider trait 实现双模式认证：Local 模式（内嵌 SQLite + JWT）用于独立部署；平台模式委托平台 RBAC/LDAP，适用于 Docker 模块场景。权限模型涵盖功能开关、配额限制（流数/码率/分辨率/时长）和 License 控制（trial/standard/enterprise）。客户端 SDK 启动时从后台拉取权限配置，缓存在本地，License Manager 在每个管线操作前校验。

### 3.1 Auth Provider 模式

```
                    ┌──────────────────────────┐
                    │    MediaServo Backend      │
                    │                           │
                    │    AuthProvider (trait)    │
                    │    ┌───────────────────┐   │
                    │    │ authenticate()    │   │
                    │    │ authorize()       │   │
                    │    │ getPermissions()  │   │
                    │    └───────┬───────────┘   │
                    │            │               │
                    │    ┌───────┴───────┐       │
                    │    ▼               ▼       │
                    │  Local         平台模式     │
                    │  (SQLite+JWT)  (gRPC LDAP)  │
                    └──────────────────────────┘
```

```rust
#[async_trait]
trait AuthProvider: Send + Sync {
    async fn authenticate(&self, credential: &Credential) -> Result<User, AuthError>;
    async fn authorize(&self, user_id: &str, permission: &Permission) -> Result<bool, AuthError>;
    async fn get_permissions(&self, user_id: &str) -> Result<HashSet<Permission>, AuthError>;
}
```

### 3.2 双模式

| | Local（独立模式） | 平台模式 |
|---|---|---|
| **用户存储** | 内嵌 SQLite | 委托平台 RBAC |
| **认证** | 本地 JWT | 宿主平台签发 token |
| **权限** | 本地 RBAC 表 | 宿主平台 LDAP 组映射 |
| **配置** | `auth.mode: "local"` | `auth.mode: "aude"` |
| **场景** | 独立部署 | Docker 平台模块 |

### 3.3 权限模型

```typescript
interface Permission {
  // 功能开关
  capabilities: {
    streaming:  { push: boolean; pull: boolean };
    remote:     { control: boolean; controllable: boolean };
    conference: { host: boolean; join: boolean };
    surveillance: boolean;
    teleop:     { operator: boolean; vehicle: boolean };
  };
  
  // 配额限制
  quotas: {
    max_streams: number;        // 最大流数
    max_bitrate: number;        // 最大码率 (kbps)
    max_resolution: "720p" | "1080p" | "4k";
    max_duration: number;       // 最长会话时长 (秒)
  };
  
  // License
  license: {
    type: "trial" | "standard" | "enterprise";
    expires_at: string;          // ISO 8601
    features: string[];          // 高级特性列表
  };
}
```

权限流：客户端 SDK 启动时从后台拉取权限配置 → 缓存在本地 → License Manager 在每个管线操作前校验。

### 3.4 参考模型

类比群晖 DSM：MediaServo 作为 Docker 模块安装在宿主平台上，使用宿主平台的用户/权限系统（类似 Jira 安装在群晖上使用 DSM 的 LDAP 账户）。

## 4. 插件体系

> 📄 详见 [modules/06-plugin-system.md](/docs/modules/06-plugin-system.md)

微内核 mediaservo-common 承载 PluginManager、LicenseManager、ProtocolBroker、PipelineEngine、AuthProvider 五大核心组件。插件按领域分为生产类（Host 采集/编码/推流）、消费类（Client 解码/渲染/拉流）、协议类（RTMP/HLS/SRT/RTSP/WebRTC）和中继类（STUN/TURN/SFU）。三层 trait 层次（Plugin → MediaSource/Processor/Sink → 具体实现）提供清晰的扩展边界。

### 4.0 Component 服务层 (D126-D134)

> 📄 详见 [modules/15-component-architecture.md](/docs/modules/15-component-architecture.md)

区别于 D1 的部署拓扑三层，D126 定义代码维度的三层逻辑抽象：

| 层 | 抽象 | 管理方式 | Phase 1 实例 |
|---|---|---|---|
| **Layer 3 Process** | OS 进程 | systemd/K8s | 单进程 |
| **Layer 2 Component** | 服务级单元 | ComponentManager | Gateway / Signaling / Relay / Admin / Auth / Monitor |
| **Layer 1 Plugin** | 管线元素 | PipelineEngine | ScreenCapture / HardwareEncoder / RtmpPublisher (Phase 2+) |

- **Plugin** 回答「怎么做」(如何编码 H.264)——通过 MediaPort (帧队列) 通信
- **Component** 回答「做什么」(管理 WebRTC 信令)——通过 ComponentBus (消息) 通信
- Component 内部可持有 Plugin 实例（通过 PipelineEngine）

**Component/Plugin/Process 部署映射**：

| 逻辑层 | 部署位置 | Phase 1 示例 |
|---|---|---|
| **Plugin 层** | 任意 Component 内 | ScreenCapture, H264Encoder |
| **Component 层** | Host / Server / Remote 进程 | SignalingComponent, RelayComponent, GatewayComponent |
| **Process 层** | OS 进程 (systemd/K8s) | hostd, mediaservo-server, mediaservo-client |

Component trait 位于独立 crate `mediaservo-component` (D127)，采用 init→run→shutdown 三阶段生命周期 (D131)，ComponentBus 支持 RPC + Event 双模式路由 (D132-D133)，ComponentManager 提供单层监督 + crash-loop 防护 (D134)。

### 4.1 微内核架构

```
mediaservo-common (微内核)
├── PluginManager     — 插件注册、生命周期
├── LicenseManager    — 权限校验、配额控制
├── ProtocolBroker    — 内部协议路由 (FlatBuffers)
├── PipelineEngine    — 媒体管线调度
└── AuthProvider      — 认证接口 (trait)

插件层 (按领域)
├── 生产类 (Host)
│   ├── ScreenCapture     — 屏幕捕获 (DXGI/PipeWire/CoreGraphics)
│   ├── CameraCapture     — 摄像头采集 (V4L2/AVFoundation/DirectShow)
│   ├── AudioCapture      — 音频采集
│   ├── HardwareEncoder   — GPU 编码 (NVENC/VAAPI/VideoToolbox/QSV)
│   ├── StreamPublisher   — 推流 (RTMP/SRT/WHIP)
│   └── InputReceiver     — 输入接收 (键鼠/触控)
│
├── 消费类 (Client)
│   ├── ScreenRender      — 画面渲染
│   ├── VideoDecode       — 视频解码
│   ├── AudioPlayback     — 音频播放
│   ├── StreamSubscriber  — 拉流
│   └── InputForwarder    — 输入转发
│
├── 协议类
│   ├── RtmpPlugin        — RTMP 接入/分发
│   ├── HlsPlugin         — HLS 打包
│   ├── SrtPlugin         — SRT 传输
│   ├── RtspPlugin        — RTSP 接入
│   ├── WebRtcPlugin      — WebRTC P2P/SFU
│   └── DataChannelPlugin — WebRTC RTCDataChannel
│
└── 中继类
    ├── StunTurnPlugin    — NAT 穿透
    └── SfuRelayPlugin    — SFU 媒体转发
```

### 4.2 核心 Trait

```rust
/// 插件基础 trait
#[async_trait]
trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> (u16, u16, u16);
    fn category(&self) -> PluginCategory;
    async fn init(&mut self, ctx: &PluginContext) -> Result<()>;
    async fn shutdown(&mut self) -> Result<()>;
}

/// 媒体源（生产）
#[async_trait]
trait MediaSource: Plugin {
    fn output_port(&self) -> &MediaPort;
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
}

/// 媒体处理器
#[async_trait]
trait MediaProcessor: Plugin {
    fn input_port(&self) -> &MediaPort;
    fn output_port(&self) -> &MediaPort;
    async fn process(&mut self, frame: MediaFrame) -> Result<MediaFrame>;
}

/// 媒体汇（消费）
#[async_trait]
trait MediaSink: Plugin {
    fn input_port(&self) -> &MediaPort;
    async fn consume(&mut self, frame: MediaFrame) -> Result<()>;
}
```

/// 多后端传输抽象（ponytail: mediaservo-common 定义此 trait，mediaservo-webrtc 的 RTCPeerConnection 作为实现层。Phase 2+ 迁入 mediaservo-common 的 MediaTransport trait 与 webrtc crate 解耦。详见 docs/modules/17-webrtc-crate.md）
/// mediaservo-common 不依赖任何 WebRTC crate，保证独立编译测试
#[async_trait]
pub trait MediaTransport: Send + Sync {
    async fn create_video_track(&self, source_id: &str) -> Result<TrackLocal, TransportError>;
    async fn create_audio_track(&self) -> Result<TrackLocal, TransportError>;
    async fn on_remote_track(&self, callback: Box<dyn Fn(TrackRemote) + Send + Sync>);
}

# ponytail: MediaTransport trait — Phase 0 webrtc-sys 为主力后端，webrtc-rs 已有 struct 骨架，str0m Phase 2+。完整多后端 trait 体系 Phase 2+。

# mediaservo-common 零 WebRTC 依赖 (D150)。

### 4.3 插件加载方式

✅ 已定：编译期 inventory + 运行时 dlopen 双模式 (D13, D29)

> ⚠️ 运行时 dlopen 风险评估：动态加载存在 ABI 兼容性风险（插件与宿主 Rust 版本不一致、依赖版本冲突），Phase 1 优先使用编译期 inventory 模式。dlopen 模式保留为 Phase 2+ 高级特性，需配套插件版本协商协议及隔离沙箱。
- ⚠️ 风险补充: 运行时 plugin 动态加载在实时媒体系统的生产环境中先例有限。OBS/LVQR/MediaMTX 均采用编译期组合 (Phase 1 选择)。

## 5. 管线模型

> 📄 详见 [modules/08-pipeline-model.md](/docs/modules/08-pipeline-model.md)

采用 LVQR 的 Unified MediaFragment Model，内部统一格式避免 N×M 协议转换矩阵。GStreamer 处理协议解析、编解码、打包等成熟热路径；自研路径覆盖 GPU 编码桥接、sans-I/O WebRTC 核心、RTCDataChannel 控制协议和输入注入管道。

### 5.1 设计选择

不采用 GStreamer 的有向图模型，而是参考 **LVQR 的 Unified Fragment Model**：

```
┌──────────────────────────────────────────────────┐
│  输入适配层                                       │
│  RTMP · SRT · RTSP · WHIP · 屏幕采集 · 摄像头     │
│          │                                       │
│          ▼                                       │
│  ┌──────────────────────────────┐                │
│  │    Unified MediaFragment     │  ← 内部统一格式 │
│  │    (编码帧 + 元数据)          │                │
│  └──────────────────────────────┘                │
│          │                                       │
│          ▼                                       │
│  PipelineEngine (可编程处理链)                    │
│          │                                       │
│          ▼                                       │
│  输出适配层                                       │
│  HLS · DASH · WHEP · WebRTC · 渲染 · 录制         │
└──────────────────────────────────────────────────┘
```

优势：
- 避免 N×M 协议转换矩阵
- 每个输入/输出协议只是一层薄的 adapter
- 中间处理链可编程组合

Phase 2+: 支持 Simulcast/SVC 多层编码 — 单一输入源产出多个质量层，由 SFU/客户端按带宽自适应选择 (Zoom/Jitsi/LiveKit 均有此能力)。

### 5.2 GStreamer 热路径

在以下场景使用 GStreamer 作为实现引擎：
- 协议解析（RTMP/RTSP/SRT 解析）
- 编解码（通过 `videoconvertscale` 合并变换）
- HLS/DASH 打包
- `webrtcbin2`（Rust 重写，每会话节省 5 线程）

### 5.3 自研热路径

- GPU 编码桥接（通过 `libloading` 直接调用 NVENC/VAAPI，避免 GStreamer 编解码开销）
- 零拷贝路径: capture→encode→network 全程 GPU 内存 (dmabuf/CUDA 句柄)，参考 Parsec 7ms 端到端延迟标杆
- sans-I/O WebRTC 核心（str0m / webrtc-rs rtc）
- RTCDataChannel 控制协议（自定义二进制协议）
- 输入注入管道（键鼠/触控低延迟转发）

**数据边界约束 (C5/D155)**: GStreamer 与 WebRTC 之间仅允许 `&[u8]` 字节传递。禁止 GStreamer buffer 直接传入 libwebrtc（内存分配器不兼容: glib malloc vs C++ new）。

### 5.4 数据语义

| 数据类别 | 语义 | 说明 |
|---------|------|------|
| 视频帧 | at-most-once | 丢帧可接受，关键帧+delta帧可跳过 |
| 控制指令 | at-least-once | 必须送达，重传+去重 |
| 紧急停止 | exactly-once (best-effort) | 独立UDP通道，多路冗余发送 |

## 6. 部署形态

> 📄 详见 [modules/02-deployment-modes.md](/docs/modules/02-deployment-modes.md)

四种部署形态覆盖从嵌入式链接到独立部署的全场景：Embed（~5 插件，第三方平台嵌入）、Sidecar（~12 插件，宿主平台容器旁路）、Standalone（全插件 + Web UI）、平台模块（Docker 容器，委托平台认证）。

```
┌─────────────────────────────────────────────────────────────┐
│  形态           架构                     适用场景           │
├─────────────────────────────────────────────────────────────┤
│  Embed         Rust crate 静态链接   第三方平台嵌入远程     │
│                → 约 5 个插件           桌面 + 遥操作         │
│                                                             │
│  Sidecar       容器 + napi-rs 绑定     宿主平台企业应用    │
│                → 约 12 个插件                               │
│                                                             │
│  Standalone    独立进程 + 完整后端    独立部署场景          │
│                → 全插件 + Web UI                            │
│                                                             │
│  平台           Docker 容器模块       融入宿主平台         │
│  模块          → 委托平台认证                               │
└─────────────────────────────────────────────────────────────┘
```

## 7. SDK 分层

> 📄 详见 [modules/04-sdk-layers.md](/docs/modules/04-sdk-layers.md)

四 SDK 主架构 (D222-D232)：**link**（连接）/ **field**（组合会话）/ **client**（舱端消费编排）/ **deck**（媒体数据）。替代早期 field/remote 双 facade 模型（D65-D69/D82 已废止）。

- **link**（连接面，纯 Rust）：frame_bus 帧总线、signal WS 信令客户端、auth 集成（复用 common PSK/JWT）、dc（Phase2, webrtc-rs）
- **field**（组合 SDK）：push/pull（webrtc-sys, C12）+ re-export link（信令/总线/auth）+ re-export deck（采集 MediaDevices/VideoSource）——一行依赖完整闭环；field→deck **静态直连默认**（D231，FFmpeg 无 openssl 实证）
- **client**（舱端消费编排）：VideoRenderer 渲染、多路会话编排、Input Forward/遥测、deck playback 集成；依赖 field（D232）
- **deck**（媒体数据面，双形态）：source 采集（GStreamer）/ codec（FFmpeg 静态）/ record / playback

依赖全单向无环：`field→(webrtc, link, deck)`；`client→field`；`deck→(codec, media, common)`。mediaservo-common/webrtc/media/codec 为底层基础。

```
mediaservo-link（连接面, 纯 Rust）          mediaservo-deck（媒体数据面, 双形态）
  ▲ rlib                                    ├─ rlib: 应用单独使用
  │                                         ├─ rlib full: field 静态依赖（默认）
mediaservo-webrtc（抽象层, C12）              └─ cdylib: deck-full.so（dlopen 可选, OTA）
  ▲ backend-webrtc-sys / backend-webrtc-rs
  │
mediaservo-field（组合 SDK = webrtc + link + deck）
  ├─ 媒体: push/pull（backend-webrtc-sys）
  ├─ 通信: signal/frame_bus/auth（re-export link）
  └─ 采集: MediaDevices/VideoSource（re-export deck[source]）
      │ 静态依赖（构建期合并, 运行时自含）
      ▼
mediaservo-client（舱端消费编排）── 依赖 field ──（+ deck playback 集成）
```

Phase 2+: 视频会议 / 直播 / 监控子场景在 field 会话面叠加场景 API；绑定体系 c/cxx/py 双后端（见 modules/04-sdk-layers.md §八）。

### 7.1 客户端 UI Module

MediaServo Client 是统一桌面应用。按后台返回的权限动态加载 UI Module：

```typescript
const permissions = await backend.getPermissions(userId);

// 权限驱动 UI
const modules: Module[] = [];
if (permissions.streaming) modules.push(StreamingModule);    // 推拉流 tab
if (permissions.remote)    modules.push(RemoteDesktopModule); // 远程桌面 tab
if (permissions.conference) modules.push(ConferenceModule);   // 视频会议 tab
if (permissions.surveillance) modules.push(SurveillanceModule); // 监控 tab

// 无权限的模块完全不加载
```

## 8. 协议与通信

> 📄 详见 [modules/07-protocols.md](/docs/modules/07-protocols.md)

协议栈分五层：内部 IPC（FlatBuffers 零拷贝）、第三方平台集成（C FFI 静态链接）、宿主平台集成（napi-rs 原生模块）、后台控制面（gRPC + REST）和媒体数据面（RTP/SRT/WebRTC）。信令采用自研 WebSocket 并长期演进（v2=控制域、v3=会话续期，S0/S0.5 已落地）；MQTT 5.0 通道制**不再是默认双轨承诺**——出现离线指令下发/大规模 presence 场景才触发评估，前置闸=「会话状态外置」改造（本仓「连接=会话」模型与 broker 总线模型不可混用）。裁决=F13/D289（预约入账）。

| 层次 | 协议 | 说明 |
|------|------|------|
| **内部 IPC** | FlatBuffers | 插件间零拷贝通信 |
| **第三方平台** | C FFI | Rust → C 静态链接 |
| **宿主平台** | napi-rs | Rust → Node.js 原生模块 |
| **后台服务** | gRPC (protobuf) | 控制面 API |
| **信令** | 自研 WebSocket（长期；MQTT=触发条款，D289 改判 D74 双轨承诺） | 房间管理、SDP/ICE 交换，详见 [信令架构文档](modules/10-signaling-architecture.md) |
| **媒体传输** | RTP/RTCP, SRT, WebRTC | 数据面 |
| **客户端 ⇄ 后台** | gRPC + REST | 认证、权限拉取、配置同步 |
| **未来探索** | MoQ (Media-over-QUIC) | Phase 3 评估: 低延迟媒体传输, WebTransport |


## 9. Cargo Workspace 结构

当前 workspace 含 7 个 Cargo member crate（D126-D155 增量）：mediaservo-common (微内核), mediaservo-host, mediaservo-client, mediaservo-server, mediaservo-webrtc (D137 新增, RTP track API), mediaservo-media (媒体管线引擎), mediaservo-codec (编解码三后端: stub+FFmpeg+GStreamer)。

```
crates/
├── mediaservo-common/         微内核 (PluginManager, LicenseManager, PipelineEngine, AuthProvider trait)
├── mediaservo-host/         Host 应用 (headless, 采集+编码+推流+信令+配置, 单体架构 D155)
├── mediaservo-client/       Remote 应用 (拉流+解码+渲染+控制)
├── mediaservo-server/       Server 应用 (信令 relay+监控+会话管理, mediasoup SFU; C21: mediasoup 仅限本 crate 依赖树，host/client 禁止依赖)
├── mediaservo-webrtc/       WebRTC 封装 (W3C API, webrtc-sys 默认后端/libwebrtc, 多后端 feature gate; C12: 所有 client crate 仅经此层使用 WebRTC，禁止与 mediasoup 同链)
├── mediaservo-media/        媒体管线 (PipelineEngine, BroadcastEngine, Transform 接口)
└── mediaservo-codec/        编解码 (stub + FFmpeg 静态链接 + GStreamer pixi)
  
Phase 2+ 计划 crates: mediaservo-component, mediaservo-transport, mediaservo-signaling, mediaservo-pipeline, mediaservo-auth, mediaservo-field, mediaservo-field-c, mediaservo-client-c, mediaservo-napi
```

## 10. 技术选型

| 组件 | 选型 | 理由 |
|------|------|------|
| **核心语言** | Rust (edition 2024) | 零成本抽象、内存安全、跨平台 |
| **WebRTC 栈** | webrtc-sys (libwebrtc FFI, 默认, D137+D139) → webrtc-rs (Phase 0 struct 骨架, 纯 Rust, D32) → str0m (Phase 2+, Embed, D11) | 多后端编译期 feature gate 分发 (D139-D140, D144-D151) |
| **GPU 编码** | libloading 桥接 NVENC/VAAPI/VT | 避免编译时绑定 GPU SDK |
| **编解码** | GStreamer (gst-plugins-rs) | 覆盖全，生态成熟 |
| **信令** | 自研 WebSocket（长期；MQTT=触发条款，D289） | 统一 Room + Protobuf 双格式 (D52-D53) |
| **内部协议** | FlatBuffers | 零拷贝、多语言 |
| **桌面 GUI** | Tauri v2 + React | 轻量、Rust 后端、跨平台 |
| **嵌入式 Web** | axum + 静态 HTML | Host 配置页，无框架依赖 |
| **构建** | Cargo workspace | 统一依赖管理 |
| **Web UI** | React 19 + Ant Design 5 | Server 管理面板, 宿主平台生态复用 (D87) |
| **数据库** | sqlx + SQLite (D101) | 编译期 SQL 校验, async, 轻量 |
| **配置** | serde_yaml + 环境变量 (D101) | 敏感值 env 覆盖 (JWT_SECRET 等) |
| **可观测性** | tracing JSON stdout + prometheus-client (D99) | Docker logs 收集 + /metrics 端点 |
| **CI/CD** | GitHub Actions 三阶段 (D-CI-01, D101) | check → test → build+docker push |
| **部署** | tarball + install.sh (Phase 1, D104) → Docker Compose (Phase 2, D110) | 渐进式部署策略 |
| **Host IPC** | tokio::mpsc (Phase 1) → iceoryx2 SHM (Phase 2, D102) | Phase 1 单进程内部通信 |
| **SDK 构建** | cbindgen + 手写 CMake/pc (D109, Phase 2: cargo-c) | .a + .so + .h (cbindgen) + .pc (pkg-config) |
| **SDK 安装** | /usr/local (D106) | FHS 标准, pkg-config 默认搜索 |
- Phase 2+: Alertmanager 用于 Prometheus 告警路由
| **SFU 引擎** | mediasoup-sys 0.14.1 (mediasoup 0.24.1) | 生产级 Rust 绑定, Router/Transport/Producer/Consumer 模型 (D138) |
| **Component Bus** | tokio::mpsc (Phase 1) → Zenoh (Phase 2, D129) | InProcessBus, 双模式 RPC+Event (D132-D133) |

## 11. 已决策项（原待决策）

| # | 议题 | 状态 |
|---|------|------|
| 1 | 插件加载方式 — 编译期 inventory + 运行时 dlopen | ✅ D13, D29 |
| 2 | 信令服务 — 单 axum HTTP+WS 同进程 | ✅ D12, D52 |
| 3 | Cargo workspace 组织结构 — 7 crates (当前) → 10+ crates (Phase 2+) | ✅ D16, D82, D127, D142 |
| 4 | 插件间通信协议 — FlatBuffers schema 设计 | ✅ D10 |
| 5 | 录制/回放能力 — fMP4 + splitmuxsink, Phase 2+ 作为一级管线路径 (D34-D40) | ✅ D34-D40 |
| 6 | Host 打包发布 — tarball + cargo-c SDK tarball + CMake | ✅ D102-D106 |
| 7 | 三层逻辑抽象模型 — Plugin/Component/Process 分离 | ✅ D126 |
| 8 | WebRTC 架构升级 — RTCDataChannel→RTP track, mediasoup SFU | ✅ D137-D138 |
| 9 | 多后端 feature gate 策略 — Phase 0 仅 webrtc-sys 实现, webrtc-rs struct 骨架, str0m Phase 2+ | ✅ D139-D140, D144-D151 |
| 10 | 统一 Gateway 模式 — 单端口 :9800 路由分发 | ✅ D128 |
| 11 | Component 框架 — 独立 crate, 3 阶段生命周期, 双模式总线 | ✅ D127, D131-D134 |

- Phase 2+: Docker + Alertmanager 配置用于生产告警
## 12. 附录

### A. 参考项目

| 项目 | 参考点 | 语言 |
|------|--------|------|
| RustDesk | 远程桌面架构、P2P 打洞、单二进制多角色 | Rust |
| LVQR | Unified Fragment Model、多协议适配、29 crate 工作区 | Rust |
| str0m | sans-I/O WebRTC 核心 | Rust |
| webrtc-rs | W3C 兼容 WebRTC、runtime agnostic | Rust |
| MediaMTX | 协议路由（零转码） | Go |
| Parsec | 低延迟远程桌面、GPU 编码管线 | C++ |
| tether-rally | RTCDataChannel 遥操作二进制协议 | JS/C++ |
| QUIC / MoQ / WebTransport | 低延迟媒体传输协议标准 | IETF |

- Multi-SIM 绑定 (Phase 3): Vay 4-SIM 并行运营商聚合
### B. WebRTC 遥操作关键指标

| 指标 | 数值 | 来源 |
|------|------|------|
| 视频延迟阈值 | <150ms MVP 验收标准 (D93)（任务完成率 93%→50% 转折点） | LAVT 研究 |
| 控制延迟 | <50ms MVP 验收标准 (D93) 目标 | tether-rally 实测 |
| RTCDataChannel | unordered, maxRetransmits=0（控制） | RFC 8831 |
| 紧急停止 | 独立 UDP 路径（不依赖 WebRTC） | CallSphere 生产经验 |
| 4G/5G 端到端 | ~200ms 视频, ~300ms 控制 | Provost et al. 2026 |
