# SDK 分层与架构

> MediaServo 第三方集成 SDK 采用**四 SDK 主架构**：link（连接）/ field（组合会话）/ client（舱端消费编排）/ deck（媒体数据）。
> 关联决策: D222-D234（2026-08-13/14，替代 D65-D69/D82 的双 facade 模型；D230-232 修订为静态直连/组合 SDK/消费端定位；D233 API 单层会话型；D234 调用约定）
> **公开 API 契约**: [20-sdk-api-contract.md](/docs/modules/20-sdk-api-contract.md)（link/field/client/deck 接口签名 + C ABI 绑定形态）

---

## 架构总览

```
mediaservo-link（连接面, 纯 Rust）          mediaservo-deck（媒体数据面, 双形态）
  ▲ rlib                                    ├─ rlib: 应用单独使用
  │                                         ├─ rlib full: field 静态依赖（**默认**）
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

依赖方向全单向无环：`field → webrtc + link + deck`；`client → field`；deck 独立双形态。

## 一、四 SDK 职责

| SDK | crate | 职责 | C ABI 前缀 |
|-----|-------|------|-----------|
| **link** | `mediaservo-link` | frame_bus（两端通用帧总线 + Registry）、signal（WS 信令客户端）、auth 集成（复用 common PSK/JWT）、dc（Phase2, webrtc 后端） | `mediaservo_link_*` |
| **field** | `mediaservo-field` | **组合 SDK**: push/pull（webrtc 经 mediaservo-webrtc, C12）+ re-export link（SignalClient/FrameBus/auth）+ re-export deck（MediaDevices/VideoSource/CameraSource）——一行依赖完整闭环 | `mediaservo_field_*` |
| **client** | `mediaservo-client` | **消费编排**: VideoRenderer（GPU interop）、多路会话编排、Input Forward/遥测绑定、deck playback 集成 | `mediaservo_client_*`（批1b 终形 28 符号，见下注） |
| **deck** | `mediaservo-deck` | source（相机/麦克风/桌面, GStreamer）、codec（FFmpeg 静态）、record（mux 落盘）、playback（回放/快放） | `mediaservo_deck_*` |

> **client C ABI 批1b 终形注（S6）**：前缀全量 `ms_client_*` → `mediaservo_client_*`（⚠ breaking，
> 零别名）。28 符号 = 自由函数 5（login 转扁平参数形 + strerror 静态文案）· 会话 12（新增
> state/on_state 状态观测、wait_video 纯等待、consume 多路形、error/last_error 句柄错误槽
> ——`mediaservo_client_error_t{struct_size,code,wire_code,retryable}` 机读位）· 消费者 3
> （consumer id/stats/close，每路独立泵互不连坐）· 控制 8（on_ack 累积形+token、off_ack 注销、
> ready_state/buffered_amount 背压观测、producer_ids needed 升形）。⊘ 旧形保留一周期（行为逐字节
> 保持）：全局 last_error / 首路 consume_video / 会话 union video_stats；登录 config 结构形退役
> （结构保留一周期）。清单真源 = `bindings/c/mediaservo-client-c/include/mediaservo/client.h`
> （ABI 对账门 check-abi-drift 四面统一 `mediaservo_<sdk>_` 前缀）。

## 二、deck 双形态与 feature 切片

| 形态 | 交付 | 使用场景 |
|------|------|---------|
| **rlib 静态** | `mediaservo-deck`（full） | 应用单独使用（采集+录制+回放闭环，无 WebRTC） |
| **rlib 静态 full** | field 静态依赖（**默认**） | field 组合 SDK 内嵌（FFmpeg 无 OpenSSL → 无 BoringSSL 冲突） |
| **cdylib 插件** | `deck-full.so`（CI 独立构建） | dlopen 可选（仅 OTA 独立升级/实现可替换需求出现时） |

field→deck **静态直连为默认**（D231 修订 D224）：FFmpeg 默认 features 不含 `build-lib-openssl`（实证）→ 与 libwebrtc(BoringSSL) 零交集 → dlopen 首要动机（静态符号隔离）消失；dlopen 保留为 OTA 可选演进（D13 PluginManager 载体，Janus 同模式，接口预留 MVP 不实现）。

## 三、消费矩阵

| 消费方 | 依赖声明 | 能力 |
|--------|---------|------|
| 车端推流集成方 | `field` | 推流 + 采集（deck[source]）+ 信令 + 帧总线，一行依赖 |
| ROS 节点 / 订阅帧 / 纯控制 | `link` | 帧订阅、信令、DC（零媒体依赖） |
| 采集+录制（本地监控） | `deck` | 直采直录，无 WebRTC |
| 推流+录制（同进程） | `field` + deck（静态 full） | 会话 + 编解码/录放（静态直连） |
| 舱端 App / 平板 | `client` | 拉流 + 渲染 + 回放（deck playback 静态集成） |

## 四、进程拓扑

- **车端**（刚性多进程, host supervisor 编排）：capture-worker（deck source → FrameBus）/ push-worker（field 订阅 → webrtc-sys 推流）/ recorder-worker（Phase2: link+deck 编码录制）/ control-worker（link only: 信令+DC+紧急）。进程边界=权限边界=崩溃面；control 独立于媒体进程存活。
- **舱端**（默认单进程 + FrameBus 按需跨进程）：交互终端单进程多会话；拉流解码后经 FrameBus 发布，第三方 App 以 link-only 订阅。

## 五、FrameBus（link，两端通用）

- 帧元数据 FlatBuffers（宽高/格式/时间戳/**is_keyframe** + monotonic/epoch **双时钟**）+ 像素裸内存
- 语义：最新帧覆盖（免疫积压/启动顺序）；多订阅者独立消费
- 车端采集链 / 舱端解码分发 / ROS 感知订阅 三场景共用

## 六、冲突纪律（PIT-71 延续, 2026-08-13 更新）

1. FFmpeg 静态符号隔离: deck 的 FFmpeg 禁止 `build-lib-openssl`/`build-zlib`（实测默认不含 openssl → 与 BoringSSL 零交集）; field→deck 为**静态默认依赖**（非插件）
2. zlib/第三方: FFmpeg 走系统动态 zlib; x264/openh264/opus 与 libwebrtc 依赖集交集逐项验证
3. 采集走 GStreamer 动态（D64 v4l2src）+ Rust 默认不导出符号 → 与 BoringSSL 零交集
4. 插件加载 RTLD_LOCAL（禁 GLOBAL）；主进程不导出符号（不加 -rdynamic）；版本握手（仅 dlopen/OTA 模式）
5. 链接冲突矩阵进 CI：field 独立 / deck 独立 / field+deck[source] 静态 / **field+deck-full 静态（新默认）** /（未来）field+deck-full 插件

## 七、应用层消费

| 应用 | 依赖 | 说明 |
|------|------|------|
| `mediaservo-host` | field + link（+可选 deck 插件/worker）| 车端设备主控（supervisor 编排 worker），四 SDK 吃狗粮验证 |
| `mediaservo-client` | client SDK（lib + bin 双 target）| 舱端 GUI（Tauri v2），SDK 的第一个消费方 |
| `mediaservo-server` | common/webrtc（P2P relay）| 信令 + relay + SFU（C21: 与 SDK 无耦合） |

## 八、绑定体系与目录布局

**绑定命名族**（D227）：C=`-c`、C++=`-cxx`、Python=`-py`（双后端：ctypes 普通 + pyo3 瓶颈）、未来安卓=`-jni`。绑定随 SDK 落地创建（YAGNI）。

```
crates/      → 核心+SDK+应用 (link/field/deck/client SDK 核心 + 现有 7)
bindings/c/  → mediaservo-{link,field,deck,client}-c    (workspace member)
bindings/cxx/→ mediaservo-{link,field,deck,client}-cxx  (workspace member)
bindings/python/ → mediaservo_{link,field,deck,client}/ (非 cargo member, 纯 py)
```

链接冲突纪律与跨语言对照测试（c/cxx/py/pyo3 同一操作序列断言一致）见 spec §7、§11。依赖时期语义（构建期/运行期）见 spec §15。

## 九、未来扩展（Phase 3+）

- 视频会议 / 直播 / 监控子场景 SDK：在 field 会话面上叠加场景 API（ConferenceSession 等）
- 安卓 JNI：统一 C ABI（mediaservo_* 系）直供 Java/Kotlin 薄包装（livekit JNI_OnLoad 模式）
- Python/C++ 绑定：link/field/deck/client 各自的薄包装包（发行层按需拆分）

## 十、关键技术选型（已论证）

| 项 | 结论 | 依据 |
|---|---|---|
| DataChannel（Phase2） | webrtc-rs（backend 已存在，与 SFU e2e 4/4） | libdatachannel=第四后端+C++ 崩溃面，排除 |
| 控制通道 | MVP 走 server relay WS（已实现）；DC 为 Phase2 低延迟增强 | 07-protocols 四通道语义不变 |
| 采集实现 | GStreamer（D64 实证全平台，含 Jetson CSI/RTSP） | webrtc-sys 无相机采集桥 |
| 桌面捕获 | deck source 后端：FFmpeg/GStreamer 优先，webrtc-sys desktop_capturer 可选 | 后者已有（LiveKit 加） |
| 本地录制（Phase2） | 双编码（主/子码流，行业 NVR 标准）：recorder 高质量固定 + push BWE 自适应 | 单编码共享会 BWE 污染录制质量 |
| FFmpeg 后端 | 不开 openssl/network feature（本地处理无需 HTTPS）→ 与 BoringSSL 零符号交集 | PIT-71 冲突根源消解（D231） |

## 十一、演进路线

| 阶段 | 范围 |
|---|---|
| **MVP** | link + field + deck[source]（推流链路：采集→FrameBus→webrtc-sys→SFU）；无 deck 编解码/录放；host 单进程组合跑通链路 |
| **Phase 2** | deck 落地（source/codec/record/playback）静态直连；recorder-worker（双编码）；链接矩阵 CI |
| **Phase 3** | client SDK 消费端交付（渲染/多路会话）；舱端 FrameBus 发布；绑定体系落地（c/cxx crate + py 双后端）；安卓 JNI |

## 十二、依赖时期语义（构建期 vs 运行期）

| 依赖 | 构建期 | 运行期 |
|---|---|---|
| link（rlib） | 静态合并进 field 二进制 | 自含（零额外 .so, 纯 Rust） |
| deck（rlib） | 静态合并进 field 二进制 | Rust 代码自含; **GStreamer 系统库为运行时环境依赖**（非 crate） |
| webrtc（.a + BoringSSL） | 静态合并 | 自含 |
| deck-full.so（可选插件/OTA） | 不链接 | 仅 OTA 模式运行时加载 |

**部署形态**: 车端 field = 单个可执行文件（含 link+deck+libwebrtc 全部代码）+ GStreamer 系统环境。**唯一动态依赖入口 = GStreamer**, 其余全静态。

## 十三、deck 与 codec 的关系（不替换, D229）

deck 依赖 codec（facade），codec 引擎保持独立——不合并。若 deck 吞并 codec 则底层抽象反向依赖顶层 SDK（分层反转）。FFmpeg 分库先例（libavcodec 引擎 / libavformat+libavdevice 封装，deck 的 record/playback 恰为后两组角色）。deck = codec 之上的媒体处理层，吸收层语义不吸收实现。
