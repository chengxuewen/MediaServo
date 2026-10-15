# 25 - SDK 产物 ABI 兼容基线（方案与实施计划）

> 状态：**已裁决定稿**（2026-10-15 五项裁决见 §8；同日团队审核 docs25-abi-review 修订已应用，见 §9 审核记录；新增 §10 平台族限定）
> 触发事故：TPHMI `tp_tst_msrtcBackend` 链接失败（2026-10-15），根因见 §1。
> 本文 = 策略（长期有效）+ 实施任务表（一次性）。任务完成后 §5-T* 打勾留档，策略部分常驻。

---

## 1. 事故与根因（实测，非推测）

链接报错：

```
ld: libmediaservo_client.so: undefined reference to
    `std::__glibcxx_assert_fail(...)@GLIBCXX_3.4.30'
    `__cxa_call_terminate@CXXABI_1.3.15'
```

| 事实 | 证据 |
|---|---|
| 工具链配置**没有变** | pixi.lock 于 94cf374..2bea311 零 diff；gcc/gxx 14.4.0、libstdcxx 16.1.0 一直入锁 |
| 是**本仓构建产物的引用**，非上游档案 | 实测：LiveKit 预编译 `libwebrtc.a` 对两符号引用 = **0**；本地 conda gcc-14 现编的 `libwebrtcsys-cxx.a`（webrtc-sys C++ 桥接 wrapper）命中 = **31/29（对象计数口径：24 assert+7 terminate / 13+16）**。档案对象按链接器拉取进入 .so——新产物拉到带引用对象即上墙；旧产物未拉到，故当时 g++-11 可链 |
| 消费方链接库过旧 | `libstdc++-11-dev` 的真身 `/usr/lib/gcc/x86_64-linux-gnu/11/libstdc++.so` 版本上限 3.4.29（Debian 正统此处应为链接器脚本，此包是实物旧 ELF） |
| glibc 不是矛盾 | 产物 GLIBC 引用上限 2.28（focal 2.31 可容）。**但这是运气**：原生 gcc 编 C 用宿主机 glibc 头，构建机换 ubuntu-22/24（2.35/2.39）同一 commit 可以合法漂上去 |
| 车端隐患更大 | 产物引用了 1 个 3.4.29 + 2 个 ≥3.4.30 节点 → **focal 原生 libstdc++（3.4.28）运行期也载不动** |

**教训一句话**：配置锁住了工具链，锁不住代码碰到新符号；一次性 shim 救今天，必须有常驻基线与门禁防明天。

---

## 2. 目标 / 非目标

- **目标**：`bindings` 产物（client/deck/field .so）在**不改造宿主**的前提下，链接与加载兼容：开发机（focal, g++-9/10/11）、车端（focal 原生 libstdc++）、未来任意更新的构建机。
- **非目标**：不追求支持 glibc<2.17；不把整套工具链降级（理由见 §4 层4 注）；**不静态链接 glibc**（NSS/locale 铁律，业界禁区）。

## 3. 主流先例对齐

| 先例 | 采纳点 |
|---|---|
| Python **manylinux**（PEP 571/599/600） | .so 禁止动态链 libstdc++（静态嵌入）；只允许白名单系统库（glibc 等）；**老发行版容器构建**钉 glibc；**auditwheel 式事后扫描门禁** |
| Android **NDK**（SDK 分发指南） | SDK 用 `c++_static` 嵌 C++ 运行时，避免与宿主 shared STL 冲突 |
| Valve Steam / 游戏 SDK | C ABI 边界 + 运行时 vendoring / $ORIGIN 随包 |
| OpenSSL / gRPC-core | `.symver`/version script 导出白名单，内部符号全 local |
| **本仓已有先例（最强论据）** | `bindings/node/.../build.rs:18` 已用 `-static-libstdc++`（N6 CXXABI 翻车后的修复；但该参数未 cfg 门控，见 T9）；`bindings/c/mediaservo-deck-c/build.rs:14` 已用 `--exclude-libs,ALL`。**唯独 client-c/field 未同方处理**——T1 = 对齐自家配方，非新发明 |

## 4. 分层方案（总览图）

```
┌─────────────────────────────── 宿主：TPHMI app（g++-11/Qt5）───────────────────────────────┐
│  只用 extern "C" 接口（client.h，异常→错误码，无 C++ 类型过界）            【层1 边界，已有】  │
│        │ 链接/加载                                                                          │
│        ▼                                                                                    │
│  ┌ libmediaservo_client.so ────────────────────────────┐                                   │
│  │ [层2] 静态嵌入 libstdc++.a/libgcc（自家实现 LOCAL）  │ ← 对系统 libstdc++ 引用 = 0        │
│  │ [层3] version script：global: mediaservo_* local: *  │ ← 内部符号不可见=不可抢占          │
│  │ 动态依赖仅剩: libc/libm/libpthread/libdl…            │                                   │
│  └───────────────┬──────────────────────────────────────┘                                   │
│                  ▼                                                                           │
│  [层4] glibc ≤2.31：在 focal 构建镜像/构建机产出（唯一静态打包救不了的一条，靠环境）           │
├────────────────────────────────────────────────────────────────────────────────────────────┤
│ [层5] CI 门禁 abibaseline_check.sh：readelf/objdump/nm 三发子弹 + 旧 g++ 直链烟   （防代码漂移）   │
│ [层6] 兜底：打包带自有 libstdc++ + RUNPATH $ORIGIN（app 自身 3.4.29 问题同款解法，可选）      │
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

**为什么层2 用静态嵌而不是"降编译器到 gcc-10"**：降版只压**引用**、不解决实现混杂，且封 C++20 顶、原生老 gcc 反而用旧系统 glibc 头之外再无收益；静态嵌则引用恒零、工具链随便浮（pixi update 从此免疫）、glibc 上限由层4 单独管。manylinux/NDK 同款取舍。

## 5. 实施任务表

```mermaid
flowchart LR
  T1[T1 静态三件套<br/>build.rs/cmake] --> T3[T3 纯C边界测试]
  T1 --> T5
  T2[T2 版本脚本白名单]
  T2 --> T5[T5 CI 门禁脚本]
  T4[T4 focal 构建镜像] --> T5
  T6[T6 编译器版本闸门] --> T5
  T5 --> T7[T7 TPHMI 验收<br/>三发扫描+冒烟+记账]
  T8["T8 车端 libstdc++ 摸底<br/>(独立,可先行)"] -.app 自身 3.4.29.-> T7
```

| ID | 落点仓 | 内容 | 验收标准 | 量级 |
|---|---|---|---|---|
| **T0** | TPHMI | ~~过渡 shim~~ **已取消**（裁决 #3=B 硬等 SDK 嵌入版；到货前 TPHMI 构建红为已裁成本） | — | — |
| **T1** | MediaServo | **补齐自家既有配方**：client-c/field 套用 node 先例（`bindings/node/.../build.rs:18`）与 deck-c 先例（`build.rs:14`）：`-static-libstdc++ -static-libgcc -Wl,--exclude-libs,ALL`（cdylib 经 `cargo:rustc-cdylib-link-arg`） | `readelf --dyn-syms -W <so> \| awk '/UND/ && /GLIBCXX\|CXXABI/' \| wc -l` == **0**（与 §6 脚本同形） | 小 |
| **T2** | MediaServo | `mediaservo.map` 版本脚本：`{ global: mediaservo_*; local: *; };` 链入 | `nm -D --defined-only` 只剩公开 API；示例程序照跑 | 小 |
| **T3** | MediaServo | 边界守卫：`tests/abi_boundary.c` 用**纯 C 编译器** include 三个公开头 + 链 .so | `cc -std=c11` 编译过 = C ABI 承诺不破 | 小 |
| **T4** | MediaServo | glibc 基线：发布构建固定 ubuntu:20.04 专用 release job（优先容器化，不绑个人机；开发容器与其余 CI 不动——裁决 #4） | `objdump -T <so> \| grep -o 'GLIBC_2\.[0-9]*' \| sort -V \| tail -1` ≤ **2.31**，且 CI 机器无关复现 | 中 |
| **T5** | MediaServo | `abibaseline_check.sh` 门禁 = 三发扫描（§6，发布硬拦/日常 SOFT=1 软报）+ `g++-9/10/11` 直链烟 + dlopen 冒烟 + **扫描结果写入 manifest `abi_floor` 字段**（裁决 #5）；挂进 bindings 发布 job 尾步 | 人为引入一个 3.4.30 引用应能拦红；manifest 字段自动生成 | 小 |
| **T6** | MediaServo | pixi.toml 显式闸门：`gcc/gxx = ">=14.4,<15"`（把浮动限制在 patch 级）；纪律入文档：`pixi update` 先 `--dry-run`，lock 中编译器行 diff 视同 ABI 变更需过 T5 | `pixi update --dry-run` 输出不含 gcc 跨 major | 小 |
| **T7** | TPHMI | 收货：三发验收命令 + 全套构建直链通过 + 实机冒烟（推流/拉流，含 san 档一轮）+ `changes-1.4.1` 记账 | `build_san.sh build` 绿、RC010 冒烟照过 | 小 |
| **T8** | 实车 | 摸底车端 `libstdc++.so.6` 现版本（app 自身 g++-11 → 需 3.4.29 的悬案）；若为原生 3.4.28 → 触发层6（$ORIGIN 随包）裁决 | 拿到版本号并回写本文档 §9 | 一次性 |
| **T9** | MediaServo | 审核发现硬 bug：node 绑定 `build.rs:18-19` 的 Linux 链接参数**未加 cfg 门控**，osx/win 真编 napi 即炸；照 C 绑定补 `#[cfg(target_os="linux")]` | 声明平台矩阵内各平台 napi 构建通过（或平台显式移出声明） | 小 |

**依赖**：T1/T2 可并行先行（不依赖别的）；T5 必须在 T1/T2/T4 后；T7 依赖 SDK 新产物到货；T8/T9 独立可先行。

## 6. 门禁脚本示意（T5 核心）

```bash
#!/usr/bin/env bash
# docs/modules/25 层5：产物 ABI 基线门禁。发布通道硬拦（默认）；日常软报用 SOFT=1
set -euo pipefail
MAX_GLIBC="2.31"
fail=0
shopt -s nullglob
files=(out/bindings/lib/libmediaservo_{client,deck,field}.so.0.1.1)
[ ${#files[@]} -gt 0 ] || { echo "FAIL: 无匹配产物（bindings 输出路径/版本漂移）" >&2; exit 1; }
for so in "${files[@]}"; do
  [ -f "$so" ] || { echo "FAIL[$so]: 产物缺失" >&2; fail=1; continue; }
  # 子弹1: C++ 运行时引用清零（层2 证明）
  [ "$(readelf --dyn-syms -W "$so" | awk '/UND/ && /GLIBCXX|CXXABI/' | wc -l)" -eq 0 ] \
    || { echo "FAIL[$so]: 仍动态引用 C++ 运行时" >&2; fail=1; }
  # 子弹2: glibc 不超承诺档（层4 证明）
  g=$(objdump -T "$so" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1)
  { [ -n "$g" ] && [ "$(printf '%s\n' "$MAX_GLIBC" "$g" | sort -V | head -1)" = "$g" ]; } \
    || { echo "FAIL[$so]: glibc 需 ${g:-无法解析} > $MAX_GLIBC" >&2; fail=1; }
  # 子弹3: 导出面 ⊆ API 白名单（层3 证明）
  [ "$(nm -D --defined-only "$so" | grep -vcE ' (mediaservo_[a-z0-9_]+|_init|_fini)$')" -eq 0 ] \
    || { echo "FAIL[$so]: 存在白名单外导出符号（核对 T2 map）" >&2; fail=1; }
done
if [ "$fail" -eq 0 ]; then echo "ABI baseline OK"; fi
[ "${SOFT:-0}" = "1" ] && exit 0
exit $fail
```

## 7. 风险与回滚

| 风险 | 评估 | 缓解 |
|---|---|---|
| 双份 libstdc++ 状态（宿主动态 + SDK 静态 LOCAL） | C ABI 边界下安全（各自 new/malloc 终点同为 glibc malloc；异常不过界） | T3 守边界不破；导出白名单防抢占 |
| 体积 +~2MB/so | 可忽略（client 现 30MB） | — |
| LSan/ASan 工作流漏报 | **无漏**：SDK 内 operator_new→malloc 仍走 PLT 被拦截器接管（已推理）；实机跑一次 lusan 冒烟佐证 | T7 冒烟含 san 档一轮 |
| 交叉编译/其他平台（Jetson aarch64） | 系统工具链路线（pixi.toml Jetson 段）本就不引 conda libstdc++，静态嵌参数按其构建脚本适配即可 | T1 实现时 aarch64 分支验证 |
| T4 换 CI 镜像牵连 server/mediasoup 构建 | mediasoup worker 在 ubuntu-22+ 有依赖要求的话，仅**bindings job** 用 focal，其余不动 | 拆 job，不整仓迁移 |
| 回滚 | T1/T2 纯链接参数，revert 即回 | — |

## 8. 裁决结果（2026-10-15，adjudication-walkthrough 逐项过，用户逐项裁定）

| # | 议题 | 裁决 | 派生动作 |
|---|---|---|---|
| 1 | 支持矩阵 | **A**：focal/JetPack5 起步（glibc≥2.31 档）；JetPack4/18.04 不采，重开条件=真实客户清单 | §9 矩阵数值定稿 |
| 2 | x86 bindings 拆墙手段 | **A**：静态嵌入三件套+版本脚本（对齐 node build.rs:18 / deck-c build.rs:14 自家先例） | T1/T2 批准，SDK 侧执行 |
| 3 | 过渡期 | **B**：不建 shim，硬等 SDK 嵌入版到货 | T0 取消；等待期构建红为已裁成本 |
| 4 | glibc 轴构建环境 | **A**：发布走 focal 档构建通道（优先容器化 release job，不绑个人机），日常 CI 不动 | T4 范围定稿 |
| 5 | 门禁硬度 | **A**：发布通道硬拦 + 日常软报 + manifest abi_floor 字段 | T5 形态定稿 |
| 6 | 平台声明与现实对齐（第二批 #1） | **A·挂账 SDK 侧随批实现**：README win-64 句撤/降级"规划中"；T9 补 node cfg 门控；osx 声明注记"仅编译级真话，不入承诺" | 随 T1/T2 同波执行（§10.5 已列）；本会话不动 SDK 代码 |
| 7 | 交付载体（第二批 #2） | **A·提交进仓，不推外网** | 本文件即交付物；同步授权主仓 build_san 整改+D3 记忆分笔提交 |
| 8 | T8 车端摸底时机（第二批 #3） | **B·挂账**：下次实车会话顺路查一条命令 | 层6 保留待裁；§9"待 T8 填入"不变 |

T6（编译器大版本闸门 + pixi update 纪律）未列为独立议题，作为实施附带项无异议处理。

## 9. 现场账（滚动更新）

- 2026-10-15 事故实测：client.so 引用分布 78 个（**75 古老** / 1×3.4.29 / 1×3.4.30 / 1×CXXABI_1.3.15）；glibc ≤2.28；conda gcc-14.4.0 + libstdcxx-16.1.0（pixi.lock 9-29 定稿后编译器/运行时锁定行零漂移）。
- 消费端环境事实：focal 20.04；系统运行时被人手动升至 libstdc++ 3.4.35（2026-03-17）；`libstdc++-11-dev` 的 .so 为实物旧 ELF（3.4.29 上限）非链接器脚本 —— 这就是"本机运行没问题、链接就炸"的原因。
- 车端 libstdc++ 版本：**待 T8 填入**。
- 实施进度：**两批裁决共 8 项定案（§8）；本文档已按 #7 提交交付。待 SDK 侧执行 T1/T2/T5/T6/T9 + 声明面缩窄（#6）；TPHMI 侧 T7 到货后验收；T8 挂账（#8）**。
- 2026-10-15 second-round analysis (abi-doc25-v2, 4 seats + lead): 2 CRITICAL / 5 HIGH / 10 MED clustered into §11; rulings #9-#11 (A/A/A: doc25→English / FFmpeg carry-in-package+$ORIGIN / guard LC_ALL fix + all-gates hard wiring); tasks extended T10-T15. §6 gate as drafted is blind to the incident class it was written for (C1) — do NOT implement T5 before T12 scope/bullet fixes land.
- 支持矩阵定稿（裁决 #1 派生）：x86 裸机族 = Ubuntu 20.04+（glibc≤2.31；T1 后 C++ 运行时引用=0）；Jetson 族 = JetPack5/L4T35（系统工具链，现状即达标）；server 族 = 容器自含（2.35 封箱内）。重开条件 = 18.04/JetPack4 客户清单出现。
- 团队审核记录（docs25-abi-review，2026-10-15，3 成员只读）：doc-auditor 1C/2H/6M/3L/4INFO；evidence-verifier 21 断言仓库可证项全 CONFIRMED（1 措辞已修，2 条决策史/叙述项不可机验）；platform-abi-researcher 产出跨平台矩阵（→§10）+发现 T9。本修订轮已应用：§6 脚本 exit 出循环（CRITICAL）/古老计数 76→75/T0 残句清除/mermaid 两处/T4 陈旧选项/T5 补 manifest abi_floor/新增 T9。
- 2026-10-15 跨会话对账（另会话结论审阅）：一致项——反对降工具链、"底限由引用而非编译器年龄决定"、msrtc-server 无 libstdc++ 需求、门禁+声明面治理、"20.04 解法=静态嵌 libstdc++"；修正项——墙归因（实测 `libwebrtcsys-cxx.a` 31/29 命中 vs `libwebrtc.a` 0 命中，本仓可改）；不采项——"≥22.04 底限声明"及其"YAGNI 等老 OS 客户"（车端就是现存的 focal 客户）。
- 2026-10-15 跨会话对账·二（Jetson 会话"构建机 OS 定地板"结论）：采纳——支持矩阵是根决策、package 时 objdump 扫地板写 manifest 字段、"conda C++ 运行时永不入裸机交付 NEEDED"三条纪律；修正——"gcc 版本与地板几乎无关"仅对系统工具链路线（C23）成立，conda 路线上 libstdc++ 代别恰由 conda 编译器决定（本事故即反例）；该会话实测矩阵缺 x86 bindings 行（其视域 Jetson），x86 族处方=本文 T1（嵌）或 focal 分箱（系统侧），手段二选一、由同一条门禁执法。三会话共识收敛点：地板=产物属性、按交付族登记、CI 扫描生成、禁止裸机 NEEDED 出现 conda 运行时。

---

## 11. 第二轮团队分析与追加裁决（2026-10-15，abi-doc25-v2 四席 + lead 亲验；裁决 #9-#11 逐项过）

> 本节约按 C51 用英文书写（新增内容规则）。四席 = build-reality / abi-strategy / consumer-ops / governance，全部磁盘实证（宿主无 binutils → pixi 内复测，防全零假干净）。

### 11.1 Findings clusters (dedup across seats + lead)

| ID | Sev | Cluster | Seats | Disposition |
|---|---|---|---|---|
| C1 | CRITICAL | deck.so/field.so ship a dynamic conda-FFmpeg closure (`DT_NEEDED libavcodec.so.63…`) + absolute conda `DT_RPATH`; focal target lacks them → load dies; all three §6 bullets pass it green today (gate blind to the exact failure class this doc exists for) | abi-strategy ABI-1/2/3, build-reality | ruling #10 (carry-in-package + 4th bullet) |
| C2 | CRITICAL | check-english-only.sh locale-dependent silent zero-match (LANG unset → grep -P CJK finds nothing): doc25 itself = live C51 violation (123 CJK lines, post-ratification Chinese artifact + Chinese commit msg), guard passed it; guard also not wired into any CI gate | governance GOV-1/2/11, lead live-repro (bare FAIL 123 vs pixi PASS) | ruling #9 (convert) + #11 (fix+wire) |
| H1 | HIGH | family delivery contracts contradict across 3 sources (link.h §6.6 "never bundle, host provides ≥1.8" vs deck/field bundling 57/64 MB vs C50 no-rolling): consumer cannot read "what must the target have" | governance GOV-3/4 | T13 unified matrix table into this doc |
| H2 | HIGH | §9 matrix "Ubuntu 20.04+" overclaims for fleet ops — subject is SDK .so only; the g++-11 app itself needs GLIBCXX_3.4.29 > pristine focal 3.4.28 | consumer-ops CO-1 | wording caveat in §9 (T13 same batch) |
| H3 | HIGH | T7 acceptance on the hand-upgraded dev box (runtime 3.4.35 + stale 11-dev ELF) can false-green a partial fix; authoritative acceptance = pristine ubuntu:20.04 container checklist | consumer-ops CO-2 | T10 acceptance spec |
| H4 | HIGH | gate scope holes: link.so omitted entirely (and trips bullet3 via `__internal_default_logger` today); no RPATH/NEEDED-closure bullet | build-reality BR-2, abi-strategy ABI-2/3 | T11/T12 |
| H5 | HIGH | bump-time breakage: gate hardcodes `.so.0.1.1` + `libavcodec.so.63` while C43③ bump checklist lacks pixi.lock/FFmpeg-soname lines → version bump fails the gate silently from the bumper's POV | build-reality BR-6/7/8, governance GOV-5 | derive all versions in gate + C43③ row (T10) |
| M | MED | ① shared version-script swallows field's bare `mediaservo_last_error` (per-so maps or explicit export); ② client∩field share 329 `cxxbridge1$*` exports → T2 is T1's HARD prerequisite (ODR), §5 dep note wrong; ③ bullet2 regex needs GLIBC_ anchor; ④ keep version node anonymous (@@VER suffix false-red); ⑤ webrtc-sys 52 extern-C seams, 0 catch_unwind (worker panic = host abort); ⑥ glibc ceiling actually comes from conda sysroot (2.28) — pin `sysroot_linux-64` (T6 incomplete), T4 container partly redundant, manifest sysroot field; ⑦ $ORIGIN runtime bundling = ops action not ruling-3 violation but useless for the LINK-time wall + masks gate evidence — trigger = T8; ⑧ T8 probe one-liner + 3-branch decision tree; ⑨ §1 root-cause wording imprecise (versioned UND lives on the .so; fetch-time happens at link) + T7 depends on TPHMI-local history (changes-1.4.1/san recipes) not in this repo → T14 self-contained acceptance; ⑩ C47's promised "symbol table patrol" materializes exactly as check-abi-drift + this gate — name it gate-abi-baseline.sh, wire beside G3 in all-gates quick (governance GOV-8, C50 rule 5 lineage) | abi-strategy, consumer-ops, governance | folded into T10-T14 + §5/§6/§7 fixes at revision batch |

### 11.2 Additional rulings (adjudication-walkthrough one-by-one, 2026-10-15, replies A/A/A)

| # | Topic | Ruling | Derived actions |
|---|---|---|---|
| 9 | doc25 language under C51 (created post-ratification, 123 CJK lines, corpus otherwise English since 2bea311) | **A: convert to English** in the revision batch; git-history commit stays (C51 history exemption) | T15 translation ticket (batch leader); guard wiring sequenced AFTER conversion so first run is green |
| 10 | FFmpeg closure delivery for deck/field .so | **A: carry in package + $ORIGIN runpath** (wheel precedent adopted for the .so surface; §7 "部署包必须携带" finally implemented); B strip-heavy re-ruling rejected (reopens C44/N7), C host-provides rejected (violates §2 no-host-modification goal) | T11 staging rewrite + T12 gate bullet 4 (NEEDED closure whitelist: libav*/libsw* allowed only when present under package-relative path; absolute build-machine RPATH = red); link.so added to scope (T12) |
| 11 | guard fix + duty | **A: LC_ALL force + wire into all-gates quick, hard block** (after #9 conversion); B manual-run (proven blind today) and C soft-report rejected | T10: guard script env line + all-gates entry; C51 first live enforcement |

### 11.3 Extended task ledger (T10-T15, all in the execution batch this doc awaits)

- **T10** guard/acceptance hardening: check-english-only LC_ALL=C.UTF-8 + all-gates wiring; T7 authoritative acceptance = pristine ubuntu:20.04 container checklist (pristine proof → bullets → cc/g++ link → dlopen → RC010+san → evidence back to §9).
- **T11** staging runpath rewrite: patchelf/`--set-rpath $ORIGIN:$ORIGIN/../lib` on all four .so + FFmpeg libs laid into package lib/ (ruling #10).
- **T12** gate expansion: scope = link/client/deck/field; bullet 4 = recursive NEEDED closure + RPATH whitelist; bullet2 GLIBC_ anchor; bullet3 keep anonymous version node; version strings derived (_crate_version / readelf), never literal.
- **T13** unified delivery matrix table (family × FFmpeg posture × min glibc × libstdc++ posture × version promise) into §9; fixes H1/H2; single source for all consumers.
- **T14** §5/§7 text corrections: T2-as-T1-prerequisite (cxxbridge ODR note), per-so maps (`mediaservo_last_error`), sysroot pin into T6, C43③ bump rows (pixi.lock + FFmpeg soname), §1 mechanism wording, webrtc-sys catch_unwind debt noted (fix另案).
- **T15** doc25 full English conversion (ruling #9), before gate wiring per sequencing.

### 11.4 Lead cross-check record

- Guard live contradiction reproduced same session: bare shell FAIL (doc25:123) vs pixi PASS — locale root cause; every historical "guard PASS" quoted from pixi contexts re-judged as unproven (this session's morning ledger line included, corrected here).
- Cross-doc keyword sweep (GLIBCXX/glibc/2.31/focal/22.04): contradictions captured as GOV-3 (link.h §6.6) and GOV-4 (wheel doc silence) — both routed to T13/T15 batch.


---

## 10. 平台族 ABI 限定（macOS / iOS / Android / Windows / Node）

> 触发条件：任一非 Linux 平台进入交付清单（§8 裁决只裁了 Linux 数值）。本章 = 设计时约束预登记 + 启用时门禁配方；继承 §9 共识：**地板=产物属性、按族登记、CI 扫描、裸机禁 conda 运行时**。全文底稿：platform-abi-researcher 交付（/tmp/opencode/abi/platform-abi-matrix.md）。

### 10.0 现实核查（REAL vs PAPER）

| 平台/面 | 状态 | 证据 |
|---|---|---|
| linux-64 / aarch64 | **REAL**（本文 §1-§9 全族） | pixi.toml:4；§5 T1–T9 |
| osx-64 / osx-arm64 | **半真**：pixi 已声明，C/C++/Py build.rs 有 `#[cfg(target_os="linux")]` 门控，**node 未门控（T9）**；无 mac CI | pixi.toml:4；bindings/c/*/build.rs |
| win-64 | **PAPER**：README:85 称 "Windows best-effort (pixi platforms must include win-64)"，而 pixi.toml **无** win-64——先修矛盾（文档降级或补平台） | README:84-85 vs pixi:4 |
| iOS | **PAPER**（全仓零证据） | — |
| Android | **PAPER/规划**：04-sdk-layers L112/132 = Phase 3 "Android JNI" | doc 04 |
| Node napi | **REAL 但 Linux-only**（doc23 L202-209 实测全在 Linux） | bindings/node |

### 10.1 概念映射（glibc/GLIBCXX 的他平台对应物）

| | macOS | iOS | Android | Win-MSVC | Win-MinGW | Node |
|---|---|---|---|---|---|---|
| C++ 运行时地板 | 系统 libc++.1.dylib，**无版本符号**；靠 availability 注解按 deployment target 拦 | 同 mac | NDK libc++，按 API level 门控 | msvcp140.dll：**v14.x 全家族 ABI 兼容**，redist≥构建版即可 | **同 Linux**（libstdc++-6.dll 带 GLIBCXX tag） | 不适用（只经 node_api.h） |
| C 地板 | minos（LC_BUILD_VERSION） | 同左 | bionic API level | UCRT+vcruntime140；OS 面=Win10/11 | msvcrt/ucrt | 宿主决定 |
| 钉定手段 | `MACOSX_DEPLOYMENT_TARGET` | `IPHONEOS_DEPLOYMENT_TARGET` | `minSdkVersion`+`-DANDROID_PLATFORM`+stubs | /MD+redist 或 /MT | 同 Linux 三件套 | `NAPI_VERSION` |

### 10.2 静态嵌入教义的逐平台正确翻译（防盲移核心表）

| Linux 层 | macOS/iOS | Windows | Android |
|---|---|---|---|
| 层2 嵌 C++ 运行时 | **不嵌**——Apple 只有一份前向兼容 libc++；压 deployment target（建议 minos=11.0 = Rust ARM64 下限 = conda-forge 基础设施；更老待客户清单）。`-static-libstdc++` 在 clang 上根本不存在；**禁 bundle conda libcxx** | /MD+app-local 三件套随包（默认）**或** /MT 内嵌（无 redist 环境变体；C ABI+同侧分配纪律下双 CRT 可容忍，§7 同理）；禁 debug CRT | `APP_STL=c++_static`（单 .so 交付；多 .so 才 c++_shared+libc++_shared.so 随包）；minSdk 建议 24（裁决项）；Rust 直构 cdylib 的 STL 策略需实测 |
| 层3 导出白名单 | `-exported_symbols_list` / visibility | `/EXPORT` 或 .def | ELF 原样（version-script/exclude-libs） |
| 层4 老构建环境 | **无需容器**：deployment target 一个 env，环境无关（clang 吃 SDK 注解） | **无需容器**：redist≥构建版；`vc_redist` 版本入 manifest | **无需容器**：ANDROID_PLATFORM+stubs 链接期强制 |
| install_name/搜索 | `@rpath/libmediaservo_X.<maj>.dylib` + `-rpath @loader_path`，交付时 install_name_tool 改写 | DLL 搜索序：app-local 优先；白名单=导入表 | RUNPATH `$ORIGIN` 可用 |
| 门禁扫描 | `otool -l`(minos)/`otool -L` 白名单/`nm -u` | `objdump -p` 导入表 DLL 白名单 | `llvm-readelf -d` 无 libc++_shared.so | napi: `engines.node>=14`（napi6） |
| FFmpeg/GStreamer 依赖 | 开放裁决 O2：delocate 式全量 vendoring+路径重写 vs 砍 conda 改 AVFoundation | conda 不可用；vcpkg/自交叉 | 自交叉或裁剪 | iOS 同 mac，conda 不存在 |

一句话：Linux 的"静态嵌+老环境"两件套，在 mac/Android/Win 分别塌缩为**"系统单一 libc++" / "NDK c++_static" / "v14.x 家族承诺"**——机制不同，目标同：**裸机 NEEDED 零第三方 C++ 运行时**。

### 10.3 Node/napi 专项

napi6 → Node≥14（版本矩阵）；跨 major ABI 承诺**仅当只用 node_api.h**（碰 V8/uv/node.h 即不保证）；OS 规则全部叠加于 .node；分发形态 = napi-rs 多 triple 预编译（现状手工单 Linux）；Electron 支持矩阵 = 开放项。

### 10.4 开放裁决项（启用各平台时再裁，勿预支）

O1 mac minos 数值｜O2 mac 依赖路线（vendoring vs AVFoundation）｜O3 Android minSdk｜O4 Win 主/次族（MSVC 主案已列，MinGW best-effort；先修 README:85/pixi 矛盾）｜O5 Node 分发形态（预编译 triple vs 源码编）｜O6 Electron 矩阵。

### 10.5 本轮新增/派生任务

- **T9**（已入 §5 表）：node build.rs Linux flag 未 cfg 门控——osx/win 真编即炸，属声明面与实现的矛盾点，优先修（小刀）。
- README:85 与 pixi.toml:4 的 win-64 矛盾：二选一（文档降级措辞 or 补平台声明），随 T9 顺路裁。
- mediaservoConfig.cmake.in 的 WIN32/APPLE 分支目前是纸面代码：各平台启用时，以"被真实消费+构建通过"为验收面。
