# 24. 应用层品牌化定制指南 (App Branding)

> 当 MediaServo 作为基石被第三方平台依赖（docs/architecture.md §1.2）时，host/client/server
> 应用层可按需白标，而 SDK bindings 与 wire 协议保持固化。本指南列出**可改面**与**固化面**。

## 快速开始

```bash
# 1. 安装品牌化 host 包（布局 + 品牌 app 符号链接 + identity 前缀）
./mediaservo.sh deploy host --brand cp --prefix /opt/cp-host

# 2. 运行时品牌（翻译/app 名/unit/设备前缀全走此 env）
export MEDIASERVO_BRAND=cp
/opt/cp-host/cp start /opt/cp-host
```

优先级：**运行时 env `MEDIASERVO_BRAND` > 编译期 `option_env!` > 默认 `mediaservo`**。
合法值：`[a-z0-9-]`，≤32 字符；非法值回落默认并 `warn`（品牌是显示层，不阻断启动）。

## 品牌映射表（默认 → legacy 串硬映射）

| 面 | 默认（零变化） | 品牌化（MEDIASERVO_BRAND=cp） |
|---|---|---|
| app 名（进程） | `host-agent`/`host-capturer-cam0`/... | `cp-agent`/`cp-capturer-cam0`/... |
| namespace（oxfile/status 过滤） | `mediaservo-host` | `cp-host` |
| systemd unit 前缀 | `oxmgr-host-*` | `oxmgr-cp-*` |
| 设备 id 前缀（identity.json） | `ms-<12hex>` | `cp-<12hex>`（**仅新 key**——存量不迁移，G2 配发需重新注册） |
| CLI 显示名/帮助/版本 | `mediaservo-host` | `cp` |
| 产品展示名（client 日志/面板） | `MediaServo` | `cp` |
| 路径/包 id（默认配置路径） | `mediaservo` | `cp` |
| bin 快捷方式 | `host` + `mediaservo-host` | `cp` + `cp-host` |
| bin/ 品牌 app 符号链接 | 无（二进制即 host-*） | `bin/cp-agent → host-agent`（install 自动创建） |

> **勿按 `<product>-` 直推**：默认品牌下 `product="mediaservo-host"` 但 app 前缀必须是
> **`host-`**（legacy）、设备前缀 **`ms-`**（identity 单测断言锁死）——见 `brand.rs` 映射表。

## 定制项（跨面参数）

| 面 | 位置 | 说明 |
|---|---|---|
| [signaling] local_port | host.toml | 多实例共存（C32 三隔离） |
| [signaling] room | host.toml | 房间名与品牌无关（默认 `vehicle`；`audio-` 前缀房间约定 C29 不改写） |
| PSK / server_url | host.toml | 部署配置面 |
| 进程拓扑 | `translate.rs` | 车端裁剪（如去掉 controller/audio）——`enabled_apps` 演进位 |
| admin 标题 | `VITE_APP_TITLE`（vite 构建 env） | 需 `pnpm build` + server 重建（C24）
| 端口 | 配置面 | daemon/API 端口按实例目录派生（oxmgr 三 env，C32） |

## 固化面（禁止品牌化）

| 面 | 原因 |
|---|---|
| **bindings/\***（C ABI 符号 `mediaservo_*`、include/mediaservo/、cxx/py/node） | 多产品同进程共存——符号冲突即链接失败（D247） |
| **wire 协议**（信令 SignalingMessage/SFU/RTP 参数、FrameMeta 线格式） | server 可能同时服务多品牌 host——品牌化割裂生态 |
| **soname/ABI 纪律**（additive-only） | 动态库升级兼容（D240） |
| **crate 名**（`mediaservo-*` workspace member） | 依赖面标识（需要独立发布时走 fork + D209 式重命名） |
| admin localStorage key（`mediaservo_admin_token`） | 保持原键（品牌化会登出用户） |
| room 命名（`audio-` 前缀约定） | 服务端房间语义（C29） |

## 校验命令

```bash
# 固化门: bindings 零 diff（品牌改造不得触碰）
git diff --stat bindings/            # 应为空

# 缺省零变化门: 默认品牌全量回归
pixi run cargo test -p mediaservo-host --lib    # 50 全绿
bash scripts/e2e-deploy-host.sh                # PASS（含 start/status/stop roundtrip）

# 品牌化验证
MEDIASERVO_BRAND=cp ./target/debug/mediaservo-host version   # cp 0.1.0
MEDIASERVO_BRAND=cp ./target/debug/mediaservo-host startup on <dir>  # unit oxmgr-cp-*.service
```

## 设计位置

- `crates/mediaservo-common/src/brand.rs`：Brand 结构 + `media_brand()`/`media_brand_from()` 读取器
- `crates/mediaservo-host/src/bin/host.rs`：USAGE/version/namespace 过滤/unit 名
- `crates/mediaservo-host/src/translate.rs`：`app_name()`（app 前缀）+ oxfile namespace
- `crates/mediaservo-host/src/identity.rs`：`generate_identity()` 设备前缀
- `scripts/mediaservo_cli.py`：`deploy host --brand` / `package host --brand`
- `www/apps/admin/vite.config.ts`：`__APP_TITLE__` define

计划: `docs/superpowers/plans/2026-08-21-app-branding-customization.md`