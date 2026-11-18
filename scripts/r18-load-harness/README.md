# R18 Small-tier SFU load harness

container-build-rework R18（用户裁决 A 法）的**多 peer 媒体负载发生器**。既有的
`e2e-sfu-consume.cjs` / `e2e-bitrate-probe.cjs` 是单流功能 E2E（需 dev 栈
host-streamer + vite），不是负载工具——本 harness 填补该缺口，用于测定生产镜像
`mem_limit`（峰值 ×1.3 向上取整 512MB）。

## 方法

- **拓扑**：每房 1 个 host（produce 720p H.264 ~2Mbps）+ N 个 remote（consume）。
  device-room 模型每房仅 1 host slot（`room.rs` RoomFull），remote 数量不限。
  Small 档 = `ROOMS=10 PER_ROOM=2` → 10 产 + 10 消 = 20 peers。
- **负载源**：headless chromium + `--use-file-for-fake-video-capture`（ffmpeg
  testsrc2 生成的 720p30 复杂 y4m，H.264 编码达 ~2Mbps）+ mediasoup-client。
- **信令**：复用 server WS 协议（PSK 首消息认证 → room_join →
  get_router_rtp_capabilities → create_web_rtc_transport → produce/consume）。
  remote 在 host produce 之后加入（server late-joiner sync 下发已有 producer；
  Router 在首次 produce 时懒创建，remote 须待 Router 就绪才能取 caps）。
- **网络**：server 用 **host networking**（容器化 WebRTC ICE 经 docker bridge
  NAT 会卡在 ice=New；host 网络免 NAT，ICE 直连）。端口避开活 dev 栈：
  listen 19800 / SFU 21000 / web 18080。
- **采样**：`docker stats` 每 5s + 结束时 cgroup `memory.peak`（内核权威）。
  稳态 `STEADY` 秒 + 突发 `BURST` 秒（突发 = 每房追加 1 remote，转发翻倍）。
- **判定**：`mem_limit = ⌈peak × 1.3 / 512⌉ × 512` MiB；pass = 无 OOM + /health 绿。

## 运行

```bash
# 完整 Small 档（~13min）
bash scripts/r18-load-harness/run.sh
# 快速冒烟（~1min）
ROOMS=1 PER_ROOM=2 STEADY=20 BURST=5 bash scripts/r18-load-harness/run.sh
```

依赖：pixi node（或 PATH node）+ ffmpeg + playwright chromium
（`~/.cache/ms-playwright/chromium-*/chrome-linux64/chrome`，`CHROME` env 可覆盖）
+ 生产镜像（`IMAGE` env，默认 `msrtc-server:0.1.3`）。`run.sh` 自动 npm install
（npmmirror）+ 生成 fake.y4m + esbuild bundle + 起 server + 跑 driver。

产物 `r18-result.json`：config / steadyStats / burstStats / dockerStatsPeakMiB /
cgroupPeakMiB / memLimitMiB / samples（5s 采样序列）。

## 已测结果（2026-11-18，msrtc-server:0.1.3，8 核宿主）

| 阶段 | peers | 出（聚合） | 入（聚合） |
|---|---|---|---|
| 稳态 10min | 10 产 + 10 消 | 16.42 Mbps | 16.27 Mbps |
| 突发 2min | 10 产 + 20 消 | 14.75 Mbps | 28.69 Mbps |

cgroup peak **56.81 MiB** → `mem_limit` **512 MiB**（512MB 取整下限，~9× 余量）。
无 OOM，/health 全绿。详见 `docs/modules/29-operations.md`。

**码率说明**：每路实测 ~1.5–1.6 Mbps（目标 2 Mbps）——8 核宿主 CPU 争用 10 路
软件 H.264 编码 + 10–20 路解码，libwebrtc 自适应降码率。内存占用主要由
per-peer transport/producer/consumer 对象图决定（与码率弱相关），故 512 MiB
上限在满 2 Mbps 下仍成立；更大档位（medium/large）在生产宿主重测。

## 文件

- `driver.mjs` — playwright 编排 + docker stats 采样 + mem_limit 计算
- `load-client.js` — 浏览器 peer（mediasoup-client produce/consume + WS 信令）
- `setup-server.sh` — host-networking server + 预置 server.yaml（init 首写胜跳过）
- `run.sh` — 一键编排
- `package.json` — mediasoup-client / playwright / esbuild / ws

生成物（不入库）：`node_modules/` `fake.y4m` `load.bundle.js` `r18-psk.txt`
`r18-result.json`。