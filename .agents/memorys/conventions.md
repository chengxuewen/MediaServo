# AUDEMSP 约定与约束

## C1: 架构决策对比格式

**约束**：任何涉及方案选择的架构讨论，必须逐项列出：
- **优缺点**：每个方案的优点和缺点
- **来源/参考**：借鉴的现有系统/开源项目/行业实践
- **影响**：选择该方案对后续开发的影响
- **推荐**：明确推荐及理由

禁止仅列举选项让用户选择而没有上述分析。

**来源**：用户显式要求（2026-07-19 架构讨论）

---

## C2: 三层抽象模型

AUDEMSP 采用三层抽象模型：

| 层级 | 概念 | 职责 |
|------|------|------|
| Layer 1 — 管线层 | Plugin | 媒体管线元素（capture/encode/decode/render） |
| Layer 2 — 服务层 | Component | 有独立生命周期的服务级单元（signaling/relay/admin） |
| Layer 3 — 部署层 | Process | OS 进程，承载 Component 运行 |

- Plugin 和 Component 是不同层次的概念，不应合并为一个 trait
- Component 内部可以持有 Plugin 实例（通过 PipelineEngine）
- Component 通过 ComponentBus 通信，Plugin 通过 MediaPort 通信

**来源**：ROS2（Node vs ComposableNode）、Janus（Plugin vs Transport）、OBS（Module vs Source）

## C3: 术语"三层"消歧

**约束**：AUDEMSP 使用"三层"描述两个不同维度的分层模型，阅读/引用时必须区分：
- **D1 三层部署拓扑架构**：部署维度——控制面（Server） / 数据面（Host+Remote） / SDK 层（napi-binding）
- **D126 三层逻辑抽象模型**：代码维度——Plugin（管线层） / Component（服务层） / Process（部署层）

D1 和 D126 是互补关系，不是替代关系。

**来源**：Doc Audit 审计 #3（2026-07-19）

---

## C4: crate 命名: host/client 对称

**约束**：远程控制场景的 crate 命名遵循 host/client 对称模式：
- **host** = 被控侧 → 推流端 → field/vehicle 侧 → `mediaservo-host`
- **client** = 主控侧 → 拉流端 → cockpit/operator 侧 → `mediaservo-client`

命名对应关系：
| AUDEMSP | Parsec | RustDesk | Moonlight/Sunshine |
|----------|--------|----------|-------------------|
| `mediaservo-host` | `ParsecHost` | Controlled host (server.rs) | Sunshine (Host) |
| `mediaservo-client` | `ParsecClient` | Controller (client.rs) | Moonlight (Client) |

**来源**：远程桌面/远程操控工业惯例分析 (2026-07-19), D154

---

## C5: GStreamer → WebRTC 数据流边界

**约束**: mediaservo-host 中 GStreamer 和 WebRTC 的接口**仅允许 `&[u8]` 字节传递**：

```
GStreamer pipeline (C, glib)
  capture → encode → appsink
                       ↓
              H.264 byte buffer (&[u8])
                       ↓
mediaservo-webrtc (Rust wrapper)
  TrackLocal::write_frame(&[u8])
                       ↓
webrtc-sys (C++, libwebrtc)
  RTP packetizer → ICE → network
```

禁止模式：
- GStreamer buffer 直接传递给 libwebrtc（内存管理边界不兼容）
- 共享内存池（glib allocator ≠ C++ new）
- 跨 FFI 边界传递原始指针

**理由**: GStreamer 和 libwebrtc 使用不同的内存分配器 (glib malloc vs C++ new)。`&[u8]` 接口强制 copy，确保 Rust 所有权语义下的内存安全。

**来源**: D155, OBS Studio 实践

---

## C6: mediaservo-webrtc 命名规范

**约束**：mediaservo-webrtc crate 遵循以下命名规范：
- **类型名**: 对外 pub 类型全大写 RTC 前缀 (RTCPeerConnection, RTCDataChannel...)，内部类型不加前缀
- **方法名**: 全部 snake_case (create_offer, add_track, on_track)，禁止 camelCase W3C 包装
- **目录名**: backend/ (uniform singular)
- **枚举变体**: PascalCase
- **常量**: SCREAMING_SNAKE_CASE

其他 crate (core, media, server, mediaservo-*) 使用 bare names，无前缀。

**来源**: D166, D167, D168 (2026-07-22)



## C7: OpenCode Instructions 内容策略

**约束**：instructions 数组只放每轮必需加载的文件。参考性文档保留在磁盘，按需读取。

**纳入 instructions 的文件类型**：
- 项目记忆（status, conventions, pitfalls）
- 编码规则（security, coding-style, testing 等）
- 项目语言专属规则（Rust）
- 编辑安全约束（edit-safety, constraints）

**不纳入 instructions 的文件类型**：
- 工具自身参考文档（agent-guide, model-tiers）
- 非项目语言规则（TS/CPP/Go/Web 等对 Rust 项目无关）
- 多语言重复翻译（zh/ 是 common/ 的中文副本，不重复加载）
- 大型归档决策（decisions.md 按需读取，仅在精简后考虑加入）

**原则**：instructions 总量控制在 ~8,500 tokens（< 30K 目标）。每新增一个文件，评估是否可移除一个。

**来源**：D199 (2026-07-28)

---

## C8: 质量门禁 Agent 模型分配

**约束**：Agent 模型分配必须符合层级映射表，高杠杆任务用最强模型。

| Agent | 层级 | 原因 |
|-------|------|------|
| oracle | premium-max | 最复杂架构决策 |
| prometheus | premium-max | 高杠杆计划生成，错误代价最大 |
| momus（计划批评家） | premium | 对抗性审查需要深度推理，temp 0.3 |

执行型 agent（explore/librarian/metis/sisyphus-junior）使用 fast 层。

**来源**：D200 (2026-07-28), D203 (2026-07-29), AUDEBase 配置实践

---

## C9: 经验教训自动沉淀

**约束**：开发过程中发现的问题、教训、经验必须在当轮会话中自动更新到相应记忆文档。

**触发条件**：识别到以下情况时，主动更新：

| 情况 | → 更新文件 | 示例 |
|------|-----------|------|
| 发现 bug / 踩坑 | `pitfalls.md` | 症状 → 根因 → 解法 |
| 用户纠正 AI 行为 | `conventions.md` | 新约束 / 新规范 |
| 架构/配置决策 | `decisions.md` | 决策编号 → 原因 → 影响 |
| 项目状态变化 | `status.md` | Phase 完成、测试数变化 |
| 编码模式/反模式 | `rules/common/coding-style.md` | 禁止模式 / 推荐模式 |
| 安全相关教训 | `rules/common/security.md` | 新增安全规则 |
| 编辑工具使用教训 | `rules/common/edit-safety.md` | 新增编辑约束 |

**格式要求**：
- `pitfalls.md`：症状 + 根因 + 解法，三要素缺一不可
- `conventions.md`：C{n} 编号，「约束」/「原则」开头，注明来源
- `decisions.md`：D{n} 编号，决策 + 日期 + 原因 + 影响
- `status.md`：更新生成日期、commit 数、Phase 状态

**原则**：不等用户要求。识别到可沉淀的经验即主动更新。宁可多记，不可遗漏。

**来源**：用户显式要求（2026-07-28）

---

## C10: OMO 插件版本监控

**约束**：每次 OMO 插件版本升级前，应检查 changelog 和与当前配置的兼容性。

**检查步骤**：
1. `npm view oh-my-opencode version` — 对比本地版本
2. 检查 breaking changes（主要版本升级）vs patch（直接升级）
3. 检查 oh-my-openagent.jsonc schema 是否变更
4. 重启 opencode 后验证 agent/技能加载正常

**当前状态**：v4.19.2（npm 最新 4.19.3，待升级）

**来源**：ecosystem-scan 审计（2026-07-29）

---

## C11: 调试前必须查阅官方资料

**约束**：遇到问题、故障、不确定的技术细节时，优先调研官方仓库源码、官方文档、社区资料（GitHub issues/discussions、Stack Overflow），禁止凭直觉盲目尝试。

**优先级**：
1. **官方文档** — mediasoup.org, webrtc-rs docs, Rust std docs
2. **官方仓库源码** — GitHub 集成测试（最权威的 API 用法示例）
3. **官方示例** — mediasoup-demo, 官方 examples 目录
4. **社区资料** — GitHub issues/discussions（同问题+解法）
5. **最后**：凭经验推断（标记为假设，需验证）

**触发条件**：
- 遇到编译错误/运行时错误/行为异常
- 不确定 API 的参数格式、字段类型、序列化方式
- 库版本升级后行为变化
- ICE/DTLS/RTP 等协议层问题
- mediasoup Worker、mediasoup-client、webrtc-rs 等第三方库问题

**禁止模式**：
- 连续 2 次尝试同一修复 → 说明方向错误，必须停下来查资料
- 凭记忆构造 API 参数格式 → 必须对照官方测试或文档
- SDP 字符串手动拼接 → 必须先查 RFC 格式或官方生成示例
- 假设 serde 字段映射 → 必须查 `#[serde(rename_all)]` 注解

**反例（本次教训）**：Host→SFU 方案中 `connect_transport` 消息从未发送、`add_track` 时序错误、Router H264 参数缺失，均因未先对照 mediasoup-demo 的完整信令流程。

**来源**：用户显式要求（2026-07-31 团队评审后）

---

## C12: 仅通过 mediaservo-webrtc 使用 WebRTC

**约束**：所有 client 端 crate（host/client）禁止直接依赖 webrtc-rs（`webrtc = "0.12"`），必须通过 `mediaservo-webrtc` 抽象层使用 WebRTC 能力。P2P 和 SFU 路径统一经此抽象层。Server 端 SFU 路径不依赖 mediaservo-webrtc（WebRTC 来自 mediasoup），webrtc feature 为 P2P relay 保留。

**后端策略**：
- 默认/当前后端 = `backend-webrtc-sys`（libwebrtc C++ via webrtc-sys FFI），不依赖 mediaservo-codec
- `backend-webrtc-rs` 为备选后端（Phase 2+），需额外依赖 mediaservo-codec

**Reason**：
- `mediaservo-webrtc` 已封装 W3C API（RTCPeerConnection + TrackSender + DataChannel）
- 三后端抽象（stub/webrtc-rs/webrtc-sys）由 mediaservo-webrtc 统一控制
- 直接依赖绕过抽象层破坏后端切换能力和可测试性

**禁止**：
```toml
# 任何 client crate 的 Cargo.toml — 禁止
webrtc = "0.12"
```

**允许**：
```toml
# Cargo.toml — 正确
mediaservo-webrtc = { path = "../mediaservo-webrtc", features = ["backend-webrtc-sys"] }
```

**来源**：用户显式要求（2026-07-31 Host SFU 评审后）

---

## C13: Server 构建双轨化（原生主 + Docker 兜底）(2026-08-26 修订)

**约束**：mediaservo-server 构建采用双轨策略——**原生编译为主路径**（开发/调试），Docker 为发布镜像与 CI 兜底。原「统一 Docker 编译」约束已放宽。

**调试主线（2026-08-28 修订）**：开发调试的**运行/取证/黑屏排查**默认连**本机 native server**（`run server` 默认 native；日志：发布壳实例 `out/server/logs/server-native.log`、裸 CLI `data/logs/server-native.log`）；Docker 容器仅用于发布镜像与 compose 路径验证。调试取证禁止默认假设 server 在容器内（`docker logs`）——先 `ss -tlnp | grep 9800` 确认监听者身份再取日志。

**构建矩阵**：
| 场景 | 命令 | 说明 |
|------|------|------|
| 开发/调试（主路径） | `pixi run build-server-native` | 原生编译，meson wrap 首次需联网 |
| 原生 check | `pixi run check-server-native` | exit 0 = 原生编译通过 |
| 原生 test | `pixi run test-server-native` | 原生测试 |
| 发布/CI 兜底 | `scripts/docker-cargo.sh build -p mediaservo-server --features sfu-mediasoup` | Docker 编译，环境一致 |
| CI check 兜底 | `scripts/docker-cargo.sh check -p mediaservo-server --features sfu-mediasoup` | Docker check |
| 原生 check（排除 server） | `cargo check --workspace --exclude mediaservo-server` | 非 server crate 原生检查 |

**检查命令**：
- `pixi run build-server-native` 应 Finished
- `pixi run check-server-native` exit 0

**原因**：
- 主流生态惯例：mediasoup 官方 Rust 绑定 = 原生 runner 进程内 meson（同架构）
- livekit/Deno/Bun/Zed 均原生构建优先，Docker 仅发布
- 本机实证（T5 模式①✓）：target/server-native 有产物
- 裸机多 IP 公告 = run server --native（PIT-143 完整路径，sfu.rs 全具体 IP）
- meson wrap 首次需联网（wrapdb.mesonbuild.com），离线前提是首次构建成功后缓存命中

**Docker 兜底保留理由**：
- CI 环境依赖一致性（Ubuntu 22.04 基线）
- macOS/Windows 开发者 Docker 统一
- 发布镜像构建（`--image runtime`）

**修订历史**：
- 2026-07-31 原版：server 统一 Docker 编译（C13 初版）
- 2026-08-26 修订：双轨化，原生主 + Docker 兜底（three-mode-build T6）

**来源**：用户显式批准（2026-08-26 three-mode-build T6）+ mediasoup/livekit/Deno/Bun/Zed 调研 + T5 实证

**2026-09-01 品牌化修订（branding-completion, D269）**: 裸机 server 交付物理名 = `{brand}-server`（deploy 装配时改名 + 根级快捷 symlink `{prefix}/{brand}-server`）；`build server` staging 仍落 cargo 名（品牌化归 deploy）；`deploy server` 支持双探源幂等（免重 build 重部署）+ 陈旧 oxfile command 自动重渲染；`mediaservo-client` 不再随 host 树；`clean server` 品牌态双名回收。命令矩阵中 `<X>/bin/mediaservo-server` 实例命令一律替换为 `<X>/<brand>-server`（或 bin/ 下同左）。

**2026-08-31 分离修订（frontend-process-split）**: 矩阵追加 `build web`（静态产物→out/server/web）
与 `run/stop/restart/status web`（过渡态 caddy）；`build server` 默认=不嵌入变体（default features 翻转），
模式②嵌入经 `--image runtime` 显式化。`run/start/stop/restart server` 将在 Phase 6 后退役→转发
msrtc-server（T21 落地时同步修订本节并更新命令矩阵）。


---

## C14: 子代理产物必须验证（编排者铁律）

**约束**：子代理返回的完成声明不可信。编排者必须验证实际产物后才标记任务完成（PIT-34）。

**验证清单**：
- 声称创建的文件 → `cat`/`ls` 确认存在 + 内容完整
- 声称修改的配置 → `grep` 关键字段
- 声称可运行的命令 → 实际执行
- 声称通过的测试 → 重新运行

**失败处理**：验证失败 → `task(task_id="ses_...", prompt="fix: <具体问题>")` resume 修复，不自行编辑。

**验证**：`ls <声称的文件> && grep <关键字段> <修改的文件>`。

**来源**：PIT-34 (2026-07-31 docker-compose.dev.yml 声称创建实际缺失)

## C15: 错误响应分支必须打日志 — 禁止静默失败

**约束**：任何返回 Error 响应/Err 的代码路径**必须**同时有 `tracing::error!`/`tracing::warn!` 日志。错误信息只进响应不进日志 = 调用方忽略响应时全链路静默（PIT-54：produce Err 分支不打日志 + Host 不读响应 → 失败表现为"挂起"，浪费数小时排查）。

**检查**：`grep -n "SignalingMessage::Error" crates/mediaservo-server/src/signaling.rs` — 每个构造点上方应有日志行。

**来源**：PIT-54 (2026-08-04 produce UnsupportedCodec 静默)

## C16: 客户端 SFU 请求必须处理响应

**约束**：Host/Client 发送 SFU 请求（produce/consume/connect）后**必须等待并处理响应**（至少打日志）。禁止 fire-and-forget——错误响应被忽略 = 服务端失败在客户端静默（PIT-54：Host 发完 produce 直接跑帧循环，Producer 创建失败完全不可见）。

**检查**：Host `main.rs` SFU 请求后应有响应 match（Produced/Error），Error 必须 log。

**来源**：PIT-54 (2026-08-04)

## C17: 帧时间戳与帧率约束 — WebRTC 编码链路硬性要求

**约束**: ① 喂给 libwebrtc 的 VideoFrame 时间戳必须**锚定单调真实时钟**（`ts_base_us(SystemTime 锚点) + Instant::elapsed()`）——假时钟（固定步进）与 livekit TimestampAligner 时间域不一致 → 编码器停摆（PIT-63）；裸 SystemTime::now() 非单调（NTP 跳变）→ ts 倒退。② **实际帧率必须匹配 libwebrtc 编码器配置**（SDP 协商 fps）——内容 draw 耗时（如 SquaresPattern 7-17ms）拖慢"固定 sleep"循环 → 帧率偏差 → rate control 异常（PIT-64）。**帧循环用绝对时间轴**（`sleep_until(next); next += 33ms;`——OpenCTK RepeatingTask 同机制），不追赶（next < now 时重置）。

**检查**: Host main.rs B5 帧循环应为 sleep_until 绝对时间轴；write_raw_i420 内部时间戳为锚定单调。

**来源**: PIT-63/64 (2026-08-05 视频帧管线加固计划)

---

## C18: 官方用法优先 — 禁止自定义推测/最小用法

**约束**：使用依赖库/项目/工具（crate、框架、FFI、协议）时，**必须优先遵循官方文档、官方仓库源码、官方示例和社区推荐用法**。禁止自创推测用法和"最小可用"接口——官方/社区示例和推荐用法已经过实践验证，能避免重复踩坑。

**适用范围**：
- **webrtc crate**（mediaservo-webrtc）：仅暴露完整的 W3C WebRTC API（RTCPeerConnection.addTrack/addTransceiver/createOffer/setLocalDescription/setRemoteDescription/onicecandidate...），禁止为"省事"裁剪成最小接口或自选语义接口
- **host/client 及 SDK**：遵循 mediasoup 官方客户端架构和示例（libmediasoupclient C++ / mediasoup-client JS / mediasoup-demo），produce/consume 用标准 offer/answer 协商，rtpParameters 从协商结果推导，禁止手工构造 SDP + 手工 JSON rtp_parameters
- **其他依赖**：mediasoup-rs、GStreamer、FFmpeg、Docker 等同样适用

**反面教材（PIT-65 教训）**：Host SFU produce 手工构造 remote SDP（`a=recvonly`）+ 手工 `build_produce_rtp_parameters` JSON + 从 answer 手工提取 ssrc，绕过标准 `addTransceiver(sendonly) → createOffer → setRemoteDescription(answer)` 协商。后果：① `x-google-max-keyframe-interval` 加在 remote fmtp 但 libwebrtc 从 local answer 读参数 → 失效（稳态 GOP 仍 ~99s）；② ssrc/PT 硬编码 vs 协商值可能不一致；③ 方向反转绕协商 → 关键帧/PLI 反馈链路异常。

**检查**：`grep -nE "build_remote_sdp|build_produce_rtp_parameters|create_answer.*set_local|a=recvonly" crates/mediaservo-host/src/` — 这些手工协商痕迹应被标准 offer/answer 取代。

**来源**：用户显式要求（2026-08-05 PIT-65 重构评审后）

---

## C19: docs/reference 组织 — Diátaxis 活参考/调研存档分离

**约束**：`docs/reference/` 按 **Diátaxis 框架** 组织，区分"活参考"与"调研存档"两种用途，不按模糊主题堆砌。

**结构**：
```
docs/reference/
├── README.md          # 索引（唯一导航，新增文档必须登记）
├── webrtc/            # 活参考：镜像 webrtc 模块（mediasoup-refs/mediasoup-client/w3c-alignment/keyframe-analysis）
├── codec/             # 活参考：镜像 codec 模块（ffmpeg/build-optimization 策略）
├── janus-gateway.md   # 活参考（根目录平铺）
└── research/          # 调研存档（历史竞品调研，不参与活跃工作）
    ├── remote-desktop/ streaming/ video-conference/ teleoperation/
```

**原则**：
- **活参考（Reference）**：按产品模块镜像目录（`webrtc/` `codec/`），克制、权威、查用；根目录平铺仅放无法归模块的活参考
- **调研存档（Explanation）**：一次性竞品/技术选型调研（parsec/zoom/mediasoup*/openvidu*/srs 等）归 `research/<领域>/`，写完后不被代码引用，仅作历史参考
- **新增活参考** → 归对应模块目录并在 README 登记；**新增调研** → 归 `research/<领域>/` 并在 README 登记
- 禁止：把调研存档混入活参考目录；为模糊主题建嵌套子目录（参考按产品结构而非主题）

**检查**：`ls docs/reference/` — 顶层应为 `README.md` + 模块活参考（webrtc/codec）+ `research/`；`docs/reference/research/` 下才是竞品调研。

**来源**：Diátaxis 框架（2026-08-06 用户确认架构），替代此前按领域子目录的平铺方案

## C20: 禁止硬编码路径规避 — 第三方硬编码须用户同意后正规修复 (2026-08-07)

**约束**: ① **禁止创建符号链接/伪造目录让第三方硬编码路径"生效"**——用硬编码绕硬编码是错误规避（PIT-70：为规避 mediasoup-sys tasks.py 硬编码的旧 pixi 路径，创建 `/home/maxsense/Documents/OMSPBase/.../ninja` 符号链接，被用户当场否决）。② 第三方依赖（cargo registry 源码、npm 包、meson wrap 等）中的硬编码路径，**不得通过修改 registry 缓存或伪造路径规避**——registry 修改会被 hash 校验检测，伪造路径是隐蔽副作用。③ 正确路径：**优先用官方支持的环境变量/配置覆盖**（如 tasks.py 是否支持 NINJA env）；若无官方机制，**必须向用户说明并取得同意**后才能做本地 patch（cargo `[patch]`/vendored）或接受环境限制。

**适用范围**: 构建工具硬编码路径（meson/ninja/pip）、CI 硬编码、Docker 路径映射。

**检查**: `grep -rn "/home/maxsense\|OMSPBase" .pixi/ scripts/ Dockerfile* docker-compose*.yml 2>/dev/null` — 应无伪造目录/符号链接规避；发现第三方硬编码 → 列方案问用户，不擅自规避。

**来源**: PIT-70 (2026-08-07 mediasoup 构建 ninja 路径规避被否决)

## C21: mediasoup 仅限 mediaservo-server 使用 — 禁止反向依赖 (2026-08-07)

**约束**: ① **mediasoup（mediasoup-sys）只允许出现在 mediaservo-server 的依赖树**——host/client/SDK 及其测试**禁止依赖 mediaservo-server 或任何含 mediasoup 的 crate**（含 dev-dependencies / 测试专用 feature）。② 跨 crate 的 SFU 集成测试必须通过 **WS 信令协议**交互（Host 模拟端连外部 server），不得 import server 内部类型（SfuManager/SignalingServer）。③ 违背 C12 的历史遗留：`mediaservo-host/tests/e2e_sfu.rs` 曾依赖 mediaservo-server（进程内 spawn SfuManager）——导致 webrtc-sys（libwebrtc 内嵌 OpenSSL）与 mediasoup-sys（静态 openssl-3.0.8）**X509 符号冲突，链接必挂**，且测试从未在 Linux 编译通过（C14）。

**原因**: ① 依赖方向单向（C12: server 用 mediasoup，host/client 用 mediaservo-webrtc，两者不交叉）；② 双 OpenSSL 静态链接符号冲突（X509_PUBKEY_it duplicate symbol）——架构性，无法绕过；③ 测试便利不得凌驾架构边界（PIT-71 教训：进程内模式设计从诞生起未真正跑通）。

**检查**: `cargo tree -p mediaservo-host -i mediasoup-sys` 应为空（host 依赖树无 mediasoup）；`grep -rn "mediaservo-server" crates/mediaservo-host/Cargo.toml crates/mediaservo-client/Cargo.toml` 应无引用。

**来源**: 用户架构强调 (2026-08-07) + PIT-71 e2e_sfu.rs 链接冲突根因。

## C22: Host 禁止在 Docker 中运行 — 含测试 (2026-08-10)

**约束**: ① **mediaservo-host（二进制、测试、E2E 流程）不允许在 Docker 容器内运行**——宿主原生编译/运行（macOS 或 Linux 宿主直接 `cargo build/run/test`）。② e2e_sfu 等 Host 侧测试在**宿主原生**执行，仅 server（mediasoup）在 Docker 容器内。③ Host 连 Docker server 时用宿主可达 IP（`MEDIASERVO_SFU_ANNOUNCED_IP=宿主IP`），不得用容器内 IP（172.18.x）作为 announced address——容器内 host 经宿主 IP 有 hairpin NAT 问题（P1 实证：容器→宿主 IP:20000 的 STUN 不通）。

**原因**: 用户显式要求（2026-08-10）——host 是边缘侧部署形态（车端/设备），开发/测试环境必须与部署一致；容器内跑 host 引入 NAT/网络抽象，掩盖真实网络行为（ICE/STUN/延迟），且 P1 已实证容器内 host 连宿主 IP 的 UDP 路径不通。

**检查**: `grep -rn "docker exec.*mediaservo-host\|docker.*cargo.*mediaservo-host\|mediaservo-host" scripts/*.sh .github/workflows/ci.yml` — host 相关命令应宿主原生；`MEDIASERVO_SFU_ANNOUNCED_IP` 应为宿主可达 IP（非 172.18.x）。

**来源**: 用户显式要求 (2026-08-10)


## C23: Jetson(linux-aarch64) 构建统一用系统工具链 — 弃 conda 交叉编译器 (2026-08-12)

**约束**: 在 linux-aarch64（Jetson）平台，host/client 构建**统一用 JetPack 系统工具链**（gcc 10.5 + 系统 binutils），
**禁止用 pixi conda 交叉编译器链 webrtc-sys/tegra 系统库**。实现与门控：pixi.toml `[target.linux-aarch64.activation.env]`
（CC/CXX/LINKER=/usr/bin/gcc + CFLAGS/CXXFLAGS/LDFLAGS 清空）+ .cargo/config.toml
`[target.aarch64-unknown-linux-gnu]`（linker=/usr/bin/gcc + `-B/usr/bin/` rustflags）。

**原因**: conda 交叉工具链（GCC14/glibc 新）无法干净链接 JetPack 系统库（glibc 2.35 冲突 + tegra 传递依赖
libEGL/libv4lconvert 仅系统目录）；`cargo:rustc-link-arg` 不从 rlib 传播；系统 gcc 原生找到全部系统库
= 上游 livekit 官方 Jetson 流程（PIT-85/D220）。

**检查**: `pixi run bash -c 'echo $CC'` 应输出 `/usr/bin/gcc`；`mediaservo.sh build host` 应 Finished；
`ldd target/debug/mediaservo-host | grep -c "not found"` = 0。**禁止**：`source .pixi/envs/default/activate` 验证环境
（pixi 0.66 静默不生效 → CONDA_PREFIX 空 → 误用系统工具链造成假成功）；把 conda gcc 的 CC/CXX 覆盖回
conda 交叉编译器（会 PIT-85 复发）。

**来源**: PIT-85 / D220 (2026-08-12)
## C24: admin dist 编译期嵌入 — 改 TS 源码后必须 rebuild 才生效 (2026-08-13)

**约束**: Admin Dashboard 前端（`www/apps/admin/src/`）修改后，**9800 生产入口（server 托管）不会自动生效**——`crates/mediaservo-server/build.rs` 在编译期 `include_bytes!` 嵌入 `www/apps/admin/dist/`（rust-embed 模式）。开发验证必须走 5173（vite dev, 热更新）；9800 入口需：`cd www/apps/admin && npm run build` + `./mediaservo.sh restart server`（build.rs rerun-if-changed 触发重新嵌入）。

**检查**: 修改 sfu-client.ts/VideoPlayer.tsx 后，9800 页面 JS bundle 是否含新字段（`curl http://127.0.0.1:9800/admin` → HTML 引用的 `index-*.js` 与 `dist/assets/` 最新产物一致）。

**来源**: PIT-87 诊断轮实证（2026-08-13: 编码耗时功能在 9800 不生效 = dist 旧构建）

### C24 修订（2026-08-31, frontend-process-split）: 适用范围收窄至模式②
- native 主路径已分离：default 构建（`admin-dashboard` 移出 default 后）**不再于 9800 托管前端**；
  浏览器入口=Caddy（过渡 `run web`，Phase 6 起归 msrtc-server/oxmgr 簇，默认 :8080）。
- 改 TS → `mediaservo build web`（→ out/server/web）+ 浏览器刷新即生效，**无需 Rust 重编译**。
- "必须 rebuild"仅存于模式②单容器（Dockerfile 显式 `--features sfu-mediasoup,admin-dashboard` 编译期嵌入）。
  上方检查法（curl :9800/admin 比对 bundle）降级为模式②专属；分离形态验证走 Caddy 入口。


## C25: iceoryx2 测试残留清理 — 跑 link/涉及 FrameBus 的测试前必须清运行时目录 (2026-08-14, 修订 2026-08-19)

**约束**: 运行 mediaservo-link / host-capturer 等涉及 FrameBus 的测试/示例前**必须**执行
`rm -rf /tmp/iceoryx2 /dev/shm/iox2_*`（iceoryx2 0.9.3 Linux 运行时根 = `/tmp/iceoryx2`（nodes/services），
`/dev/shm/iox2_*` 仅 node global_mgmt——**只清 /dev/shm 不够**）。上次运行残留的 service 状态
会导致 subscribe/open 持久 SystemInFlux（重试也无效，C1 实证 2026-08-19：固定 topic `camera/cam0`
跨 run 二次打开必失败，全量清后恢复）。测试内部已用唯一 topic（含 `std::process::id()`）隔离并发，
但**跨 run 残留**（固定 topic / 被 SIGTERM 的发布端）仍需外部清理；生产级清理机制归 C2 计划。

**检查**: `ls /tmp/iceoryx2 /dev/shm/iox2_* 2>/dev/null | wc -l` 应为 0（跑测试前）；
涉及 FrameBus 的测试失败先全量清残留再重跑。

**来源**: PIT 2026-08-14 Phase 1 测试轮 + C1 实证（2026-08-19: /tmp/iceoryx2 残留 service → SystemInFlux）。

## C26: reasoning 分层 — thinking 不摊进 content，按模型级裁剪 (2026-08-17)

**约束**: ① **cf. D246**: 推理模型（supportsReasoning=true 的模型）的 `reasoning_content` 必须走结构化 thinking part 存储/展示，**禁止被适配器拼进 content 文本**——快模型不得消费推理模型的思维链；② fast 层 agent（librarian/explore/metis/sisyphus-junior/artistry/quick/writing/unspecified-low）必须显式 `reasoningEffort: "low"`，premium-max 层才有权 high；③ `supportsReasoning` 标注必须与模型实际能力一致（gateway 别名映射修改时同步检查）；④ 会话压缩必须保留 tail_turns（最近轮次思维链 verbatim），压缩摘要不得把 thinking 揉进 content。

**检查**: `grep -c '"type":"thinking"' storage/session/*.json` — 应为结构化 part（非 content 内 `<thinking>` 文本）；`grep 'supportsReasoning' ~/.config/opencode/opencode.jsonc` — 推理模型应为 true；`grep -c '"reasoningEffort": "low"' .omo/omo.jsonc` — 应为 8。

**来源**: D246 (2026-08-17)，OMO 多 agent 多模型上下文治理（延续 D213）

## C27: OMO 配置字段必须对照官方 schema — z.$strip 静默丢弃未知 key (2026-08-18)

**约束**: ① OMO agents/categories 模型配置字段必须是 `model`（单数主模型）+ `fallback_models`（复数回退链）；**禁止 `models` 复数**（仅 categories schema 存在且语义是回退别名，agents 用了即被静默丢弃）；② 插件 schema 是 `z.$strip`（非 passthrough）——未知 key 静默丢弃，**配置不生效无任何报错**，迁移/手工改动后必须 grep 验证关键字段；③ 配置类调查（"为什么不生效"）直接读插件源码（`node_modules/oh-my-opencode/dist/config/schema/*` + validate.ts）并与迁移备份 diff，命令批量并行，禁止串行小命令猜测路径（用户 2026-08-18 明确批评浪费 token）。

**检查**: `grep -c '"models"' .omo/omo.jsonc` 应为 0；`grep -c '"model"' .omo/omo.jsonc` 应为 19；`grep -c '"fallback_models"' .omo/omo.jsonc` 应为 19；配置迁移后先 `diff` 新旧再重启。

**来源**: PIT-97 (2026-08-18)，延续 D246/C26 的 OMO 配置治理

## C28: napi-rs 绑定 API 要点 — napi 3.12 实证 (2026-08-18)

**约束**: ① 回调参数用 `Function<T, ()>`（FromNapiValue ✓），TSFN 构建 = `cb.build_threadsafe_function::<T>().build()`（Builder 的 Args 须 == T；T 自动转 JS 参数，元组 → **JS 数组**如 `[meta, data]`）；② 同步 napi 方法**无 tokio 上下文**——`tokio::spawn` 必 panic，需全局共享 runtime（OnceLock multi_thread，field-c 同款）；③ 事件/帧回调经 ThreadsafeFunction（线程安全，JS 主线程执行）；④ 方法名自动 camelCase（publish_video → publishVideo）；⑤ 结构体字段 u64 用 i64（napi FromNapiValue 无 u64）；⑥ async 方法参数需 'static（Function 生命周期）。

**检查**: `grep -rn "build_threadsafe_function" bindings/node/rust/mediaservo-node/src/` — 应为 `build()` 形态（非 build_callback 闭包）；`grep -rn "tokio::spawn" bindings/node/rust/mediaservo-node/src/` — 应仅 event_runtime().spawn。

**来源**: Node 绑定实现实证（2026-08-18，D249）

## C29: 音频会议房间约定 — audio-<vehicle-id> 前缀 + 复用 SFU 机制 (2026-08-19, H2)

**约束**: ① 音频会议房间 = room_id 前缀约定 `audio-<vehicle-id>`（**不新增 RoomType**）——SFU room 由字符串隔离，transport/produce/consume 机制全复用；房间语义（全互连 opus，每参与者 publish 1 路 + subscribe 其他所有）由服务端 produce 门 + 客户端全订阅表达。② **音频房间 produce 门**: room_id 以 `audio-` 开头 → 只允许 `kind=Audio` producer（视频 producer 4031 + audit，signaling.rs Produce 分支）。③ **网关 rewrite_room 对 `audio-` 房间直通**（子进程已用规范名；重写会并入视频房间破坏每车独立音频房语义）。④ 权限 = 既有 RoomJoin 门（`room_owners` 存设备 ID = 车辆 ID，账号 allowlist/dispatcher/admin 任意车天然正确）+ 既有 can_produce（账号禁发 — 舱端只消费；两方发言需 D-H11 修订）。⑤ 音频 track 语义: `TrackSender::write_frame(kind=Audio)` = **PCM i16 10ms 帧**（非编码字节）→ AudioTrackSource.capture_frame → libwebrtc 内部 opus（webrtc-sys 后端；track id 必须为 "audio" — sender_get_parameters 按 libwebrtc 内部 label 匹配）。

**检查**: `grep -n "is_audio_room" crates/mediaservo-server/src/sfu.rs`（前缀判定单测）; `grep -n "audio rooms allow audio producers only" crates/mediaservo-server/src/signaling.rs`; `grep -n "audio-" crates/mediaservo-host/src/gateway.rs`（rewrite 直通）。

**来源**: H2 实现实证 (2026-08-19), PIT-105 阻塞说明见 pitfalls.md

## C30: libwebrtc 内嵌 FFmpeg 符号抢先满足 — host 二进制内 FFmpeg 调用必须进程内验证 (2026-08-20)

**约束**: ① host 树任何二进制若同时链接 mediaservo-webrtc（backend-webrtc-sys →
livekit libwebrtc.a）与 mediaservo-deck/codec 的 ffmpeg-the-third（动态
libavformat），**禁止假定 FFmpeg 调用走动态库**——libwebrtc.a 内嵌 demuxer-only
静态 FFmpeg（av_guess_format 等符号），最终链接时抢先满足 ffmpeg-the-third 的
UNDEF（PIT-107 实证），muxer 类调用（如 mp4 落盘）必失败。② 涉及 FFmpeg 的
新代码路径验证必须**在目标进程内**做（gdb 断 av_guess_format 看符号落点 / 
`ld --trace-symbol=` 看定义来源），禁止用"独立探针/另一二进制能跑"推断。③
修复方向：webrtc-sys build.rs 对 libwebrtc.a 的 av* 符号做 objcopy 前缀化或
控制链接顺序（静态库晚于动态 -l）。

**检查**: `gdb -batch -ex 'break av_guess_format' -ex run --args <host-bin> ...` →
符号地址在 PIE 主二进制（0x5555...）= 静态副本抢先；`nm <libwebrtc.a> | grep
ff_.*_muxer` 应为 0（demuxer-only 实证）。

**来源**: PIT-107 (2026-08-20 整支审查回归轮, recorder_e2e 2/4 既有失败根因)

## C31: 任务执行分级 — 父会话直做 vs 派发 subagent（2026-08-20）

**约束**: 按任务规模选择执行路径，避免"一律派发"（派发隐含成本 = 整个父会话上下文传递给子代理——PIT 实证 277K tokens，慢且易超限失败）：

| 任务规模 | 执行方式 |
|---|---|
| 小改动（单文件、明确、<30 分钟）| **父会话直接工具执行**（edit/write + 快速验证）——省派发开销 |
| 中任务（2-3 文件、带测试）| 直接做 + 测试验证（cargo test 局部）|
| 大任务（跨 crate、架构性、需独立推理）| 派发 deep/ultrabrain（load_skills 带齐）|
| 审查/对抗性检查 | 派发（fresh eyes——父会话自审会自我确认）|

**派发纪律**: ① prompt 精简——给 brief 文件路径 + ≤10 行关键上下文，不贴长文/不贴历史；② 简单任务用 quick（fast 层），勿 unspecified-high/deep 大材小用；③ 独立任务 run_in_background=true 并行；④ 上下文 >~200K 或派发报 token 超限 → 先 /handoff 开新会话再派发（父上下文小则派发也快）。

**检查**: `grep -c "C31" .agents/memorys/conventions.md` = 1；派发前自问：这任务需要独立上下文/审查吗？不需要 → 直接做。

**来源**: 2026-08-20 长会话实证（90 commits 后上下文 277K、派发反复失败/超慢；小任务直做秒级完成）

## C32: host 实例隔离三原则 — OXMGR_DATA_DIR/端口/房间（2026-08-20）

**约束**: host 多实例共存必须三隔离（PIT-113 多 daemon 分裂教训）：
① **oxmgr 双 env 实例化**: `OXMGR_HOME=<dir>/run/oxmgr`（数据/日志/state）+ `OXMGR_DAEMON_ADDR=127.0.0.1:<18000+dir哈希%400>`（**daemon 互斥 = TCP 端口——只隔离数据目录不够，全局 daemon 占默认端口会阻断实例 daemon 启动（DaemonAlreadyRunning）→ apply 复用全局**）；② **[signaling] local_port**（host-agent 网关端口，默认 17980——同端口启动被检测拒绝）；③ **[signaling] room**（server 房间——同房间多 Host 语义混乱）。

**竞争防护**: start 前端口探测（被占 → 交互 y 接管[停旧启新]/退出；非交互自动退出）；开机自启**全局唯一**（startup on 检测其他实例 unit → 交互接管——相机等共享资源防竞争）；install auto-stop 三连停（**systemd unit 枚举逐个停**——systemctl 不支持 glob + `oxmgr.service`（service install 装的全局 unit）是隐藏复活源 → daemon → 进程族）。

**检查**: `grep -rn "OXMGR_HOME\|OXMGR_DAEMON_ADDR" crates/mediaservo-host/src/` — 应见三处注入（oxmgr_env/oxmgr_apply/startup unit）；`host startup on` 二次实例应被拒。

**来源**: 2026-08-20 多实例部署实操（PIT-111/112/113/115 系列）

## C33: 应用层品牌化（Brand）边界 — bindings/wire 固化 vs host/client/server 可定制（2026-08-21）

**约束**: MediaServo 作为基石被第三方依赖时可白标：① **SDK bindings 固化**（C ABI 符号 `mediaservo_*`（D247）、include/mediaservo/、cxx/py/node、soname/ABI（D240））——品牌改造 `git diff bindings/` 必须为空（固化门）；② **wire 协议固化**（信令 SignalingMessage/SFU/RTP/FrameMeta）——server 同时服务多品牌 host；③ **应用层可定制**: host/client/server 的用户可见面走 `mediaservo_common::brand`（env `MEDIASERVO_BRAND` > 编译期 > 默认）——默认品牌**legacy 串硬映射**（app 前缀 `host-`、unit `oxmgr-host-`、设备 `ms-`、namespace `mediaservo-host`——与 `product` 不同源，勿按 `<product>-` 直推）；④ install/package `--brand`（快捷名 + `bin/<brand>-<app>` 符号链接 + init env 注入）；⑤ 设备前缀仅新 key（存量不迁移，G2 需重注）。

**检查**: `MEDIASERVO_BRAND=cp ./target/debug/mediaservo-host version` → 应输出 `cp 0.1.0`；`git diff --stat bindings/` → 应为空；`grep -n '{ns}' crates/mediaservo-host/src/translate.rs` → 应为 0（PIT-118 禁）。**禁止**: 品牌化进 bindings/wire/crate 名；模板字符串用 replace 自指（PIT-118）。

**来源**: D252 (2026-08-21)，Momus 计划审核后实施

## C34: WebRTC 黑帧排查链 — announced → 订阅 → 房间 → producer (2026-08-25)

**约束**: 视频黑帧（track 收到但无帧/ICE 不连）按固定链排查，每层有证据命令，禁止跳层乱试：

| 层 | 检查点 | 证据命令 | PIT |
|----|--------|---------|-----|
| ① announced IP | SDP candidate 是否可达（0.0.0.0/172.x=不可达） | 浏览器 offer SDP / `docker logs server \| grep announced` | PIT-128/58/44 |
| ② 推流端订阅 | streamer 是否拿到帧（acl denied=token role 错） | `grep "订阅\|acl denied\|OpenH264" <host>/run/logs/msrtc-streamer-*.err.log` | PIT-126 |
| ③ 房间一致性 | streamer room == 消费端 Play room | `grep "streamer ready" ...out.log`（room=xxx）+ server `found N producers in room X` | PIT-127 |
| ④ producer 存在 | 房间 producer 数 > 0 | `docker logs server --since 1m \| grep "found.*producers in room"` | — |
| ⑤ ICE 状态 | Connected/Completed | host: `grep "ICE connection state" ...out.log`；浏览器 console | — |

**检查**: 黑帧先跑 ①（30s）→②（30s）→③（30s）——90 秒内分层定位，禁止先猜编码/先改前端。

## C35: 全 SFU 架构 — 禁止 P2P 直连路径（2026-08-25）

**约束**: ① **媒体与控制全部走 SFU**（mediasoup）——P2P 直连（host↔client WebRTC）因 NAT 不可达问题废弃（用户决策 2026-08-25）；② RoomType 仅 DeviceStream（无 P2P 房间——Host 推流 / Remote|Consumer 消费，多舱端共存）；③ 网关无 p2p_owner 协商归属——Sdp/RTCIceCandidate 下行**按房间路由**（rewrite 后 room 一致的 conn 全量；无匹配丢弃+日志）；④ 控制 DC（底盘/云台）SFU 化 = 经 SFU data 域（H1 DataProducer/DataChannel label）——**Phase B 待办**（host-controller 改 SFU data consumer + 舱端 SCTP over mediasoup）。

**检查**: `grep -rn "p2p_owner\|RoomType::P2P" crates/` 应为 0；`grep -n "Sdp.*按房间\|全 SFU" crates/mediaservo-host/src/gateway.rs` 存在。

**来源**: 用户显式要求（2026-08-25 P2P 不可达 → 全 SFU）；替代 D 系列 P2P DC 决策（decisions.md 修正）。
