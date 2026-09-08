# WebRTC 弱网模拟工具调研（Network Emulation Landscape）

> 类型：调研存档（Explanation）。核对时点 = 2026-09-08（stars/archived/pushed 均经 GitHub API 实测）。
> 底稿 = 团队提供的工具清单，本档逐条核证修正（修正处以 **⚠** 标记）。
> 本项目落地形态 = 主仓 ms_rtc `scripts/weaknet.sh`（tc/netem + docker sidecar），
> 使用手册 = 主仓 `docs/modules/development/weaknet-harness.md`，纪律 = skill `weaknet-testing`。

## 1. 系统级 / 通用工具

| 工具 | 平台 | 能力 | 优点 | 缺点 | 现状（实测 2026-09） |
|------|------|------|------|------|---------------------|
| **tc + netem**（iproute2） | Linux 内核 | delay/jitter/loss(含 gemodel 突发)/reorder/duplicate/corrupt/limit；带宽需 htb/tbf 组合 | 内核原生、逐端口(u32)精确、所有高级工具的底层 | 纯 CLI；**GSO 吞丢包粒度**（`ethtool -K gso off`）；root 或容器 NET_ADMIN | **基石**，随内核演进 |
| Clumsy | Windows | 丢包/延迟/节流/复制/乱序/篡改 | 免安装、GUI、系统级任意 UDP/TCP | 仅 Windows（WinDivert） | 活跃，6.2k★（2025-11 有推） |
| Network Link Conditioner | macOS/iOS | 带宽/延迟/丢包/DNS | Apple 官方、预设场景 | Apple 生态限定 | 官方维护（无 GitHub 计数） |
| Comcast | Linux/macOS | 对 tc/ipfw 的轻封装 | 简单 | 功能基础；本质仍是 tc | **10.5k★ 但低频**（2025-03 尾推）——「作者持续更新」说法偏乐观 **⚠** |
| Wondershaper | Linux | htb 带宽整形（+netem 可选） | 一行限速 | 无精细损伤面 | 稳定/低维 |

## 2. 专用 / 高级工具

| 工具 | 定位 | 优点 | 缺点 | 现状（实测） |
|------|------|------|------|-------------|
| **ATC (Augmented Traffic Control)** | Facebook 网关 + Web UI | 免被控端安装、团队协作 | **仓库已归档**（facebookarchive，2018-04 停更）——「FB 持续维护」不成立 **⚠** | 存档 |
| **Toxiproxy** | 混沌代理（REST API） | 12.3k★ 活跃、自动化集成标杆 | **TCP-only，UDP 不可用**（issue #54 长挂）→ WebRTC 媒体流不适用；只可打信令 WS | 活跃但选错场景 |
| WANem | LiveCD/VM 广域网模拟 | 高仿真 WAN | 部署重；2026 视角技术栈陈旧 | 基本停摆 |
| QNET | 腾讯移动弱网 App | 免 root、2G/3G/地铁预设 | 仅移动端、闭源、需登录 | 移动端活跃（无公开仓库） |
| ToNetScale | Docker 网络拓扑仿真 | tc 的容器编排壳 | **GitHub 搜索已不可得（2026-09 复核 404/删除）** **⚠**（前轮调研曾引用） | 消亡 |
| **udptoxy** | Rust UDP 中继 + **TUI 实时拨盘**（l/j/p/r 键） | 免 root、逐端口双向、时延/抖动/丢包/限速 | 7★ 社区件（loss 真实性需自标定）；中继形态只罩被引流的流 | 活跃（2026-08）；本项目列为 roadmap F3 |

## 3. 代理 / 开发辅助（对 WebRTC 媒体的适用性 = 否，防误区）

| 工具 | 结论 |
|------|------|
| Charles / Fiddler | HTTP(S) 层代理，UDP 媒体不经它——**不适用** |
| Chrome/Firefox DevTools 网络节流 | 作用于 URLLoader（HTTP），**不影响 WebRTC UDP 媒体**——常见误区，别用它"调弱网"后困惑画面毫无变化 |
| toxiproxy-dashboard 等 Web UI | 随 Toxiproxy，同受 TCP-only 限制 |

## 4. 代码内仿真（自动化测试的正解）

| 设施 | 适用 | 要点 |
|------|------|------|
| **webrtc.org `test/network/`**（NetworkEmulationManager / emulated_network / schedulable_network_behavior） | C++ 集成测试/性能测试 | 进程内逐包仿真（时延/丢包/带宽/交叉流量/调度突变）；无法用于真部署链路 |
| **Pion vnet**（`pion/transport/vnet`） | Go 单测 | 虚拟 NIC/链路拓扑，丢包乱序时延可编程；pion 系生态标配 |
| mediasoup 侧 | Node/Rust 面 | `WorkerSettings.libwebrtcFieldTrials`（BWE 行为注入）；transport `setMax{In,Out}goingBitrate/setMinOutgoingBitrate`；stats `rtpPacketLossSent/Received`(fractionLost f64)；**mediasoup-demo 的 applyNetworkThrottle = @sitespeed.io/throttle 包装（tc 的 CLI 皮肤），其 localhost 模式只有 delay 没有 loss/rate**；上游自身 CI 零 netem（RTCP 逻辑走合成输入单测） |
| aiortc / 浏览器 getStats | 观测面（非仿真） | packetsLost/jitter/availableIncomingBitrate；注意 **OOO 扣减**（NACK 重传假象）与 SVC 降层对 fractionLost 的污染 |

## 5. tc 编排层（写脚本/CI 时的可选项，实测活跃度）

| 工具 | 星 | 活跃度 | 一句话 |
|------|----|--------|--------|
| pumba | 3.1k | ✅ 2026-09 | docker chaos CLI（netem/htb 进容器执行），fail-closed 守卫与 replace 语义的教科书 |
| tcconfig | 853 | ✅ 2026-04 | pip 装的文件级 tc 编排（htb+netem+ifb 自动拼装） |
| docker-tc | — | ❌ ~2020 | sidecar 双向（ifb）先例，机制可抄、件本身停更 |
| quic-network-simulator | — | ✅ IETF interop 在用 | per-link 容器 + 配置自描述回显——scenario 形态的成熟范本 |
| sitespeed.io throttle | — | ✅ 活跃 | half-RTT 入参口径 + 外因 qdisc「attach-and-leave」纪律 + **change 不清 rate 属性需哨兵重放**（netem_impair 注释实证） |
| Mahimahi | 280 | ✅ 2025 | record/replay 链路仿真（学术拥塞研究向），UDP 实时非所长 |
| 本项目 weaknet.sh | — | ✅ | tc 直用 + sidecar 免 sudo + 房间级定向 + 基线配对判据 + 三层保险丝（dev 面全家桶，见主仓文档） |

## 6. 硬件损伤仪

HoloWAN / Spirent / Apposite Netropy 等 = 真实链路仿真天花板（RF 建模、地理时延），采购重；dev 阶段用 netem 族 + 真机（Jetson/手机+adb 网络整形）已覆盖需求，硬件留给验收/认证场景。

## 7. 选型速查（含本项目映射）

| 场景 | 首选 | 本项目落点 |
|------|------|-----------|
| Linux dev 单机（本项目主形态） | tc/netem 直用（root 或 docker NET_ADMIN sidecar） | ✅ 已交付：`msrtc.sh weaknet apply --profile remote-burst --stream <房间名>` |
| 车端 Jetson（有 root） | tc 打在设备网卡（wlan/eth，egress=上行、ingress→ifb=下行） | roadmap F1（scp 即用执行层，触发=Jetson 复测排期） |
| 移动端/Web 端演示要「拨盘手感」 | clumsy(Win)/NLC(mac) 现成；Linux/无 root → udptoxy TUI | roadmap F3 |
| 服务端 API/信令混沌 | Toxiproxy（TCP 域） | 信令 WS 可用；媒体域排除 |
| 代码级自动集成测试 | webrtc test/network、Pion vnet、mediasoup 合成 RTCP 单测路 | 上游先例（worker TestTransportCongestionControlServer 形） |
| CI 每 PR 回归 | lo netem 全命中 + 注入侧真值断言（netem dropped>0） | ✅ 已交付：子模块 `test-weaknet` job |
| 团队共享网络环境 | ATC 思路（件已死）→ 自建网关脚本或各端自打 | 不需要（dev 单机形态） |

## 8. 深坑清单（本项目实测，跨工具通用）

1. GSO 大段聚合吞 loss；lo 单口 delay 双向=2× 口径；`netem limit` 默认 100 包。
2. 施加后必回读 + 命中实证（tc -s 指纹 + leaf 计数增量）——静默 no-op 是本域头号事故（PIT-184）。
3. `tc qdisc change` 全量替换语义 + 独立 rate 属性不清 → 全参数重放 + 哨兵（sitespeed 实证；本内核实测 counters 不重置）。
4. UDP transport 服务端不判死（mediasoup/IceState 语义）：活性谓词用 ICE tuple，别信 closed()/stats 可达（PIT-185 族）。
5. 判据用基线配对保持比（ClickHouse no-fault twin / J2 >0.95），绝对阈值=算力差异假绿假红。
6. netem seed 需 iproute2≥6.6 且**只保 GE 模型 loss 序列复现**（uniform/时延/corrupt 无种子熵）。
7. gaiadocker/iproute2=2015 版 tc：无 seed、非空 root 树不可 replace、`qdisc show parent` 读空——镜像选版或脚本适配。
8. 对照流 vs 定向流判据分轨：retention>0.95 只对非定向流成立；恢复半判据留 ≥25s 驻留（BWE ramp-up）。

## 参考链接

- iproute2/tc-netem: https://man7.org/linux/man-pages/man8/tc-netem.8.html
- clumsy: https://github.com/jagt/clumsy | Comcast: https://github.com/tylertreat/comcast
- ATC(归档): https://github.com/facebookarchive/augmented-traffic-control | toxiproxy: https://github.com/Shopify/toxiproxy
- udptoxy: https://github.com/9mothers/udptoxy | pumba: https://github.com/alexei-led/pumba | tcconfig: https://github.com/thombashi/tcconfig
- quic interop: https://github.com/quic-network-simulator/quic-network-simulator
- sitespeed throttle: https://github.com/sitespeedio/throttle（demo 引擎出处）
- webrtc 内仿真: https://webrtc.googlesource.com/src/+/main/test/network/ | Pion vnet: https://github.com/pion/transport（vnet/）
- mediasoup 面: WorkerSettings.libwebrtcFieldTrials、Transport.setMax*Bitrate、demo applyNetworkThrottle（versatica/mediasoup-demo）；复现配方 issues #1115/#1536（versatica/mediasoup）
- 项目内：主仓 `.sisyphus/plans/weaknet-simulation-research.md`（全景+引文）/ `weaknet-harness/`（计划+证据）/ `weaknet-followup-roadmap.md`（F1-F4）
