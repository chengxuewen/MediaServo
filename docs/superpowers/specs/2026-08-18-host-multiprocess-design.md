# Host 多进程架构设计 — capturer/stitch/streamer/recorder/controller/emergency/monitor + OxMgr

**日期**: 2026-08-18
**状态**: 设计讨论确认（brainstorming 交互式完成）
**前置**: D102（host IPC tokio::mpsc Phase 1 → iceoryx2 SHM Phase 2）、D222-D243（link 四 SDK）、D235（去中心化 Registry，跨进程发现留 Phase 2）、deck closed_loop e2e（capture → FrameBus → recorder 实证）、host control.rs（P2P DataChannel 控制雏形）

## 1. 背景与动机

**场景**：车端（Jetson）8-9 个相机**全部实时推流**，舱端实时拉流遥控；车端环视拼接（AVM）视频推流；x86-Windows 跑 CARLA 仿真源；舱端 HMI 画面推流。当前 host 为单进程（mediaservo-host main.rs 770 行，采集+编码+推流+信令+配置合一）。

**核心动机**：**崩溃隔离 + 独立重启**——采集驱动崩溃不拖垮推流；一路媒体进程崩溃不影响其他路与控制；单个模块可热重启。线程模型无法提供真隔离（panic 传染/内存踩踏），进程模型是唯一选择。

**部署矩阵**（进程管理器必须跨平台）：
| 形态 | 平台 | 推流负载 |
|---|---|---|
| 车端 Jetson | linux-aarch64 | 8-9 相机 + 1 环视拼接 ≈ 10 路实时 |
| 边缘盒子 | linux-amd64 | 1-4 路 |
| CARLA 仿真机 | x86-Windows | 仿真相机 1-4 路 |
| 舱端 HMI | 待定 | 1 路 HMI 画面 |

## 2. 进程清单与链路

```
OxMgr（进程总管：重启策略/健康检查/日志轮转/CPU-RAM 指标/file-watch 配置热生效）
├── host-capturer × N      每相机一进程 → FrameBus 发布 I420（ts_mono/ts_epoch 对齐）
├── host-streamer × N      每路一进程：订阅任意 link topic → 编码 → WebRTC 推流
│                          （源不限于 host-capturer——第三方发布者同样可推；协商 WS → 本地总线）
├── host-recorder × 1      聚合：订阅任意路（相机路 + 拼接路）→ 各自编码落盘
├── host-controller × 1    控制通道：一条 PC（只开 DC 不开 track）+ 多 DataChannel label
├── host-emergency × 1     急停：独立进程 + 独立 PC + 本地兜底（最高可靠性通道）
└── host-agent × 1         信令网关（单 WS 聚合）+ 拓扑/数据流/信令状态监控
                           + 云端配置镜像 + 远程上报 Server

外部生产者（不归 OxMgr 管，自带生命周期，roslaunch 管）：
├── ROS 拼接节点（如环视 AVM）经 link SDK（C++ 绑定）attach → 订阅 N 路相机
│   → GPU 拼接 → 发布 1 路全景 topic → host-streamer 推流 + host-recorder 录制
└── ROS 视觉节点 经 link SDK attach → 订阅每路相机视频 → 检测（障碍物/人物/车道线等）
    → 发布视觉结果 topic（对象坐标 bbox + 提示文本 + 建议显示颜色，帧关联 ts/seq）
    → host-streamer 订阅后经 DataChannel 转发 → 舱端 HMI overlay 渲染
```

**link 角色 = 共享库（非进程）**：FrameBus（iceoryx2 SHM）+ 去中心化 Registry（D235：attach 即注册，iceoryx2 service discovery 枚举）+ 静态 ACL + 能力令牌，内嵌各进程。

**媒体链路**：`host-capturer →(FrameBus I420)→ host-streamer / host-recorder / 外部节点(ROS 拼接)`；`外部节点 →(FrameBus 1 路全景)→ host-streamer(stitch) / host-recorder`
**控制链路**：`舱端/Server →(WebRTC DataChannel)→ host-controller`（P2P 直连；SFU 经 Server data 域）
**视觉结果链路**：`ROS 视觉节点 →(FrameBus 视觉 topic)→ host-streamer →(独立 transport B: DataChannel label "vision")→ 舱端 HMI overlay`
——视频走 RTP（transport A）、视觉结果走**独立 transport B 的 DC**（mediasoup 官方: client 端 send/recv 须分离 transport + libwebrtc SCTP 与 RTP 混用互相拖累 → media/data 分离）；"视频+视觉"关联靠 ts/seq 帧关联（非 transport 关联）
**信令链路**：`各进程 →(WS→127.0.0.1:PORT)→ host-agent（信令网关）→(单 WS)→ Server`——一个 host 在 Server 侧 = 一个 peer 会话

## 3. 关键决策记录

### D-H1: 进程管理器 = OxMgr（Rust 轻量，PM2 跨平台替代）
- **来源**: github.com/Vladimir-Urik/OxMgr（Rust，226 stars，2026-08-17 活跃）；PM2 现状；Jetson 约束 C22/C23
- **优劣**（vs pm2/systemd/自研）:
  - pm2：跨平台但需 Node 运行时（车端增重 ~50MB+），非车端生态
  - systemd：Linux 最强但 **Windows CARLA 机无 systemd**（双轨运维），Jetson 裁剪可能无
  - 自研 Rust supervisor：生态一致但重造轮子（重启/日志/守护化 3-6 月），违背 C18/ponytail
  - **OxMgr**: 三平台全命中（Rust 交叉编译）+ 轻量单二进制 + 重启策略/健康检查/日志轮转/CPU-RAM 指标开箱 + oxfile.toml 配置即代码 + **file-watch 配置热生效（云端配置的关键机制）** + PM2 ecosystem 兼容
- **风险对冲**: OxMgr 只负责生命周期；进程拓扑/数据流/信令监控由自研 host-monitor 承担 → 未来换 pm2/systemd 时 monitor 接口不变
- **影响**: oxfile.toml 声明进程组（capturer/stitch/streamer/recorder/controller/emergency/monitor），restart_policy=always/on-failure

### D-H2: 帧总线 = RAW I420（非编码帧）
- **理由**: deck closed_loop 已验证同款链路（capture → FrameBus I420 → recorder 落盘）；streamer/recorder 各自编码（双编码器开销在 Jetson 硬编 NVENC/MMAPI 下可接受）；录制画质与推流码率解耦；1080p30 ≈ 93MB/s/路 SHM 零拷贝无压力（iceoryx2 实证 684MB/s 单 service，10 路独立 topic service）
- **否决**: H264 总线（录制画质=推流画质，低码率时本地录像同步受损）；双总线（YAGNI，双码率需求出现再上）

### D-H3: 控制通道 = WebRTC DataChannel（多 label）
- **理由**: 延迟（SCTP over UDP vs WS/TCP）+ 复用 WebRTC 基础设施 + 与推流协商解耦（controller 独立 PC）+ 可靠/顺序每通道可配（急停 reliable / 云台 partial-reliable）
- **现状**: mediaservo-webrtc 已有 create_data_channel/on_data_channel/RTCDataChannelRx（webrtc-sys 后端）；host control.rs 已有 DC 使用雏形（P2P E2E 验证）；**Server SFU data 域（DataProducer/DataConsumer）未实现——SFU 模式控制需 Server 后续补齐**
- **进程边界**: DC label = 通道边界（chassis/gimbal/light）；**emergency 独立进程 + 独立 PC**（PC 崩不影响急停 + 本地兜底直连执行器）

### D-H4: 监控拓扑 = 声明式期望 + 发现式实际
- **期望态**: 云端下发配置的本地镜像（config push → monitor 存储）或本地 oxfile/拓扑声明
- **实际态**: oxmgr list（进程存活）+ link Registry/iceoryx2 service discovery（进程间连接/发布者枚举）+ FrameBus 统计（数据流）+ streamer 信令状态
- **对比告警**: 期望有 N 路 capturer → 实际只有 N-1 → 告警"capturer-cam3 丢失"

## 4. host-monitor 监控维度

| 维度 | 数据源 | 产出 |
|---|---|---|
| 节点关系（拓扑）| oxmgr list + link Registry 枚举 | 期望 vs 实际对比图、缺失节点告警 |
| 数据流 | FrameBus 统计（发布/订阅、帧率、带宽/路）| 流健康度：帧率达标、停滞检测、带宽曲线 |
| 状态 | 每进程健康（oxmgr CPU/RAM/重启次数）+ 应用级心跳 | 状态面板、异常重启计数、OOM 检测 |
| 信令状态 | streamer/controller/monitor 的 WS/PC 连接状态（connected/disconnected/ICE 状态）| 信令连接矩阵、断连告警 |

**产出形态**：本地 Web/CLI（车端调试）+ 远程上报 Server（云端 dashboard 复用 admin 9800 扩展）。

## 5. 云端远程配置闭环

```
云端 Server ──(信令 WS 扩展：ConfigPush 消息 + PSK/JWT 认证 + 审计)──▶ host-monitor
host-monitor ──(写 oxfile/进程配置)──▶ OxMgr file-watch 检测 → 重启对应进程
host-monitor ──(期望态镜像)──▶ 拓扑验证/告警闭环
```

- **通道**: 信令 WS 扩展（复用连接 + 现有认证；配置下发/急停指令同一扩展面）
- **安全**: 远程配置 = 安全敏感面（远程改采集/推流参数）→ 现有 PSK/JWT 认证 + 审计日志（C15/C16 纪律）
- **生效**: 进程参数热生效（OxMgr file-watch debounce restart）；链路变更（增删路）动态启停进程

### D-H5: 进程命名规范 — host- 前缀进程族
- 所有 host 单元进程以 `host-` 前缀命名：host-capturer/host-stitch/host-streamer/host-recorder/host-controller/host-emergency/host-agent
- 多实例区分：实例后缀（host-capturer-cam0、host-streamer-cam0）；OxMgr 命名空间（namespace: host）组织
- **理由**: 进程族统一标识"车端 host 单元"；与 Server 侧进程（mediaservo-server）命名空间区分；agent 命名无占用（gateway 撞 GatewayComponent、signaling 撞模块名）

### D-H6: 单 WS 信令总线（WS 代理模式）— Server 零改动
- **形态**: 各进程 WS 连本地 127.0.0.1:PORT → host-agent 做 WS 网关（本地 accept + 远端单 WS + 双向转发 + 会话区分）→ Server 只见一个 peer = 一个车
- **理由**: 多 peer 语义缺"车"聚合层（踢下线/凭证/拉流路由/admin 视图都按设备）；Server 零改动（多路 produce = 同 peer 多 transport，mediasoup 原生支持；P2P relay 仅 SDP/ICE 交换）
- **影响**: 各进程信令地址一个配置项（连本地网关）；**RoomJoin 由 agent 拦截**（signaling.rs 实证：relay 循环内再收 RoomJoin 被静默丢弃——子进程不得逐进程 join，agent 以整车身份单次 join）；响应路由按 msg_peer_id/transport_id 映射回本地连接（SFU）或协商归属追踪（P2P relay）；host-agent 兼信令网关（职责混合可接受——信令状态监控天然在手）；controller 的 PC 协商借道总线（不持独立信令）
- **演进**: 真多车时升级 Server 设备聚合（方向 2），agent 网关平滑过渡

### D-H7: 环视拼接默认外置 — 第三方节点经 link SDK 接入
- **决策**: host 不内置 stitch 进程；拼接由外部节点（如 ROS）经 link SDK（attach → 订阅相机路 → 发布全景路）完成，host-streamer/host-recorder 订阅其结果推流/录制
- **理由**: 拼接算法（透视变换/融合/相机标定）是 ROS/算法团队的领域，不是媒体伺服平台职责；link SDK（C++ 绑定矩阵——ROS 生态）正是第三方集成契约面（D222-D243）；宿主进程与算法进程生命周期解耦（roslaunch 管 ROS，OxMgr 管 host 族）
- **安全**: 外部节点 attach 走 link ACL（D237 静态 ACL）+ 能力令牌（D238）——ROS 节点配置订阅/发布权限
- **影响**: host-streamer/host-recorder 输入通用化为"任意 link topic"（源不限于 host-capturer）；FrameMeta.format 支持 I420 及变体

### D-H8: 视觉处理结果 = 元数据流，经 streamer 的 DC 转发（舱端 HMI overlay）
- **决策**: ROS 视觉节点经 link SDK 发布检测结果 topic（对象 bbox + 提示文本 + 建议显示颜色 + 帧关联 ts/seq）；host-streamer 订阅后经**同一 PC 的 DataChannel（label "vision"）**转发；舱端 HMI 本地 overlay 渲染（视频 RTP + 检测 DC 同连接）
- **理由**: 检测结果是帧级元数据非视频帧——DC 传输（延迟低、按帧关联）；overlay 在舱端渲染（车端不烧录标注，带宽与画质无损）；"视频+视觉同连接"使 HMI 天然对齐（无需额外订阅关系）
- **通道语义**: 视觉 DC 是信息展示流（非控制流）——不并入 controller 的 PC（控制/信息分离）；挂在 streamer 进程的**独立 transport B**（mediasoup 官方: mediasoup-client/libmediasoupclient 设计上 send/recv 须分离 transport + libwebrtc SCTP 与 RTP 同 PC 调度互相拖累 → media/data 分离 transport；controller/emergency 纯 DC transport 天然合规）
- **消息格式**: JSON 起步（对象数组: class/confidence/bbox/text/color）；量级小（每路 10-30Hz 检测）；帧关联用 ts_mono/seq（FrameMeta 对齐语义）
- **安全**: 视觉节点 attach 走 link ACL + 能力令牌（同 D-H7）

### D-H9: host 二进制薄封装 — 业务配置模型 + 运维入口（不常驻）
- **决策**: host（Rust CLI，按需执行不常驻）承载业务配置模型（host.toml：cameras/streams/record/control 业务语义）+ 翻译器（业务 → 进程拓扑 + oxfile.toml）+ oxmgr CLI 代理（业务视图运维）；**不重实现进程管理**（OxMgr 直管 host-* 进程）
- **启动链**: systemd/TaskScheduler → oxmgr daemon（OxMgr 自装服务）→ host-* 进程；本地运维 `host start/stop/status/ps`（按相机/流视图）→ 代理 oxmgr
- **配置链**: 云端 ConfigPush → host-agent → host.toml → `host apply`（翻译器）→ oxfile.toml → OxMgr file-watch 热生效（增删路 = 增量 apply）
- **理由**: 云端下发业务配置而非进程管理配置（restart_policy 等是运维细节）；翻译层在 host 侧使 OxMgr 可替换（翻译器输出目标可换 systemd/pm2）；Windows CARLA 机同入口
- **功能面**: host init/start/stop/restart/status/apply/doctor/version

## 5.5 安全设计（授权/认证/加密全景）

### D-H10: link 节点间授权 — 授权文件 + 长期令牌（固定）
- **形态**: host init 签发长期 Ed25519 令牌文件（如 /etc/mediaservo/link/ros-vision.token），ROS 节点 link SDK attach 时 from_file 加载——**配置一次永久使用，零动态改动**
- **信任根**: host-agent 持 signing key（本地信任根）；各 host 节点 + ROS 节点持 verifying key + 各自令牌（host token issue --role --topic 签发）
- **固定令牌对冲**: 令牌 claims 带 topic 白名单（最小权限——泄露的 ROS 令牌只能发布声明的事）+ 文件权限 0600 + 吊销 = 重新签发部署；Ed25519 不可伪造
- **不做**（YAGNI）: 短期 ttl + 自动续签（agent 持 signing key，未来加签发端点即可）

### D-H11: host↔server 双类身份 + 舱端分级授权
- **车端（host，无人设备）**: 设备凭证（device_id + device_secret）→ Join 认证 → 短期 session token；开发/内网保留全局 PSK（渐进）；单 WS 聚合 = 单点认证（一个设备会话一次认证）
- **舱端（client，操作员）**: 操作员账号（人）——双类身份模型；**三级角色**:
  | 能力 \ 角色 | viewer | operator | admin | dispatcher |
  |---|---|---|---|---|
  | 拉流（视频+视觉）| ✅ | ✅ | ✅ | ✅（任意车，含视觉 overlay）|
  | 音频对话（会议）| ✅ | ✅ | ✅ | ✅（任意车房间）|
  | 控制（底盘/云台）| ❌ | ✅ | ✅ | ❌ |
  | 急停 | ❌ | ✅（强审计：谁/何时/来自哪个舱端）| ✅ | ❌ |
  | 配置下发 | ❌ | ❌ | ✅ | ❌ |
  | 状态/告警查看 | ❌ | ✅ | ✅ | ✅ |
- **授权矩阵**: 车端 produce 自己的流/接收配置/接收控制转发；舱端按角色 consume 授权车的流/发控制/发急停；dispatcher 可拉流监控任意车（含视觉 overlay）+ 任意车音频房间；车×舱授权关系表（租户隔离：车 A 不可见车 B）
- **加密**: 信令 TLS（wss）生产必开；WebRTC DTLS/SRTP 自带；SHM 不加密（同机可信域声明边界）

### D-H12: 音频会议房间 — 每车一房间，N 方全互连
- **模型**: 每车一个音频房间（车为单位的会议）；参与者 = 车端（host-audio 进程，始终在）+ 舱端（viewer+）+ 调度后台（dispatcher，任意车房间，浏览器 WebRTC）；音频走 WebRTC opus track 直连 Server SFU（不经 FrameBus）
- **拓扑**: 全互连（每参与者 publish 1 路 + subscribe 其他所有人）；opus ~50kbps/路，≤10 人/房间转发模式够用（更大规模才需 Server 端混音——YAGNI）
- **车端**: host-audio 进程（麦克风采集 + 扬声器播放 + opus 编解码——codec FFmpeg 后端）
- **Server 扩展**: 音频房间管理（join/leave + 房间成员列表 + 权限校验——dispatcher 任意车 / 舱端仅授权车）+ admin dashboard 音频面板（调度端）

### D-H13: 应用包布局 — host 包 + SDK 包分开发布
- **源码**: crates/mediaservo-host 改 lib + 多 bin（host.rs/host-agent/host-capturer/host-streamer/host-recorder/host-controller/host-emergency/host-audio，8 个 [[bin]] 共享 lib）；实例参数化（host-capturer --camera cam0）
- **部署包**: /opt/mediaservo-host/{bin（8 进程 + oxmgr 随包锁定版本）,etc/{host.toml,link/{signing.pem,*.token}},run/{oxfile.toml,logs,oxmgr.db},recordings/,identity.json(0600)}；发布 tar 内含版本顶层目录 `{brand}-{target}-{ver}/`，直接落前缀可用 `tar xzf <pkg>.tar.gz -C <prefix> --strip-components=1`
- **发布形态**: 分两包——mediaservo-host（车端）+ mediaservo-sdk（install bindings 现有布局）；消费方不对称（ROS/算法只要 SDK）；版本兼容靠协议契约（FrameMeta version/令牌 claims schema）显式配对，非同包隐含
- **link 令牌配发**: 令牌 = 车端部署产物（host init 签发）不随 SDK 包；**单文件自描述令牌**（verifying key + claims + signature 合并）→ 部署编排（脚本/Ansible）配发到 ROS 节点；ROS 端启动参数/env 指定路径，固定使用

### D-H14: 顺序无关健壮性（启动时序容错）
- **原则**: 所有跨进程/跨服务交互必须启动顺序无关——纯本地（验签）/容错重试（open_or_create/WS 重连）/停滞检测兜底（monitor 帧率）；OxMgr 启动顺序仅性能建议非正确性依赖
- **已内建**: attach 验签纯本地（零 RPC 时序依赖）；iceoryx2 open_or_create 双向兼容（订阅者先起创建 service，发布者后 join）+ SystemInFlux 瞬态 5 次重试；service 配置统一（API 固定）
- **增强点 1**: SignalClient 重连（指数退避 + jitter——coding-style retry_with_backoff 模式；重连 → 重新认证 → 会话恢复）
- **增强点 2**: host init 导出 ros_bridge.yaml（topic 清单 + token 路径）——ROS 侧配置单一来源（车端），零手工漂移
- **启动窗口 vs 故障**: monitor 期望态 + grace period（启动窗口告警抑制），区分"启动中"与"故障"

## 6. 已知缺口与后续工作

1. **Server SFU data 域**：mediasoup DataProducer/DataConsumer（SFU 模式 DC 控制的前置）
2. **外部节点（ROS：拼接 + 视觉处理）**：经 link SDK 接入（D-H7/D-H8）；帧同步/ts 对齐由外部节点承担（FrameMeta ts_mono/ts_epoch 提供对齐输入）
2b. **视觉 DC 消息格式**：JSON 起步，量级增长（高频多对象）时评估二进制紧凑编码
2c. **音频会议（D-H12）**：每车一房间 N 方全互连；剩余：Server 音频房间管理（join/leave/成员/权限）+ 浏览器调度面板 + host-audio 进程
3. **帧同步策略**：stitch 缓冲对齐窗口（多路帧到达时刻差异）
4. **采集 zero-copy**：MIPI/CSI 采集进 SHM 的零拷贝优化（immediate transfer）
5. **emergency 本地兜底**：执行器直连形态（CAN/GPIO/串口）与控制器冗余
6. **Server 侧**：10 路/车 × N 车 的会话规模与 admin 视图（云端多车监控）

## 7. 测试策略

- **多进程 e2e**：复用 deck closed_loop 模式（capture 发布 → FrameBus → recorder 落盘）扩展到真进程（spawn 子进程对）
- **故障注入**：杀任意进程 → 验证 OxMgr 拉起 + 其他进程不受影响 + monitor 告警
- **链路回归**：host 现有 9/9 E2E + e2e_sfu 4/4 + codec_prefs 6/6 不回归
- **Windows 验证**：CARLA 机（x86-Windows）capturer 采集 + 推流最小闭环

## 8. 范围边界

- **本次范围**: host 多进程形态设计 + 进程清单 + monitor 子系统 + OxMgr 接入 + 云端配置通道设计
- **不在本次**: client（舱端拉流，骨架阶段）；Server SFU data 域实现；拼接节点（外部团队，link SDK 契约面）；client 消费侧
