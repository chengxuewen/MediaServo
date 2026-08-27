# MediaServo Status

**生成**: 2026-08-26| 决策: 53 条目 (D196-D254, 含跳号)| Phase: 3 完成 + deck 三域 + field MVP + H1 data 域 + H2 音频会议 + 整支审查 C1/I1/I2/I3 + three-mode-build || 379 commits | 22 skills | mediasoup 0.24.1 | PIT-107 | 分支: main (C1 transport_id 注册表 + I1 room 接线 + I2 dev 守卫 + I3 StatusReport 门 + T6 C13 双轨化) || Crate | Lib Tests | Integration | 备注 |
|-------|:---------:|:------------:|------|
| mediaservo-common | 82 | — | +SfuStatsRequest/SfuStats (H2) |
| mediaservo-media | 107 | — | |
| mediaservo-webrtc (stub) | 48 | 67+ | track_sink + set_video_encoder_backend |
| mediaservo-webrtc (webrtc-sys) | 20 | 49+ (4 ICE 预存) | +AudioTrackSource 音频发送 (H2, PIT-105 阻塞 RTP) |
| mediaservo-webrtc (webrtc-rs) | 11 | 29 (9 SDP/ICE 预存) | |
| mediaservo-codec (stub) | 0 | 32 | |
| mediaservo-codec (FFmpeg) | 0 | 35 | |
| mediaservo-codec (GStreamer) | 0 | 27 | pixi 环境 | 决策: 50 条目 (D196-D246, 含跳号)| Phase: 3 完成 + deck 三域 + field MVP || 373 commits | 22 skills | mediasoup 0.24.1 | PIT-97 | 分支: main (field Push/Pull 协商全通 + SFU 多 IP) || Crate | Lib Tests | Integration | 备注 |
|-------|:---------:|:------------:|------|
| mediaservo-common | 72 | — | EncoderStatus 信令 + codec 字段 |
| mediaservo-media | 107 | — | |
| mediaservo-webrtc (stub) | 48 | 67+ | track_sink + set_video_encoder_backend |
| mediaservo-webrtc (webrtc-sys) | 20 | 49 (4 ICE 预存) | get_stats 接线 + setCodecPreferences |
| mediaservo-webrtc (webrtc-rs) | 11 | 29 (9 SDP/ICE 预存) | |
| mediaservo-codec (stub) | 0 | 32 | |
| mediaservo-codec (FFmpeg) | 0 | 35 | |
| mediaservo-codec (GStreamer) | 0 | 27 | pixi 环境 |
| mediaservo-server | 67 | 32 (27 e2e + 5 integration) | +3 SFU E2E (Linux only) |
| mediaservo-host | — | E2E 脚本 9/9 ✅ | macOS native |
| mediaservo-client | — | E2E 脚本 9/9 ✅ | macOS native |
| mediaservo-link | 32 | 跨进程 e2e 4 | 设备侧 SDK: FrameBus/Registry/ACL/令牌 + SignalClient |
| mediaservo-deck | 10 | — | source(采集 stub)+record(MP4 mux)+playback(回放) 三域 + 闭环 e2e |
| mediaservo-field | 8 | push_e2e 6 | 推流链路完成: 信令/协商/帧发布 + C ABI (field-c) |
| mediaservo-field-c | 4 | — | C ABI 绑定 (bindings/c, 11th member) |

### macOS E2E 验证 (2026-07-24)
```
Host (macOS) → WS :9800 → Docker Server → WS :9800 → Client (macOS)
                         └── P2P WebRTC (574 bytes relayed) ──┘
9/9 tests pass: Server health → Build → Host connect → Client connect → SDP → DC → Relay
```

| Phase | 状态 |
|-------|:----:|
| 0-1 基础设施 | ✅ |
| 2a-2d mediasoup SFU | ✅ |
| 3A P0 安全+容错 | ✅ |
| 3B P1 日志+文档 | ✅ |
| 3C P2 高级特性 | ✅ |
| Docker/CI/DevContainer | ✅ |
| macOS E2E 验证 | ✅ |
| Admin Dashboard (P1-P5) | ✅ |
| Admin Dashboard (P6) | 🟡 |
| 设备侧四 SDK 设计 (D222-D243) | ✅ |
| link IPC Phase 1 (FrameBus/Registry/ACL/令牌) | ✅ |
| link Phase 1b (SignalClient WS 信令) | ✅ |
| deck Phase 2 最小闭环 (采集→FrameBus→落盘) | ✅ |
| deck playback 域 (Player demux+decode) | ✅ |
| field MVP (组合门面, 10th member) | ✅ |
| OpenCode 配置优化 | ✅ |
| Doc-Audit 完整审计 | ✅ |
| OMO 插件版本审计 | ✅ (4.19.2→4.19.3 patch) |

## H2 音频会议房间完成 (2026-08-19, 4 commits)

- **音频房间 = 复用 SFU 机制**: room_id 前缀约定 `audio-<vehicle-id>`（非新 RoomType）——同机 Router 隔离，transport/produce/consume 全复用；房间语义（全互连 opus）由 ① produce 门（音频房间禁视频 producer, 4031+审计）② 客户端全订阅 表达
- **协议**: +SfuStatsRequest/SfuStats（producer/consumer RTP 统计 — e2e 媒体面证据 + H3 面板数据源）; 网关 rewrite_room 对 audio- 房间直通（子进程已用规范名）
- **G3 门**: 音频房间 join = 既有 RoomJoin 门（device 自动 / 账号白名单 / dispatcher/admin 任意车 — room_owners 存设备 ID 天然正确）; produce = 既有 can_produce（账号禁发 — 舱端只听，两方发言待 D-H11 修订）
- **webrtc-sys 音频发送**: TrackKind::Audio 真实后端 — AudioTrackSource + capture_frame(PCM i16) → libwebrtc opus; create_track_sender/audio + factory/peer_connection 接线; write_frame(kind=Audio) = PCM 10ms 帧（非编码字节）
- **host-audio 进程**: 真实实现（信令→Send transport→协商→Produce→tone 合成源 10ms 推流 + NewProducer→Recv transport→Consume 全订阅 + SfuStats 周期日志 + SIGTERM/--duration 优雅退出 0）; ALSA/MMAPI 麦克风 = Phase I+ 文档化后续
- **PIT-105（阻塞）**: libwebrtc 音频编码不产 RTP（capture_frame 成功、sink 交付实证，outbound 零包）— 音频媒体面证据 byte_count>0 挂起; e2e 断言 wiring 证据（3 producer + 6 consumer 全 Audio kind + 统计可达）
- **测试**: server 135（Docker）+ e2e_audio_conf 2/2 + host_audio_e2e 2/2 + e2e_sfu 4/4 + codec_prefs 6/6 全绿

## 决策状态

| 决策 | 内容 | 状态 | Phase |
|------|------|:----:|:-----:|
| D124-D190 | (见 decisions.md) | ✅ | 0-3 |
| D196 | Admin Dashboard 架构 | ✅ | 4 |
| D197 | D87 范围限定 (Client GUI only) | ✅ | 4 |
| D198 | SFU Server-Offer 架构 | ✅ | 4 |
| D199 | Instructions 精简化 | ✅ | Config |
| D200 | OMO Agent 模型分配优化 | ✅ | Config |
| D201 | Pre-commit Hook | ✅ | Config |
| D202 | Global Provider Config 修复 | ✅ | Config |
| D203 | Agent 模型层级最终确认 | ✅ | Config |
| D204 | ecosystem-scan 技能体系 | ✅ | Config |
| D205 | skill-router 技能创建 | ✅ | Config |
| D206 | Docker 国内镜像加速（部分修订: cargo tuna→rsproxy） | 🟡 | Config |
| D207 | 预构建 dev 镜像（机制修订: compose pull） | 🟡 | Config |
| D208 | 构建优化策略实施（详见 docs/reference/codec/build-optimization-strategy.md） | 🟡 | Config |
| D209 | 项目重命名 OMSPBase→AUDEMSP（217 文件/2363 处） | ✅ | Config |
| D210 | 帧时间戳锚定单调真实时钟（11s→2.35s 关键帧间隔） | ✅ | Pipeline |
| D211 | 帧率必须匹配 libwebrtc 编码器配置 — 帧循环绝对时间轴 | ✅ | Pipeline |
| D212 | docs/reference Diátaxis 重组 + 计划体系清理（C19） | ✅ | Docs |
| D213 | Agent 上下文爆炸治理 — instructions 瘦身 + 六模型 1024K + .agents 精简 | ✅ | Config |
| D214 | audemsp-webrtc 补全 W3C API 面 + Host SFU 标准协商（C18） | ✅ | WebRTC |
| D215 | client P2P 迁移到通用 W3C API — 修复 feature 不匹配 | ✅ | WebRTC |
| D222-D226 | 设备侧四 SDK 主架构 (link/field/client/deck) + API 单层会话型 | ✅ | SDK |
| D235-D239 | link IPC: 去中心化 SHM 注册/静态 ACL/能力令牌 Ed25519/派生 topic | ✅ | SDK |
| D240-D241 | 交付单动态库 + soname/ABI 纪律 (additive-only) | ✅ | SDK |
| D242 | link 底座选 iceoryx2 0.9.3 (spike 实证 684MB/s) | ✅ | SDK |
| D243 | FrameMeta 定长 LE + format/version, FlatBuffers 推迟 | ✅ | SDK |

## Admin Dashboard 测试

| Crate | Lib Tests |
|-------|:---------:|
| audemsp-common | 71 (+3) |
| audemsp-server | 32 (新增 admin) |
| audemsp-server e2e | 25 |
| audemsp-server integration | 5 |

## SFU Video Playback

| Phase | 状态 |
|-------|:----:|
| Docker SFU Foundation | ✅ |
| Browser SFU Client | ✅ (Server-Offer) |
| Admin WS SFU Routing | ✅ |
| Web UI (Video Grid + Metrics) | 🟡 |
| Host SFU Produce | ✅ (标准 answerer 协商, squares) |
| Integration E2E | ✅ (4/4 纯外部模式 + 浏览器渲染) |

### SFU 已完成

- ✅ `connect_transport()` 实现 (sfu.rs:331-371)
- ✅ signaling.rs ConnectWebRtcTransport handler 调用实际连接
- ✅ admin.rs 同步修复
- ✅ 浏览器 sfu-client.ts consume 消息补充 rtp_capabilities
- ✅ `default_router_options()` — Router 默认 codec (Opus+VP8+H264)
- ✅ signaling.rs peer_id 一致性修复 — 统一使用 session peer_id
- ✅ `e2e_sfu_consume_pipeline` 测试 — Host produce → Consumer consume 全链路
- ✅ SDP BUNDLE MID 修复 — `a=mid:video`/`a=mid:audio`
- ✅ Consumer late-joiner sync — `list_producers()` + pending producer queuing
- ✅ Host RTP parameters 修复 — payloadType + H264 codec
- ✅ WebRtcServer 单端口 — port 20000

### 下一步

1. Host RTP 发送 — 需要 ICE/DTLS 握手完成（当前 webrtc-rs PeerConnection 无 candidate pairs）
2. Playwright 端到端验证
3. 浏览器 ontrack → video.srcObject → 视频帧渲染

### 三种模式构建 (2026-08-26, three-mode-build T6)

| 模式 | 构建 | 运行 | 调试 | 说明 |
|------|------|------|------|------|
| ① 原生 | `pixi run build-server-native` | `pixi run run-server-native` | `pixi run run-server-native --foreground` | 开发/调试主路径 |
| ② 单容器 prod | `./mediaservo.sh build server --image runtime` | `./mediaservo.sh up --env prod` | — | 发布镜像 |
| ③ compose dev | `docker compose build` | `./mediaservo.sh up --env dev` | compose 附着 | 开发环境（延后） |

**C13 修订**：原「统一 Docker」→ 双轨（原生主 + Docker 兜底），详见 conventions.md C13。

## VideoSource 统一帧源接口 (2026-08-11, 计划 video-source-unification T1-T4)

- WebRtcTrackSink (audemsp-webrtc): 同步 VideoSource 广播 → bounded(3) channel → 异步 write_raw_i420_with_ts (c56bd87)
- Host B5 手写循环 → VideoFrameGenerator + TimestampOverlay (Combined/TopLeft) — 时间戳水印修复 (acd28d9)
- PIT-81: generator 绑定 main 级作用域修复 (7642960); e2e 脚本 headless shell (6268cf4)
- 验证: 关键帧 2.0s 不回归 + 浏览器首帧渲染 + 水印像素确认 + e2e_sfu 4/4

## setCodecPreferences 实现与验证 (2026-08-11, 计划 set-codec-preferences T1-T5)

- transceiver_set_codec_preferences: track_id 定位（mid 协商前不存在）+ fmtp 双向映射 (fc49f07)
- 6 场景矩阵 e2e_sfu_codec_prefs + offerer 机制验证 (732845e)
- 实证结论: ① offerer 偏好生效（offer codec 序重排, H264>VP8）② answerer(SFU) 偏好对
  answer 无效（libwebrtc 按 offer 序取交集）→ SFU 固定 codec 走 reduceCodecs
  ③ VP9/AV1 负向: InvalidAccessError 语义（set 拒绝/空列表）

## 编码器软/硬后端 + codec 配置 (2026-08-11, 计划 encoder-backend-codec-config T1-T7)

- set_video_encoder_backend: PcBackend track_id 分派 → SetEncoderSelector (d4e641e)
- offer codec 参数化 (config.encoder.codec) + backend 接线 + EncoderConfig.codec (78d95c4)
- H264 42e01f 全链路: router profile 统一 + produce parameters + 浏览器 consume 双 codec (75a849a)
- 验证: auto→VP8 / h264→浏览器 1280x720 渲染 / vp8 / vp9→Error 5000 / backend=software

## Web 端编码状态展示 (2026-08-11, 计划 web-stream-stats T1-T6)

- EncoderStatus 信令 + webrtc-sys get_stats 接线（ToJson 解析, encoder_implementation）(1a46296)
- Host 2s 周期上报 + server room 广播 relay（should_relay + DeviceStream 放行）(1678e8c)
- sfu-client StreamMetrics 扩展 + VideoPlayer ToDesk 风格分组面板（连接质量/编解码器/系统性能）(da16c33)
- 验证: 面板显示"软编/OpenH264/H264/30fps/1280x720" + encoder_status 4 次接收 + 全量回归

## stats 面板修复 (2026-08-11)

- 闪烁: 双数据源交替覆盖 → mergedMetrics 合并累加器（6df4630）
- 码率: 累计 bytesReceived 当瞬时 → 增量计算
- 验证: 3 采样稳定（libvpx/VP8/30fps/软编）

## 2026-08-11 长会话总览（7 计划 + 3 修复）

- OMO 配置迁移: .opencode/oh-my-openagent.jsonc → .omo/omo.jsonc（c2e9dd2）
- VideoSource 统一帧源: WebRtcTrackSink 桥接 + B5 替换 + PIT-81（c56bd87→9349a2c）
- setCodecPreferences: track_id 定位 + 6 场景矩阵 + D217（fc49f07/732845e）
- 编码双轨: set_video_encoder_backend + offer codec 控制 + D218（d4e641e→75a849a）
- Web stats 面板: EncoderStatus + get_stats + ToDesk 分组 + D219（1a46296→5ef7ff5）
- 修复: 面板闪烁(PIT-82) + 码率增量 + 解码器降级链 + 透明度（6df4630→4302f93）
- Router 5 codec: VP9(99)/AV1(97) 启用, H265 待 mediasoup 绑定（fdcd708）
- 记忆: PIT-81/82/83/84 + D217/218/219 + edit-safety 规则 9/10

## Jetson 构建 + H264/AV1 硬编可用 (2026-08-12)

- pixi.toml 补 `linux-aarch64` 平台 + `[target.linux-aarch64.activation.env]` 统一系统工具链（gcc 10.5）
- .cargo/config.toml `[target.aarch64-unknown-linux-gnu]` linker=/usr/bin/gcc + `-B/usr/bin/` rustflags
- vendor/webrtc-sys/build.rs 回滚至上游（conda workaround 移除）
- **验证**: `audemsp.sh build host` Finished; ldd 0 not-found; C++ 全链路 gcc 10.5
- **人工验证: Jetson H264 + AV1 硬编码器可用**（backend=hardware + codec=h264/av1 走 Jetson MMAPI 编码器）
- 记忆: PIT-85 + D220 + C23

## BWE 反馈链路恢复 (2026-08-12, sfu-negotiation-completion T1-T4)

- **三段缺口修复**: ① host 自构 offer 补 transport-cc/abs-capture-time extmap + nack/pli/fir rtcp-fb（3809273）② produce 参数补 headerExtensions（d807b2c）+ codecs rtcpFeedback transport-cc（6e67fc1, mediasoup TCCS 启用条件）③ 浏览器 consume 侧 buildRemoteSdp extmap/rtcp-fb + rtpCaps headerExtensions（6e67fc1）
- bitrate_kbps 语义修正: 仅 GStreamer 管线, WebRTC 编码码率用 min/max_bitrate_kbps（885f784）
- **验证**: 浏览器 1987-2003 kbps（max=2000 命中）+ 1280x720（修复前 502k@640x360）+ 30fps + 关键帧 2s; e2e_sfu 4/4
- 记忆: PIT-86 + 审计文档 docs/reference/webrtc/sdp-negotiation-bitrate-audit.md

## 平均编码耗时上报 (2026-08-12, encode-time-stats T0-T4)

- EncoderStatus.avg_encode_ms（ΔtotalEncodeTime/ΔframesEncoded 增量, 2s 窗口）+ 面板"系统性能"组显示
- 实证: 9.0ms/帧 (AV1 软编 1920x1080); libaom 软编白名单修复
- T0 顺手修: admin 路由 /admin/ 尾斜杠 + SPA fallback（axum 路由语法必须统一 *path, Arc 共享 html）
- commits 2fde281/a814ae5/059abc0

## ICE Failed 自愈 (2026-08-13, PIT-87)
- host `on_ice_connection_state_change(Failed) → exit(1)`, systemd Restart=always 拉起（05d89db）
- 验证: restart server → Disconnected → Failed → 进程退出; 重新拉起全链路恢复

## 重命名 MediaServo (2026-08-13, D221)

- AUDEMSP → MediaServo 全量重命名（T1: 259 文件机械替换 + 7 crate 目录/CLI mv; T2: AUDE 生态剥离 docs; T3: compose name/service/pixi 名）✅
- 定位: 独立部署实时媒体伺服平台（监控/NVR + 会议 + 桌面 + 遥操作），脱离 AUDE 生态（D221 修订 D209）
- 命名冲突实证 0/0/0（crates.io/npm/GitHub）; 保留面: 仅 .agents/（历史提及）; docs 调研存档/vendor 已统一 MediaServo
- T4 构建测试: 部分阻塞（webrtc-sys workspace 级构建失败, 疑磁盘/并行竞态, 与重命名无关已取证）; T5 运行时验证待完成

## 2026-08-13 长会话总览（重命名 MediaServo 全链完成）

- **重命名执行**（T1 eb7c0f7 / T2 dc46fbb / T3 480327d）: 259 文件机械替换 + 7 crate 目录/CLI mv + AUDE 生态剥离 + compose name/service/pixi 名
- **保留面收窄**（01a1f92）: 用户指令"仅 .agents 保留"——docs 调研存档/vendor/.sisyphus/.omo 统一 MediaServo（47 文件 1118 处）; 全仓 audemsp 仅剩 .agents 82 处
- **计划清除**（8bb8d19）: .sisyphus/plans 8 个 + .omo/plans 3 个全部清除（备份 /tmp/plans-backup-20260813）; git 跟踪残留 audemsp-codec acceptance-criteria 同步移除
- **doc-audit 三轮**: ① 9 项发现全修复（D221/conventions/status/AGENTS/技能）② 外部 16:10:58 批量替换污染 33 文件 → 恢复保留面（PIT-89）③ 回归闭环（仅 commits 漂移同步）
- **D221 修订**: 保留面 memorys/plans/research → 仅 .agents/; .sisyphus/.omo plans 已清除
- 遗留: gitee 仓库改名（外部）、T4/T5 待完成（webrtc-sys workspace build + 运行时验证）

## link IPC Phase 1 + SignalClient Phase 1b (2026-08-14)

### 设计落盘（D222-D243）
- 四 SDK 主架构: `docs/architecture.md §7` + `docs/modules/04-sdk-layers.md`（Rust crate 静态链接 / napi / C++ 绑定三角取舍）
- API 契约: `docs/modules/20-sdk-api-contract.md`（单层会话型; link=attach(frame,header)-per-session/publish/subscribe/close）
- link IPC 专题: `docs/modules/21-link-ipc.md`（五决策 + 风险登记表）
- 计划: `docs/superpowers/plans/2026-08-14-link-ipc-phase1.md`

### mediaservo-link 实现（8th member, 32 tests 全绿）
- **FrameBus** (iceoryx2 0.9.3, D242): topic_service 统一 `subscriber_max_buffer_size(1)+enable_safe_overflow(true)+max_publishers(1)`;
  open_or_create SystemInFlux 重试; publisher 缓存持防交付丢失; subscribe 后台线程 latest-slot; attach=验签→ACL→registry
- **Registry** (D235): 进程本地 NODES+PUBLISHERS 表, attach 即注册, mark_publisher 活跃追踪; 跨进程发现留 Phase 2
- **静态 ACL** (D237): Role(Capture/Processor/Pusher/Recorder/Control/Perception/Puller) 矩阵 + 通配 `camera/*` + deny 审计日志
- **能力令牌** (D238/D243): Ed25519 非对称签发/验签 (leeway=0), FrameMeta 定长 LE=36B (seq/w/h/format/version/is_keyframe/ts_mono/ts_epoch)
- **e2e** (T6): capture→processor 拼接→pusher 三进程零拷贝 (1080p 3.1MB) + ACL/单发布者负例
- **SignalClient Phase 1b**: WS 信令复用 common SignalingMessage/PeerRole; PSK 认证→RoomJoin→RoomJoined;
  SignalSession::events(broadcast)/send/close; LinkError::Signal; mock WS server 测试 2 个
- 多进程测试: framebus_pub 子进程 (tests/framebus_multiproc.rs); 跑 link 测试前需 `rm -f /dev/shm/iox2_*` 清残留

## deck Phase 2 最小闭环 (2026-08-17)

- **mediaservo-deck** (9th member): MediaDevices/CameraSource(stub 彩条, VideoFrameGenerator 复用) + FrameStream(有界 chan latest)
- **Recorder**: I420→x264→MP4 mux (ffmpeg-the-third 6.0, spawn_blocking worker, StopSignal 共享 running)
- **闭环 e2e** (closed_loop.rs): Capture 发布 I420 → FrameBus 传输 → Pusher 订阅 → Recorder 落盘 → ffprobe 实证 h264/55帧/1.80s/解码零错误
- **环境/依赖变更**: codec ffmpeg-the-third 5→6 (FFmpeg 9.0 Linux pixi + 8.1 macOS 双平台; 5.0 绑定编译失败); media backend-native 首次编译暴露 P010 未覆盖 match (顺手修)
- **关键踩坑**: ① Recorder worker 残留重复循环段 → first 帧二次编码 → muxer 报错无 trailer (moov 缺失) ② codecpar 手动填缺 SPS/PPS extradata → codec_name=unknown → 改 copy_parameters_from_context(enc.0) ③ pts 单位: time_base 必须 1/1_000_000 (µs 标尺) 否则 µs 值当 tick → duration=117s 假时长
- FrameBus 非 Clone → 发布泵 Arc 共享; iceoryx2 跑前 `rm -f /dev/shm/iox2_*`

## deck playback 域 + workspace 回归 (2026-08-17)

- **Player**: demux(format::input) + decode(decoder::Video 即 open 后 Opened), next_frame/duration_secs
- **e2e**: 录制→回放 roundtrip 实证 37 帧解码 @320x240; duration 校验
- **deck 三域契约主体全部落地**: source/record/playback (10 tests)
- **workspace 兼容修复**: ① deck 依赖 media 默认 backend-yuv-sys (原 backend-native 与 host
  默认特征并集冲突 → compile_error "Only one backend") ② playback feature gating
  (无 backend-ffmpeg 时明确报错) ③ yuv-sys 需 `LIBCLANG_PATH=$PIXI/lib`
- **磁盘教训**: target/debug 16G + 根分区 99% 满 → `ld: Bus error` (collect2 signal 7),
  全量回归假失败; `cargo clean` 释放 17G 后恢复 (PIT-95)

## field 组合 SDK MVP (2026-08-17)

- **mediaservo-field** (10th member): FieldError(From<LinkError>/From<DeckError>) + re-export 闭环
- **组合 re-export**: link(SignalClient/FrameBus/CapabilityToken/NodeAcl/Role/FrameTopic) + deck(CameraSource/Recorder/Player/Container/DeviceId)
- **会话门面**: PushSession/PullSession + SessionEvent 类型; connect 明确报 Phase 2 (避免静默)
- **依赖方向**: field → webrtc(默认 stub) + link + deck (C21 单向无环); webrtc 无 feature 时回落 stub (零外部依赖)
- 4 tests: re-export 一行依赖闭环 / 错误代理 / session stub / 令牌 API
- 下一步: PushSession/PullSession 接 host 推流链路 (webrtc-sys Linux 构建注意)

## OMO/OpenCode reasoning 治理 (2026-08-17, D246 + PIT-96)

- **根因修复**: 全局 provider 5 个推理模型 `supportsReasoning: false→true`（premium-max/-1/-2, deepseek-v4-pro, deepseek-v4-flash）→ reasoning_content 走结构化 thinking part（不进 content）
- **源头抑制**: fast 层 8 agent（librarian/explore/metis/sisyphus-junior/artistry/quick/writing/unspecified-low）显式 `reasoningEffort: "low"`（覆盖 80% 调用量）
- **压缩治理**: 项目 `compaction: { auto: true, tail_turns: 15 }` 保留最近 15 轮思维链 verbatim
- **顺带**: metis models 列表 premium→fast 对齐; apiKey 明文 `{env:NEW_API_KEY}` 脱敏
- **生效前提**: `export NEW_API_KEY=...` + 重启 opencode（配置启动时加载）
- **验证待办**: 重启后 oracle 一轮 → `grep '"type":"thinking"'` session 存储应为结构化 part；若无 → 网关别名侧拼接 content，下一层修网关

## 重命名 T4/T5 基线验证完成 (2026-08-17)

- **原生 check**: `pixi run check` 通过（3m58s, 33 警告无错误）— webrtc-sys 并行竞态未复发（PIT-95 磁盘 clean 后确认解除）
- **server Docker**: 修 Dockerfile fetch 层缺 link/deck/field manifest（新增 8/9/10th member 未同步, dev/builder 两 stage）→ check-server 通过（4m16s）
- **运行时**: e2e_sfu 4/4 + codec_prefs 6/6 全绿（host 原生 + Docker server, PSK=mediaservo-dev）— 重命名后首次运行时实证
- commit 4d8aff8

## 重命名 T4/T5 基线验证完成 (2026-08-17)

- **原生 check**: `pixi run check` 通过（3m58s, 33 警告无错误）— webrtc-sys 并行竞态未复发（PIT-95 磁盘 clean 后确认解除）
- **server Docker**: 修 Dockerfile fetch 层缺 link/deck/field manifest（新增 8/9/10th member 未同步, dev/builder 两 stage）→ check-server 通过（4m16s）
- **运行时**: e2e_sfu 4/4 + codec_prefs 6/6 全绿（host 原生 + Docker server, PSK=mediaservo-dev）— 重命名后首次运行时实证
- commit 4d8aff8

## field PushSession 推流链路完成 (2026-08-17, 3 slices)

- **Slice 1** (0078f9c): PushConfig/PullConfig/PublishOptions 落地 + connect(cfg) 信令建连 + 事件桥 + sfu.rs 协商纯函数
- **Slice 2** (777b588): publish_video 全链路（transport→answer→Connect→Produce）+ push_e2e 3/3（外部 server, C21）
  - 修复 broadcast 竞态: 先订阅 signal.events() 再发 CreateWebRtcTransport（否则 server 响应快于 subscribe 丢消息）
- **Slice 3** (5487a1a): start_video_frames/stop_video_frames — VideoFrameGenerator→WebRtcTrackSink→TrackSender
  - PIT-81 遵守: frame_generator owned 存会话字段; e2e D4 sender stats 实证 bytes_sent>0 + frames_encoded>0
- field 测试: sfu 3 + 单测 4 + push_e2e 4 = 11 全绿
- 下一步: PullSession 消费链路（subscribe→consume→FrameStream）+ field C ABI 绑定

## MCP 服务器修复 (2026-08-17, 3 commits)

- **context7/grep_app 405 根因**: opencode 1.18 对 remote MCP 走 SSE(GET)，context7 v4/grep.app 只接受 streamable HTTP(POST)
  → 本地桥 `init-mcp-streamable-bridge.mjs`: stdio↔streamable HTTP 转发（SDK 从 oh-my-opencode 包内解析, 零新增依赖）
  · 实测 context7 2 tools + grep_app 1 tool 全通
- **local-github 超时**: `${GITHUB_TOKEN}` 是错误插值语法（opencode 用 `{env:}`）→ 空 token 认证挂起 → 改 `{env:GITHUB_TOKEN}`
- **local-openspace**: 脚本 execSync('pixi') 找不到 → PATH 显式补 ~/.pixi/bin；剩余 pip install github 网络受限（环境性）
- **websearch**: 无 TAVILY_API_KEY → enabled: false
- commits 3d089a2/01c28cd/f9461db

## field PushSession 推流链路完成 (2026-08-17, 3 slices)

- **Slice 1** (0078f9c): PushConfig/PullConfig/PublishOptions 落地 + connect(cfg) 信令建连 + 事件桥 + sfu.rs 协商纯函数
- **Slice 2** (777b588): publish_video 全链路（transport→answer→Connect→Produce）+ push_e2e 3/3（外部 server, C21）
  - 修复 broadcast 竞态: 先订阅 signal.events() 再发 CreateWebRtcTransport
- **Slice 3** (5487a1a): start_video_frames/stop_video_frames — VideoFrameGenerator→WebRtcTrackSink→TrackSender
  - PIT-81 遵守: frame_generator owned 存会话字段; e2e D4 sender stats 实证 bytes_sent>0 + frames_encoded>0
- field 测试: sfu 3 + 单测 4 + push_e2e 4 = 11 全绿

## PullSession 消费链路 (2026-08-17, 2e356af — 协商完成, 收帧挂起收口)

- webrtc-sys: 实现 add_transceiver(kind) 版（add_transceiver_for_media, recvonly 纯接收）— 之前 NotSupported
- PullSession::subscribe: Recv transport → 标准 answerer → Connect → Consume → Consumed
  · on_track 必须在 set_remote_description 前注册（remote sendonly m-line 触发即丢）
  · transport_connected 确认消息跳过（server 惯例, 非真错误）
- sfu.rs: build_remote_sdp 方向参数化（RemoteDirection: ServerSendonly/ServerRecvonly）
- 协商收敛 (37e257d/99e6c85): ssrc 注入 + mid=0 + 完整 rtp_capabilities + W3C 协商顺序 + rtcp-rsize
- **收口结论 (2026-08-18)**: field = 遥控车端 SDK, 只需推流（已完成）; PullSession 消费方是
  client（舱端, 骨架阶段）→ 收帧问题挂起不阻塞 field 交付
  · tcpdump 实证: RTP 完全正确到达 libwebrtc socket (ssrc=Consumed 值, PT=96, seq 连续)
  · track=Live enabled=true, sink 挂载 — 但 on_frame 不触发 = libwebrtc 接收管线/webrtc-sys 集成缺陷
  · C++ 对照程序半成品在 /tmp/opencode/pull_webrtc_test.cpp（编译通过, 链接未解）
  · 归属: client 端开发时攻关（判别: C++ 收帧=Rust 绑定 bug; C++ 不收=libwebrtc 管线问题）
- 诊断工具: push_observe/pull_observe examples + server producer on_trace/dump 观测

## SFU 多 announced IP (2026-08-17, db88829)

- server: MEDIASERVO_SFU_ANNOUNCED_IP 支持逗号分隔多 IP（宿主多网卡）→ 每个 IP 一个 WebRtcServer ListenInfo
- CLI: ip -o addr 按接口名过滤 docker 网桥(br-*)/VPN(tun*)/虚拟接口，仅真实网卡
  · 实测 3 网卡只报 ens32 (192.168.2.127)，排除 docker0/br-*/tun0
- compose: 注释更新（CLI 自动注入; 直接 compose 需手动设 env）
- 不写死要求: 宿主 IP 变化/多 IP 全动态

## field 定位澄清 + PullSession 收口 (2026-08-18)

- **field = 遥控车端 SDK**: 只需视频推流（PushSession 已完成并实证）— 拉流是舱端 client 的事
- PullSession 收帧挂起为已知限制（libwebrtc 接收管线缺陷, RTP 全对但 on_frame 不触发）
- field 剩余收尾: C ABI 绑定（ms_field_*）+ 推流示例/文档 + PushSession 测试补强

## field 收尾完成 (2026-08-18, 3 commits)

- **C ABI 绑定** (cd0fd29): bindings/c/mediaservo-field-c (11th member, cdylib)
  · ms_field_push_connect/publish_video/start_video_frames/stop/close + ms_last_error/ms_field_version
  · include/mediaservo_field.h 手工维护（D241: 稳定 C ABI 面）+ catch_unwind 防护 + 4 tests
- **示例/文档** (16e2f16): vehicle_push.rs (Rust 完整流程) + vehicle_push.c (C ABI 消费)
  + docs/modules/22-field-guide.md（车端集成指引: 流程/前置/已验证能力/已知限制）
- **测试补强** (1fd2895): config 单测 5 + D6 重复 publish 报错 + D7 低码率帧验证
  · 实证: libwebrtc BWE 自适应降分辨率（低码率 scaling down）— 正常行为
  · D5 (PullSession 收帧) 标 #[ignore] 文档化已知限制
- field 测试: lib 8 + push_e2e 6 + field-c 4 = 18 全绿

## field C ABI 交付 — cxx/py 待决策 (2026-08-18)

- **已交付**: C ABI (bindings/c/mediaservo-field-c, cd0fd29)
- **未实现**: C++ (header-only RAII over C ABI) / Python (ctypes 加载 cdylib) —
  按 D227/D240 设计是薄包装（各 ~30min 成本）
- **待确认**: 车端真实消费语言（Rust 主控则 C ABI 即最终交付; C++/Python 主控则补绑定）
- **扩展面**: 契约 §7 规划 link/field/deck/client × c/cxx/py = 12 绑定, 当前仅 field-c

## OMO 配置 schema 修复 (2026-08-18, PIT-97 + C27)

- 迁移把 `model`+`fallback_models` 写成 `models`（复数）→ v4.19.4 schema z.$strip 静默丢弃 → agent 模型回落默认值
- 修复: `.omo/omo.jsonc` 全部改回 `model`/`fallback_models`（19/19 处），保留 D246 reasoningEffort 分层
- 验证: `grep -c '"models"' .omo/omo.jsonc` = 0；重启 opencode 生效

## MCP bridge 退役 (2026-08-18)

- oh-my-openagent@4.19.4 内置 context7/grep_app MCP（dist/index.js 直接 StreamableHTTPClientTransport 注册）→ 移除 `.opencode/init-mcp-streamable-bridge.mjs` + opencode.json 显式配置（1.18 SSE→405 时期的权宜之计，见 2026-08-17 记录）
- 验证: 本会话 context7/grep_app 工具可用、无 bridge 进程

## 绑定矩阵完成 — link/deck/field × c/cxx/py (2026-08-18, D247/D248)

- **C ABI 三件套**: field-c（加固: 共享 runtime/closed/struct_size）+ link-c（signal 4 + bus 4 + 事件泵线程）+ deck-c（camera/recorder/player + backend-ffmpeg + --exclude-libs,ALL）— live e2e 全通（server 收帧/事件泵/91 帧闭环）
- **C++**: 三 header-only RAII（FfiHandle/Result 模式, namespace mediaservo::{field,link,deck}）— 测试全过
- **Python**: ctypes 三子模块（_ffi.py 加载层 + argtypes 全覆盖 + 回调防 GC）— 22 tests + live e2e
- **D247**: 符号前缀 ms_ → mediaservo_（全名, 三重对齐）+ 头文件布局 include/mediaservo/（D248: 手工维护 + abi-drift 门禁, cbindgen 押后）
- **pixi tasks**: build-c / test-cxx / test-py / parity-bindings / abi-drift
- **PIT-98**: 代理并发仓级重命名被 git checkout 冲掉（edit-safety 规则 14）
- workspace 16 members（10 crates + 3 c ABI + 3 空 cxx 载体）; 测试: field-c 8 + link-c 20 + deck-c 19 + cxx 3 套 + py 22

## 绑定矩阵四语言完成 — c/cxx/py/node (2026-08-18, D249/D250)

- **C++ 迁移**: Result 完全迁移 tl::expected 1.2.0（CC0 vendor 3rdparty/ + 原生 API + C++11 门禁，单 commit 5d0aa5c）——计划 docs/superpowers/plans/2026-08-18-cxx-tl-expected.md
- **Node 绑定**: napi-rs 直绑（field/link/deck + Recorder/Player，livekit 同构）+ TS 薄包装 + node:test 5/5 + CLI 接入（build/install bindings 含 node）——真 server 推流/事件桥/录制回放闭环实证
- **关键修复**: FFmpeg 链接补齐（PIT-99）、Recorder 死锁（PIT-100）、libstdc++ ABI（PIT-101）
- workspace 17 members（+ mediaservo-node）; 测试: c 47 + cxx 4 套 + py 22 + node 5
- 运行前置: node 需 LD_PRELOAD pixi libstdc++ 或平台编译; FFmpeg 动态库 LD_LIBRARY_PATH

## C2 streamer 进程完成 (2026-08-19, 105e854)

- **host-streamer 真实现**: --stream/--config/--token → host.toml [[streams]]（camera/codec 缺省 id/vp8）→ FrameBus 订阅 camera/<id>（FrameMeta+紧凑 I420, C1 线格式）→ field PushSession（connect→publish_video 全链路复用）→ TrackSender write_raw_i420_with_ts（C17 ts_mono_ns 透传）→ 2s 出站 stats 日志 → 10s 无帧看门狗退出待重启 → SIGTERM 优雅 0
- **复用**: field PushSession 推流链路 + video_sender() 访问器（additive 6 行）+ link FrameBus::subscribe（latest-slot 背压）+ field D4 证据模式
- **测试**: streamer_e2e 2（坏参 exit 2 + capturer/streamer 双进程→外部 Docker server 收流 bytes_sent=8144 frames_encoded=55, SIGTERM 双 0）; translate +1; host 全量 51 绿; field 18 绿; pixi run check 0 error
- **遗留**: ① oxfile streamer 令牌文件角色须为 Recorder（签发属 B 阶段）② SFU_E2E_* 环境变量是测试约定名，Phase D 正规化

## C5 crash_recovery e2e 完成 (2026-08-19, 1874d6a + 2dd8c9f)

- **实证反转**: "订阅端跨发布端崩溃 stale"是测试断言工件，非 iceoryx2 缺陷 — latest-slot 吞掉重启归零帧（PIT-102）。seq 全量记录证实旧订阅端连接自动重建（重启点 2→0 归零后连续）
- **link 兜底** (1874d6a): 发布端列表每重启只变一次，若那次连接创建被 degradation handler 吞掉则永不重试 → 订阅线程 5s 无帧重建 subscriber（FrameStream 句柄不变，D241；失败保留旧句柄 30s 冷却重试）+ framebus_crash_recovery 测试 2/2（64B@10fps + 1080p@30fps）
- **e2e 完成** (2dd8c9f): 杀前基线 ≥30 帧 + 后台 drainer 确定性断言归零帧；[record] enabled → host-recorder 真实长生命周期订阅端全程存活验证
- 验证: crash_recovery 3/3 稳定（~3s/run）、host 全量 51 绿、link 全绿、pixi run check 0 error

## G4 设备身份配发完成 (2026-08-19, b21c05e)

- **Wire 契约（G2 server 侧实现面）**: RoomJoin 增加可选 device_id/device_secret（serde skip-if-none,
  additive 双向兼容）——缺省 = PSK 路径；携带 = G2 设备认证；失败回 Error（client 已实证明确报错）
- **identity.json**（D-H13 实例根, 0600）: `{device_id: "ms-<12hex>", device_secret: "<64hex>"}`;
  `host init` 幂等——仅缺失时生成, 覆盖会使 server 注册失效; 损坏显式报错（C15）
- **携带链**: host-agent --config → 实例目录 → load_identity → GatewayConfig.device →
  SignalClient::with_device_credentials → RoomJoin（PSK 并存, G2 切换校验）
- **测试**: link wire 2 新增（携带 + 4010 报错）+ common 序列化契约 + identity 单测 3 +
  CLI init e2e + gateway e2e 携带断言; 回归 host 96/link 55/client 13 全绿 + e2e_sfu 4/4 +
  codec_prefs 6/6（live server）
- 验证: `--tests --benches` 编译挂已在 G2 顺手修（d49bd7f bench.rs cfg 门控 + async 特征感知构造）

## G3 舱端分级授权完成 (2026-08-19, c825e76/9c13dfa/e17160f)

- **账号模型（D-H11 选项②, JWT 复用）**: accounts.yaml `{username: {password_hash: "sha256:<hex>", role, vehicles}}`
  — sha256(username:password) 单向哈希 + username 盐（同 G2 devices 存储决策）; 未知用户/错密 wire 逐字一致防枚举;
  POST /api/auth/login → JWT {sub, role, vehicles, iat, exp}（HS256 与 admin_jwt_secret 同 secret 同算法, 12h）
- **四级角色矩阵（roles.rs 纯函数, 表驱动 11 测试）**: viewer/operator/admin/dispatcher ×
  pull(白名单)/control/emergency/config/status/audio; SessionIdentity(Device/Account/Legacy) —
  additive: 仅账号与设备会话启用强制, PSK legacy 不受限（未配置账号部署行为不变）
- **强制点**: ① RoomJoin 门（账号禁 Host 防抢占 + 按房间主车白名单/租户隔离"车 A 不可见车 B" +
  车端 join 登记 room_owners）② EmergencyCommand（operator/admin+车访问权 → 强审计
  谁/何时/车/命令 + 转发车端; 经信令转发 = 可审计 — P2P DC 常规控制协商期已授权, 边界文档化）
  ③ SFU Produce 账号拒绝/车端自动允许（回归实证）④ SFU Consume 账号仅有权车 producer
  （producer_owners 纵深防御）⑤ ConfigPush 入站一律拒绝（server 单向下发）⑥ admin REST
  config push 按 role==admin（check_auth sub→role）
- **审计**: audit.rs EmergencyCommand + AuthorizationDenied 事件 + 有界 256 环形缓冲
  （audit::recent() 运维/测试读; tracing 日志仍是主通道; C15 denial 全审计）
- **错误码**: 4011 未知角色（握手期拒）; 4031 授权拒绝（join/consume/produce/emergency/config）
- **测试**: server 122 全绿（Docker test-server: lib 71 + admin_e2e 6 + e2e 25 + e2e_sfu 4 +
  integration 16）; 原生 --no-default-features 117 绿; live server 回归 field push_e2e 6/6 +
  controller_e2e 1/1; pixi run check 0 error
- **P2P 边界**: 底盘/云台控制走 P2P DC（协商期按角色授权 = 控制权的强制点）; 急停走信令
  转发（强审计要求 — P2P 流量服务端不可见）; DeviceStream 房间 SDP 帧过滤天然阻断
  P2P 协商绕过（租户隔离竞态关闭）

## H1 SFU data 域完成 (2026-08-19, 4 commits)

- **wire (common)**: CreateDataProducer/DataProducerCreated/NewDataProducer/ConsumeData/DataConsumed + SctpStreamParameters (camelCase, mediasoup 官方 sctp-parameters 对齐) — 9 roundtrip 测试
- **sfu.rs**: WebRtcTransport enable_sctp=true（additive, 纯媒体流不受影响）+ create_data_producer/consumer/list_data_producers + SfuPeer data vecs
  · 实证: produce_data 未连 DTLS 也成功（worker 允许）；transport dump sctp_parameters 非空
  · data_message_roundtrip_direct #[ignore] — PIT-104: mediasoup-rs 0.24.1 worker→app 通知通道
    部署级失效（on_message/on_data_producer_close/worker_close 全静默, 官方测试复刻同样失败;
    请求响应正常; worker 侧路由实证 messages_sent=1）
- **signaling**: CreateDataProducer/ConsumeData 处理（方向校验→G3 门→producer_owners 登记→
  NewDataProducer 广播→响应; 拒绝 4031+audit）+ late-joiner list_data_producers 重放
- **e2e**: e2e_sfu_data_domain 4 段（车端 produce_data 放行 + 广播到达 + 授权 consume_data 放行 +
  账号 produce_data 4031）; Docker 全量 134 绿; 原生 75 绿; clippy 0
- **遗留**: 消息内容端到端接收证明阻塞于 PIT-104（upstream）；host 侧 SFU-DC 接线归 H2+


## 整支审查 C1+I1+I2+I3 修复波 (2026-08-20, 67eec0e + 1a11942)

- **C1 (CRITICAL, D-H14 顺序无关)**: Produce/Consume/CreateDataProducer/ConsumeData
  加可选 transport_id（skip_serializing_if, 双向兼容）; SfuPeer 单槽 →
  send/recv_transports 注册表 + producer→transport 绑定表 + 绑定访问器;
  produce/consume/data 按 transport_id 指名绑定, None = legacy 最近创建回退;
  connect_transport 按 id 双注册表查找。客户端绑定链完成: field Push/Pull +
  host-audio + host-legacy 发 Some(transport_id)。TDD: 2 新注册表测试 +
  4 wire 兼容测试。Docker server 95 lib + 6 e2e_sfu 全绿; field push_e2e
  6/6 live 走新路径。
- **I1**: host.toml [signaling] room → oxfile host-agent --room → GatewayConfig.room
  （D3 TODO 关闭; 缺省 vehicle 保持; translate 测试 14/14）。
- **I2**: accounts.docker.yaml dev 占位哈希（admin123/dispatch123/operator123）
  启动 fail-fast（DEVELOPMENT CREDENTIALS DETECTED）; MEDIASERVO_ALLOW_DEV_CREDENTIALS=1
  豁免（dev compose ×2 已设）; 2 新单测。
- **I3**: StatusReport 仅 Device 会话或 Host 角色可上报; 拒绝 4031 + 审计; 2 新单测。
- **streamer_e2e**: admin_rooms() 适配 H3 auth 强制 JWT（dev 账号登录取 token）。
- **recorder_e2e 2/4 既有失败（非本波）**: PIT-107 — livekit libwebrtc.a 内嵌
  demuxer-only 静态 FFmpeg 抢先满足 ffmpeg-the-third 符号 → mp4 mux 失败;
  clean HEAD stash 实证; C30 规则沉淀。修复 = webrtc-sys 符号前缀化/链接序（独立任务）。
- **I4（macOS client e2e 9/9）**: 环境阻塞（Linux 宿主），记录欠账，macOS 回填。

## host 多进程计划收官 + 部署演练 + 工具链治理 (2026-08-20)

- **9 阶段计划完成**（docs/superpowers/plans/2026-08-18-host-multiprocess.md，90+ commits）：OxMgr 管 8 进程（agent/capturer/streamer/recorder/controller/emergency/audio + CLI）——一车一会话（单 WS 网关）+ 崩溃隔离实证（C5）+ 监控四维 + 云端配置闭环 + SFU data 域 + 音频会议（audio-<vehicle-id> 前缀）+ 双包发布
- **人工部署演练**（stub 彩条 VideoFrameGenerator 模拟相机）：install host → init → token → start → 7 进程全 running → server 收 RTP 关键帧实证（bytes_sent=5506/239 帧）；发现并修 3 集成缺口（host-audio --room、Pusher stats 发布权、devices 配发实操——PIT-108）
- **安全**：PIT-103 admin 零认证修复（JWT 中间件）+ 登录页/路由守卫（用户驱动）+ 生产 compose 移除 dev 豁免（PIT-110）+ G2 设备认证（devices.yaml 配发链）
- **工具链**：C31 任务分级（小任务父会话直做——277K 上下文派发实证）；host CLI 完善（-h 完整帮助、位置参数 + .host/ 默认、模板提为源码文件 include_str!、[signaling] server_url/psk 配置面、mediaservo-host 改名中）
- **遗留**：macOS client e2e 9/9 回填；PIT-104/105 vendor 域；PIT-107 符号前缀化（独立任务）；mediaservo-host 改名待完成

## host 部署运维体系（用户驱动，2026-08-20）

- **CLI 完善**: mediaservo-host 改名（避 /usr/bin/host）+ -h 完整帮助 + install 双快捷方式 + [signaling] server_url/psk 配置面 + 模板提源码文件（include_str!）+ 智能默认目录（实例根可用）
- **oxmgr 集成**: monit/ps/logs 代理 + OXMGR_DATA_DIR 实例化（PIT-113 根治）+ namespace mediaservo-host + 日志同步 run/logs（PIT-112 symlink 桥）+ startup 自管 unit（三端/全局唯一/交互接管）
- **竞争防护**: start 端口检测 + 交互接管；install 自动 stop（host+daemon，PIT-115）；SystemInFlux 复发防护（start 清 SHM + streamer 订阅重试）
- **web play 修复链**: SDP 广播污染（PIT-114 sfuMode 全程忽略）→ Playwright 实测 1280x720 真实帧（vehicle-live/vehicle-content spec）；画面增强（方块移动+时间戳水印）
- **遗留**: server 重启后媒体面自愈（PIT-111 长期修复待实现）；macOS/Windows startup 自管 unit；真实终端交互验证（接管路径）

## app-branding-customization 完成 (2026-08-21, D252/C33 + PIT-118~121)

- **Brand 机制**: `common::brand`（env MEDIASERVO_BRAND > 编译期 > 默认）——默认品牌 legacy 串硬映射（app `host-*`/unit `oxmgr-host-`/device `ms-`/namespace `mediaservo-host`——勿按 `<product>-` 直推）；product/display/id 三语义分离
- **固化边界**: bindings/*（C ABI mediaservo_* D247 + cxx/py/node）+ wire 协议 + crate 名——零 diff（固化门）; 可定制 = host（app 名/namespace/unit/device/help/install --brand）+ client（标题/路径）+ server（admin __APP_TITLE__ vite define, C24 编译期）
- **回归门三件套全绿**: e2e-install-host.sh（start roundtrip 硬化）+ e2e-package.sh（PIT-119 strip: 1.2GB→60MB）+ e2e-brand.sh（品牌化全链断言）; live: e2e_sfu 4/4 + field push_e2e 6/6（含 MEDIASERVO_BRAND=cp 模式——wire 无回归）
- **测试**: common 89 + host lib 50 全绿; workspace check 0 error; install --brand cp 布局（cp/cp-host 快捷 + bin/cp-* + identity cp-<12hex>）
- **PIT-118~121**: ① translate namespace {ns} 占位符残留→oxmgr 拒收→apply 挂(regression 测试已加) ② package 打包 strip/压缩超时 ③ pkill set -e 自杀（清理 pkill 必须 || true）④ cmd_oxmgr ps/monit/logs 按 cwd 推断 dir（status 才吃参数）
- **计划**: docs/superpowers/plans/2026-08-21-app-branding-customization.md（Momus APPROVE-WITH-CONDITIONS 3 HIGH 全采纳）
- **遗留**: e2e-package staging 残留（dist 手动清）; 品牌化 macOS/Windows startup 待三端; PIT-111（server 重启媒体面自愈——大债务待立计划）

## build-deploy-unify 三种模式 + install→deploy 重构 (2026-08-27)

### 三种模式命令体系（完成）

| 模式 | 命令 | 状态 |
|------|------|------|
| ① 本机原生 | `build server --native` + `run server` + `status server` + `stop server` | ✅ |
| ② 单容器 prod | `up --env prod`（Docker runtime 镜像——entrypoint 自举） | ✅ |
| ③ compose dev | `up --env dev`（源码挂载 + cargo run 热更） | ✅ |

- **C13 双轨化**（D255）：原生主路径 + Docker 发布/CI 兜底
- **默认 native**（D256）：不带模式=原生，容器全显式（--mode compose/--env）
- **-h 帮助面审计**（5 角色团队）：epilog 三模式速查 + pixi 横幅 TTY 守卫 + 退出码契约 + 术语统一

### install→deploy 重构（完成）

| 改动 | commit | 状态 |
|------|--------|------|
| `build host` 组装 out/host/bin（品牌化） | 95782dc | ✅ |
| `build server` 组装 out/server/bin + etc/server.yaml | 7141dc6 + 00cb936 | ✅ |
| `install → deploy` 重构（_derive_brand/D4/D1） | a9020c1 + 32f5998 | ✅ |
| `package` 源修复（host tar 契约 + bindings 布局补齐） | eb04220 | ✅ |
| msrtc.sh PURE_BRAND 移除 + install→deploy 转发 | 4bbe684 + 6d62739 | ✅ |

- **D257**：install→deploy（build 无状态 vs deploy 有状态分离）
- **D258**：server 默认配置路径 bin/../etc/server.yaml（相对二进形）
- **D259**：accounts/devices.docker.yaml → accounts.yaml/devices.yaml（去 docker 后缀）

### 当前命令面

```
build server|host|bindings     → out/<target>/（交付布局——品牌化组装）
deploy host|bindings           → 有状态部署（--prefix 必填，identity/oxmgr/env.sh）
package host|bindings          → dist/ tar（out + staging deploy 组装）
run server                     → 裸机运行（优先 out/server/bin/，配置 bin/../etc/server.yaml）
status server|host             → 健康探测（退出码 0/1/2）
install                        → 改名提示 + exit 2（退役）
```

### 遗留
- server 多 ListenInfo 完整验证（多网卡 host 场景）
- build-deploy-unify 团队审核后退守 deferred minors
- macOS 启动命令（launchd）待补
