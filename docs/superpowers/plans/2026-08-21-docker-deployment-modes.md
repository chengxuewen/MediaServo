# 2026-08-21 docker-deployment-modes — Docker 部署形态改造（单容器 + 命名卷 + prod 参考）

**状态**: 计划待审核 | **范围修正**: 现有 Dockerfile 已有多阶段（base/dev/builder/runtime + USER + HEALTHCHECK）——本计划聚焦**缺口**：数据卷规范化、prod 参考编排、镜像瘦身审计

---

## 1. Proposal

### What

完成 MediaServo Docker 部署形态标准化（参考 3rdparty/AccessBase 的构建工程）：① **数据卷命名卷化**（bind-mount 单文件退役——C34 EBUSY 根治；配置模板首次启动初始化）② **prod compose 参考样例**（单镜像 + 命名卷——AccessBase prod 同构，第三方集成交付物）③ **镜像瘦身审计**（builder strip + runtime apt 依赖精简——17GB 镜像瘦身）。运行形态不变：**单容器 all-in-one**（server + SFU + admin 嵌入；无 DB——YAGNI）。

### Why

- C34 教训：bind-mount 单文件（devices.yaml）rename=EBUSY——原子写失效被迫回退直接写；命名卷内为普通文件系统——原子写恢复
- 生产/第三方集成缺参考编排（dev compose 有 server+caddy 雏形——无 prod 单镜像样例）
- 镜像 17GB（system df Total——**dev 工具链镜像**；`--target runtime` 交付镜像预期 ~200-400MB）——strip 与 libssl3 审计（AccessBase runtime 精简参考）——**体积基线以 runtime 为准**

### Scope

**In scope**:
- compose volumes 命名卷（`mediaservo-data` → 容器 `/opt/mediaservo/etc` + `mediaservo-recordings`）
- 启动初始化：命名卷首次挂载时配置模板落位（entrypoint 或 server 内置 fallback——server.yaml/accounts.yaml/devices.yaml 缺省生成）
- `docker-compose.prod.yml`：单镜像（`mediaservo-server:latest`）+ 命名卷 + healthcheck + 资源限制（AccessBase prod 同构）
- 镜像瘦身：builder release 加 strip；runtime 基线 = `--target runtime`（~200-400MB——dev 9GB 含工具链非交付物）
- 验证矩阵（体积/构建缓存命中/功能回归——**全部从 mediaservo.sh 入口**）
- **mediaservo.sh 命令面重构**（用户决策：不背兼容——主流 CLI 设计：up/down 部署生命周期、--env 全局环境、e2e 子套件化、data 域、doctor 统一）

**Out of scope**:
- 数据库引入（YAGNI——26-docker-deployment-modes.md 演进路径文档化）
- dev compose 热更机制（保持——源码 bind-mount + cargo-cache 卷）
- caddy proxy 启用（保持未启用——可选）
- 前端 docker 化构建（产物已嵌入——无必要，见 26 文档）

### Risks

1. **命名卷初始化**：首次 `docker run -v mediaservo-data` 卷为空——**现状**：缺失 server.yaml 不 fail-fast（回落 DEFAULT_SERVER_CONFIG——**硬编码已知 psk**）；缺失 accounts/devices 容忍（空表）——**init 的目的 = 密钥配发（消除已知 PSK 默认）+ 持久化语义**，非解除 fail-fast；初始化优先级：卷已有文件 > 模板；**prod 模板禁止 dev 占位（I2 守卫拒启）**；**entrypoint 自举保证 server.yaml 必生成（缺失 exit 1——容器内不依赖 DEFAULT fallback——sec F6）**
2. **配置热更回归**：dev compose 从 bind-mount 转命名卷会破坏"改 config 即生效"（卷不共享宿主文件）——**dev compose 保留 bind-mount**（生产才用命名卷）——双轨：dev 文件挂载 + prod 命名卷
3. **镜像瘦身误删**：runtime apt 包删错 → 运行期缺库（libav/ssl）——审计须对照 ldd
4. **设备配发 EBUSY 回归**：命名卷下 rename 原子写应恢复——save() 回退逻辑保留（无害兜底）——验证点

### Success Criteria

- [ ] 根 compose（单一 prod 入口）：空卷首启（entrypoint 自举——**日志 `psk_set=true` 认证闭合**）→ admin 200 + admin 引导登录 + 配发原子写（无 EBUSY fallback）+ 卷持久化（重建保留）+ **旧 bind-mount 迁移演练（devices/账号保留）**
- [ ] dev compose 行为不变（bind-mount 热更 + cargo-cache 卷）
- [ ] 镜像体积：`--target runtime` 基线 <400MB（strip 后；dev 工具链镜像不列为交付物）
- [ ] 功能回归：e2e_sfu 4/4 + push_e2e 6/6 + admin Playwright（设备配发 spec）+ 构建缓存命中验证（改源码不重拉依赖）
- [ ] `docs/modules/26-docker-deployment-modes.md` 已落盘（设计依据）+ 记忆沉淀
- [ ] **mediaservo.sh 命令面**: `up --env prod`（单容器+命名卷+初始化）/ `up --env dev`（热更）/ `down` / `ps` / `logs [svc] --follow` / `exec <svc> -- cmd` / `e2e <suite>`（sfu|push|ui|host|package|brand|bindings|smoke）/ `data backup|reset|inspect` / `doctor` / `config set`——**全部部署/验证命令无裸 docker**；`install/package/clean/test/ci/version` 保留（e2e 脚本直调依赖）

### References

- `3rdparty/AccessBase/Dockerfile` + `docker-compose.dev.yml` + `docker-compose.prod.yml`（构建工程参考）
- C13/C24/C34 + PIT-122/123/124 + `docs/modules/26-docker-deployment-modes.md`

---

## 2. Design

### Architecture

```
生产（单容器）:
  docker run -v mediaservo-data:/opt/mediaservo/etc -v mediaservo-recordings:/opt/mediaservo/recordings \
    mediaservo-server:latest
  └─ entrypoint: 卷空 → 拷贝配置模板（server.yaml/accounts.yaml/devices.yaml 缺省）
                卷有 → 直接用（持久化）
  └─ server: yaml 加载（accounts/devices）→ 9800 + SFU
       └─ 设备配发: save() 原子写（命名卷普通文件——rename OK——EBUSY 兜底保留）

dev（保持）: bind-mount config/ + cargo-cache 卷（热更/调试不变）

第三方参考（docker-compose.prod.yml）:
  server: mediaservo-server:latest + volumes + healthcheck + resources
```

### Files to Touch

| 操作 | 文件 | 目的 |
|---|---|---|
| Modify | `docker-compose.dev.yml` | server volumes：devices.yaml 挂载保留（dev 双轨）；**不动**（bind-mount 热更）——仅加 volumes 注释 |
| Modify | `docker-compose.yml`（根——**已是 prod 单镜像参考**，勿并行新建）| bind-mount → 命名卷（mediaservo-data/recordings）+ entrypoint 语义——单一 prod 入口 |
| Create | `entrypoint.sh`（容器内） | 命名卷初始化：缺失模板 → 拷贝内置模板（模板 include 进镜像 /opt/mediaservo/templates/） |
| Modify | `Dockerfile`（runtime stage） | ① COPY entrypoint + 模板目录 ② ENTRYPOINT（**替换**现有 169 行——exec-form 保 PID-1）③ `RUN mkdir -p /opt/mediaservo/{etc,recordings,templates} && chown -R mediaservo:`（USER 前——命名卷 copy-up 保留属主——空卷 EACCES 根治）④ apt 审计（ldd 验 libssl3） |
| Modify | `Dockerfile`（builder stage） | release 构建后 `strip` 二进制（镜像瘦身主力——同 PIT-119 经验） |
| Modify | `crates/mediaservo-server/src/main.rs`（如需要）| server.yaml 缺失时的 fallback 语义（若 entrypoint 覆盖则不动） |
| Modify | `scripts/mediaservo_cli.py`（argparse 重构）| 目标命令面：up/down/ps/logs/exec/e2e 子套件/data/doctor/config set/--env 全局选项（不背兼容——一次性重写） |

### Data Flow

1. **首次启动（空卷）**: `docker run -v mediaservo-data` → entrypoint 检测 `/opt/mediaservo/etc/server.yaml` 缺失 → 从 `/opt/mediaservo/templates/*.yaml` 拷贝（含 dev 占位账号/空设备表）→ exec server → 9800 就绪
2. **后续启动（卷有数据）**: entrypoint 跳过拷贝 → server 用卷内配置（升级镜像配置不覆盖——持久化语义）
3. **设备配发**: POST /devices → save() rename（命名卷普通文件——**原子写恢复**——EBUSY 回退保留兜底）

### Error Handling

- entrypoint 拷贝失败（卷只读/权限）：stderr 明确 + exit 1（C15——不静默）
- 卷半初始化（server.yaml 在、accounts 缺）：按文件级拷贝（逐文件判断缺失）——不整目录覆盖
- 镜像瘦身：strip 失败不阻断（warn）；apt 删除前 `ldd /usr/local/bin/mediaservo-server` 对照（漏删 → 运行期报错——验证矩阵覆盖）

### Testing Strategy

- 构建：`docker compose -f docker-compose.prod.yml build` 体积对比（before/after）
- 部署冒烟：空卷首次启动 → 配置初始化 → admin 200 → 设备配发（rename 原子写验证——日志无 EBUSY fallback）→ 容器重建（卷保留——配置/设备持久）
- dev 回归：dev compose 热更行为不变（改 config 生效）
- 功能：e2e_sfu 4/4 + push_e2e 6/6 + Playwright 设备配发 spec
- 构建缓存：`docker compose build` 二次（无源码变更）缓存命中（依赖层不重建）

### Dependencies

无新依赖。Dockerfile/compose/entrypoint 纯构建面改动。

---

## 3. Tasks

## Phase 1: runtime 镜像基线 + 缓存分层（实证驱动——非假设杠杆）

- [ ] **runtime 基线测量（现状从未构建过——build-reviewer 实证）**
  - File: 无（构建/记录）
  - 内容: `docker build --target runtime`（当前唯一未构建的交付 stage）→ 记录实际体积（预期 150-200MB：ubuntu 78MB + curl + mediasoup 静态二进制）+ `ldd` 全量对照基镜像 .so 清单（**libuv1 是真项**——meson 可能动态命中；libssl3 基镜像已有——no-op 不预设）
  - Verify: 基线数字回填 criterion（口径统一：`docker images` 单镜像 <400MB——设计 §5 同步改）+ 容器启动全功能 + `ldd` 无 not found
  - 注: **strip 任务删除**——root Cargo.toml `[profile.release] strip=true` 已有（build-reviewer 实证——PIT-119 是 debug 语境非此场景）；仅加 verify 确认 strip 生效

- [ ] **builder node 工具链层缓存修正**
  - File: `Dockerfile`（builder stage——node 安装在 `COPY . .` 之后导致每次源码变更重跑网络安装）
  - 内容: nodesource+pnpm 安装移到 COPY 前 + `pnpm install` 拆到 www 清单 COPY 层后（cargo 依赖缓存同构）
  - Verify: 二次 build（仅改 Rust 源码）node 层缓存命中（日志无 nodesource 重装）

## Phase 2: 命名卷 + entrypoint 自举初始化（团队审核 C1/C2/C5/H8 吸收）

- [ ] **entrypoint 自举（弃 env_file——compose 容器创建时宿主侧解析读不到卷内 .env；signaling.rs env 缺失默认放行）**
  - File: `Dockerfile`（COPY entrypoint.sh + templates/）+ `entrypoint.sh`
  - 内容（顺序即安全契约）:
    ① `USER root` → entrypoint（root 写卷）→ 末尾 `exec su-exec mediaservo mediaservo-server`（降权保 PID-1 信号）
    ② **密钥单源生成**（一次生成——accounts.rs fail-fast 要求两 secret 一致）: 随机 ≥32B → server.yaml `jwt_secret`+`admin_jwt_secret` **同值**写入卷 + PSK 写卷 `.env`（chmod 600）→ `set -a; . .env; set +a` → **进程环境注入**（非 compose env_file——WS 认证只读 env 的路径闭合）
    ③ 模板: **prod = `accounts.production.yaml`（accounts:{}）+ 空 devices + server.yaml 无任何 secret 字段**（绝不拷 dev 占位——I2 拒启）；逐文件缺失拷贝；**先写 .env（原子 tmp+rename）再拷模板**；任一缺失未完成 → exit 1 不启 server（首启幂等——ops H5）
    ④ **首启 admin 引导**（空账号表无法登录——配发冒烟可达性）: `MEDIASERVO_ADMIN_PASSWORD` env 首启创建管理员（stdout 一次性提示）或 `exec <svc> -- admin create` 子命令
    ⑤ 失败 exit 1（C15）
  - Verify: 空卷首启 → 配置生成（无 dev 占位）→ **日志 `psk_set=true`（认证已闭合——非默认放行）** → admin 200 → 登录成功（引导账号）→ 二次启动不覆盖；重启容器 PSK 稳定（卷 .env 持久）

- [ ] **设备配发原子写恢复验证**
  - Verify: 命名卷下 POST /devices → save() rename 成功（日志无 "falling back"——EBUSY 兜底未触发）

## Phase 3: prod compose 迁移（单一入口 + 升级路径——ops C3/H4/arch M8 吸收）

- [ ] **根 docker-compose.yml 迁移 + 既有部署升级路径**
  - File: `docker-compose.yml`（改造——单一 prod 入口）
  - 内容: ① bind-mount → 命名卷（`mediaservo-data`/`mediaservo-recordings`——**去 `:ro`**：现 devices.yaml 挂 `:ro` 写入必败）+ healthcheck/resources/restart: unless-stopped（server 服务现缺——ops L15）② **image 名修正**（现占位 `ghcr.io/org/mediaservo-server:latest`）+ **双 tag pin**（`<ver>`+latest + `image: mediaservo-server:${TAG:-latest}`——回滚路径）③ **显式迁移步骤**（既有 bind-mount 部署升级: `docker run --rm -v ./config:/src -v mediaservo-data:/dst ... cp` 迁移 devices/账号 → entrypoint 跳过生成——设备表/PSK 保留）④ 移除硬编码 `MEDIASERVO_PSK`（C1 已由 entrypoint 自举闭合）⑤ RUST_LOG: debug → info ⑥ **caddy/proxy 表态**（现已启用 80/443——prod 保留为可选 ingress——计划明示）
  - Verify: `docker compose config --quiet` + 空卷冒烟（admin 200 + 认证闭合 + 配发原子写）+ **迁移演练**（旧 bind-mount 配置 → 命名卷：devices/账号保留断言）+ 回滚演练（旧 tag 重启——卷配置兼容）

- [ ] **dev 轨道 C34 根治（bind-mount 单文件持续 EBUSY——arch M6）**
  - File: `docker-compose.dev.yml`
  - 内容: `./config/devices.docker.yaml` 单文件挂载 → **挂目录** `./config:/opt/mediaservo/etc`（热更保留 + rename 原子写恢复——dev 不再走 save() 直写回退）
  - Verify: dev 配发无 "falling back to direct write" 日志

## Phase 4: 命令面重构 + 验证回归（全部从 mediaservo.sh 入口）

- [ ] **mediaservo.sh 命令面重构（argparse 重写——目标命令表）**
  - File: `scripts/mediaservo_cli.py`
  - 内容: ① `up/down [--env dev|prod] [<svc>]`（部署生命周期——**--env 为子命令级参数**（cli M4）；prod=根 compose 命名卷；dev=热更）② `restart` **host 进程专用**（删除部署 restart——cli H1 矛盾修正；host 无 --env）③ `ps`/`logs [<svc>] --follow`/`exec <svc> -- cmd`（运行态）④ `e2e <suite>` 子套件——**suite→执行映射表**（cli H3）: `sfu`→`cargo test -p mediaservo-host --test e2e_sfu`、`push`→`cargo test -p mediaservo-field --test push_e2e`、`ui`→Playwright（device-provisioning 等）、`host`→e2e-install-host.sh、`package`→e2e-package.sh、`brand`→e2e-brand.sh、`bindings`→e2e-bindings.sh（其 :19 `start server` → `up --env dev server`）、`client`→e2e-test.sh（macOS 9/9——I4 环境阻塞可跳过）、`smoke`→Phase 4 生产冒烟（**原 verify prod 幽灵引用消除**——计划内唯一定义）——**`_cmd_ci()` 末尾 `_cmd_e2e()` 改 `_cmd_e2e("sfu")`**（cli H2 断链）⑤ `data backup|reset|inspect`（**语义定义**——ops C4: reset = stop→清卷→up（entrypoint 重生成）+ 交互确认/--force；backup = stop 后 tar 两卷 + 恢复命令记录；卷名 pin `mediaservo_mediaservo-data`）⑥ `doctor` = 现 `_cmd_status` 环境诊断语义（与 `mediaservo-host doctor` 不同二进制——help 文本区分）⑦ `config show|validate|set`（set 作用面 = 宿主 config/ 目录——dev 轨道；prod 卷内标注 exec 路径——arch L12）⑧ **命令命运枚举**: `install`/`package`/`clean`/`test`/`version` **保留**（e2e 脚本直调 python 入口不可破）；**别名 fate**（cli M5）: 现别名 up/down/build-host/build-server/run-host——up/down 升一级命令（显式删除别名定义）、build-*/run-* 删除——**引用面清点**（README 8 处 + run-e2e-sfu.sh:31,33 提示文案 + docstring 用法行 + field-guide:68 + vehicle_push.rs:3 同步）⑨ 帮助/文档同步（`build --image` 仅作用于 build server——compose build --target；其他 target 拒绝该参数——cli M7）
  - Verify: `./mediaservo.sh -h` 目标命令面 + `up --env dev`（热更回归）+ `up --env prod`（生产冒烟）——**无裸 docker 调用**

- [ ] **功能回归（统一入口）**
  - Verify: `./mediaservo.sh e2e sfu`（4/4）+ `e2e push`（6/6）+ `e2e ui`（Playwright 设备配发 spec——9800 单容器形态）+ `e2e host`/`package`（回归门三件套）
- [ ] **生产冒烟（原 verify prod）**
  - Verify: `up --env prod` 空卷首启 → admin 200 + 配发原子写（无 EBUSY fallback 日志）+ 卷持久化 + 密钥非默认 + I2 守卫通过
- [ ] **dev 双轨确认**
  - Verify: `up --env dev` 热更行为不变（bind-mount 保留——改 config 即生效）
- [ ] **构建缓存**
  - Verify: 二次 build（无变更）依赖层缓存命中（耗时显著下降）

## Phase 5: 文档与记忆

- [ ] **docs/modules/26-docker-deployment-modes.md 定稿**（设计依据——已建，补验证结果）
- [ ] **记忆**: D 决策（单容器默认 + 命名卷 + prod 参考编排）+ C 约束（卷初始化语义）+ PIT（如 strip/apt 教训）+ status