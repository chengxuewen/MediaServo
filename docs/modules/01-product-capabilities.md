# 产品能力概览

> MediaServo 提供七个产品能力，覆盖远程桌面、视频会议、直播推拉流、监控接入、遥操作等场景。

---

## 一、七个产品能力

| 能力 | 说明 | 典型场景 |
|------|------|---------|
| **远程桌面** | 屏幕捕获、GPU 编码（H.264/H.265）、输入注入，<100ms 延迟 | IT 运维、远程办公 |
| **视频会议** | 多方音视频通话、SFU/MCU、屏幕共享 | 团队协作 |
| **推拉流** | RTMP/HLS/SRT 接入与分发、直播转码 | 直播、内容分发 |
| **监控接入** | ONVIF/GB28181 相机发现与流管理 | 安防、巡检 |
| **WebRTC 遥操作** | 低延迟视频 + RTCDataChannel 控制 | 车辆遥控、机器人 |
| **车端推流** | 车辆摄像头推流到云端 | 车联网 |
| **舱内拉流** | 舱内屏幕观看远程视频 | 远程驾驶 |

---

## 二、技术栈

| 组件 | 选型 | 理由 |
|------|------|------|
| **核心语言** | Rust (edition 2024) | 零成本抽象、内存安全、跨平台 |
| **WebRTC 栈** | webrtc-sys (libwebrtc FFI, 默认) / str0m (LAN) / mediasoup-sys (Phase 2, D118) | MVP Phase 1: libwebrtc P2P + RTP Track 级 API
| **GPU 编码** | libloading 桥接 NVENC/VAAPI/VT | 避免编译时绑定 GPU SDK |
| **编解码** | GStreamer (gst-plugins-rs) | 覆盖全，生态成熟 |
| **信令** | WebSocket（长期；MQTT=触发条款——D289/F13 改判 D74 的双轨承诺） | D74→D289
| **内部协议** | FlatBuffers | 零拷贝、多语言 |
| **桌面 GUI** | Tauri v2 + React | 轻量、Rust 后端、跨平台 |
| **嵌入式 Web** | axum + 静态 HTML | Host 配置页，无框架依赖 |
| **构建** | Cargo workspace | 统一依赖管理 |

---

## 三、关键设计决策

### 3.1 MVP v2 中继模型 (D118)

MVP 采用 3 组件中继架构：Host → Server(relay) → Client。Server 负责信令中继和媒体转发，P2P 穿透仅 第三方平台 Studio LAN 场景开启。mediasoup SFU 作为 Phase 2 多方会议后端（D118 附录），MVP Phase 1 优先完成 1:1 relay。

行业共识，所有产品均采用此连接策略。默认走 SFU/TURN 中继 (D118 relay-default)。

### 3.2 UDP 为第一优先级传输协议

TCP 的队头阻塞在实时场景是致命的。Parsec、ToDesk 经验表明 UDP + 自定义可靠性层是最优解。

### 3.3 硬件编解码优先 + 软件 fallback

平衡性能和兼容性。硬件编码提供低延迟（NVENC 中位数 ~5.8ms），软件编码作为老旧设备 fallback。

### 3.4 多进程权限分离

网络层与 UI 层隔离为不同进程（AnyDesk/RustDesk/Chrome Remote Desktop 均验证此模式）。

### 3.5 Protobuf 定义所有协议

类型安全、多语言支持、版本演进友好（RustDesk 经验）。

### 3.6 Unified Fragment Model

参考 LVQR，所有输入协议产生统一 Fragment，所有输出协议消费 Fragment。一种内部媒体类型，添加新协议仅需约 50 行桥接代码。

---

## 四、参考项目

| 项目 | 参考点 | 语言 |
|------|--------|------|
| RustDesk | 远程桌面架构、P2P 打洞、单二进制多角色 | Rust |
| LVQR | Unified Fragment Model、多协议适配、29 crate 工作区 | Rust |
| str0m | sans-I/O WebRTC 核心 | Rust |
| webrtc-rs | W3C 兼容 WebRTC、runtime agnostic | Rust |
| MediaMTX | 协议路由（零转码）、Media-over-QUIC | Go |
| Parsec | 低延迟远程桌面、GPU 编码管线 | C++ |
| tether-rally | RTCDataChannel 遥操作二进制协议 | JS/C++ |

---

## 五、性能基准

| 场景 | 视频 G2G 延迟 | 控制延迟 | 说明 |
|------|-------------|---------|------|
| LAN/WiFi | ~100ms | <15ms | 理想环境 |
| 4G LTE | 150-250ms | 15-40ms | 蜂窝网络 |
| 5G NSA | 100-200ms | 10-30ms | 当前商用 |
| 5G SA + MEC | <100ms | <10ms | 边缘计算 |
| 5G NR 理论 | <50ms | <1ms | 理论极限 |
