# SDP 协商审计:Host 构建方式 + 码率链路（2026-08-12）

> 关联: [`mediasoup-client.md`](mediasoup-client.md)（官方客户端架构对照）、[`webrtc-w3c-alignment.md`](webrtc-w3c-alignment.md)（D214/D216 重构）、`conventions.md` C18（官方用法优先）、PIT-65（手工协商教训）
> 结论采纳: **方案 A+C**（补全自构 offer 的 transport-cc/rtcp-fb + bitrate_kbps 语义修复）→ 实施计划见 `docs/plans/sfu-negotiation-completion/plan.md`

## 1. 结论摘要

1. **协商 API 已全部标准**（PIT-65 整改完成）: `setRemoteDescription → add_track → createAnswer → setLocalDescription`（SFU 路径）/ `createOffer → setLocalDescription`（P2P 路径），produce 参数从 `get_sending_rtp_parameters` 官方路径推导。
2. **remote SDP 文本仍由 host 手工拼接**（`sfu_media::build_remote_sdp`）——这是 mediasoup 架构的必然（server 从不产出 SDP，信令只传 `iceParameters/dtlsParameters/iceCandidates` JSON），官方 SDK（libmediasoupclient `RemoteSdp`）同样自构 SDP，只是**自构的是 answer 而非对端 offer**（client-offer 模式）。
3. **与官方的真实差距 = 自构 offer 内容简化**（单 codec、无 rtcp-fb、无 extmap、add_track 非 W3C 签名），而非"协商语义错误"。
4. **码率设置链路存在系统性断点**: 自构 offer 无 extmap → transport-cc 未协商 → BWE 反馈断裂 → `max_bitrate` 达不到目标、`min_bitrate` 从 best-effort 退化为实际固定码率；`bitrate_kbps`（默认 2000）只进 GStreamer 捕获管线，对 WebRTC 编码器零影响（死参数）。

## 2. Host SDP 构建方式（当前实现, main.rs:289-429）

```
① 自构对端 offer:  sfu_media::build_remote_sdp()  ← 手工字符串拼接（模拟 server ICE-Lite offer）
② set_remote_description(offer)                    ← 标准 W3C API
③ add_track("video")                               ← 非 W3C 签名（无 track/无 sendEncodings）
④ create_answer() + set_local_description(answer)  ← 标准 W3C API（answer 由 libwebrtc 生成）
⑤ produce 参数: get_sending_rtp_parameters()       ← 官方推导路径，非手工
   → build_produce_rtp_parameters_from_rtp()       ← 仅 JSON 序列化适配
```

自构 remote offer 内容（`sfu_media.rs:29-86`）:

| 元素 | 值 | 说明 |
|------|-----|------|
| 会话级 | `a=ice-lite` / `a=setup:actpass` / `a=group:BUNDLE video` | 对齐 mediasoup WebRtcTransport（ICE-Lite, DTLS role auto） |
| 方向 | `a=recvonly` | server（offerer）为接收方, 正确 |
| codec | 单 codec rtpmap/fmtp（h264=101/vp8=96/vp9=99/av1=97） | 由 `config.encoder.codec` 定; auto=VP8 |
| candidates | 信令 `iceCandidates` 塞进 media section（m= 行之后） | PIT-48: 会话级 candidate 被 libwebrtc 忽略 |
| **缺失** | **`a=rtcp-fb`（nack/pli/fir）** | ↓ 第 5 节影响 |
| **缺失** | **`a=extmap`（transport-cc/abs-capture-time）** | ↓ 第 5 节影响 |

## 3. 官方对照（mediasoup-client.md §2/§4）

| # | 官方（libmediasoupclient/mediasoup-client JS） | MediaServo Host 现状 |
|---|------|------|
| 角色 | client = **offerer**: `addTransceiver(sendonly) → createOffer → setLocalDescription(offer)` | client = **answerer**: 自构对端 offer → `setRemoteDescription` |
| 自构侧 | **answer 由 SDK 自构**（`RemoteSdp::Send()`, RemoteSdp.cpp:157-185）→ `setRemoteDescription(answer)` | **offer 由 app 层自构**（`build_remote_sdp`）→ 协商, answer 由 libwebrtc 生成 |
| 参数来源 | ssrc/PT/fmtp 从 offer 推导（`getSendingRtpParameters`） | `get_sending_rtp_parameters()`（同性质, 官方路径） |
| server | 只收 rtpParameters, **从不返回 SDP** | 同（`signaling.rs` 无任何 sdp 代码, 实证） |

被否定的旧方向（PIT-65/§4 表格 5 行）: 绕过协商（角色反转、手工 produce 参数硬编码、手工解析 answer ssrc）——**已全部消除**。

## 4. 遗留差距（自构对端 offer vs createOffer 生成 offer）

| 差距 | 影响 | 严重度 | 状态 |
|------|------|--------|:----:|
| 自构 offer **无 `a=rtcp-fb`**（nack/pli/fir） | NACK 重传、PLI 反馈可能未协商; 丢包网络质量下降 | 中 | 计划 T1 |
| 自构 offer **无 extmap**（transport-cc/abs-capture-time） | **BWE 反馈断裂**（第 5 节核心）; 浏览器端延迟统计缺失 | **高** | 计划 T1 |
| **单 codec offer**（config 定死一个） | router 5-codec 能力一次只协商一个; auto=VP8 | 低 | 接受现状 |
| `add_track("video")` 非 W3C 签名（§4 遗留） | 无 sendEncodings → 无 simulcast/多 encoding | 低 | 接受现状 |

## 5. 码率链路全景（2026-08-12 分析）

```
host.conf encoder.bitrate_kbps=2000 ──→ Pipeline::new (GStreamer 捕获管线, pipeline.rs:81)
                                    └── SFU 主路径 (test_pattern→WebRtcTrackSink→libwebrtc 编码)
                                          → bitrate_kbps 是死参数（误导, 计划 T3）
host.conf encoder.min/max_bitrate_kbps ──→ main.rs:361 set_encoding_bitrate(min_bps, max_bps)
  → webrtc_sys.rs:669 SetParameters(enc.min/max_bitrate_bps) → libwebrtc ReconfigureEncoder
  → produce rtpParameters.encodings[0].maxBitrate（build_produce 反射, sfu_media.rs:124）
  → mediasoup RtpEncodingParameters.max_bitrate（mediasoup-0.24.1 rtp_parameters.rs:117/158 正确解析存储）
```

各环节判定:

| 环节 | 状态 | 说明 |
|------|:----:|------|
| 配置校验 | ✅ | `validate_bitrate()`（main.rs:76, config.rs:184）: min>0、min<max, 防 libwebrtc 双失效 |
| 调用时序 | ✅ | 协商后、首帧前（main.rs:361）, ReconfigureEncoder 生效前, 只调一次 |
| libwebrtc 语义 | ✅ | max=硬上限（可靠）; min=分配层下限（BWE 低于时编码器保底, best-effort） |
| Server 端 | ✅ | mediasoup 是**接收方**, maxBitrate 仅存储/供 consumer 协商（rtp_parameters.rs:351/380）, 无 `maxIncomingBitrate` → 不会卡码率 |
| **BWE 反馈链路** | ❌ | transport-cc/REMB 均未协商（下节） |

## 6. BWE 反馈链路断裂（核心问题）

```
自构 offer 无 extmap → answer 无 transport-cc 扩展（RFC 8285: answer 只能收 offer 集合）
  → RTP 包无 transport-cc 头扩展
  → produce headerExtensions=[]（sfu_media.rs:131 硬编码）
  → mediasoup 无 transport-cc 上下文, 不生成/转发 feedback 给 host
  → libwebrtc BWE 无任何输入（transport-cc + REMB 都没有, rtcp-fb 也未协商）
  → 编码目标 = clamp(初始估计, min, max) —— 初始估计默认 ~300kbps（需实证 webrtc-sys 默认值）
```

后果:

1. **max_bitrate=2000k 形同虚设**——只做上限截断, BWE 停在初始值 → 实际码率 ≈300k~1M, 达不到目标
2. **min_bitrate 从 best-effort 退化**——无 feedback 时 BWE 不降也不升, min 成为编码器保底的实际固定值; 弱网下 min 过高 = 恒定高码率拥塞且无法自适应
3. **bitrate_kbps=2000（默认）对 WebRTC 编码器零影响**——默认配置（min/max 均 None）下 libwebrtc 无任何码率提示, 实际码率无保障且不可控
4. **实证缺口**: 此前 stats 面板验证了 encoder_implementation/fps/分辨率, 未记录实际码率数值——断裂与否需实测确认（浏览器 consume 侧 bytesReceived 增量）

## 7. 方案对比与决策（A+C 采纳）

| 方案 | 内容 | 优点 | 缺点 | 决策 |
|------|------|------|------|:----:|
| **A. 补全自构 offer** | `build_remote_sdp` 加 `a=extmap`（transport-cc/abs-capture-time）+ `a=rtcp-fb`（nack/pli）; produce headerExtensions 从协商结果推导 | 最小改动（一个函数）; 同时修复 PLI/NACK 与 BWE; mediasoup 侧自动获得 transport-cc 上下文 | 仍非官方 client-offer 模式 | **采纳** |
| **B. 迁 client-offer 官方模式** | `addTransceiver(sendonly)→createOffer→自构 answer` | createOffer 天然带完整 extmap/rtcp-fb, 一劳永逸; 与 libmediasoupclient 完全对齐 | 改动大（host+浏览器两端+自构 answer 下沉 SDK）; 全量回归 | 后续纯正度项 |
| **C. 修 bitrate_kbps 语义** | bitrate_kbps 改为 min/max 默认值来源或标注 deprecated; host.conf 注释更新 | 消除误导 | 不解决 BWE 断链 | **采纳** |

## 8. 相关文档

- `mediasoup-client.md` — 官方客户端架构（§2 协商流程 / §3.2 自构 answer / §4 对照表）
- `webrtc-w3c-alignment.md` — D214/D216 重构分析
- `keyframe-black-screen-analysis.md` / `gop-control-internal-encoder.md` — PIT-65 关键帧根因（PLI 链路背景）
- `.agents/memorys/conventions.md` C18 — 官方用法优先
- `docs/plans/sfu-negotiation-completion/plan.md` — A+C 实施计划
