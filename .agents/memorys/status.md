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

#### 2026-09-01: branding-completion — 裸机物理名品牌化收口（D269）

| 项 | 状态 | 说明 |
|----|------|------|
| server 品牌装配（`bin/{brand}-server`+根级快捷） | ✅ | 双探源幂等/`_derive_brand_server`/oxfile 陈旧检测强刷/白名单/clean 品牌态（Momus 3 BLOCKER 全修） |
| `mediaservo-client` 出 host 树 | ✅ | tar 契约收窄；白名单同轮清存量；build/stop client cargo 面不动 |
| `host-streamer` 兼容链退役 | ✅ | 实证冗余（oxfile 全指 msrtc-*、src 字面量仅测试）；streamer 自报 src 改 app_prefix 派生（默认品牌逐字节不变） |
| 验收 | ✅ | /tmp 矩阵全绿：fresh/迁移/二次部署/空brand防护/clean/package tar/host 布局/真实起停+登录 200 |
| 事故 | 2 | PIT-171 内联凭证脱敏假 401（系统无 bug）；PIT-172 cp -al 夹具 pid 硬链 → clean 误杀生产 9800（已恢复为 oxmgr 簇管理，restart_policy 自愈） |
| UX 缺陷补刀（用户报障轮） | ✅ | ① `-h/-H/--help` → USAGE（原先 -H 落 daemon 撞 C35 panic）；未知选项 exit 2（守护实际只认 --config，回落面收紧无兼容损失）；② **oxfile 迁移 env 回吸收**：改名重渲染时旧 [apps.env] 按 app 名自动补入新文件（只补缺不覆盖 baked 值）+ 旧文件备份 run/oxfile.toml.bak——PIT-172 同族的"迁移税"根治，用户实测复现（start 失败=env 丢）后落地 |
| 生产迁移 | ⏳ 待用户 | `out/server` 重跑 `build:deploy server` 自动改名迁移（oxfile 重渲染+[apps.env] 手工项需重加——当前 live 有 RUST_LOG/ALLOW_DEV 两条，迁移后按提示重加）；out/host 集群拔链切换 = 下次例行 deploy 生效（本轮二进制未重编，旧链保险丝随 cli 拔除——**注**：现存 8/28 msrtc-host 二进制 spawn 走 oxfile command 路径，无风险） |

## 下一步

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
- **计划清除**（8bb8d19）: docs/plans 8 个 + .omo/plans 3 个全部清除（备份 /tmp/plans-backup-20260813）; git 跟踪残留 audemsp-codec acceptance-criteria 同步移除
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


## 2026-08-31: frontend-process-split Phase 0-5 完成（子模块侧）

- **fix(security)**: signaling JWT 守卫授权旁路修复 + admin_psk_test E0382（PIT-163/164/165）
- **feat(health)**: `/ready`←`worker_alive()=!Worker::closed()`（sfu/monitor/signaling；PIT-167 线程模型）
- **feat(build)**: `admin-dashboard` 出 default（翻转），Dockerfile 显式双 features；
  `build web`/`run·stop·restart·status web`（过渡态）/`dev web` 后置于 Phase 6；
  deploy/caddy/{Caddyfile.native,Caddyfile.split}；根 Caddyfile/docker-compose 不动（实况修正）
- **fix(docker)**: runtime su-exec→setpriv（**模式②镜像自 e56650c 起从未构建成功**，PIT-169）
- **验收**: 双姿态全测绿（e2e_sfu 6/6、integration 18/18）、Playwright 经 Caddy 出画 1280x720、
  WS 330s 存活、host restart 免刷新自愈 V1 过、runtime 镜像 /admin 内嵌 SPA 恢复
- **待做**: Phase 6 = msrtc-server 单二进制双角色（T15-T22）；遗留：video streamer SIGKILL 重放
  （PIT-168）、/ready 自动看门狗、protocol_version 握手

### 2026-08-31 续: Phase 6 完成（T15-T22 实证）

- **feat(server)**: 单二进制双角色（main.rs USAGE/派发/`run`=daemon 回落，既有直启零破坏）+
  `lifecycle/`（mod/templates/inspect/startup ≈1517 行含 320 测试）：init（模板+secret 0600 幂等+
  静态 oxfile+快照式端口烘焙）、start [--no-web]（C32 四 env 全作用域/端口守卫/macOS parity）、
  stop/restart（闲置前缀幂等无泄漏——事故复现过）、status(/ready 列,0/1/2)、doctor、logs、
  startup on/off（systemd 锚点 unit，二次实例拒绝）、monit/ps 代理
- **feat(cli)**: deploy server --prefix（源=out/ 唯一，deploy 不触发构建；dev 模板账号 fail-fast
  警告）、package server、dev web、run/start/stop/restart server 退役→exit 2、clean server 扩展、
  _pids_using exe-inode 精确占用判定（PIT-170 修复）
- **T22 端到端**: /tmp/t22 全环绿——deploy→适配→整簇 status=0→SPA/探针/代理/登录发证→
  startup on/off→restart 自愈→stop 零残留、生产 9800/host 簇无损
- **T14**: docs/modules/development/frontend-split-deploy.md（终态手册）
- **修复（用户报障）**: build:deploy server 糖注入 --prefix out/server=源树 → SameFileError/rmtree 源自毁风险；_cmd_deploy_server 增 inplace 守卫（bin/web 跳拷贝、init 照常渲染，out/server 可原地起实例）。一行教训：deploy 糖默认前缀语义对 server（源即交付树）与 host（源=target）不同构
- **演练暴露的既有项（未动，报主）**：`_E2E_SUITES` 未定义（cli e2e NameError，HEAD 存量）；
  `oxmgr-host-...-MediaServo-install-host.service`（enabled，指向已不存在的旧树——开机噪音）；
  init 端口快照语义（改进项 init --port 未做）

### 2026-09-01: play-layout-stats — 网格列数/全宽/mini stats/遮罩/lucide（验收 16/16 绿）

| 项 | 状态 | 说明 |
|----|------|------|
| ① 列数选择器 | ✅ | 默认 3/上限 4（产品裁决），localStorage `mediaservo_play_cols` + 脏值防御；F5 持久实测 |
| ② 全宽自适应 | ✅ | `.dashboard` 960px 锁除；2560 视口 grid 2312=可用宽 100%；tile aspect 裁切连带修复（16:9 归 vp-body） |
| ③ mini stats 常驻卡 | ✅ | tile 左上 connected 即显（T+16ms）；**产品精化（用户采纳 A 案）**：核心六指标 2列×3行常驻（帧率/分辨率/码率/抖动/延时/丢包，遮挡 ~15% 实测），编解码/系统详情走**浮层二次点击（Portal→body，fixed 浮于卡旁，与 tile 尺寸解耦——4 列小 tile 免遮挡；点外部/ESC 收起；面板已去重六项以卡为唯一呈现）**；tile 底部 bar 退役、✕ 解禁；**双击 tile→独立 modal 大窗（方案②）**|
| ③ 断联遮罩双态 | ✅ | 「连接失败/无法建立 WebRTC 连接」vs「连接已断开/视频流已中断」双态实盘（A9+A14）；遮罩盖画面不盖 top-bar |
| ④ lucide 全站 | ✅ | 32 处 emoji→SVG 零残留；bundle +12KB gzip+3KB |
| Uptime→Peers 卡 | ✅ | T1 定性改判：server 无 uptime 源、前端契约臆造（StatsResponse 三字段对齐 server 实况）；用户裁决换 Peers |
| encoder_status 断链 | ✅ 已修（09-02） | 实际断点**两处串联**：host 从不发（E3 拆分迁移丢，补 build_encoder_status+log_stats 发送）+ gateway rewrite_room 截胡整车房间（补：该消息豁免改写 + server `relay_target_room` 按消息子房间路由，单测钉住）。教训：白名单放行≠路由正确（t8-deferred.md 有修订全案）|
| 验收 | ✅ | 16/16（t11-results.json + 4 截图 + look_at 视觉复核）；Momus 两轮 0 BLOCKER |

- 过程事故：开局撞 PIT-168 型黑洞（server 迁移致 streamer 会话死、房间无 producer）→ `msrtc-host restart out/host` 恢复——**PIT-168 触发面扩展：SIGKILL 之外，server 重建同样**；A14 验证法沉淀为 PIT-174（playwright 拦不了 WS，用构造注入）。
- 提交：子模块 `feat(admin)`（www 12 文件 + 记忆）；主仓 gitlink+记忆+计划四件套+evidence+roadmap（C41 合规）。
- 下一步衔接：D270-a 内包化（sfu-client→packages/mediaservo-sfu，独立小轮禁止混入本轮，范围外注已锁）。

### 2026-09-02: encoder-status-chain 修复（play-layout-stats 遗留 c 象限闭环）
- host streamer 补发 EncoderStatus（2s stats 循环，room 声明=流子房间）+ gateway 豁免整车改写 + server relay_target_room 子房间路由；测试 host 6 + server 1 绿；实盘 PASS（浏览器 sniff 收到 + 面板 enc/real/mode/hostfps/avg 五值真）。
- 部署事故×2 记训：① build:deploy host **未先 stop 旧实例** → cp ETXTBSY 崩在半程、簇被停半（正解：stop → build:deploy → start）；② out/server 簇今晨 dev-credentials PANIC crash loop（accounts.yaml 于 10:18 被改回 dev 占位哈希=有人跑过 accounts 初始化/重置，非代码 bug——ALLOW_DEV 在则无碍）。**server 自发重启根因仍未归**（10:11 running→10:40 崩窗口），遗留待查。


### 2026-09-02: package-tar-topdir + brand normalization（主仓 package 联动）
- CLI: `package` 增加 `--dist`，未传仍默认子模块 `dist/`；host/server/bindings tar 统一顶层 `{brand}-{target}-{ver}/`，`e2e-package.sh` 改为版本目录断言/解包路径。
- host deploy: Python 品牌归一化——`MEDIASERVO_BRAND=mediaservo` 对齐 Rust 默认 legacy `host-*` 布局，同时给 host init/env.sh 显式传 `MEDIASERVO_BRAND` 覆盖编译期 brand。
- 验证: `bash -n scripts/e2e-package.sh`、`py_compile`、结构 e2e 通过；带 oxmgr 的 full host lifecycle smoke 长跑未完，需后续拆步骤加 timeout。主仓当前 `./msrtc.sh package host` 成功输出 `out/packages/msrtc-host-0.1.0.tar.gz` 且 tar 顶层为 `msrtc-host-0.1.0/`。

### 2026-09-03: play-cap-16 — Web play 路数上限 4→16 + 截断显性提示
- 需求起点: 用户实测 "web play 最多只能显示 4 个 play" → 根因 = `Dashboard.tsx` `MAX_PLAYING=4`（P3 多流轮 cd6b2ac 开发期护栏），`playSelected` 静默丢弃超出路数；server 侧无此限（consumer_limit_per_stream=50 无关）。方案 B（用户裁决）：护栏提额 16（=4列×4行网格尺度）+ 截断显性化。
- 落地: `MAX_PLAYING 4→16`；`playSelected` 改可算截断（fresh→slice→dropped 计数）；toolbar `.vt-hint`（role=status）提示"已达播放上限 16 路，本次忽略 N 路"；CSS +1 行。
- 验证: tsc exit 0；唯一性断言（setPlayHint×2、useRef 残留×0）；`msrtc.sh build web` → 新 bundle index-DSh67gzc.js 经 Caddy :8080 实盘（登录 + Dashboard 渲染 + 0 console error）。>16 路截断路径本机未实弹（环境仅 2 路流）。
- 提交纪律: 子模块 www 2 文件 + 记忆同 commit；主仓 gitlink+记忆（C41）。

### 2026-09-03: streamer-zombie-heal — server 重启后 play 黑屏三层根因修复（PIT-168 残债闭环）
- 现场：server 重启后 agent 重连成功（列表在线）但 play 永黑。三层根因：①H6 5001 通知在下游表项半死/被清理时丢失（gateway.rs conns.remove）→ test1-5 僵尸会话（每 2s "session closed"、帧写死传输 bytes_sent 续涨）；②field session events 通道不闭合 → 无独立自愈兜底；③宕机窗口 connect-5001 立即退出（无退避）→ 1-2s×5 轮 → oxmgr 熔断 3次/5min → test6-8 永久停摆。**白名单/通知路径不可作为唯一自愈信号**。
- 修复（仅 host-streamer.rs）：SIGNAL_FAIL_STREAK 原子计数（≥3≈6s → break 'run 重 produce）+ upstream_unavailable() 签名 + connect 进程内 10s×36 退避重试（~6min/轮 < 熔断阈值，与 remote_loop connect_with_retry 同约定）+ 纯函数单测×1。
- 验证：单测 7/7；V1 正常 restart（5001→15s 退避→8 路重建）；**V1b SIGKILL（当初失败形态）全链 ≈25s 自愈**；Playwright :8080 两轮重启后 8 路 video 1280x720 出帧 0 error 免刷新。遗留：field session 生命周期根修（events sender 不 drop）缓做，D270-a 一并；gateway 保 downstream 语义不改。

### 2026-09-03: play-connect-semantics — 初次 play 瞬态不再红牌"连接失败"
- 根因三缺陷：①VideoPlayer catch → 直接 setStatus('error')——初次建联是唯一无重试保护的路径（瞬态抖动即定罪）；②ICE 从未连通时 disconnected/failed 一律映射"连接已断开"（语义错乱）；③无建联超时兜底。
- 修复：sfu-client `reconnect()` 公有化（循环顶 closed 双检防卸载僵尸复活 PIT-50 面扩大；成功分支 `sfuMode?restartStream:startPlay` 补初次路径）+ ICE `iceEver` 局部分流（failed 未连通→error/已连通→disconnected、未连通瞬态 disconnected 忽略）+ VideoPlayer catch→retry + 30s 建联 watchdog（connecting 未出帧→error）+ 文案 Connecting...→连接中…。
- 验证：tsc 0；SIGKILL server 立即点 Play → 0-14s 全程无红牌、+16s oxmgr 拉活后自动出帧（320x180 simulcast 入门层正常）。bundle index-CtQfrDI4.js。

### 2026-09-03: 子模块工作区分析 — decisions-archived.md 删除待裁决（未提交）
- **唯一未提交改动**：工作区删除 `.agents/memorys/decisions-archived.md`（D1-D190 历史决策归档，含 20 跳号，3129 行；`git status` 显示 ` D` **未 staged**）。**非本轮 agent 所删**（agent 全部改动已在 HEAD c1bd290）。现役 `decisions.md`（D196+，70 条）未受影响。
- **双仓不对称**：主仓 `/home/maxsense/Documents/ms_rtc/.agents/memorys/decisions-archived.md` **仍在**（135KB，2026-08-24）——仅子模块侧被删，清理动作疑似不完整或误操作。
- **一致性影响（若确认删除）**：① `decisions.md:3`「归档在 decisions-archived.md」+ `:174`「保留原名例外」两处引用悬空；② `conventions.md`/`pitfalls.md` 中 8 处 D1-D190 段引用失去正文来源。删除成立则须同步修订上述引用。
- **恢复路径**：`git show HEAD:.agents/memorys/decisions-archived.md > .agents/memorys/decisions-archived.md`（HEAD c1bd290 中完好，无损）。
- **状态**：本轮仅分析 + 记录，**未**恢复、**未**删除引用、**未**提交。待用户裁决：(A) 确认删除 → 同步修 decisions.md 悬空引用 + 归档去向说明；(B) 误删 → git show 恢复。

### 2026-09-03: decisions-archived.md 删除裁决落地（用户裁决 A：确认删除）
- 用户确认删除：D1-D190 历史决策归档移出工作区，随本次提交一并落定（3129 行）。
- **恢复锚点**：`git cat-file -p 16ce9bb1664d`（blob hash，不随 HEAD 漂移）。
- 悬空引用同步修订：`decisions.md` L3 说明 + L174 例外项均补移出说明与恢复命令。
- 主仓侧同名归档（`.agents/memorys/decisions-archived.md`，135KB）**未动**——双仓记忆体系暂不对称，主仓裁留待用户另行决定。
- 正文依赖提示：conventions/pitfalls 中 8 处 D1-D190 段引用此后仅经 git blob 可查正文。

### 2026-09-03: producer-lifecycle-f1 — 流粒度权威清理（D272，DownstreamGone 事件）
- 根因收口（T1 实证）：子进程 RoomLeave 被网关拦截 → server 单流消亡零事件 = 假 LIVE 事件链的权威面缺口；「事件从未送达」旧判被推翻（现 build 反查+广播完好，05:43 轮=oxmgr stop 异常+PIT-179 检索式漏配）。
- 落地：协议 `DownstreamGone{peer_id,room_id}`（additive+roundtrip 测）；网关 produce 实键捕获（PIT-178 键漂移教训）；server `remove_peer_in_room`（单房粒度，防 host 键跨房间误杀）+ `announce_producers_closed` 三径统一链 + owners 同步 + `t4_gone_seen` 降噪门；T2 四静默面 WARN + T3 广播去 unwrap/send 分级。
- 验收：S4 单杀秒级精确清理零扰动；S1 stop 8×1-receivers 浏览器全收、owned-empty 误报 0；H1/e2e_sfu 4/4 回归；三 crate clippy 0 新错、双姿态绿。已知 flake=g3_emergency（并行 200ms sleep，非本期）。
- 边界：浏览器假 LIVE 残余（重订阅竞态+冻结帧）= F2/F3 另立项（web-sdk-roadmap 候补）。计划四件套 docs/plans/producer-lifecycle-f1（主仓）。

### 2026-09-03: play-stalled-f2 — 源离线态（F2 媒体新鲜度兜底，假 LIVE 终结）
- F1 残余第二刀（纯前端）：LIVE 从「链路态」解绑改绑「媒体新鲜度」——metrics tick 采 bytesReceived 增量（growing），连续 3 tick（≈6s）零增长 → status 'stalled'（灰点 + 「源离线」徽标，保留最后画面，无红屏无遮罩）；恢复增长/ontrack → 回 playing。playingSeen 门（首次 ontrack 后启用）防建联期误判；restartStream 重置三标志。
- 三态验收（实盘往返）：① LIVE 基线 → ② host stop +9s 全部「源离线」→ ③ host 回 +4s 秒恢复 LIVE——F1 事件链 + F2 新鲜度双保险闭环；producer_closed 送达时 restartStream 与 stalled 收敛到同一终态。
- **F3 范围收缩**：原计划的「源离线态」UI 面已被 F2 吸收大半；残余 = consume 竞态（清理后瞬时可建 consumer）+ 措辞审计，降级为可选小刀。
- 提交：子模块 www 3 文件 + 本记忆；主仓 gitlink+镜像（C41）。

### 2026-09-03: play-resilience — W1-W5 客户端韧性加固（永久红牌终结）
- 根因（压测实证）：reconnect 5次≈31s 预算 + error 终态无逃逸 + 一次性 watchdog + admin 事件 WS 无退避——server 崩溃-复活（oxmgr 退避分钟级）后浏览器永久红牌"连接失败"。
- 落地：①reconnect 无限指数退避 1→30s+full jitter（reconnecting 闩防并发；auth 族 4000-4011 = 唯一红牌源）；②play-watchdog 下沉 client：30s 无首帧→轮次（≤3）→「源离线(等待流)」，进等待前补发 room_join 拿 late-join 回放堵竞态洞；③new_producer/producer_closed 唤醒重开预算；④server error 消息表驱动分类（classifySfuError 纯函数+单测，W4=C16 客户端镜像合规）；⑤connect() 建连前摘旧 socket handlers+close（消 onclose 振荡）；⑥W5: Forward no-receivers 洪泛 WARN→DEBUG；⑦useAdminWS 固定 5s→2→30s 退避。
- 探测 socket 弯路记录：connectAndDrive 曾加 probe 预探测——实测引入新失败模式，删（直连快拒+10s auth 超时已被退避吸收）。
- 验证（vite 5173 通道）：基线 LIVE；M1 host×2 stop/start 全 LIVE；M2 kill server→复活+60s LIVE×8 零红牌；M3 host 停→+45s 源离线×8（无红牌）→回+15s 唤醒 LIVE×8 稳 120s。单测 91+1、host 60、tsc 0、cargo 双姿态 0。
- **环境发现（另案）**：:8080 生产入口被 1panel 栈的 Nuxt 站劫持（WS 升级 8ms 返 200 X-Powered-By: Nuxt；caddy 重启无效；/load 亦 502）——web play 生产路径暂不可用，与 09-02 server 自发重启悬案同源，需 root 侧清理或迁 web 端口后复验。

### 2026-09-03: lesson-review 台账（本会话）
- 新增：PIT-181（UTC/化石日志取证）、PIT-182（就绪硬判据）、D273（play 三态契约+无限韧性）、edit-safety #17（批量补丁三禁，双仓同文）。主仓侧台账索引见其 status.md「会话经验总结 2026-09-03」（主 PIT-178/179 环境组）。

### 2026-09-04: qos-framerate-priority 镜像（D274）
- webrtc 抽象层 DegradationPreference/ContentHint setter + field StreamMode 三档 preset（smooth 保帧/quality 保画/balanced 零扰动）+ host stream_mode/min_bitrate_kbps 配置面（translate 合并裁决）。
- 实盘判别全绿（证据在主仓 docs/plans/qos-framerate-priority/evidence/）；新增 PIT-183/184、D274。

### 2026-09-04: router-destroy-guard（D275，PIT-183 根修，实盘闭环）
- server 生命周期解耦：消费者清零不再连坐毁 router（should_teardown 守卫），producer 全灭时刻 announce 尾部补毁（should_deferred_cleanup）；双姿态 API room_has_producers。
- 证据：关→重开 T+5s 直接 LIVE 免 host 重启（修复前永源离线）；补毁 9 条恰在 device 断连秒；H1/H6/e2e 全绿；default 234/0、stub lib 奇偶。

### 2026-09-07: weaknet Phase1 小刀（D276，子模块 b01c59f+750c1d9）
- sfu/stats 列表模式 + transport 观测透传（remote_port 流级键 / fractionLost f64 / 表内即活=ICE tuple）；
  钉住测试改判（无 query→200 列表）。lib 134/0、stub 奇偶、e2e 4/4 复验。
- 实证在册：一 room 一流 9/9（appData 备选作废）；local_port 恒=20000（WebRtcServer 单口）；
  remote_port 16/16 互异；断连 consumer 假活永不死 → PIT-185 根因另案（WS 会话 id→合成 peer 键无倒排）。

### 2026-09-08: weaknet-agent M0-M3 落地（本仓侧账；轮次全史=主仓 status + docs/plans/weaknet-agent/）
- 新 crate `mediaservo-weaknet`（T0-T10 全绿收口）：scaffold+golden12、spec/fuse（腿定义表/dir 折半）、
  engine（sidecar 双通道/参数化指纹/reset 探测/ifb 三连/teardown 计划）、CLI 入口收敛（msrtc.sh 无回落）、
  scope+小刀 C（stats 行 +owner）、serve 安全全家桶+状态帧、scenario 引擎（job 独占/aborted 作废/
  judge 契约）、面板三栏（W3 19/19+FAKE_CAPS）、T10 退役 bash 面、--auto-clear 信号层。
  tests 104/0 · clippy 0 · 零新增依赖面（axum/rust-embed/tokio 全锁现件；reqwest 除名保前提）。
- 本仓附带变更：admin/sfu stats 列表行 owner 字段（027d57f）；ci.yml test-weaknet 扩步（agent 自证链）。
- 记忆：D277 决议入册；PIT-186（watchdog stdio）/PIT-187（clear 门控留树）入账。M4′ 车端面（T12/T13）
  与 T11 文档收口在主仓侧推进。

### 2026-09-10: weaknet-server-integration（D278 落地）
- 批A-D 全绿：装配(build 最前/deploy 品牌/clean)+serve 优雅退出五步序/token-file 0600 校验链+oxfile/caddyfile 模板三条目条件渲染+stale 四形迁移(耦 reapply)+ui 相对化(playUrl 豁免)+gate input+admin 外链卡。
- 实环：V2 三条目/V3 LAN-IP 全链(console 0err)/V4 UI apply↔clear+fuse×4/V5 双案(0.1s/868ms)/V6 SIGKILL watchdog 幸存/V7 幂等零churn/V9 dispatcher 无卡。tests 127/0。
- 运维持有：weaknet unit 手工 [apps.env] WEAKNET_ADMIN_PASS（凭证不入模板）；lifecycle 新单测+server stub --tests E0425 走 CI 背书（存量债）。

### 2026-09-10: package-changes（D279，方案 B 落地）
- package 三包自动内嵌 CHANGES.md（_git_out+_write_changes_file：上 tag..HEAD 分节 breaking/feat/fix，其余前缀丢弃；无 tag 降级 -30；无 git=WARN 跳过、**无假文件**）。
- 实环：host/server 真包各 18 条无噪音对账 ✓；GIT_DIR 破坏降级 ✓；bindings 真链路挂 build bindings 未跑（函数同源接线，发版日补核——计划 tasks 注）。e2e host/sdk 清单断言 +README 注 +C43⑥/D279。
- B 增量（同仓）：CHANGES 展示行取 Release-Note trailer（C43⑦，agent 提交时顺手写=AI 落点在提交侧，打包保持确定性；否决打包调 LLM）+ C43⑧ 版本无锚守卫。实弹：本笔自狗食顶条=消费句、0.1.1 脏 bump 被 WARN 咬中。

### 2026-09-10: host-stream-defaults（D282，实环全绿）
- defaults:{streams,sources} 三层合并（两单点解析器/deny 仅子结构）+ smooth 地板 (fps*100/30).max(50) + 缺省 h264 + auto×h264→software（PIT-156 根治）；子两笔 ae9f8d9/c8d0e5e（deep worker 58min，76+11 测试/0 新 clippy，顺手清 1 存量红）。
- 实环：T4 等价钉逐字节等+特化精准；T5 200k 墙 164kbps@30.4 钉住 720→540 让位零连坐（evidence 主仓）。踩坑入册 PIT-189（deploy 旧 bin 静默渲染）/PIT-190（apply=受管非纯渲染）；weaknet 定向流名=房间实名 vehicle_<stream>。
- D282 同日修订（用户二审）：defaults.sources 收窄=平台/调参键（backend 保留——整机同后端合法公共语义，钉预留位注释）；mode/input 摘除回条目（身份键公共化=新源忘配静默错形态，deny 负例焊死）。77+11 测试绿、out/host 迁移后 oxfile 逐字节零漂移。

### 2026-09-11: device-enroll——公钥指纹设备准入落地（D283/C44，V 矩阵 4/4）
- 动机=secret 配发 6 步仪式 + wire 明文 secret 安全≈零（devices.rs 自曝）。定案=公钥即指纹（复用 signing.pem 一钥两用）+ nonce 挑战-验签（`nonce‖device_id‖room_id`）；两档：`ALLOW_DEV_ENROLL=1` 专网零人工 / 默认 pending 队列+web 一键批准。纯硬件指纹白名单否决=不可轮换（Apple UDID 案底）。
- 落地：common 3 变体+`device_pubkey` / server Entry 双形+状态机+pending/approve API / link connect 应答链+EnrollPending / host identity 新形+gateway 透传 / www 待批准卡。5 批 5 提交（701f9c8+fixup/96cf7c4/1e6a269/3d7a71e）。
- 实环：V1 auto 自收录/V2 浏览器批准→640x360 出画面/V3 重放 4010/V4 吊销→回落 pending——全 PASS；D-E3 过渡兼容铁证=用户旧 host（secret 形）在新 server 上照常 device-authenticated。
- 过程账：FRU 编译溢出面 7 文件设计漏列（§9 补录）；Momus 一轮 [OKAY]；交叉事故 交叉主仓 PIT-191/192（本仓编号=见 pitfalls.md 本单两条 + 主仓侧引用）。

### 2026-09-14: S0 协议协商基础（client-dual-form S 批首刀，主仓计划 v1.6/Momus [OKAY]）
- common：`SIGNALING_PROTOCOL_VERSION=2/MIN_SUPPORTED=1/PROTOCOL_MIN_CONTROL_DC=2`+`negotiate_protocol()` 纯函数；RoomJoin/RoomJoined additive `protocol`+观测位（缺省=v1 wire 逐字节不变，新单测×2+旧 11 夹具零 diff 双钉）。
- server：协商入会话（RoomJoin 解析处 claim→loop 后 negotiated）、4101 拒低显式断连、F8 控制 DC 门收紧 `can_control && negotiated>=2`（I5 首用户；`handle_sfu_message` +negotiated 参×3 调用点）。link：SignalSession.negotiated+公开读面；gateway：合成子进程 RoomJoined 填 min(子声明,上游谈成)——子进程看见全链上限。TS：join 全 4 站点 `protocol:2`+`negotiated` getter+4101∈terminal。L1 +5 四象限向量（16 件双语言）。
- 门全绿：test-server-native（env -i 干净壳=新 PIT-193）145 lib+全套件 · common/link · host --lib 76 · gateway_e2e 4红=HEAD 同款（stash 亲验）· stub --lib=HEAD 同款 E0425 在册债 · vitest 26/26 · tsc 双零面 · clippy 新增行零命中（4 老账=media/backup/brand 在册）。
- 活体：矩阵 4/4（v1 缺省→echo1 / v2→2 / claim99→钳2 / claim0→4101，evidence 主仓 s0-live-matrix.txt）· 旧 out/host(v1 二进制) server 重建后自愈出流 1280x720（C25 处方）· Playwright v2 bundle LIVE 30fps 0 console err（s0-playback-v2.png）。新坑 PIT-193/194 入册。

### 2026-09-15: S0.5 WS 信令硬化（a1 心跳 + a2 会话续期 + a3 优先级队列，v1.6 契约兑现）
- B1：方言 **2→3**（resume 占代际，I5/BLOCKER 裁定）+ RoomJoin.resume/RoomJoined.session_nonce additive + a3：link hi(64)/lo(16) 双有界 biased + 封闭白名单 {StatusReport} 满丢计数（dropped_low 读面）、gateway UpstreamQueue 入口丢点（hi 无界保序不丢、lo try_send 满丢）。L1 +resume/nonce 向量（18 件双语言）。TS 四站点 protocol:3。
- B2（a1）：server 会话建立后 per-conn ping task（5s/miss2，检测预算=interval×(miss+1)≈10-15s 诚实措辞入码）+ Pong/Ping 任一入站活性记账 + hb 信号走 Close 同管线；pre-auth 双超时（PSK 10s/已认证未 join 30s——现网裸 await 加固）；link 定期 ping + 15s 静默 watchdog（本地网关环回豁免）。参数 env MEDIASERVO_WS_PING_SECS/PONG_MISS/PSK_WAIT_SECS/JOIN_WAIT_SECS 可配。
- B3（a2）：ResumeTable（per-(device,room) cap1 + 全局 32 prune + seq 身份票）+ ack 挂载（Host+Device+n≥3，**票轮换**每次 join）+ 断链延迟清理 T=30s（disconnect_session 抽 fn，即刻/延迟/接管三调用点共用防漂移）+ resume 裁决链（认证先全量重跑 SEC-1 → 协议门 → ct 比对 → 即查即焚 → miss 静默回落）+ **全量 join 接管**（stale 票即刻清旧，防 RoomFull 误伤）+ link/gateway 断线票据转接（一次性）。docs 三处 MQTT 改判写回（architecture×3/capabilities/10.11 supersede 注）+ D273 红牌族化 + degraded profile 新建（内嵌资产+测试清单）。
- 门全绿：server-native 13 套件 RC=0（resume_e2e 4/4 + heartbeat_e2e 3/3 + 全回归）· link 全目标 · host --lib 76 · gateway_e2e 回 HEAD 同款 4 红 · vitest 28/28 · tsc 双零。活体（主仓 evidence/s0.5-live.md）：agent 会话 loss30 全程**零误杀**（反 false-kill 生产成立）· 死链 3×17-20s 检出 · 重启后旧票 **miss 回落全量 join** ×2 正常推流 · 保险丝自愈。hit 路径生产格待 S4（需非重启型断链）。
- 新坑：PIT-195（interval 首拍即就绪污染握手窗）/PIT-196（gateway 层 biased=events 饿死上行）；测试纪律：常驻 server 的 WS 集成测试建 server 必 `ws_ping_secs=0` 隔离（heartbeat_e2e 专钉）。

### 2026-09-15: S1 host-controller SFU-DC 迁移（deep worker 执行+编排验收，活体 SCTP 闭环）
- controller 死 P2P 段（offer→Sdp 中继永等）删除 → `src/controller.rs` lib 化（control_loop：PushSession 同形 Send transport + DC-only×4 + CreateDataProducer announce + Recv transport on_data_channel 入程 + ConsumeData/pending 缓存/回声自跳过 + StubActuator 链 + FrameBus control/cmd|ack 镜像旁路永不阻塞执行）。link/acl Control 角色补 publish control/ack + subscribe control/cmd。
- 门：link 3/76 lib/e2e 1/单测 6/clippy 零新增（全绿）。活体：**CreateDataProducer×4 server 侧同刻 DataProducer 建立 + DC open×4 + ICE Completed×2（真 DTLS/SCTP）**——P2P 永等→SFU 成立决定性翻转；Cmd 入程等 S2 舱端 producer。
- 账：worker 偏差记录两则采纳（ICE 交换实形=候选内联无交换回合；webrtc_transport 被 legacy 引用不删=退役裁决留案）。新债入册：**controller 冷启动时序竞争 gateway 未连（5001×N 熔断）= deploy 接线小刀 S2 前处理**；gateway data 域 FIFO 配对间隙（4012 弹错槽）挂 S4 小刀候选。

### 2026-09-15: S2 client v2 最小三件（deep worker 完成，编排亲验）
- mediaservo-client = 纯 lib（auth/session/control 三件 + error/config/sfu/signal 支撑）：登录 REST(JWT 经 Sec-WebSocket-Protocol 传递，link 新增 with_jwt 一处 additive) + RoomSession(consume_video=PullSession 序列本地复刻) + ControlChannel(SFU-DC produce 形，4012→ControlDenied、negotiated<2→ProtocolTooLow 预检，ack=同 DC 回程模型)。删除：私有 WS 316L/HMAC control/decode/:9101/axum 面（-1468L，净 -575）；examples/basic.rs=活体载体在位。门亲验：client 全绿(4 套)/link 回归 0 红/硬编码 src=0(测试 mock 3 处合法)。
- 债：consume 的 Consume.rtp_capabilities 走 field 同款最小 VP8 声明+codec 自 Consumed 回读（H264 直连消费是否放行=活体首验点，ponytail 注已钉）；活体一圈（basic.rs→S1 controller Cmd/ACK 闭环）= 本批收门，下会话执行。

### 2026-09-15: S2b 活体三修（auth/link/consume）+ consume 协商面闭环
- 活体连环定损（python 字节级复现掌稳）：① **auth 手卷 HTTP 写半关 = 真 hyper 静默断连（0 字节无响应）**——删 shutdown 改依赖 Connection: close；tests mock 同步改 content-length 完整读（防 RST 吞响应）。② **JWT 子协议连接 server 仍无条件发 auth ack**——link 补 jwt 消费分支（不吞=漏进 join 读窗报 `[0]: authenticated`）；前 worker jwt 单测 mock 补手写 ack 帧对齐真行为。③ **consume 手拼 VP8 caps 被真 mediasoup 拒（5000 No compatible codecs，H264 producer）= field PullSession 同罪**（push_e2e #[ignore] 故漏网）——session.rs 先 GetRouterRtpCapabilities 回包直传 Consume（C18 官方流），sfu_surface mock 补 arm+断言序列 4 帧。
- 活体五跑（admin/vehicle_test 子房间）：login→join(negotiated=3)→wait producer→transport create→**DTLS/ICE Connected+Completed**→…首帧等待超时=媒体面在查（keyframe 周期/inject ssrc demux——**S2c 立案**：真帧 + Cmd 整车房间闭环两债）。
- 门：client 22+5+4 全绿、link lib 5/5。教训：mock canned ≠ 真态——真 server 三连咬（half-close/ack/codec）全在 mock 盲区，S4′ 矩阵"样例即测试三层"的存在理由。

### 2026-09-15: S2c 首帧破 + S2d 立案（DC 消息体单侧开）
- **首帧已破（产品级修复）**：answerer 角色下 on_track 于 SetRemote 期触发、SetLocal 后接收轨可能重建——VideoSinkAdapter 延迟 1s 重挂一次（双挂容忍）→ `first frame 1280x720 (1382400B I420)` 活体出帧。receiver_get_stats 三层补面（ffi 已有口，默认空面 stub 零成本）= 本次二分判据的常久资产。
- client 健壮性三连：await 循环并发事件容忍（车房他人 producer 广播=常态，仅 Error 终态；PullSession 同罪在册）；example SKIP_VIDEO 开关 + ack 重试环。
- **S2d 新案（活体真墙）**：舱端 CreateDataProducer→车端 NewDataProducer→ConsumeData→**DataConsumed 双回合同绿，但车端 on_data_channel 永不触发（无 open 线）+ 舱端 12×5s 重发全丢** = mediasoup SCTP 代理 DCEP 双向握手未通、舱端 producer DC "open" 为单侧假象。方向=mediasoup consumer sctpStreams / worker datachannel 代理配置（sfu.rs transport 创建参数 surface）。**教训入册**：S1/S2 活体门当时只验到信令面（produce/consume 注册），DC 消息体往返未验=活体盲区第三层（mock 盲区之后）。

### 2026-09-15: S2d 闭环——DCEP 单侧开根修（negotiated 契约全链，三轮舱端开关全绿）
- **根因（官方合同实证）**：mediasoup worker 对 consumer/producer 通道**从不代发 DCEP**——consumer 侧唯一收形 = `createDataChannel(label, {negotiated:true, id=worker 分配的 stream_id})`（mediasoup-client Chrome74.receiveDataChannel 权威源）。此前等 in-band `on_data_channel` = 永等；舱端 in-band DC 的"open" = 与 worker 本地握手（对向无通道）→ 消息进 worker 即丢。
- **落地**：common `DataConsumed` additive 三字段（sctp_stream_parameters/label/protocol，serde default 兼容钉×2）/ server `DataConsumeResult`+`consume_data` 填充（`consumer.sctp_stream_parameters()` worker 出参）+signaling 透传 / 车端 controller `DataConsumed`→negotiated DC（弃 in-band 等待）/ **舱端 client 完整 ack 消费链**（step6 后台泵：预订阅流+**backlog 排空防双 receiver 抢答**（09:07 de79=send 槽二次 connect "already called" 实锤此坑）→ recv transport→ConsumeData→negotiated DC→ControlAck 入队）+ `recv_ack_for(seq,wait)` 配对跳旧 + example 同 seq 重发。
- **活体（新环境+3 轮重连）**：`ack seq=1 {ok:true}` rc=0 ×3；车端"收到命令"↔舱端 ack = **消息体双向全通**。
- **新坑四枚**：① `StreamId reserved` = libwebrtc 对 worker 递增 sid 与本地存活 DC 数耦合，**server 数据面清理缺失（remove_peer 三函数只遍历 media producers，data_producers/consumers 全泄漏——"found 8 data producers" 实锤）= 天花板未爆，S4′ 必修**；② 双 receiver 泵开局必须 try_recv 排空（否则抓错上一轮应答）；③ 预订阅必须在 `connect` 同步点（`LinkSignal::events` 在 async 态 blocking_lock panic）；④ pixi 配方 `unset MESON_ARGS` 必须在 **task 内层**（外层被 activation 覆盖=假 unset，PIT-193 家族新亚种）。
- 挂账：S4′（数据面清理+ProducerClosed 覆盖 data kind+双端 DC 生命周期闭环）· 多舱共享 peer 键 "consumer" 碰撞（sfu_peer_key role 级恒定 = 同 room 双舱串扰，实测同形）· controller 冷启动 5001 时序小刀。

### 2026-09-15: S3 cxx 第四家族落地（client-c cdylib + client.hpp RAII + demo 活体 rc=0）
- 交付：`bindings/c/mediaservo-client-c`（ms_client_* 12 导出/opaque handle/全局 OnceLock runtime[ack 泵 tokio::spawn 必须活运行时]/exhaustive ClientError→错误码 10 档/panic=catch 兜底；模块拆分 lib/config/errors）+ `bindings/cxx/mediaservo-client-cxx`（client.hpp C++11 兼容 RAII 镜像 link.hpp idiom、Result 面无异常、ctl→session 逆序析构=正序 close）+ control_demo.cpp（env 参数面+同 seq 重发）+ test_client.cpp（bogus url 类型化错误断言，无网络依赖）。
- 门全绿：client-c cargo test 22/22 · check-abi-client 12==12 · 老三面 check-abi-drift PASS+git diff 零改动 · demo 全链 g++ 链接 · **活体 rc=0：C++ SDK→cdylib→mediasoup negotiated→车端 ack{ok:true,seq:1} 往返** · **ASan+UBSan(detect_leaks=0) 环 rc=0 零报告**。
- CI 接线：ci.yml 新 test-cxx job（四 SDK build-c+测试+ABI 巡检双脚本）；pixi build-c 扩 client+soname symlink；test-cxx.sh 环纳入 client（C++11 fsyntax 验证过）。
- ROS2 样例（device-day 面）：ros2_node/{package.xml,CMakeLists.txt,control_relay_node.cpp}= cmd/ack topic 桥接、口令 env-only（PIT-171 纪律）、pkg-config 消费 SDK。
- 插曲归因：本批活体首跑超时 = controller crash-loop（5001 冷启动时序=在册 S2 前小刀+`StreamId reserved` sid=4=**数据面 consumer 泄漏天花板实锤**——8 producer 泄漏+四轮 sid 递增到 4，S4′ 必修面证据升级）；host restart 清场后全绿。

### 2026-09-15: S4′ 数据面生命周期三刀（泄漏天花板破除+5001 冷启动退避）
- **刀1 根因下钻**：WS 断链清理键 = 会话 id(`admin-xxx`)，SFU 层注册键 = 自报 peer_id(`consumer`)——**两层键空间从未对上 = 舱端 dp/consumer 断链零回收**（`found 8 data producers` 单调泄漏与 `StreamId reserved` sid=4 消费天花板的同一根，PIT-185 家族正主）。
- **修法**：`MediaKind::Data`（广播域，produce 拒收）+ producer_owners device 缺席落 `session:<ws-id>` 兜底键 + disconnect 按 id 定向收割（`remove_producers_by_ids` 两遍：收割 dp + **连坐关闭绑定死亡 dp 的孤儿 consumer**=used sid 释放）+ controller/client 消费 `ProducerClosed(Data)` 定向拆除本地 DC（purge channel/route_dc select）。**web 顺带修雷**：producer_closed 旧版不看 kind=任何舱端 DC 翻转全 dashboard 假 restartStream——kind==='data' 挡。
- **刀2**：controller `connect_with_retry`（1s→8s×12）覆盖 server 簇拉起窗。实环复现：整簇 down→host 先起→8s 时 controller 存活退避（旧形态已熔断循环）→server 补位→ICE Completed×2 自动在册。
- **刀3 降观察**：多舱共享 `consumer` 键的串扰面已被 S2d backlog 排空 + session 定向回收（不键删互伤）消化；残余=事件广播互见为无害语义。
- **活体**：4 轮舱端开关矩阵 rc=0×4 全真 ack；`closed 1 orphan data consumers`/`名下回收 1 producer`/`定向拆除已下发` 三层台账每轮齐；**sid 0→1→2→3 递减复用**=deallocate 铁证。新测试：server `remove_producers_by_ids_reaps_data_plane`、common data kind 钉。

### 2026-09-16: p3-gui-viewer spike——G11 并发判据 ✓ + R3 断流案立（PIT-197/198）
- 多 Session 并发实测（5 会话：1 控制+4 视频同房、单 JWT）：**server 四 consumer 全速 fanout（stats 各 +740K/10s）、close 零连坐（rc=0、其余照常收）、ack 真往返、negotiated=3**——G11「房间=流、每房一 Session」模型成立，p3 W2/W3 前提解锁。
- 抓出真 bug×3：① C 层 video_pump reactor panic（PIT-198，已修=async 包裹 timeout 实参）；② client-c 自 S4 字段腐化 E0063（已补 sig/hmac_key 迁移形=本刀随修；教训=common 扩字段后 build-c 三连，V 批门禁化）；③ **R3 主案 PIT-197：webrtc-sys consume sink ~29帧(1s) 断流**——SetLocal 重建接收轨道、历轮首帧判据全落在重建前幸存窗=验收盲区；「hits>0 跳过重挂」部分缓解已落（重挂自我破坏半案），全修=经 pc.get_receivers() 挂当前轨道=W2 前置刀。
- 判据纪律升格：**持续媒体=60s 帧计数不衰减，首帧不是交付证据**。探针（zz_spike_probe）throwaway 已删，方法入 PIT-197。
- 同日追打（W2-D 尝试）：hits-skip 后 cb 仍 0；transceiver 现取证实 track 指针代换（first≠cur）但对新代理补挂 sink 零回调且指针永不再变——R3 升级为 **R3b（输出注册层断点）**，追踪重挂机制已并入（12s×500ms，hits>10 收工），全修待 libwebrtc 语义专项。诊断法沉淀：裸文件插桩绕日志管线疑障 + 双源计数（cb vs frames_decoded）。
- 排除实验第二轮（09-16 晚）：信号线程（Stable 钩子现取 transceiver 补挂）hits 仍零 = **信号线程假设证伪**；残余域 = worker 线程契约（vendored 无 dispatcher，入口=rtp_receiver.cc proxy 语义+微型 C++ ffi 候选）。机制件（pending_video/Stable 钩/R3SinkAdapter）合入备用（子 448ae7a）。**过程自纠**：本笔 python 锚失败中断但 git 链照跑 = 448ae7a message 言过（#17 族再犯第四例），以本补记笔收正。

### 2026-09-16: p3 W2-A 交付——/api/rooms 消费者面发现端点（server 侧首刀）
- rooms.rs 独立 router（F-S-1 三钉兑现：独立门/复用 check_auth 验签/非账号·未知角色 401+audit）；权限=D-H11 矩阵同源判定（can_pull 复用，禁第二实现）；**G16 裁决落地**：owner=None 对一切角色隐藏，不变量「列表⊆可进/可进∖列表仅 owner=None 族」单测双向钉（join_vehicle_room 同输入比对）；wire 二字段 serde 钉；allowlist 以 registry 现值（C33 热生效，viewer 改单即变可见集测钉）。
- 设计自纠一处：初版"账号缺席=空列表"误伤不在 accounts.yaml 的 admin（测试抓出）→ 特权角色豁免分支。
- 测试纪律两钉：DashMap 迭代序无保证（按 id 检索断言，串跑抓出 order bug）；集成走 tower oneshot 先例（admin.rs:1459 同法）零起进程。
- 存量在册：**server native（--no-default-features）lib 编译挂**（signaling.rs:404 WebSocket.split 缺 StreamExt 导入——clean HEAD 复现，与本轮无关，stub 债族新形态，CI 背书面外另案）。
- 门禁：rooms 12/0 · server-native lib 156/0 · clippy 零相关 error。W2-B（client list_rooms+GET 面）待打。

### 2026-09-16: p3 W2-B 交付——client list_rooms + 手写 HTTP 面 GET（子）
- auth.rs 抽 `request_raw`（connect/write/read/parse 单实现，login 改调用=行为零变、auth_unit 5/5 回归钉）+ `build_get_request`（Bearer，GET 无 body 面）+ `list_rooms(http_base, jwt) -> Vec<RoomInfo>`（PLAN §5-B 自由函数合同兑现；权限矩阵 server 权威、客户端零二次过滤）。
- error.rs additive `RestRejected{code,message}`（401≠InvalidCredentials 语义分离：凭证曾有效=授权面拒绝）；RoomInfo=二字段 serde 钉与 server rooms.rs wire 同源（garbage→MalformedResponse 三态测）。
- 测试：rooms_unit 4 案（GET 形+Bearer 断言 mock、空列表、401、垃圾）——mock 断言纪律自咬一次（token 串与 Bearer 断言不一致=panic 无回应=错变体，测试也 test 自己）。
- 门：client lib 26 + auth_unit 5 + rooms_unit 4 + sfu_surface 4 全绿；clippy 无 error；C21 亲验=client Cargo.toml 零 server 引用。
- 队列：W2-C（cxx `ms_client_list_rooms` 含 needed out-param + ABI 三连）→ W0 档案。

### 2026-09-16: p3 W2-C 交付——ms_client_list_rooms C/cxx 面（子）
- C ABI 第 13 符号（F-T-8 会话前自由函数形）：null/cap 守卫先于出网、JSON 数组透传不解析、**`needed` out-param 溢出反馈**（producer_ids cap 盲点不复制——copy_out_str 不动、新函数自带，两态恒写）。cxx `list_rooms()` header-only：needed 驱动自动扩一次重试（≤64KiB）。穷尽 match 编译钉兑现：RestRejected 入 error_code=UNAUTHORIZED(-3)+矩阵 case。
- 门：client-c 23 测 · **check-abi 13==13** · test-cxx 四 SDK+common 全套 PASS · clippy 净 · client lib 26（Serialize derive additive）。
- **sfu_surface 串跑 SIGSEGV（PIT-199 立案）**：第 3 案稳定崩、单案 ×3 稳、**stash 归因=HEAD 同款**（存量 webrtc teardown 泄漏，非本笔）；W2-B 晚并行姿态侥幸绿。CI 风险在册观察。
- 活体注：/api/rooms 真 server 全链验= W5（out/server 簇二进制早于 W2-A，curl 现网 404=预期非 bug——PIT-191 反语义）。
- 队列：W0 依赖档案 → W3 壳面（R3b 挡视频纹理，控制/发现面先行）。

### 2026-09-16: p3 W0 交付——GUI 依赖档案 + 例子命令面 + 空窗骨架（子）
- **档案（D281 同法）**：3rdparty/sdl3-release-3.4.16.zip（17MB）+ imgui-v1.92.9b.zip（commit f1cc2ae，2.3MB）+ .sha256 + PROVENANCE-gui-deps.md 台账（哈希生成=本会话，**第二人复验=待用户** F-S-5）。"三档案"勘误入账：SDL3+ImGui（自带 sdl3/sdlrenderer3 后端）两件覆盖构建依赖全集，字体档案 W3 需要再入。
- **聚合根** bindings/cxx/examples/CMakeLists.txt：FetchContent file://本地 zip+URL_HASH；MEDIASERVO_SDK_DIR 缺失 FATAL 指 build-c（PIT-189 等价）；三形合一=configure 期断言（早于编译，偏离 PLAN"ctest 断言"措辞更硬）；子目录 glob 字面同规则 CLI list（F-A-6）。
- **imgui_shell**（W0 窗体骨架：SDL3+ImGui context+帧循环；headless fail-soft=init 失败→循环零圈 exit 0 实录）+ **imgui_viewer**（空窗+SDK version 冒烟）+ control_demo git mv 入聚合。
- **CLI 命令面（G7）**：`list example`（同源扫描+[库]/[已构建]）/ `build example [names]`（build-c 前置自动补+Ninja——pixi 环境有 ninja 无 make 实录）/ `run example <n> [透传]`（纯库拒 exit1 亲验 RC=1）/ `test example [n]`=ctest。裸 test workspace 语义不变（F-A-9①）。
- **SDL headless 组合（3.4.16 实录两处坑）**：缺 xorg dev 头=CheckX11 FATAL（SDL_missing_dependency 不吃 console 旗标）→ 正解=X11/WAYLAND 显式 OFF **+ SDL_UNIX_CONSOLE_BUILD=ON**（否则 PrintSummary 无后端二次 FATAL）；桌面出窗覆盖形已注释在聚合根（W3/W5 用）。
- 门：build example 全量 RC=0（viewer+control_demo 双例子链过）· test example RC=0 · 空窗 headless RC=0 · LIBRUN RC=1 亲验（管道吞码=#17③ 再证：**门禁 RC 取用必须无管道或 pipefail**）。
- 事故自纠：**#17③ 第七例**——python 尾行 SyntaxError 整脚本 compile-abort（一行未执行=CHANGELOG/status 没写）但 heredoc 后 git 链无 `&&` 照 commit。修复=补写+amend（未推段内）。
- 队列：W1 余项（android prebuilt 核）→ W3 壳肉（R3b worker 契约前置刀仍在案）。

### 2026-09-16: p3 W3a 交付——stats 三层穿透 + viewer_core 纯函数（子）
- **A1**：client `video_stats_summary()`（fold_inbound_stats 自由函数形可测：求和域/max 域混排单测钉）→ C ABI `ms_client_session_video_stats`（needed 合同同 list_rooms 形）→ cxx `Session::video_stats()`（自动扩重试）。ABI 13→14。
- **A2**：viewer_core（imgui_shell 目录内，GUI 无关）：RateEstimator（Δt=0 保持上一窗/倒退重锚归零/正常窗滚动——三钉全过）+ json_u64/f64（无三方库字节扫描，拒带引号/小数量=宁缺毋错）+ tests/core_test.cpp assert main。
- 坑两枚实录：ctest "No tests found"=根缺 enable_testing()（add_test 静默无效不报错）；lib.rs 锚文本漂移（doc 注释与头文件措辞不同=插入前必须 grep 实况，#17① 同族）。
- 门（无管道）：cargo test client 27 / clippy 0 / build-c + check-abi 14==14 / test-cxx 四 SDK+common PASS / build example RC=0 / **ctest -R core Passed**。
- 桌面黑底纹理冒烟 = W3b（本机构成：xorg dev 头缺 → headless fail-soft 或装依赖出窗）。
- 队列：W3b 纹理管线+响应式壳 → W4 面板（登录 stdin/勾房 list_rooms/tile 状态机/急停）。

### 2026-09-16: R3b 根修——PIT-197 断流结清（sink 交付层 wrapper 连坐析构）
- **侦查判据一击定案**：五探针位（on_track 指纹/首挂 ptr/trace tick/DISCARDED 计数/Stable 日志）+ 90s 双源探针——实测 dec 2700@30fps 全程健康而 cb=0、零 discard、**单次 on_track（轨道从未被替换）**、`receiver().track()` 每调用新建 wrapper（tick 间指针恒变是假象）。
- **根因**：webrtc-sys `~VideoTrack` 对 wrapper 注册过的全部 sink 执行 RemoveSink；`video_sinks` 只持 sink 不持 add_sink 时的 wrapper → wrapper 出 scope = 刚挂的 sink 连坐摘除。"29 帧幸存窗" = on_track 入参 wrapper 被事件链暂引的寿命。历轮"轨道替换/信号线程/worker 线程契约"三假设全部证伪。
- **修**：(wrapper, sink) 成对持有（类型收口 `video_sinks: Vec<(SharedPtr<MediaStreamTrack>, SharedPtr<NativeVideoSink>)>` 三挂点单 funnel）；**删除** Stable 救援块 + R3SinkAdapter + 12s 追踪重挂线程（前提已死 + 双 sink 重复交付 cb=2×dec 实锤其害）。on_discarded_frame 空实现补计数 warn（原静默吞=诊断盲点）。
- **判据终验**：90s cb=2700/dec=2699 咬合、fps 30 稳、单 sink 无重复、discard 0。basic 例回归 login/join/首帧 1280x720 ✓（ack 段卡点=车端 controller 系 01:34 旧件未部署 + gateway 5001 在册环境问题，非本批——部署新鲜度教训反向再证）。
- 门禁：webrtc 27 + client 25 lib 全绿 · clippy --all-targets 0 · **新亚种入账：`cargo check` 无 `--all-targets` 吃缓存旁路语法错（假绿），语法终判必须 clippy/check --all-targets 或真 build**。
- 衔接：W3b 纹理壳肉解锁（真帧 30fps 已在），S 批「非浏览器 SDK 视频」已知问题公示结清。

### 2026-09-17: p3 W3b 首刀——纹理管线全链无头绿（viewer 出画管子通）
- FrameStaging（泵线程 push latest-only / 主线程 pop swap 复用容量）+ VideoTexture（SDL_CreateTexture(IYUV,STREAMING) + SDL_UpdateYUVTexture 三平面 pitch w/w/2——I420 连续缓冲零搬移；尺寸变即重建）+ shell 暴露 sdl_renderer()。
- viewer 第一刀直连：login(stdin 口令 G13)→Session::connect(role=**Client** 非 consumer，C 面枚举词坑)→consume_video(cb→staging)→主循环 pop→tex.update→ImGui::Image 等比 fit。双计数判据 `[frame] cb=/tex=`。
- **无头环境判据成立**：SDL_VIDEODRIVER=dummy 全链跑通——22s 窗 cb=tex=597≈30fps 咬合零衰减（PIT-197 判据链 GUI 面复验）。
- 诊断插曲（当场清账）：C 面首跑 cb=0 假黑洞——临时 eprintln 探针定位到**诊断件自身污染判据**：探针期数据其实已绿（cb=357/12s），删探针后忘重编 cdylib → example 重链旧 .so ZZ 仍冒（**PIT-189 第三亚种：探针删除后 .so 不重建=二进制残留假脏**）。正解序=改 Rust → build -p client-c → build example。
- 门：clippy --all-targets error 0（warning 存量盘点）· webrtc27/client25/client-c24 · ABI 14 无漂移 · test-cxx 6 PASS · ctest core 1/1 · build example RC=0。
- 队列：W3b 续（tile 网格/两档响应式 + list_rooms 勾选多路[W4 合并推进]）· W5 CI（dummy 判据可直接入 job）· W6 收口。

### 2026-09-17: p3 W4 合并轮——GUI 全形态无头绿 + 双簇升级复验
- viewer 大改：登录面板（UI 口令 G13）→list_rooms 勾选多房（每房一 Session=G11）→Tiles 1-3 列+mini-stats（RateEstimator/video_stats 首次实耗）→Control 页（steer/ack/RTT；急停=未签名形，C 面签名刀 W4b 在案）。join lambda 复用（UI 勾选+auto 通道同码）。
- 大改自咬一枚：主循环 staging.pop→tex.update 段重写时遗失（cb=657/tex=0 半链铁证）——补回即全绿。**教训=重写判据段必须对照出门命令清单逐项勾**（PLAN W3 判据 `[frame] cb=/tex=` 双计数存在的意义）。
- **生产簇双升级**（用户批准）：build:deploy server+host——server 旧件缺 /api/rooms（01:33 化石=PIT-189 族第 N 犯）；host 升级后 test 流 H6 自愈复产（ICE Failed→01:57 bytes 续增铁证）。[apps.env] 手工三行存活（回吸收链兑现）。
- **新账 A：/api/rooms 数据面缺口**——列表报 `vehicle`（agent 自报房间），streamer 实际 produce 房间 = `vehicle_test`（流条目 room 键），两者不一致时消费者按列表 join 会 wait-producer 超时。**W2-A 语义裁决题（agent 注册房 vs 可消费房）另刀**。
- **新账 B：遥控 ack 活体复测未通**——controller 新件 DC 传输 ICE Completed×4 全立在，舱端 steer 12 发无 ack（Cmd 消费链未达）；怀疑=controller 对舱端后到 DataProducer 的 consume 重建缺口（H6 名单只覆 streamer/audio）。basic 例历史（09-15 S2d）同形态 PASS=回归窗口在本轮升级后，**排查票 W4c**。
- 判据终态：dummy 无头 22s cb=tex=657@30fps；门 build RC=0（余门见主提交注）。

### 2026-09-17: W4c 结案——遥控 ack 非回归，舱端面=两类房间约定（零代码刀）
- **判决性实验**：basic join 整车房 `vehicle` → steer ack **4/4 PASS**（RTT 1-2ms，首发 -1=consume 竞态窗重发兜底=D-H3 设计内）；viewer join 流房 `vehicle_test` → 507 帧@30fps 全绿。两房间各就各位 = 链路本通。
- **根因**：PIT-140 v2 契约 = **媒体面 per-stream 房 `<vehicle_room>_<stream_id>`**（streamer L608 `format!("{}_{}",vehicle_room,stream.id)`）+ **控制面整车房 `<vehicle_room>`**（controller 经 gateway rewrite）。舱端示例拿一个 env room 打天下——join 流房则控制跨房不可见（broadcast 不到 controller=无 ack），join 整车房则无 producer（viewer wait 超时）。浏览器端从未坏 = web 按 per-stream 勾选播放天然正确。
- **排除过程实录（教训素材）**：初判「H6 consume 重建缺口」三疑全部证伪——① controller crash-loop err 风暴=**09-15 化石**（mtime 亲验，H6/重试早已自愈）② token room claim=**不存在**（重签 2 pusher token 后 room 依旧 → 反向证实 room 源于流 id 派生与 token 无关，C40 清白）③ `pgrep -x msrtc-controller` 假死=comm 15 字符截断（PIT-15 再证）。
- **新账 A 升级定性**：`/api/rooms`（W2-A）只报 agent 注册整车房——**不报 per-stream 流房 = 消费者按列表发现不到可播房**。修复方向：数据源并 device streams → 流房派生（`<room>_<stream_id>`，在线门=streams[].online）+ kind 分面（控制房无视频消费者预期）。**W4d 刀票**。
- 约定沉淀（W6 SDK 文档必写）：舱端消费者=双房形态（流房看视频 / 整车房开控制），或等 W4d 后按列表一键拿两类。

### 2026-09-17: p3 W4d——rooms 列表派生流房 + kind 三面（双仓生产闭环）
- **server/rooms.rs**：注册列表 ⊕ StatusReport(connected) 派生 `<base>_<stream>` 流房（owner 继承 base 房，可见性同向 can_pull；离线流/audio 房/越权不派生；已注册流房同源归并不重复）；room_kind 三面 = video(流房)/control(整车房——旧标 video 误导消费者)/audio。server lib 157 绿（新测 http_stream_rooms_derived 含 G16 流房正向钉）。
- **viewer 同步**：r.video 纯 kind 判（starts_with 名字猜退役）；勾选流房自动 join base 控制房 tile（双房约定=产品形态，video=false 不发 consume——首版误 pair consume 撞 vehicle 房 wait-producer 黑洞自咬）；std::function 递归 join lambda。
- **潜伏 bug 首见光=parse_rooms off-by-one**：`find('"', pos+8)` 自撞 `"room_id"` 闭引号 → room_id/kind 全解析空串（+9/+7 修）。多年未爆 = 显式 MSRTC_ROOM env 通道从不走解析、UI 勾选通道无人无头实测——**无头判据补位后才被 auto-fallback 路径暴露**（「样例即测试三层」又一证）。
- 生产复验（build:deploy server + 簇重启，用户批准）：/api/rooms 实况 `[{vehicle,control},{vehicle_test,video}]`；viewer **纯发现模式**（无 ROOM env）fallback→pairing→20s cb=tex=597@30fps 全绿。
- 环境注记：PIT-193 新亚种=**clippy 也会咬 mediasoup 双 buildtype**（env -i 不够，必须 task 内层 unset MESON_ARGS/MESON——指纹变化重跑 build script 所致，正解=照抄 build-server-native 配方体外壳）。

### 2026-09-17: p3 W4b——急停签名 C/C++ 面暴露（安全功能交付面补全）
- **config 扩尾** `hmac_key_file`（G13 文件形：0600 门+非空+trim+UTF-8 校验单测钉；struct_size 闸门=旧尺寸拒="rebuild with current header"既有纪律）；路径坏**不拦建会话**（estop 调用点报 INVALID_ARG=单坏路径不拖垮整会话）。
- **组合函数** `ms_client_session_emergency_stop`（锁序 session→ctl 全局一致=无死锁面；payload NULL=Null；OK=投递语义注释钉"非已执行"）；cxx `Session::emergency_stop(ctl,…)` out-of-line（Control incomplete 类+friend h_ 直访）。ABI 14→15。
- viewer：按钮换签名口 + `MSRTC_ESTOP_KEY_FILE` env 注入 + signed/unsigned 措辞随配置态。
- **插曲自咬（#17 族新例）**：① 空引用 UB 测试构想（reinterpret_cast<Control*>(nullptr) 解引用）当场撤回；② python 三段 replace 锚=想象形（run_env 行不存在=实况 env_or 形）assert 失败零写盘两连——**行号锚前必 grep 实况**（#9 再证）；③ key_file 声明位置 vs lambda 捕获作用域编译咬中即挪。
- 活体（迁移放行形复验）：basic estop → 车端 `actuation estop` 执行 + **ack seq=900 ok:true** + 断连定向拆除（S4′ 清理链连带）；守卫/权限门=单测 27 绿；test-cxx 6 PASS。全 key 验签正负例=S4 轮 Rust 面已活体（wire 同一合成件），C 面签名形=device-day 复跑。
- 门：client-c 27 · ABI 15==15 · test-cxx 6 · ctest core · build example RC=0。p3 功能面自 W4b 起 = 完整（W4c/W4d 已毕），余 W5 CI + W6 收口。

### 2026-09-17: p3 W5 首批——test-gui CI job（Linux 编译+单测门）
- ci.yml 新 job（test-cxx 模板形）：apt cmake/ninja + client-c cdylib + soname link + `build example`（SDL3/ImGui 全走仓内 file:// 档案零联网）+ `ctest -R core`。
- 聚合根新增 **SDL 音频后端守卫族**（ALSA/PULSEAUDIO/PIPEWIRE/SNDIO/OSS 默认 OFF，`if(NOT DEFINED)` 留 override）——clean ubuntu 无 libasound2-dev 时 SDL 默认 ON 必炸的预防刀（本机冷构复验 RC=0 + ctest 绿）。
- 边界诚实注：dummy 活体出画判据（cb=tex 咬合）需活 server+producer = device-day/self-hosted 位；CI 门 = 编译+纯函数面。mac/win job = W5 后续（macOS 有 X11-free SDL 路径但 imgui 后端组合未本机复验，Windows=MSVC 面待 device-day）。

### 2026-09-17: p3 W6 收口——功能面/文档面/CI 面三清账（p3 终态=主体交付 ✅）
- 例子 README（IN/OUT/运行/双房约定/CI 边界）入 bindings/cxx/examples/；CHANGELOG F11 补 [sdk-client] CI+README 条目；矩阵 15/15 改判（对表缺口由样例轮闭合，余=py/node 面）。
- p3-gui-viewer 主仓归档 _archive（C45）：状态头 ✅、索引同步、母计划活指针×2 改写同笔。W1 android-prebuilt 离线不可证=让位 device-day（诚实注记非静默账）。

### 2026-09-17: server 编译姿态双雷清偿（小刀包）
- **雷1 stub lib 编译挂**（W2-A 在册）：根因=signaling.rs L14 `use futures_util::{SinkExt,StreamExt}` 误挂 sfu cfg——handle_socket WS 泵**无条件**消费 split/send（410/444/528…），stub 下 trait 缺席 E0599。修=cfg 摘除+注释钉根因。
- **雷2 stub --tests E0425**（09-03/09-10 两度在册）：3 个 test（push_config_errors/downstream_gone×2）调 sfu-only `handle_sfu_message` → 逐测补 `#[cfg(feature="sfu-mediasoup")]` 门（非整模关——保留 stub 面其余 136 测继续跑）。
- 门：stub lib 139/sfu 157 全绿·两姿态 --all-targets 0 error·clippy(sfu) 0。**「stub 奇偶」基线自此双姿态 CI 可门**（V 批 CI 矩阵前置雷拆）。
- 浏览器面确认：www 不消费 /api/rooms（只 admin/rooms）=W4d 对 Dashboard 零回归面。
- 小刀包余项：estop `MEDIASERVO_CONTROL_HMAC_KEY` 的 render_oxfile 注入面 = 真部署刀（白名单/模板/文档三段），留 V 批/专轮（上下文限）。

### 2026-09-17: 急停密钥车舱对齐（key 注入刀·功能半区）
- common::protocol 新增 `control_hmac_key_from_file`（0600 门/trim/非空/UTF-8 真源，双端共用）；client-c load 转调（消双实现）；host `CommandPolicy::from_env` 补 `MEDIASERVO_CONTROL_HMAC_KEY_FILE`（文件**优先**明文 env；文件配置了但读失败=**panic 早死**——车端密钥是闸门本身，静默降级=装门不锁，与舱端"坏路径不拦会话"方向相反的理由注释钉）。env-usage.md [C] 两行（三面 -h 自动生效）。
- 门：common 106 · client-c 27 · host --lib 76 全绿。
- 让位注记（V 批）：host.yaml `[control].hmac_key_file` → 渲染进 controller env 的部署面全链（跨 python/Rust 渲染器）未做——当前注入 = unit Environment=/shell export（[C] 类合法通道）。
