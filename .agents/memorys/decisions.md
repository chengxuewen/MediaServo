# AUDEMSP 架构决策记录

> **说明**: 本文件包含活跃决策（D196+）。历史决策（D1-D190）归档在 `decisions-archived.md`（含 20 个历史跳号）。
> 决策格式: `## D{N}: 标题` — 决策 + 日期 + 原因 + 影响
## D196: Admin Dashboard Architecture

**Decision**: React+TypeScript admin SPA embedded in server binary (rust-embed + build.rs). Phase 1 targets Remote Control scenario (DeviceStream). Room model refactored to per-stream rooms with N consumers. Zustand for frontend state. Admin JWT separate from signaling JWT.
**Date**: 2026-07-24
**Reason**: 
- Operators need visual monitoring of media streams (91 commits, zero visibility)
- Remote Control is the immediate use case (vehicles pushing camera streams)
- Per-stream room model simplifies relay routing
- Unified AdminEvent enum serves both audit logging and WS push
- build.rs auto-resolves dist/ path for rust-embed
**Supersedes**: D136/D142 (consolidated MVP admin plan — larger scope, different architecture)

## D197: D87 Scope Limitation — Client GUI Only

**Decision**: D87 (React + Ant Design for Server management panel) applies only to audemsp-client GUI. AUDEMSP Server admin dashboard uses CSS Modules for zero-dependency lightweight panel.
**Date**: 2026-07-24
**Reason**:
- D87's rationale (share components with AUDEBase Admin UI) is irrelevant for embedded server admin
- Admin dashboard is a monitoring tool, not a user-facing application
- CSS Modules = zero runtime, smaller bundle, no framework lock-in
- Ponytail principle: don't add Ant Design for a few cards and a table
**Limits**: D87 remains in effect for audemsp-client (Tauri desktop app) and any AUDEBase-shared UI
## D198: SFU Video Playback — Server-Offer Architecture

**决策**: 浏览器视频播放使用 mediasoup 的 Server-Offer 模式（SFU 创建 transport offer，客户端创建 answer）。Host 通过 SFU Produce 推流（非 P2P WebRTC）。
**日期**: 2026-07-27
**修订**: 2026-07-29（transport connect 已实现）
**状态**: ✅ 实现完成
- `connect_transport()` 已实现 (sfu.rs:331-371)
- signaling.rs + admin.rs handler 已调用实际连接
- 浏览器 sfu-client.ts consume 消息补充 rtp_capabilities


## D199: OpenCode Instructions 精简化

**决策**: 将 instructions 数组从 23 个文件精简为 19 个（D199 后续新增 docker/platform/lesson-memory），移除中英文重复（zh/）、无关语言规则（TS/CPP）、参考文档（agent-guide/model-tiers）。
**日期**: 2026-07-28
**修订**: 2026-07-29（数量从 17→19，新增 docker.md/platform.md/lesson-memory.md）
**原因**:
**日期**: 2026-07-28
**原因**:
- zh/ 10 个文件是 common/ 的完整中文翻译，约占 1,950 tokens — 纯冗余
- TypeScript/CPP coding-style 对 Rust 项目无关 — 约 1,200+800 tokens 浪费
- agent-guide.md 和 agent-model-tiers.md 是工具参考文档，非每轮必需 — 约 1,900 tokens
- constraints.md 和 edit-safety.md 已存在但未加载 — 比 agent-guide 更重要
- Rust 专属规则（coding-style + hooks）未在 instructions 中
- 3 个小的 memorys 文件（status/conventions/pitfalls）加入 instructions
**效果**: ~11,700 → ~8,500 tokens（节省 27%，且加载了更相关的规则）

## D200: OMO Agent 模型分配优化

**决策**: metis 保持 fast；momus 从 fast 升级到 premium；prometheus 从 premium 升级到 premium-max；explore 温度从 0.0 调整到 0.1。
**日期**: 2026-07-28
**修订**: 2026-07-29（参考 AUDEMSP+AUDEBase 联合评估）
**原因**:
- metis（度量/数据分析）本质是 pattern matching，复杂度低，调用频率高 → fast 足够
- momus（计划批评家）是质量门禁，对抗性审查需要深度推理 → premium
- prometheus（计划生成）是最高杠杆 agent，计划错误 = 下游全部返工 → premium-max
- 温度：metis 0.1，momus 0.3（创造性批评），explore 0.1
- oracle 保持 premium-max（最复杂架构决策）
**新增**: teams.dev 预定义（implementer + test-writer + reviewer），staleTimeoutMs 300000→600000

## D201: Pre-commit Hook — Rust 质量门禁

**决策**: 创建 `.git/hooks/pre-commit`，对暂存 `.rs` 文件运行 `cargo fmt --check` + `cargo clippy -- -D warnings`。
**日期**: 2026-07-28
**原因**:
- 规则会被遗忘，hook 不会。pre-commit 是最后一道防线
- `grep` 无匹配时用 `{ grep ... || true; }` 包装，避免 `set -euo pipefail` 下误中断
- 仅 .rs 文件暂存时触发，不阻塞非 Rust 提交

## D202: Global Provider Config — lite 上下文修复 + Reasoning 启用

**决策**:
1. lite 模型的 context limit 从 40,960 修正为 131,072
2. deepseek-v4-pro 和 deepseek-v4-flash 的 supportsReasoning 设为 true
**日期**: 2026-07-28
**原因**:
- lite 路由到 Qwen3-32B，实际支持 128K+ context，40,960 是配置错误（参考 lite-1 Qwen3-8B 也有 131K）
- DeepSeek V4 系列支持 reasoning API，设为 true 使 opencode 可使用 reasoning 特性
- Fallback 上下文窗口经别名映射验证全部正确：premium-max-1(256K)=Kimi K2.6, premium-2(205K)=GLM-5.1, fast-1(131K)=Qwen3.6 Flash 等
- apiKey 硬编码未修改（用户选择保持现状为内网环境）

**Phase**: Config

## D203: Agent 模型层级最终确认

**决策**: prometheus 从 premium 升级到 premium-max；metis 保持 fast（非 premium）。
**日期**: 2026-07-29
**原因**:
- prometheus（计划生成）是最高杠杆 agent — 计划错误 = 下游全部返工
- metis（度量分析）本质是 pattern matching，复杂度低，调用频率高 → fast 足够
- 参考 AUDEMSP+AUDEBase 联合评估：oracle + prometheus 为 premium-max 双高杠杆
**影响**: oh-my-openagent.jsonc 已更新，agent-model-tiers.md 已同步

**Phase**: System

## D204: ecosystem-scan 技能体系

**决策**: 创建 ecosystem-scan 技能（双层 Quick/Full + 社区对比 + 安全门禁），同时创建 doc-audit AUDEMSP 适配版。
**日期**: 2026-07-29
**原因**:
- .agents/ 体系需要定期审计和外部对标
- 社区先例：autoskills、agent-skill-discovery、skill-update-team、agent-self-audit
- doc-audit 从 AUDESYS 直搬未适配，需改写
**影响**: 21 个技能，ecosystem-scan + doc-audit + 9 个从社区移植的技能

**Phase**: System

## D205: skill-router 技能创建

**决策**: 创建 skill-router 技能，用于意图模糊时自动分析并推荐最佳技能组合。
**日期**: 2026-07-29
**原因**:
- 21 个技能导致用户不知道何时用哪个
- context-engineering 路由表处理关键词匹配，skill-router 处理模糊意图
- 与 context-engineering 互补：规则文件处理明确场景，技能处理模糊意图
**影响**: 22 个技能，AGENTS.md 目录表已更新

## D206: Docker 构建国内镜像加速 (A 方案)

**决策**: Dockerfile 使用国内镜像源加速构建（apt 清华源 + rustup 清华镜像 + cargo sparse 清华镜像）。
**日期**: 2026-07-31
**原因**:
- 国内网络下 Ubuntu apt/rustup/crates.io 直连慢或不可达（PIT-31/PIT-33 教训）
- mediasoup-sys flatbuffers wrapdb 不可达 → 统一走 Docker 构建（C13）
- 镜像加速只解决网络瓶颈（2-5min），mediasoup C++ 编译（15-30min）为硬瓶颈
**影响**: Dockerfile base 阶段 3 处加速点。后续 B 方案（预构建 dev 镜像）将进一步缩短到 <5min。
> **修订 (D208)**: cargo 镜像由清华 tuna 改为 rsproxy（tuna 只镜像 index，.crate 二进制 404 实测）；apt/rustup 清华源保留。

## D207: 预构建 dev 镜像推送 ghcr.io (B 方案)

**决策**: 构建一次含全部编译依赖的 dev 镜像，推送 `ghcr.io/{org}/audemsp-server-dev:latest`，后续 Dockerfile FROM 直接拉取。
**日期**: 2026-07-31
**原因**:
- mediasoup C++ Worker 编译 15-30min 无法在每次构建时重复（OpenVidu pre-built binary 模式）
- 层缓存（P0.1）只对 Cargo.toml 不变时生效，首次构建仍慢
- 预构建镜像一劳永逸：apt/rustup/crates 全部跳过
> **修订 (D208)**: 机制改为 compose `image:` + `pull_policy: always`（本地零构建）；命名统一 `audemsp-server-dev` / `audemsp-server-builder`；预烘焙按需启动（团队扩张时实施）。
**影响**: 待用户确认 ghcr org 名称后实施。Dockerfile base 阶段改为 FROM 预构建镜像。

---

## D208: 构建优化策略实施 (2026-08-03)

**决策**: 采纳 docs/reference/codec/build-optimization-strategy.md 方案 B（dev+builder 双镜像预烘焙 + 国内镜像修复 + lto 优化），分三阶段执行（本周修复 / 本月结构 / 下月按需）。
**日期**: 2026-08-03
**原因**:
- 首次 Docker 构建 15-30 min（mediasoup C++ 45% + Rust deps 35%），dev 镜像无预编译依赖 + gha cache 本地不可达
- 团队模式 4 分析师 + 4 审核员交叉验证：审计发现全部属实，方案经 H1-H4/M1-M6 修正
- 实测发现：pixi 无国内镜像（最慢层）、rsproxy sparse URL 失效、tuna 不镜像 cargo 二进制、ghproxy 停运
**修订**:
- **D206 部分修订**：apt/rustup 清华镜像保留；cargo 镜像 tuna → rsproxy（tuna 只镜像 index，.crate 二进制 404）
- **D207 机制修订**：FROM 预构建 base → compose `image:` + `pull_policy: always`（本地零构建，命名卷 copy-on-first-use 灌入烘焙产物）；镜像命名统一 `audemsp-server-dev` / `audemsp-server-builder`
**关键约束**（审核修正，实施时强制执行）:
- 卷 copy-on-first-use 仅空卷生效 → 落地必须显式 `docker volume rm audemsp_cargo-cache`
- 预烘焙镜像 amd64 only，Apple Silicon 走仿真，dev service 显式声明 platform
- GHCR 清理 workflow（sha tag 保留 N=10）+ path-filter（仅依赖变更时推 dev 镜像）
- ghcr 可达性未实测前不实施预烘焙（PIT-14/31 背景下可能 30min+）
- 生产 runtime 缺口是 admin dist 产物（非 feature）→ Docker 构建需 `pnpm build:admin` 先于 cargo build（PIT-23）
**影响**: 本地首次构建 15-28 min → 2-5 min（预计）；日常增量每轮省 30-60s。实施细节见 docs/reference/codec/build-optimization-strategy.md。

---

## D209: 项目重命名 OMSPBase → AUDEMSP (2026-08-03)

**决策**: 项目对外名称与全部标识符统一由 OMSPBase 更名为 AUDEMSP（AUDE 生态多媒体系统）。范围：crates/ 7 个目录与包名（audemsp-*）、Rust 代码标识符（281 处 import 路径）、环境变量（OMSPBASE_PSK→AUDEMSP_PSK）、Docker 镜像/服务/卷名、www npm 包名、docs（73 文件）+ .agents 记忆/规则/技能（20 文件）+ README/AGENTS.md + 脚本/CI（含 /opt/omspbase→/opt/audemsp 及 oomspbase 笔误修正）。
**日期**: 2026-08-03
**原因**: 项目归属 AUDESYS/AUDEBase 生态，统一 AUDEMSP 命名消除「OMSPBase 是独立项目」歧义，与生态命名体系一致。团队 4 分析师交叉验证（217 文件/2363 处）。
**例外（保留原名）**: decisions-archived.md 历史档案（174 处实测旧名引用）、git 历史/commit 消息、.omo/.sisyphus 归档快照、node_modules 生成物。
**影响**: ① 改名后 Docker 镜像层缓存全部失效（路径变化），首次构建回滚全量编译（一次性成本）；② 旧 env（OMSPBASE_PSK）与 localStorage 键失效——项目未发布，接受破坏；③ git mv 保留历史，单 commit 可 revert 回滚；④ 后续所有文档/命令使用 audemsp-* 命名。


## D210: 帧时间戳锚定单调真实时钟 (2026-08-05)

**决策**: write_raw_i420 的 VideoFrame 时间戳用 `ts_base_us(SystemTime 锚点) + Instant::elapsed()`（锚定单调），废弃假时钟（+33333us 固定步进）与裸 SystemTime::now()。

**原因**: 假时钟与 livekit TimestampAligner（delta-preserving，映射到 wall-clock 时间域）不一致 → 编码器帧率估计异常 → 停摆（PIT-63，T2.5 假设验证门证实）；裸 SystemTime 非单调（NTP 跳变 → ts 倒退）。

**影响**: 帧时间戳真实化是相机接入（V4L2 buffer timestamp）的前提；`write_raw_i420_with_ts` 参数化留口（T4）。

## D211: 帧率必须匹配 libwebrtc 编码器配置 — 帧循环绝对时间轴 (2026-08-05)

**决策**: Host 帧循环用绝对时间轴（`sleep_until(next); next += 33ms;`），禁止"固定 sleep + 耗时操作"模式；帧率目标 = libwebrtc 编码器配置（30fps）。

**原因**: SquaresPattern::draw 耗时 7-17ms 拖慢固定 sleep 循环 → 实际 ~20fps ≠ 配置 30fps → 编码器 rate control 异常（PIT-64）。OpenCTK RepeatingTask 同机制（审核评估的 tokio sleep_until 等价落地）。

**影响**: 任何视频源（生成器/相机）接入必须保证帧率匹配；C17 约束固化；E2E 连跑不稳定（PIT-65）为剩余问题。

## D212: docs/reference Diátaxis 重组 + 计划体系清理 (2026-08-06)

**决策**: ① `docs/reference/` 按 **Diátaxis 框架**重组——活参考（Reference，按产品模块镜像 webrtc/ codec/ + 根目录平铺）与调研存档（Explanation，`research/<领域>/`）分离，README 作唯一索引（C19 约束固化）。② codec 验收标准从 `docs/sdd/` 迁入 `.sisyphus/plans/audemsp-codec/`（pre-implementation 产物归计划区）。③ 计划体系收敛为单一权威源 `.sisyphus/plans/`——移除已全部完成的 `video-framepipeline-hardening`（.sisyphus+.omo 双副本）、去重 `.omo/plans/phase3-production` 副本、清理空 `.omo/plans/` 目录。

**原因**: ① 原 34 篇平铺 + 领域子目录重叠，混入 28 篇一次性竞品调研 → 活参考被污染（Diátaxis"按用途分离"原则，参考对齐 VitePress/Docusaurus 主流）；② acceptance 是 Phase 2 规划产物，Phase 1 sdd/ 目录放它格格不入（未编号+内容形态+Draft 状态不符）；③ 已完成计划/重复副本是死重，`e2e-acceptance-matrix.md` 断链暴露内容已内部化。

**影响**: 文档按用途可预测；计划唯一权威源，无重复无死链；历史调研保留在 `research/` 不碍事。保留：`phase3-production`（Phase 编号约定被 5 篇文档引用）、`host-sfu-w3c-alignment`（活跃待办，C18 待实施）。

**验证**: `ls docs/reference/` 顶层 = README + webrtc/ + codec/ + janus-gateway.md + research/；`find .sisyphus/plans/` 剩 3 个计划；无 `e2e-acceptance-matrix` 断链残留。

## D213: Agent 上下文爆炸治理 — instructions 瘦身 + 模型容量 + .agents 精简 (2026-08-06)

**决策**: 针对「当前项目配置容易上下文爆炸满」，实施三层治理：
1. **instructions 瘦身**：`.opencode/opencode.json` 的 `instructions[]` 移除 `pitfalls.md`（59KB 历史调试日志，占原 19 文件 130KB 的 46%）→ 改为按需读取（`read`/`grep` 查询），保留 18 文件（~70KB）。
2. **模型容量**：全局 `~/.config/opencode/opencode.jsonc` 将 premium-max-1（256K）与 premium-2（205K）的 `limit.context` 提升至 1024K，使 premium-max/-1/-2、premium/-1/-2 六模型全部 1024K（原 premium-2 fallback 是当前会话实际模型，205K 减去 40-50K 静态基线即紧张）。
3. **.agents 精简**：删 `rules/zh/`（11 文件，common 的中文翻译副本，C7 已声明不重复加载）；瘦身 `skills/book-to-skill/`（删 docs/.github/tests/tools/CHANGELOG 等，956KB→192KB，保留运行必需的 SKILL.md+scripts+book_to_skill 包）。**非项目语言规则保留**（用户明确改口，不删 cpp/csharp/dart/golang/.../swift）。

**原因**: ① 静态基线 ~60-100K tokens 主要来自 instructions 全量注入（pitfalls 59KB 是最大单一文件）+ 重复 codegraph MCP（oh-my-opencode 自动注册 `codegraph` + 项目 `local-codegraph` 双实例）+ 插件注入块；② premium-2 205K 上下文偏小，静态基线占比过高；③ zh/ 与 common/ 内容重复违背 C7；book-to-skill 混入完整 Python 库属异常膨胀。

**影响**: ① 每轮静态上下文估算 -59KB（pitfalls）+ 去重 codegraph，估算节省 ~40-50K tokens/turn；② pitfalls 不再常驻，调试时需主动 `read .agents/memorys/pitfalls.md` 查历史坑；③ 配置类变更需重启 opencode 生效；④ `limit.context` 是客户端声明，实际取决于 New API 网关后端是否真有 1024K 窗口。

**验证**: `python3 -c "import json; json.load(open('.opencode/opencode.json'))"` 通过；`node` 字符串感知注释剥离后 JSON.parse 通过（opencode.jsonc / oh-my-openagent.jsonc / ~/.config/opencode/opencode.jsonc 均有效）；六模型 `limit.context` 均 1024000；`.agents/` 从 1.7MB → 984KB。

## D214: audemsp-webrtc 补全 W3C API 面 + Host SFU 标准协商 (2026-08-06)

**决策**: ① 补全 audemsp-webrtc 的所有 W3C API 接口——新增 `RTCRtpTransceiver`/`RTCRtpTransceiverInit`/`RTCRtpTransceiverDirection`/`RTCRtpCapabilities`/`RTCRtpCodecCapability`/`RTCRtpHeaderExtensionCapability` 类型，`RTCRtpParameters` 补 `mid`、`RTCRtpEncodingParameters` 补 `codec`/`dtx`；PcBackend trait 扩展 19 个同步方法（get_transceivers/add_transceiver(+track 版)/sender-receiver get_parameters/capabilities/restart_ice/config/descriptions/transceiver 对象方法）；PeerConnectionApi + RTCPeerConnection 包装层同步扩展；RTCRtpSender 加 backend 句柄实现 get_parameters 等 W3C 对象方法。② Host SFU produce 走标准协商（add_transceiver_with_track → create_offer → set_local → get_sending_rtp_parameters → produce），删除 sfu_media.rs 的 build_remote_sdp/negotiated_ssrc_from_sdp/build_produce_rtp_parameters（C18 检查 src/ 无残留）。

**原因**: ① 用户要求尽量补全 W3C API（团队审核 + MDN spec 对标确认）；webrtc-sys 0.3.x FFI 已暴露 ~95% 接口无需新 C++；仅 RTCDTMFSender/identity 等缺 FFI 标注未来实现。② PIT-65 黑屏根因是 Host 绕过标准协商手工构造 SDP——对齐官方 mediasoup-client/Handler.cpp 标准流程。

**影响**: ① Host produce rtp_parameters 从 transceiver.sender.get_parameters() 推导（含 ssrc/PT），非手工硬编码；② 三后端（webrtc-sys/webrtc-rs/stub）对称实现，stub 状态化；③ 无法实现的 API（DTMF/identity/浏览器专属）在 docs/reference/webrtc/webrtc-w3c-alignment.md §5 标注未来实现；④ webrtc-sys 下 w3c_api_tests 有 4 个预存失败（ice/sdp 测试假设 stub 宽松状态机，非本次改动）；⑤ client crate 预存 feature 不匹配（用 webrtc-rs 方法却配 webrtc-sys feature），待 P4 回归处理。

**验证**: `cargo test -p audemsp-webrtc` (stub 46 passed) + `cargo test -p audemsp-webrtc --features backend-webrtc-sys` (除 4 预存失败全过) + `cargo check -p audemsp-host` 通过 + C18 检查 `grep build_remote_sdp src/` 无残留。

## D215: client P2P 迁移到通用 W3C API — 修复 feature 不匹配 (2026-08-06)

**决策**: ① audemsp-webrtc 加通用 `RTCPeerConnection::on_data_channel`（三后端，替代 webrtc-rs cfg 专属版），webrtc-sys observer 的 on_data_channel 接线到 callbacks。② client `webrtc_transport.rs` 迁移：`on_data_channel` 用通用版（`Fn(RTCDataChannel)` 异步 spawn spool），删 `from_webrtc`；`on_ice_candidate_native` → 通用 `on_ice_candidate`；`handle_ice` 用本地 serde struct 解析 camelCase ICE JSON（替代 webrtc-rs `RTCIceCandidateInit` 类型）。

**原因**: client Cargo.toml 配 `backend-webrtc-sys`（C12 webrtc-sys 为主），但 `webrtc_transport.rs` 仍用 webrtc-rs 专属方法（`on_data_channel`/`from_webrtc`/`on_ice_candidate_native`，均 `#[cfg(backend-webrtc-rs)]`）→ 编译失败。历史遗留：client 代码是 webrtc-rs 时代写的，未随 C12 迁移，且 audemsp-webrtc 之前只对 webrtc-rs 暴露这些方法。

**影响**: client 现可编译（5 crate 全通过）；通用 on_data_channel 补全了 W3C 接口面；webrtc-rs cfg 专属 on_data_channel 删除（被通用版取代）。client P2P 收帧走 webrtc-sys DataChannel（future: spool 需 webrtc-sys 实现，当前 stub）。

**验证**: `cargo check -p audemsp-client` 通过（之前 E0433/E0599 失败）+ `cargo check -p audemsp-host -p audemsp-client -p audemsp-webrtc -p audemsp-common -p audemsp-media` 全通过。

## D216: SFU E2E 统一 Docker + C21 架构回归 (2026-08-07)

**决策**: ① e2e_sfu.rs 改为**纯外部模式**——Host 模拟端通过 WS 信令协议连 Docker server（SFU_E2E_WS_URL），不 import server 类型（C21）。② Host SFU produce 走**标准 answerer 协商**：用 server transport 参数构造 remote SDP（build_remote_sdp）→ set_remote_description → add_track → create_answer → set_local，对齐 libmediasoupclient Handler.cpp。③ local answer 注入 `x-google-max-keyframe-interval=2000`（PIT-65 正解：libwebrtc 从 local answer 读 GOP 配置，remote 注入无效）。④ 浏览器 sfu-client.ts codec 对齐 VP8 96（router 默认）。

**原因**: ① PIT-71 webrtc-sys×mediasoup-sys 双 OpenSSL 链接冲突（架构性）+ C21 用户架构强调。② main.rs 原 offerer 流程（create_offer）从不 set_remote_description → ICE 无远端信息 → 30s 超时；add_transceiver_with_track 空 staged 队列 + 空 send_encodings → answer inactive。③ 稳态 GOP ~99s > 浏览器 90s 等待 → 黑屏（PIT-65 遗留）。④ 浏览器 capabilities 只有 H264（PIT-55 时代 router 配置）与当前 VP8 producer 不匹配 → No compatible media codecs。

**影响**: 全链路验证通过——Host produce → mediasoup → 浏览器 consume → 视频渲染（640×480, 153 帧, jitter 0.001）；关键帧间隔 99s→0.3s；e2e_sfu 4/4 通过（首次 Linux 真跑）。

**验证**: `docker exec audemsp-server-1 sh -c 'cd /workspace && SFU_E2E_WS_URL="ws://127.0.0.1:9800/ws" SFU_E2E_PSK="audemsp-dev" cargo test -p audemsp-host --test e2e_sfu'` 4/4 + `node scripts/e2e-sfu-consume.cjs $TOKEN` videoWidth>0。

## D217: setCodecPreferences 实现与 answerer 无效性实证 (2026-08-11)

**决策**: 实现 RTCRtpTransceiver.setCodecPreferences W3C API（track_id 定位 transceiver），
并实证 6 场景协商矩阵（H.264/VP8/VP9/AV1）。

**实证结论**:
1. **offerer 模式偏好生效** — create_offer 的 codec 序按偏好重排（H264 全在 VP8 前）
2. **answerer 模式（SFU server-offer）偏好对 answer 无效** — libwebrtc 按 offer 序取交集；
   SFU 固定 codec 必须走 **reduceCodecs**（mediasoup 官方模式: produce rtpParameters 裁剪）
3. **VP9/AV1 负向** — 偏好不在 getCapabilities 支持列表 → set 失败（InvalidAccessError 语义）
4. **mid 参数化不可行** — 协商前 transceiver 无 mid（offerer 核心场景）→ track_id 定位
   （与 request_key_frame 同模式）

**影响**: ① API 以 track_id 定位 ② SFU 固定 codec 需求（车端 H264）实现为 reduceCodecs
等价物（build_produce_rtp_parameters_from_rtp 后裁剪, ~5 行）③ setCodecPreferences 的
实际用途限定 offerer/P2P 场景。

**参考**: W3C WebRTC REC、libmediasoupclient Handler.cpp（reduceCodecs 模式）、
e2e_sfu_codec_prefs.rs / offerer_prefs_test.rs 实证

## D218: 编码器软/硬后端 + codec 双轨配置 (2026-08-11)

**决策**: 方案 C 双轨 — ① codec 固定: SFU answerer 用 **offer codec 控制**（build_remote_sdp
参数化, config.encoder.codec 驱动）② 硬编码器: **set_video_encoder_backend**（PcBackend track_id
分派 → SetEncoderSelector）。

**关键实证**:
1. **produce 参数裁剪不可行**（Oracle 审核）: 不影响 libwebrtc 实际编码（按协商交集 offer 序）→ 正解是
   **控制自造远程 offer 的 codec 列表**（D198 server-offer 架构下完全可控）
2. **H264 profile 统一 42e01f**: router 原 4d0032（constrained baseline）浏览器解码不渲染 →
   统一 42e01f（OpenH264 能力 + 浏览器通用）; offer fmtp = router profile → 协商结果保留 offer profile
3. **produce 必须带 codec parameters**（PIT-54 实证）: VP8 空参数侥幸匹配; H264 缺
   profile-level-id/packetization-mode → Unsupported codec (Error 5000)
4. **浏览器 consume 必须请求匹配 codec**: sfu-client.ts offer 硬编码 VP8 → producer H264 时无视频;
   改为 VP8+H264 双请求
5. SetEncoderSelector 语义: 偏好非强制（不可用自动 fallback + warning）

**影响**: host.conf encoder.codec/backend 全链路可控; 车端 H.264 硬编路径就绪（codec=h264 + backend=hardware 组合）;
P2P offerer 路径 setCodecPreferences 留后续接线。

**验证**: 5 场景矩阵（auto/h264+浏览器渲染/vp8/vp9 负向/backend=software）+ 全量回归

## D219: Web 端视频流编码状态展示（ToDesk 式诊断） (2026-08-11)

**决策**: VideoPlayer 内嵌 ToDesk 风格 stats 面板 — Host 编码状态经 **room 广播 relay**
（非 admin WS）→ 浏览器现有 /ws 直接收到; Host get_stats FFI 接线（纯 Rust）提供实际编码器。

**关键实证**:
1. **转发路径**: admin WS 推送通道（event_tx）signaling.rs 无访问权, 且浏览器播放只连 /ws →
   EncoderStatus 走 should_relay 白名单 + DeviceStream 过滤放行（NewProducer 同模式, 零新通道）
2. **get_stats**: webrtc-sys FFI 已就绪（ToJson 含全部字段含 encoderImplementation）→
   纯 Rust 解析, 零 C++ 改动; RTCOutboundRtpStreamStats 加 encoder_implementation
3. **实际编码器优于请求值**: backend=hardware（无 GPU）→ 实际 fallback OpenH264 软编 →
   面板显示"软编"+OpenH264（请求值会误报"硬编"）
4. **浏览器侧 inbound-rtp 数据**: headless shell 环境 getStats 为空（环境限制, 真实浏览器有数据）

**影响**: host.conf codec/backend 全链路可见; 车端硬编状态可诊断; P3（CPU/GPU 系统性能）留后续。

## D220: Jetson(linux-aarch64) 构建统一用 JetPack 系统工具链 (2026-08-12)

**决策**: 在 linux-aarch64 平台，host/client 构建**统一改用 JetPack 系统工具链**（gcc 10.5 + 系统 binutils），
弃用 pixi conda 交叉编译器（GCC 14.4）。实现：pixi.toml `[target.linux-aarch64.activation.env]` 覆盖
CC/CXX/CARGO_TARGET_..._LINKER 为 /usr/bin/gcc + 清空 CFLAGS/CXXFLAGS/LDFLAGS；.cargo/config.toml
`[target.aarch64-unknown-linux-gnu]` linker=/usr/bin/gcc + `-B/usr/bin/` rustflags（裸 cargo 兜底 +
强制系统 binutils，防 pixi PATH 首位 conda bin/ld 劫持 collect2）。

**原因**: ① conda 交叉工具链与 JetPack 系统库**根本性不兼容**——可执行链接（-pie）传递依赖搜索
不用 -L（只用 -rpath-link/-rpath），`cargo:rustc-link-arg` 不从 rlib 传播，把系统 multiarch 目录加入
搜索会拉入系统 glibc 2.35 与 conda glibc 冲突、并遮蔽 libstdc++（GCC14→GCC10）；② 上游 livekit 官方
Jetson 流程即系统工具链（C18 官方用法优先）；③ 系统 gcc 原生找到 libv4l2/tegra/系统 glib——零 hack。

**影响**: ① Jetson 上 host/client 构建全绿（`audemsp.sh build host`），ldd 0 not-found，
C++ 全链路 gcc 10.5；② **Jetson H264/AV1 硬编码器可用**（人工验证：backend=hardware + codec=h264/av1
实际走 Jetson MMAPI 编码器）；macOS/x86_64 CI 零影响（全部 linux-aarch64 门控）；
③ 后续若启 GStreamer codec 后端需单独评估 conda gstreamer 与系统工具链混用。

## D221: AUDEMSP → MediaServo 独立平台重命名 (2026-08-13)

**决策**: 全量重命名 AUDEMSP → MediaServo。品牌名 **MediaServo**（PascalCase, 文档/UI/正式名 "MediaServo Platform, 实时媒体伺服平台"）+ 技术前缀 `mediaservo-`（7 crate + 二进制 + CLI + env）+ **脱离 AUDE 生态**为独立部署的视频/媒体服务平台（监控/NVR + 会议 + 桌面 + 遥操作 + 推流）。命名冲突实证: crates.io/npm/GitHub **0/0/0**（6 轮全维度检查）。**修订 D209** 的生态归属结论（原"统一命名、生态一致"被本次"独立平台"取代）。

**原因**: ① 品牌化——不再是"AUDE 生态多媒体系统"，独立定位媒体伺服平台（Servo=精确低延迟驱动，契合项目帧时间戳/帧率/BWE 控制基因）；② 与 AUDE 解耦（后续不依附 AUDESYS/AUDEBase 生态）；③ 冲突检测零背书。

**范围**: T1 机械替换 259 文件/1436 行（env MEDIASERVO_* + 品牌 MediaServo + 小写 mediaservo + audemedia→mediaservo）+ 7 crate 目录/二进制/CLI 文件 git mv；T2 AUDE 生态剥离（README/AGENTS/docs 11 文件 80 处 → 中性平台表述）；T3 基础设施名（compose `name: mediaservo`、service 文件、pixi 名、audemsp_cli.py）；doc-audit 修复 H1-H3/M1-M3（decisions/status/conventions/AGENTS 同步）。

**影响**: ① Cargo.lock 随 T1 同步（7/7 mediaservo, 0 audemsp）；② Docker 层缓存全失效一次性重编译；③ env 改名 `AUDEMSP_*`→`MEDIASERVO_*`（scripts 侧零残留）；④ 保留面: **仅 `.agents/`**（decisions/pitfalls/status/conventions 历史提及保留, 史实不可篡改）; `.sisyphus/.omo plans`（含 mediaservo-rename 对照记录）已随 2026-08-13 政策更新/清除（用户指令: 仅 .agents 保留, 弃用计划已清除）；⑤ 后续约定/检查命令统一 `mediaservo-*`（conventions C4-C22 同步）。

## D222: 四 SDK 主架构 — link/field/client/deck (2026-08-13)

**决策**: 设备侧第三方集成 SDK 定为**四个交付形态**：`mediaservo-link`（连接面: frame_bus/signal/auth/dc）、`mediaservo-field`（媒体会话面: push/pull）、`mediaservo-client`（舱端组合: 依赖 field + 渲染抽象/会话编排/控制绑定）、`mediaservo-deck`（媒体数据面: source/codec/record/playback）。命名: field=现场端（D67/D68 回归）、link=连接、deck=录放一体机（广播术语，record/play/trick 为原生语义）、client=主控侧（C4）。

**原因**: ① 消费剖面四类（link-only 轻量 / deck-only 录放 / field 推拉流 / client 舱端全量），每类最小依赖；② 依赖正交——轻量消费者（ROS 节点/订阅帧/纯控制）不背 libwebrtc 媒体栈；③ 全单向无环依赖图：field→link(+deck[source])，client→field，deck 独立；④ 推翻 D65-D69/D82 的"field/client 双 facade 拆分"（IPC-only 轻量面与依赖不对称论据在 2026-08 架构下已失效——共享代码收敛在 common/webrtc/media，角色合并单 field 即可）。

**影响**: 后续第三方 SDK 交付以四 crate 为准；host/client 二进制成为四 SDK 的消费方与吃狗粮验证；04-sdk-layers.md 同步重构；行业对标 LiveKit（单核心+薄绑定）、libmediasoupclient（单库 Device）、Janus/OBS（dlopen 插件）。

## D223: field 采集归属 — 纯会话面 + deck[source] 静态依赖 (2026-08-13)

**决策**: **field 不提供采集能力**（修正初版 spec 的"field 内建 GStreamer capture"倾向）；采集统一由 deck 的 source 域提供（GStreamer 动态，D64 v4l2src 实现）。field 编译期静态依赖 `deck[source]`（仅 GStreamer 采集切片，无 FFmpeg 符号），并 re-export deck 的 `MediaDevices`/`VideoSource` API（对标 getUserMedia 体验）；field 依赖 link（signal/frame_bus）+ mediaservo-webrtc（backend-webrtc-sys，C12: WebRTC 必经抽象层）。

**原因**: ① 采集消费者多样（推流/录制/感知）→ 单一实现源避免分裂（deck 双形态下 source 切片可被静态联，full 切片走插件）；② GStreamer 动态插件 + Rust 默认不导出符号 → 与 libwebrtc(BoringSSL) 零静态冲突；③ field 纯会话面 = 最小面 + 与 LiveKit 对齐（核心 SDK 不做设备采集，VideoSource=帧注入源，LiveKit Python 音频采集用 sounddevice 同构）。

**影响**: field 一行依赖即完整推流体验（传递 deck[source]）；采集唯一入口是 deck；桌面捕获（webrtc-sys desktop_capturer 已有）进 deck 后端矩阵备选。

## D224: deck 双形态 — rlib 独立 + cdylib 插件 (2026-08-13)

**决策**: `mediaservo-deck` `crate-type=["rlib","cdylib"]`；feature 切片：`source`（GStreamer 采集，可被 field 静态联）/ `ffmpeg·record·playback`（FFmpeg 静态，仅出现在①deck rlib 独立应用②deck-full.so 内）。**field/client 与 deck 同进程 = cdylib 插件形态**（D13 PluginManager 载体；接口 `deck_plugin_init/encode/record/playback` + version 握手；RTLD_LOCAL + 主进程不导出符号）；deck 也可被应用单独使用（rlib full）。deck-full.so 由 CI 独立构建交付。

**原因**: ① FFmpeg 静态 OpenSSL × libwebrtc BoringSSL 静态链接冲突（**PIT-71 延续**：X509_PUBKEY_it duplicate symbol 链接必挂，C21 同因）→ 插件 ELF 符号空间隔离是结构性消解；② dlopen 插件=单进程组合便利但**丢崩溃隔离**（插件 C 崩溃拉崩宿主）→ 默认多进程、组合模式插件，二选一给消费者；③ Janus/OBS dlopen 插件先例。**关联 D13**（核心插件编译期 feature + 扩展插件运行时 dlopen）——deck 即"扩展插件"定位。

**影响**: 链接冲突矩阵进 CI（field 独立/deck 独立/field+deck[source] 静态/field+deck-full 插件，每格 build+smoke）；deck 的 FFmpeg 后端禁开 openssl/network feature（本地处理无需 HTTPS）。

## D225: 本地录制双编码架构（Phase 2 落地）(2026-08-13)

**决策**: 本地录制与推流编码**各自独立（主/子码流）**；帧总线双 topic（`video_raw` I420 MVP / `video_enc` H264 Phase2+）；recorder-worker = link + deck（订阅帧→编码→mux 落盘），轻量独立于 field。**MVP 不实现录制**（YAGNI，先跑通推流链路）。

**原因**: ① 推流随 BWE 动态降码率/分辨率，共享单编码会**污染录制质量**（弱网抖动拖累存储流）；② 双码流是监控行业标准（NVR 主码流存储、子码流网络）；③ webrtc-sys 无外部编码帧输入接口（无 VideoCaptureModule/注入桥）→ 推流保持 I420 输入零改动；④ Jetson MMAPI 多并发编码器可承载（C23 已验证硬编可用）。

**影响**: 录制能力推迟 Phase 2；帧总线元数据需 is_keyframe 标志 + monotonic/epoch 双时钟戳（MP4 keyframe 对齐）。

## D226: 控制面通道 — server relay 优先, DC 为 Phase 2 增强 (2026-08-13)

**决策**: 控制/紧急/遥测消息 **Phase 1 走 server 房间 relay**（WS，`should_relay` 已实现验证）；DataChannel 为 Phase 2 低延迟增强（**webrtc-rs** 后端——backend 已存在且与 SFU e2e 4/4 通过；libdatachannel = 第四后端 + C++ 崩溃面，排除）。紧急通道独立 control-worker（link only），**不依赖媒体进程存活**。

**原因**: ① 崩溃隔离动机（车端 #1）要求控制面不能绑媒体连接生死；② 07-protocols 四通道（control/telemetry/emergency/heartbeat）语义不变，仅承载面切换 relay→(可选 DC)；③ control-worker 只依赖 link（纯 Rust 轻量）。

**影响**: MVP 无 DC 开发；libdatachannel 的定位=第三方 C++ 客户端的自选直连工具（同 FFmpeg/GStreamer 之于 broadcaster），不进 SDK 依赖树。

## D227: 绑定体系 — c/cxx crate + py 双后端两步走 (2026-08-13)

**决策**: 绑定命名族（D68 `-c` 惯例延续）：C=`-c`、C++=`-cxx`（ICEoryx2 同构先例 iceoryx2-c/iceoryx2-cxx）、Python=`-py`、未来安卓=`-jni`。**绑定位次**: `-c`（cbindgen 生成 + C 测试 + cargo-c 分发, 必要）/ `-cxx`（header-only RAII .hpp over C ABI + C++ 测试, 独立 crate, 分发可并入 -c 包）/ `-py` **两步走**: ① py = 纯 Python（ctypes 加载 cdylib, 普通场景, LiveKit 模式, 非 Rust crate）② pyo3 = 瓶颈触发的加速后端（iceoryx2 模式, maturin wheel）。py 双后端共享同一 Python API 层 + 自动探测回退 + 同一套行为测试双跑。

**原因**: ① 覆盖两类消费者——普通场景全价快（ctypes 零编译）与性能路径（pyo3 numpy 零拷贝/GIL 控制）; ② 维护主体=Rust 团队, pyo3 写 Rust 优于手写 ctypes; ③ 瓶颈驱动不预埋（触发条件量化: 帧路径调用 >帧预算10% / GIL 拖累>20% / numpy 转换>5%）; ④ 修正 D79 的"pyo3 原生唯一路线"为两步走; ⑤ 跨语言对照测试（c/cxx/py/pyo3 同一操作序列断言一致）防 API 漂移。

**影响**: 绑定 crate 体系 = c + cxx 两核; py 独立构建/包（同 repo, bindings/python/, 非 cargo member）; 未来 jni 复用 C ABI; pyo3 阶段 python 绑定才可能成为 workspace member（iceoryx2 同）。

## D228: 项目目录布局 — crates/ + bindings/ 分区 (2026-08-13)

**决策**: 根 workspace members 跨目录: `crates/*`（现有 7 + 新 SDK 核心 link/field/deck + client 升级 lib+bin 双 target）+ `bindings/c/*` + `bindings/cxx/*`; `bindings/python/` 非 cargo member（纯 Python, poetry 独立构建）。绑定目录只随 SDK 落地创建（当前文档预留, YAGNI）。

**原因**: ① 单 repo 不分裂（D221 延续）——LiveKit 多 repo 带来版本同步成本, 不采用; ② cargo 能编的进 workspace 原子 CI（C/C++ 绑定是 Rust crate）; ③ python 构建系统异构（maturin/poetry）不阻塞 cargo 矩阵; ④ 与 ICEoryx2（全 member）与 LiveKit（python 独立 repo）的折中: c/cxx 全 member + python 独立。

**影响**: 现有 7 crate 路径零破坏; mediaservo-client 已列为 lib+bin 双 target（SDK lib + GUI bin）; CI 增加绑定测试矩阵（C/C++ 编译器）与 python 构建分离。

## D229: deck 与 codec 的关系 — 依赖不合并 (2026-08-13)

**决策**: **deck 依赖 codec（facade）, codec 引擎保持独立 crate**——不替换/不合并。deck = codec 之上的媒体处理层（source 采集 + codec 编排 + record/playback 文件语义）。

**原因**: ① **分层不能反转**——mediaservo-codec 唯一现有生产消费者是 mediaservo-webrtc 的 backend-webrtc-rs（解码）；deck 吞并则底层抽象反向依赖顶层 SDK（架构性错误）; ② FFmpeg 分库先例（libavcodec 引擎 / libavformat+libavdevice 封装——deck 的 record/playback 恰为后组角色）; ③ codec 94 道测试资产（stub 32/FFmpeg 35/GStreamer 27）不迁移; ④ 未来消费者（field GStreamer 采集、webrtc-rs 后端、独立转码）都指向引擎独立。

**影响**: 依赖链 deck → codec + media + common 确认; 后移条款: deck 落地后若 facade 包 codec 产生两层 API 摩擦 → 优先 codec API 重构, crate 合并仅在 codec 失去全部非 deck 消费者时重评。

## D230: field 组合 SDK 定位（修订 D223「纯会话面」表述）(2026-08-13)

**决策**: field = **组合 SDK**：媒体（push/pull via mediaservo-webrtc, C12）+ 通信（re-export link 的 SignalClient/FrameBus/Registry/ControlSession/auth）+ 采集（re-export deck 的 MediaDevices/CameraSource/AudioSource/ScreenSource/VideoSource）。**一行依赖 field 即得 采集+信令+推流+总线 全链路闭环**。「纯」指实现归源（field 无自采/自编解码实现，单一事实源在 deck），非能力缺失。

**原因**: ① 某些场景仅使用 field（相机采集+推流+信令）要求单依赖闭环（D69 facade 精神最终形态）; ② link/deck 仍是切片供轻量消费者（ROS/订阅/录放），field 是超集; ③ **re-export 白名单纪律**防 API 膨胀——只透会话级类型（SignalClient/FrameBus/MediaDevices/VideoSource/PushSession/PullSession 等），不透内部原语。

**影响**: field 依赖（全静态）: webrtc + link + deck; 字段真实现全部来自 webrtc/link/deck 三源; 白名单变更过审。

## D231: field→deck 静态直连默认, dlopen 后移（修订 D224）(2026-08-13)

**决策**: field 对 deck 采用**静态 rlib 依赖为默认**（source 必需, full 可选）; dlopen 插件模式（原 D224 的默认机制）降级为 **OTA 独立升级/实现可替换需求出现时的可选演进**（D13 接口预留, MVP 不实现）。

**原因**: ① PIT-71 冲突根源 = BoringSSL × 静态 OpenSSL 同类库共存; ② **实证**: ffmpeg-the-third 5.0.0 默认 features 不含 `build-lib-openssl` 且本项目未开启 → deck 的 FFmpeg 无 OpenSSL 符号 → 与 libwebrtc(BoringSSL) **零交集** → dlopen 的首要动机（静态符号隔离）在默认配置下消失; ③ 静态直连零插件机制复杂度（traits/发现路径/版本握手全免）; ④ 崩溃隔离靠多进程拓扑（同进程 dlopen 本就无隔离收益）。

**影响**: 链接矩阵新增 **field+deck-full 静态**组合（新默认）; 纪律锁定: FFmpeg 禁 `build-lib-openssl`/`build-zlib` + zlib 系统动态 + x264/openh264/opus 交集逐项验证进 CI; deck-full.so 分发与 D105 tarball 关系 Phase2 定稿; 同步修订 spec §6/§7 与 04-sdk-layers。

## D232: client 消费端定位（修订 D222 client 定义）(2026-08-13)

**决策**: client ≠ 第二个会话 SDK; = **消费端编排 SDK**。独有能力 = VideoRenderer（GPU interop, D47）/ 多路会话编排（多屏, D66）/ Input Forward·遥测绑定（D18/D66）/ deck playback 集成（舱端回放）。依赖 field（含传递链）; 传输面复用 field 的 PullSession, 不重复实现。

**原因**: ① 会话层能力 field 已齐（含 PullSession）——client 若无消费编排则无存在价值; ② 行业对照: 海康 PlayCtrl（消费端独立渲染回放库）有独立先例; LiveKit 渲染在平台层（Flutter/SwiftUI 零成本）而车机原生 GPU interop 是高成本, client 封装对第三方有真价值; ③ 后移条款防僵化。

**影响**: MVP client 独有面 = VideoRenderer trait + 单路 PullSession 编排; **后移条款**: v2 独有内容 <~500 行有效代码 → VideoRenderer 转 field 的 render feature 并归档 client。

## D233: SDK API 形态 — 单层会话型（选项 B）(2026-08-14)

**决策**: field/client SDK 对外 API 采用**单层会话型**（`PushSession`/`PullSession`），**不暴露** mediasoup 细粒度对象（Device/Transport/Producer/Consumer）作为公开 API。高级需求用**富 Options** 承接（codec/simulcast/bitrate/encoder_backend）；真·底层控制走 **mediaservo-webrtc 直接依赖**（逃生舱，field 不 re-expose）。未来若出现 Options 表达不了的细粒度需求 → **加法式新增**（向后兼容，不返工）。

**原因**: ① YAGNI/ponytail——细粒度对象需求当前为推测性（车端推流/舱端拉流/遥操作全是会话级），为推测需求建双层=过度设计；② **C18 范围解读**——C18 约束的是**内部协商机制**（标准 offer/answer、禁手搓 SDP/rtp_parameters，反 PIT-65），**不强制公开 mediasoup 对象形态**；单层会话型内部照做标准协商即合规；③ **逃生舱已有**——mediaservo-webrtc 是独立公开 crate，真需底层控制直接依赖它，field 无需 re-expose → 双层冗余；④ 富 Options 承接高级需求（LiveKit TrackPublishOptions 先例：codec/simulcast/encoding 全在 options）；⑤ 面最小 → Hyrum 暴露最小 → 最易守向后兼容；最贴"一行依赖完整闭环"组合 SDK 定位。

**否决**: 双层 A（为推测需求付双倍维护成本、两套 API 致用户困惑、Hyrum 双层暴露）；单层 mediasoup C（简单场景被迫走 Device.Load→CreateTransport→Produce 全流程、违背一行闭环、与"信令内建"决策打架）。

**影响**: field/client 公开 API 只含会话层；API 子细节（事件模型/品牌化 ID/帧 API）见后续 API 契约设计。

## D234: SDK API 调用约定（事件/ID/错误/选项/帧）(2026-08-14)

**决策**: 四 SDK 公开 API 统一调用约定：① Rust 全 async（tokio）返回 `Result<T, XxxError>`；② 事件模型 Rust 用 enum+channel/Stream、C/C++ 用回调函数指针；③ ID 品牌化（RoomId/PeerId/TrackId/SessionId/StreamId/NodeId/DeviceId）；④ 错误每 SDK 一个 thiserror enum + `#[non_exhaustive]`（LinkError/FieldError/DeckError/ClientError）；⑤ 选项纯 struct + Default（不用 builder）；⑥ 帧注入保留 `write_raw_i420_with_ts` 为注入点、deck source 对象产帧喂入。

**原因**: ① 事件 enum+Stream 比 trait-listener 可组合（LiveKit Dispatcher 先例）；② 品牌化 ID 防串参（api-interface-design 技能）；③ thiserror 库错误约定 + RTCError 先例；④ 纯 struct 选项 = LiveKit 实证 + 我们 config 惯例，builder 属过度设计；⑤ `write_raw_i420_with_ts` 已用且对齐 LiveKit captureFrame。

**影响**: 完整接口契约见 `docs/modules/20-sdk-api-contract.md`；向后兼容只加法（`#[non_exhaustive]`、新 variant 追加末尾、新字段 `Option<T>`）。

## D235: link IPC 注册中心 — 去中心化 SHM + attach 即注册（B+C 融合）(2026-08-14)

**决策**: link IPC 注册中心采用**去中心化 SHM service registry + attach 即注册，无专用 registry daemon**（B+C 融合）。数据面 SHM 零拷贝直连（发布写 SHM、订阅直读，不经 broker）；注册/发现/活性为控制面——节点 attach 时自描述注册（topic 声明 + role + capabilities），heartbeat/lease 活性，掉线自动摘除；权限经签名令牌在 attach 本地校验（不依赖中央裁决）。

**原因**: ① 零拷贝帧硬需求锁定 SHM 数据面 → 排除 broker 型（A 数据路径前提不成立）；② 车端 7x24 可靠性 → 避免 registry daemon 单点故障；③ <10 节点规模，A 的强一致集中视图优势用不上、代价（SPOF+常驻进程）全吃；④ 权限可无中央强制（签名令牌 + attach 本地校验），化解去中心化最大劣势；⑤ iceoryx2 若采用则发现机制现成（C11）。

**否决**: A 专用 registry daemon（SPOF + 常驻进程，规模用不上其优势）。

**影响**: 数据面 SHM 零拷贝；注册去中心化；演进条款——未来节点规模/一致性需求上升可引入轻量 registry，现在不预埋。ROS 集成/权限细节见后续决策。

## D236: link IPC ROS 集成 — 帧路径 ROS 节点直连 FrameBus（选项 1）(2026-08-14)

**决策**: 需要低延迟帧的 ROS 节点（感知/拼接）**直接加入 FrameBus**（link link-SDK，`attach(endpoint, credential)` + role 令牌），不经桥接；不需要低延迟帧的 ROS 子系统可用桥接（保持纯 ROS）作逃生舱。ROS 节点从设备配置/环境变量/约定 SHM 路径取 bus endpoint。

**原因**: ① 感知订阅相机帧、拼接订阅多路再发布——全是低延迟多路帧操作，SHM 零拷贝直连是唯一不牺牲性能的路径；② 桥接引入帧拷贝 + 延迟 + 额外组件，恰好伤在拼接/感知痛点；③ 拼接节点需同时"订阅多路 + 发布派生 topic"，直连零拷贝读多路 + 直接发布 `video/stitched`；④ 权限无缝：ROS 节点持 role 令牌、attach 本地校验 ACL，与 D235 去中心化权限自洽；⑤ 少一个组件（ponytail）。

**否决**: 桥接为主（选项 2）——帧拷贝 + 延迟 + 潜在单点，伤在痛点。

**影响**: ROS 节点双协议栈（ROS-DDS 对 ROS 内部 + FrameBus 对 MediaServo）；py 节点走 ctypes/pyo3（D227 两步走），C++ 走 cxx；桥接仅作非帧场景逃生舱。

## D237: link IPC 权限载体 — 静态 ACL（role 预置 + 节点覆盖）(2026-08-14)

**决策**: link IPC 权限采用**静态 ACL**：每个 role 一套预置 ACL（`publish_allow`/`subscribe_allow` topic 通配模式），允许按节点覆盖；`attach` 时校验凭证 + 载入 ACL，**每次 publish/subscribe 逐次查 ACL**，越权拒绝 + 审计日志。最小权限 + 隔离（感知节点不能 publish `control/cmd`）。

**原因**: ① 与 D235 去中心化自洽——动态授权需中央授予方，与无 daemon 冲突；② 车端节点集合固定，不需运行时动态授权；③ 静态可审计（读配置即见全部权限）+ 可版本管理；④ attach 后离线自足，契合边缘设备；⑤ ponytail 不过度设计。

**否决**: 动态授权（需中央权威，动摇 D235 去中心化）。

**影响**: 权限变更走"改配置→重签→重启节点"（低频可接受）；ACL 配置纳入设备配置统一管理；演进——未来需运行时吊销可加轻量 revoke list，MVP 不预埋。令牌机制（ACL 携带形态 + 签发）见后续决策。

## D238: link IPC 令牌机制 — 能力令牌（ACL 签进 JWT）(2026-08-14)

**决策**: link IPC 采用**能力令牌（capability token）**：ACL（`publish_allow`/`subscribe_allow`）签进 JWT，**设备私钥签发、公钥校验**；ACL 源配置（role 预置 + 节点覆盖）作签发与审计源，令牌是签名快照。**复用 link JWT，不建独立节点证书 PKI**。PSK 不参与令牌签名（PSK 对称、用于对 server 认证，属另一关注点）；节点能力令牌用非对称 JWT（Ed25519/ES256），claims 含 `node_id/role/acl`。

**原因**: ① 去中心化自洽——令牌自描述、本地验签，无中央、无配置分发依赖（D235）；② ROS 直连友好——ROS 节点一个签名令牌即接入（D236）；③ 复用 JWT 基建不新增 PKI（ponytail）；④ 非对称契合边缘——私钥集中设备权威、公钥广布各校验点；⑤ 可审计——ACL 源配置一处编写，令牌为签名快照。

**否决**: 身份令牌+配置查询（bus 校验需配置分发，去中心化下不便）；独立节点证书 PKI（对固定节点集合过度设计）。

**影响**: ACL 变更走"源配置→离线重签令牌→节点重启"；签发方=设备权威组件（provisioning/host supervisor）部署时离线签发。

## D239: link IPC 派生 topic 治理 — 自由创建 + ACL 兜底 (2026-08-14)

**决策**: processor 节点创建派生 topic 采用**自由创建 + ACL 兜底**：publish 时查能力令牌（ACL），落在 `publish_allow` 模式内即放行、越权拒绝 + 审计；SHM registry 仅**自描述登记**（供发现），不做批准；约定**单 topic 单发布者**（同名 topic 已有发布者则后到者拒绝）；派生 topic 层级命名规范。

**原因**: ① 与 D235 去中心化自洽——无中央批准方，ACL 即闸门；② 不与 D237/D238 重复治理——ACL 已管"能否 publish"，再套批准冗余（ponytail）；③ processor 灵活产出派生流（拼接/感知），正是处理节点模式价值；④ registry 登记供发现、ACL 管权限，职责单一。

**否决**: registry 显式批准（需批准方，动摇 D235 去中心化）。

**影响**: 治理靠 ACL 模式 + 命名规范，无审批环节；topic 泛滥由 ACL 边界约束；演进——未来需更强治理可在 ACL 源配置加 topic 白名单/配额（仍静态、无中央运行时批准），MVP 不预埋。

## D240: SDK 库交付形态 — 单动态库（.so/cdylib）为主 (2026-08-14)

**决策**: 四 SDK 对外 C/C++ 绑定交付采用**单动态库（.so/cdylib）为主**，不预建静态库 .a。静态 .a 待确有"单一自包含二进制"的嵌入式集成需求出现时再加（additive：crate-type 加 `staticlib` + 补测试，不返工）。交付物：`link.so` / `field.so`（打包 link+deck）/ `client.so` / `deck.so`（+ `deck-full.so` OTA 插件）。Rust 内部消费者走 rlib，不受此决策影响。

**原因**: ① 多进程车端拓扑（capture/push/control/recorder + ROS 节点）→ 动态共享内存（libwebrtc 不必每进程一份，车端 RAM 受限）；② 与 deck-full.so dlopen OTA 插件体系一致（D224/D231）；③ ponytail/YAGNI——单一形态最省构建/测试/维护，双格式为推测需求付双倍成本；④ LiveKit 先例（cdylib）；⑤ 易更新/OTA 对车端运维友好。

**否决**: 双格式 .a+.so（双倍构建/测试/ABI 面，多数场景只用其一）；单静态（多进程内存重复、更新要重链、与 dlopen 插件方向相悖）。

**影响**: `crate-type = ["cdylib"]`（外部）+ rlib（内部）；C++ 绑定 header-only RAII over .so；Python ctypes 加载 .so（D227）；车端镜像统一带 SDK .so（LD 路径固定）；需定 .so 版本/ABI 策略（soname + C ABI 稳定性，D109 opaque handle 奠基）；演进——嵌入式自包含需求出现加 staticlib，不预埋。修订早期双格式（.a+.so）取向。

## D241: SDK .so 版本与 C ABI 稳定策略 (2026-08-14)

**决策**:
- **soname**: `libmediaservo_<sdk>.so.<MAJOR>`（如 `libmediaservo_field.so.1`）；实体文件 `libmediaservo_field.so.<MAJOR>.<MINOR>.<PATCH>`；开发 symlink `libmediaservo_field.so`
- **MAJOR = C ABI 版本**：破坏性 ABI 变更才 bump MAJOR；MINOR/PATCH = additive/fix（不改 ABI）
- **C ABI 稳定承诺**（within MAJOR）：① 只加法——新增函数可以，既有函数签名/语义不变；② opaque handle 隐藏内部布局 → 结构体演进不破 ABI；③ C ABI 只用 C 兼容类型（无 C++/Rust 类型泄漏）；④ 需演进的结构用 version/size 字段或新增 `_v2` 函数
- **SDK semver 与 soname 对齐**：.so soname 取 SDK MAJOR
- **兼容规则**：同 MAJOR 二进制兼容（可直接换 .so 免重链）；跨 MAJOR 需重链
- **cbindgen 纪律**：只导出稳定 C ABI；Rust 内部 API 可 semver 演进，C ABI 面 within MAJOR 稳定

**原因**: ① 单动态库（D240）需明确版本/ABI 规则才能安全升级/多进程共享；② opaque handle + int 错误码（D109）天然支持 ABI 稳定；③ additive-only 与既有向后兼容纪律一致；④ Unix soname 惯例，集成方熟悉。

**影响**: 车端镜像按 soname 装载；SDK 升级 MINOR/PATCH 可直接换 .so；破坏性变更走 MAJOR + 迁移指南；cbindgen 导出面受控。

## D242: link IPC 传输底座 — iceoryx2 + FrameBus 薄层 (2026-08-14)

**决策**: link IPC 的 SHM 传输底座采用 **iceoryx2**（crates.io 0.9.3，锁版本），**FrameBus 作为其上的薄层**（latest-frame 覆盖语义、topic 命名 `camera/*`、FlatBuffers 帧元数据、ACL 强制点、能力令牌 attach）。自研仅作回退备选（非单行道）。

**Spike 实证**（/tmp/opencode/shm-spike，1080p I420=3,110,400B，两进程零拷贝，纯丢弃）：
- 零拷贝成立：iceoryx2 `loan_slice_uninit`+`write_from_slice`+`send`，订阅方 `&*sample` 读 SHM 视图
- 稳态延迟 @~100fps：iceoryx2 avg 0.98ms（min 0.32/max 2.4）；自研 avg 0.36ms（min 0.25/max 1.7）
- 吞吐：iceoryx2 burst 684 MB/s / 220fps；pacing ~300 MB/s @100fps —— 对 30fps 遥操作（33ms 帧预算）绰绰有余
- 性能非决定因素（两者都达标），选型看功能/维护

**原因**: ① iceoryx2 白给最难最易错的部分——零拷贝 SHM 管理、无锁队列、**服务发现/注册/活性**（正是 D235 的 Registry）、多订阅者 fan-out；② 契合 C11/C18"成熟方案优先"与 D235（去中心化发现）；③ C/C++/Python 绑定现成，利于 ROS/多语言节点；④ 权限本就要自建（与底座无关），FrameBus 薄层承载 ACL/令牌。

**风险与缓解**: pre-1.0 API 可能变 → 锁 0.9.3 + FrameBus 薄层隔离；引入 ~40 传递 crate → cargo-deny 审计；latest-frame 需适配（iceoryx2 默认队列/buffer → buffer_size=1 或薄层取最新）；不合适则回退自研（spike 已跑通）。

**影响**: Phase 1 link IPC 落到 iceoryx2 底座；FrameBus/Registry/ACL/能力令牌为其上薄层；iceoryx2 纳入依赖审计。

## D243: link 帧元数据 — 定长 LE + format/version，FlatBuffers 推迟 (2026-08-14)

**决策**: link IPC 的 `FrameMeta` 采用**定长 LE 编码**（seq/width/height/**format**/version/is_keyframe/ts_mono_ns/ts_epoch_ns），**FlatBuffers 推迟**（跨语言需求真正出现再上，靠 version 字段兼容演进）。

**原因**: ① YAGNI/ponytail——MVP 仅 Rust 内消费，定长 LE 最简单、零依赖、零解析开销；② 补齐审核发现的 **format（像素格式）+ version（元数据版本）** 字段（原 plan/spec 缺失，订阅方无法解释 payload、无演进位）；③ FlatBuffers 的价值在跨语言（ROS C++/py 消费，D236），届时再引入不迟；④ 此为对 D242"FlatBuffers 帧元数据"字面的**有意偏离**，按 C9 记录为决策。

**影响**: FrameMeta 定长 LE 含 format+version；跨语言 ROS 消费者按定长布局解析；未来上 FlatBuffers 时靠 version 字段平滑迁移。20-sdk-api-contract §3 与 21-link-ipc §4 同步（FrameStream 已改 latest-slot）。

## D244: SignalClient 连接面 — connect(url, psk, room_id, role) + broadcast events (2026-08-14)

**决策**: link Phase 1b 的 WS 信令客户端采用 **connect(url, psk, room_id, role) → SignalSession** 形态；
`SignalSession::{events()/send()/room_id()/close()}`，事件走 `tokio::sync::broadcast`（`SignalEvent` 派生 Clone）；
复用 common crate 的 `SignalingMessage`/`PeerRole`（与 host/client 同协议），新增 `LinkError::Signal` 变体。

**原因**: ① 单层会话型 API（D224）在连接面上的具体化——把 WS 生命周期封装为 session，用户只拿 receiver + send；②
复用 SignalingMessage 避免第三套协议（host/client/server 已用），Phase 2 信号中继可直接对接；③ broadcast 天然支持多消费者，
future（断线重连/多路订阅）无需改 API；④ connect 内完成 PSK→Error{code:0}→RoomJoin→RoomJoined 握手，失败返回明确的 LinkError。

**影响**: SignalClient 是 field/deck/client SDK 的信令基座；PSK 凭据从配置/环境注入（不硬编码）；mock WS server 测试模式
（tests/signal.rs）可复用到其他 SDK 的信令测试。

## D245: deck Recorder — FFmpeg 直依赖 + copy_parameters_from_context + µs time_base (2026-08-17)

**决策**: deck 的 Recorder 直接依赖 `ffmpeg-the-third 6.0`（与 codec 同版本，单 ffmpeg-sys 防双符号冲突），
不经过 codec 的 Encoder facade；流参数用 `copy_parameters_from_context(enc.0)`（含 SPS/PPS extradata，
手动填 codecpar 会 codec_name=unknown）；time_base 用 1/1_000_000（µs 标尺）匹配输入帧 pts 单位。

**原因**: ① ffmpeg-the-third 6.0 支持 5.1-9.0（Linux pixi=9.0, macOS=8.1 双平台），5.0 绑定编译失败——
统一升级 codec+deck 单版本，避免双 ffmpeg-sys 符号冲突（PIT-71 教训延伸）；② ctx 被 `encoder()` consume，
官方 muxing.c 的 avcodec_parameters_from_context 不可复用 → 从 open 后的上下文手动复制；③ pts 是 µs（源帧
ts_mono_ns/1000），若 time_base=1/fps 会把 µs 当 tick → duration=117s 假时长（实证 PIT 记录）。

**影响**: deck 后续 record/playback 同构（demux/解码也走 ffmpeg-the-third）；codec crate 的 encoder/decoder
facade 仍是纯编解码（不吞 mux，D229）；后续如果 playback 落地复用同版本单 FFmpeg。

## D246: reasoning_content 处理 — supportsReasoning 标注 + 分层推理控制 (2026-08-17)

**决策**: ① 全局 OpenCode provider 配置中 5 个推理模型（premium-max/-1/-2, deepseek-v4-pro, deepseek-v4-flash）`supportsReasoning: false→true`——适配器据此后将 `reasoning_content` 解析为结构化 thinking part，而非摊进 content 文本；② fast 层 8 个 agent（librarian/explore/metis/sisyphus-junior/artistry/quick/writing/unspecified-low）显式 `reasoningEffort: "low"`；③ 项目配置 `compaction: { auto: true, tail_turns: 15 }` 保留最近 15 轮原文；④ metis models 列表 premium→fast 对齐；⑤ apiKey 改 `{env:NEW_API_KEY}` 脱敏。

**原因**: 推理模型双通道输出（reasoning_content 思考 + content 答案）与对话历史只认 content 的矛盾（转换机制）；OMO 多 agent 混合模型（6 模型 1024K）放大上下文爆炸 + 格式串扰 + 错误锚定；`supportsReasoning: false` 与 OMO 层 `reasoningEffort: "high"` 配置直接矛盾（根因）；fast 层无 reasoningEffort 导致网关默认全量 thinking 输出。

**影响**: 分层策略 "展示全量、存储分离、注入限量、按模型裁剪、续写特例"；后续若 session 存储仍无 thinking part（网关侧拼接 content），下一层修网关别名配置。配置生效需重启 opencode（启动时加载）。

## D247: C ABI 符号前缀 — ms_ 改为 mediaservo_ 全名 (2026-08-18)

**决策**: 绑定矩阵三 SDK（link/deck/field）的 C ABI 符号/类型/宏前缀统一为 **mediaservo_**（如 `mediaservo_field_push_connect`、`mediaservo_err_t`、`MEDIASERVO_FIELD_ERR_*`），废弃计划中的 ms_ 前缀（D109/D241 起草形态）。

**原因**: ① 唯一性——ms_ 与微软 MS_ 宏/毫秒/大量小库冲突，dlopen 场景（D240 deck-full.so OTA 插件）下同名符号可被 LD 抢先解析（正确性/安全问题）；② 品牌自描述 + 审计友好（`nm -D | grep mediaservo_` 一键区分归属）；③ 与 libmediaservo_*.so / MEDIASERVO_* 头文件 guard / mediaservo-* crate 全链一致；④ 行业惯例全部是品牌唯一前缀（gst_/av_/iox2_/livekit_）；⑤ 执行窗口：cxx/py 层未建、link-c/deck-c 未发布——pre-1.0 唯一低成本的改名时刻。

**影响**: C++ 命名空间用全名 `mediaservo::{link,deck,field}`（命名空间无运行期成本）；Python 类名对应；0.1.x ABI 破坏性变更（未发布 MAJOR，记录在案）；契约 §7 示例/22-field-guide 已同步。**执行纪律：仓级重命名必须在所有子代理完成后再做**（PIT-98：deck-c 代理将重命名误判为污染并 git checkout 还原全仓）。

## D248: C 头文件策略 — 手工维护 + nm 漂移门禁，cbindgen 押后 (2026-08-18)

**决策**: 绑定矩阵三 SDK 的 C 头文件（bindings/c/mediaservo-<sdk>-c/include/mediaservo/<sdk>.h + 共享 common.h）**手工维护**，配 `scripts/check-abi-drift.sh`（nm/readelf 导出符号 ↔ header 声明集合对照，pixi task `abi-drift`）作为 CI 门禁。cbindgen 不引入（押后至头文件纯化为纯签名投影时再评估）。

**原因**: ① 头文件是"契约型"而非纯签名投影——含审核 R2 生命周期/线程契约注释、`#pragma pack(1)`（cbindgen packed 支持有限）、`sizeof` 初始化宏、跨 crate 共享 common.h（cbindgen 单 crate 生成需复杂 include 处理）；② 函数面小（三 SDK ~30 函数），手工维护成本低；③ 主流对照：iceoryx2 用 cbindgen（其头是纯签名投影）而 livekit C++ API 层/Arrow C Data Interface 均手工（契约型）；④ L2 漂移门禁比 cbindgen 更直接——头文件含 C 专属内容，生成+手工合并维护成本更高。修订 D227"-c（cbindgen 生成）"字面取向。

**影响**: 头文件必须随 ABI 变更 review（git diff = ABI 变更记录）；新导出函数必须同步头文件（drift 门禁强制）；cbindgen 迁移路径保留（iceoryx2 模板在 .refinfo）。

## D249: Node 绑定路线 — napi-rs 直绑 Rust（livekit 同构），非 FFI (2026-08-18)

**决策**: Node.js 绑定采用 **napi-rs 直绑 Rust SDK**（`bindings/node/rust/mediaservo-node`，napi 3 cdylib → .node），TS 薄包装层 `lib/index.mjs`。否决 koffi/ffi-napi（FFI 加载 .so）首步。

**原因**: ① livekit 实证（.refinfo/livekit-rust/livekit-ffi-node-bindings + node-sdks/livekit-rtc）：Node 生态用 napi-rs 编译 .node + 独立 TS 包装层（ffi_client/async_queue 模式），非 FFI 动态加载；② **Node 单线程事件循环使同步阻塞 C ABI 首步不可行**（connect/publish 阻塞 = 冻结进程；ctypes 在 Python 可行因多线程）；③ 与 README 技术栈承诺（napi-rs）一致；④ 绕 C ABI 分层代价被 livekit 先例接受（napi 绑 Rust ffi 层）。

**影响**: node 走 Rust async API 面（与 pyo3 二步同位置）；C ABI 面不变（C/cxx/嵌入式契约）；napi 平台二进制按目标系统编译（conda libstdc++ 6.0.35 vs 系统 6.0.30 → LD_PRELOAD 或平台编译）。

## D250: C++ 绑定完全迁移 tl::expected — C++11 硬约束裁决 (2026-08-18)

**决策**: C++ header-only 绑定（bindings/cxx）的 Result 完全迁移为 `tl::expected<T, Error>` alias（原生 API：has_value()/value()/error()/value_or()，无 operator bool，误用抛 bad_expected_access），vendor 1.2.0（CC0）至 `mediaservo/3rdparty/tl/expected.hpp`。执行计划: docs/superpowers/plans/2026-08-18-cxx-tl-expected.md（单 commit 5d0aa5c）。

**原因**: ① **C++11 起步可用为硬约束**（用户；车端嵌入式旧工具链）——手写 std::variant Result 是 C++17（-std=c++11 编译 38 错误实证），tl::expected C++11 全绿（0 错误实证）；② C++11 手写替代（aligned_storage/union）= 重造轮子（C18）；③ 零真实消费者 → source-breaking 可接受（D241 只锁 C ABI）；④ 完全迁移优于兼容层（"半迁移比不迁移糟"——integration 视角）；⑤ 未来 Jetson 编译器升级 → 一行 swap std::expected。

**影响**: 契约变更（误用异常 logic_error → bad_expected_access，catch(std::exception) 兼容）；error() on success 为标准 UB（旧手写防御抛被标准替代）；契约测试 test_result_common.cpp 为 std::expected swap 回归锚；三头+测试全站 -std=c++11 门禁。

## D251: G3 舱端分级授权 — 账号 JWT 复用 + 急停经信令强审计 (2026-08-19)

**决策**: ① 舱端操作员账号 = accounts.yaml 注册表 + POST /api/auth/login 签发 JWT
`{sub, role, vehicles, iat, exp}`，**复用 admin_jwt_secret/HS256/既有 JwtAuth 中间件**
（D-H11 选项② 最小实现）——不引入第二套签名体系；② **急停命令经信令转发**（新增
EmergencyCommand 变体）而非 P2P DC——强审计要求"谁/何时/哪个车/什么命令"全量留痕，
P2P 流量服务端不可见；底盘/云台常规控制仍走 P2P DC（协商期按角色授权 = 强制点）；
> **修正（2026-08-25, 用户决策）**: P2P 直连 NAT 不可达 → **全 SFU**——媒体+控制全部经 mediasoup；
> 控制 DC 走 SFU data 域（H1, Phase B 待办）；RoomType 仅 DeviceStream；网关 Sdp 按房间路由（无 p2p_owner）。
③ 授权矩阵为纯函数层（roles.rs）+ SessionIdentity 三态（Device/Account/Legacy），
仅账号与设备会话启用强制（PSK legacy 部署行为不变，additive）。

**原因**: ① 已有 admin JWT 体系（bootstrap token + 中间件 + /ws 子协议认证）零改造复用；
账号与 admin 同 secret 同算法 = 运维面最小；② 急停是唯一必须强审计的命令，信令转发
同时解决"多舱端并发急停可见性"与"DC 未建立时急停可用性"；③ 矩阵纯函数层使全组合
测试无 mediasoup 依赖（原生可跑），WS/SFU 强制点为薄接线。

**影响**: 错误码 4011（未知角色）/4031（授权拒绝）; audit 事件 EmergencyCommand +
AuthorizationDenied（有界 256 环形缓冲 recent() 可查）; 车端 host-agent 需处理
EmergencyCommand 转发（H 阶段 host-emergency）; 状态/告警的 operator 消费端点归 H
（当前 status_registry 仅 admin API 读）。

## D252: 应用层品牌化机制（Brand）+ 固化边界（2026-08-21）
- **决策**: 引入 `mediaservo-common::brand`（env `MEDIASERVO_BRAND` > 编译期 option_env > 默认 "mediaservo"）；host/client/server 全部用户可见串（app 名/namespace/unit/device 前缀/帮助/面板标题）走 Brand；deploy/package `--brand` 参数化（build 组装物理改名——D3/T1 落地）。默认品牌映射 **legacy 串硬映射**（host-*/oxmgr-host-/ms-/mediaservo-host）——零行为变化
- **边界**: 🔒 固化 = bindings/*（C ABI 符号 mediaservo_* D247 + cxx/py/node + soname D240）+ wire 协议（信令/SFU/FrameMeta）+ crate 名；🎨 可定制 = host/client/server 应用层
- **原因**: 基石定位（第三平台静态链接/独立部署嵌入）——白标免 fork；"数据面固化/应用层可定制"显式化
- **影响**: Brand 三语义分离（product=CLI 名/display=产品展示/id=路径）；identity 设备前缀品牌化仅新 key（G2 配发需重注）；crate 名/bindings 不动——独立发布仍走 fork+D209
- **参考**: 计划 docs/superpowers/plans/2026-08-21-app-branding-customization.md；Momus 审核（HIGH-1 默认映射表/HIGH-2 23→24/HIGH-3 admin dist 编译期机制）

## D253: oxmgr 同目录发现接线 — 安装即用，免 export PATH (2026-08-21)

- **决策**: oxmgr 查找顺序统一为「host 二进制同目录（current_exe().parent()/oxmgr，D-H13 打包于 bin/）优先 → PATH 回落」——接线到所有调用点：host.rs cmd_oxmgr/run_oxmgr_in(None)/doctor + translate.rs oxmgr_apply/delete/list。deploy 打包 oxmgr 进 bin/ 使安装目录**零配置即用**（无需 export PATH）
- **原因**: ① oxmgr_path()/oxmgr_cmd() helper 早已存在（current_exe() 模式）但 6 个调用点仍 `Command::new("oxmgr")` 纯 PATH——同目录能力"存在未接线"（触达性 bug，非设计缺失）；② 发布壳消费方（MSRTC --pure-brand 等）期望「安装即用」，PATH 是交互文档约定，不应是运行前提；③ multi-instance 下同目录绑定实例自己的 oxmgr（C32 隔离增强）
- **影响**: host start/apply/monit/ps/doctor 全部零 PATH 可用；exe 同目录无 oxmgr 时回落 PATH（向后兼容）；D-H13 打包语义兑现

## D254: Server 构建双轨化（原生主 + Docker 兜底）(2026-08-26)

- **决策**: mediaservo-server 构建从「统一 Docker」放宽为「双轨」——原生编译为主路径（开发/调试），Docker 为发布镜像与 CI 兜底。pixi 任务：`build-server-native`/`check-server-native`/`test-server-native`（原生）；`docker-cargo.sh`（Docker 兜底）。
- **原因**: ① 主流生态惯例：mediasoup 官方 Rust 绑定 = 原生 runner 进程内 meson（同架构）；livekit/Deno/Bun/Zed 均原生构建优先，Docker 仅发布；② 本机实证（T5 模式①✓）：target/server-native 有产物；③ 裸机多 IP 公告 = run server --native（PIT-143 完整路径，sfu.rs 全具体 IP）；④ Docker 兜底保留：CI 环境依赖一致性、macOS/Windows 开发者统一、发布镜像构建。
- **参考**: mediasoup-rs（原生 meson wrap runner）、livekit-server（原生构建）、Deno/Bun（原生编译优先）、Zed（原生 Rust 构建）
- **影响**: ① 开发者可用原生编译快速迭代（秒级 check vs 分钟级 Docker）；② 多 IP 公告在 run 阶段生效（sfu.rs bind 全具体 IP）；③ CI 保持 Docker 兜底（依赖一致性）；④ meson wrap 首次需联网（离线前提是首次构建成功后缓存命中）
- **来源**: 用户显式批准（2026-08-26 three-mode-build T6）+ mediasoup/livekit/Deno/Bun/Zed 调研 + T5 实证

## D255: C13 双轨化——原生主路径 + Docker 发布/CI 兜底 (2026-08-26)
- **决策**: C13 "server 统一 Docker 编译"放宽为"原生 + Docker 双轨"——`build/check/test server --native`（pixi 工具链）为主路径；Docker 保留发布镜像（`--image runtime`）与 CI 兜底（`docker-cargo.sh`）
- **理由**: 主流生态惯例（mediasoup 官方 Rust 绑定即原生 runner 进程内 meson；livekit/Deno/Bun/Zed 原生优先）+ 本机已实证（target 有 mediasoup-sys 产物；wrap .wrap 文件指向外部 URL 首次需联网，非 vendored）
- **影响**: 裸机多 IP 公告可用（多 ListenInfo 仅裸机生效）；dev 同机 host hairpin 可解；Docker 构建路径保持作为发布/CI 兜底
- **来源**: Librarian 调研（media-server 生态 + Rust 混合项目）；Momus 团队审核吸收 6 项 HIGH

## D256: 默认 native 命令模式（不带模式=原生）(2026-08-27)
- **决策**: 用户裁决 B——"不带 mode 默认 native"；进程族命令（run/start/stop/status/logs/clean/restart）默认 native，容器/镜像全显式（--mode compose/--env/--image）
- **理由**: 避免默认 Docker 的隐藏依赖；与"原生主路径"（D255）一致；主流 Rust/media-server 项目原生构建优先
- **影响**: start/status/logs 翻转（原默认容器）；run 删除 --native 死参数；clean server 双清（native 产物+容器）；build server 默认 native
- **来源**: 用户决策 + 三模式构建团队评审

## D257: install→deploy 重构（无状态 build vs 有状态 deploy 分离）(2026-08-27)
- **决策**: `install` 命令退役（隐藏改名提示 exit 2，不 alias），职责分裂：
  - 无状态组装（拷贝布局）→ 并入 `build host`（`_stage_to_out` 物理改名→ out/ 交付布局）
  - 有状态部署（identity/oxmgr/systemd/env.sh）→ 新 `deploy host --prefix`（必填，防污染 out/）
  - `package host` 走 staging deploy 组装（host tar 恢复 D-H13 契约）
- **理由**: 主流 build/install 分离惯例（cargo/npm/pip/make）；CI artifact 化（只 build→out/）；install 名在构建系统误导（语义属 Ops 阶段）
- **影响**: `_derive_brand()` 唯一品牌来源；`install` 残留调用点 7 处迁移；e2e-install-host.sh→e2e-deploy-host.sh；docs/architecture.md 3 处；D252 品牌化描述更新
- **来源**: 用户讨论 + 4 角色团队审核（arch/cli/deploy-ops/docs）+ Momus HIGH 裁决 b（host 包内容契约）

## D258: server 默认配置路径改为 bin/../etc/server.yaml（相对二进形）(2026-08-27)
- **决策**: main.rs 默认配置从 `/opt/mediaservo/etc/server.yaml`（容器绝对路径）改为 `bin/../etc/server.yaml`（相对二进形）；`build server` 组装时生成 `out/server/etc/server.yaml`（accounts/devices 相对路径）
- **理由**: 直接跑二进制不需 --config；tar.gz 解压结构自然（bin/+etc/同级）；与 host 的 out/host/bin 对称
- **影响**: 容器 Dockerfile target entrypoint 改 `/usr/local/etc/`；run server 优先 out/server/bin/；accounts/devices.docker.yaml 去 docker 后缀（dev 占位模板历史包袱）
- **来源**: 用户决策（"默认改为上级目录下的 etc/server.yaml"）+ 现场实证

## D259: accounts/devices.docker.yaml 重命名去掉 docker 后缀 (2026-08-27)
- **决策**: `config/accounts.docker.yaml` → `config/accounts.yaml`，`config/devices.docker.yaml` → `config/devices.yaml`
- **理由**: 名字"docker"是历史遗留——文件本质是 dev 占位账号/设备模板，裸机 run 也用——名字误导；容器卷内已是 accounts.yaml
- **影响**: cli.py 4 处引用 + Rust 源码注释 + config/accounts.production.yaml 注释 + server.docker.yaml 模板引用
- **来源**: 用户识别（"为什么有 docker 字段"）+ 重命名后验证 admin 200

## D260: streams 编码参数配置面补齐 — encoder_backend/bitrate_kbps/keyframe_interval (2026-08-27)

**决策**: host.yaml `streams[]` 增加 3 个可选编码参数：`encoder_backend`（auto/software/hardware/nvenc/vaapi，缺省 auto）、`bitrate_kbps`（缺省 2000）、`keyframe_interval`（GOP 秒，缺省 2）。透传链：StreamConfig → streamer CLI（--encoder-backend/--bitrate-kbps/--keyframe-interval）→ field PublishOptions/PushConfig（能力已存在：SetEncoderSelector + max_bps/keyframe）。host.yaml.template streams 段补全字段注释（codec 可选值 vp8/h264/vp9/av1 + 各 backend 平台语义 + 示例块）。

**理由**: 能力链路（field 层）早已实现但 host 配置面断点（streamer 硬编码 encoder_backend="auto"）——使用者无法配置软编/硬编/码率/GOP。

**影响**: StreamConfig +3 字段（serde Option）；host-streamer +3 CLI 参数；oxfile 命令透传（有值才追加）；27 translate 单测全绿。

**警示（PIT-156）**: Jetson 上 backend=hardware/auto 会匹配 MMAPI AV1 编码器——codec 与 backend 组合必须验证实际编码（streamer stats codec 字段）。

## D261: host 接管品牌兼容 + 定位失败中止 (2026-08-27)

**决策**: ① `find_other_instance_dir` 品牌兼容（exe basename `-agent` 后缀匹配，PIT-155）；② 接管时 old_dir 定位失败 → 中止启动（提示手动停旧实例）——防新旧实例资源竞争（相机 EBUSY/iceoryx2 死节点/SHM 断链 → web 黑屏）。

**理由**: 品牌化部署（msrtc-agent）下原 "host-agent" 硬编码导致接管路径永远失效——y 接管不杀旧进程，新 start 清 SHM + apply 与存活进程混战。

**影响**: +3 单测（官方名/品牌名/非 agent 拒绝）；接管失败路径从"静默混战"变"明确中止"。

## D262: frontend-process-split — 前端进程分离 + feature 翻转 + oxmgr 同构管理 (2026-08-31)

**决策**：① Admin SPA 移出 server 进程：`admin-dashboard` 出 default features（默认构建=纯后端 API），
产物经 `build web`→`out/server/web` 合并树，由 Caddy（deploy/caddy/Caddyfile.native/split）托管静态 +
`@api` 反代；根 Caddyfile 保持全透传=模式② ingress/回滚资产。② D-2=B：host-agent `/ws` 直连 9800
不变，Caddy 仅收敛浏览器面（二期迁 A 前置=protocol_version+url 配发）。③ server 裸机/开发进程管理
**采纳 oxmgr**（Rust 单二进制，与 host 同引擎：C32 数据目录隔离/D-H13 锁定打包/startup 三端）；
systemd 仅作开机锚点；compose 轨道无 oxmgr。④ `msrtc-server` 单二进制双角色（Phase 6 落地）：
子命令管理面 + `run`=现 main（无参回落，既有直启链零破坏）。⑤ dev 闭环：`dev web`(vite 透传) +
`start --no-web`（caddy 缺失自动降级）。⑥ `/ready` 接入 worker_alive（liveness≠readiness，PIT-167 范围）。

**理由**：管理台为一等公民服务（视频会议/远程桌面路线）；C24 重编译绑架不可持续；分离钩子已在
（cfg feature/动态 location.host/compose proxy 现成）改动面最小；oxmgr 初版"Node 包"否决系误判
（PIT-169 同源文本）实证 Rust 后反转——同引擎复用 host 全部运维资产。

**影响**：C24 收窄至模式②；`run/start/stop/restart server` 将在 Phase 6 后退役→转发（C39 同待遇）；
PIT-163~169 本轮入档；Dockerfile/entrypoint setpriv 修复模式②可构建性。

**参考**：.sisyphus/plans/frontend-process-split/（主仓，含 Momus 审核修复轮）。
