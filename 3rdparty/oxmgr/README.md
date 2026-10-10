# 3rdparty/oxmgr — 不可镜像下载点固化（方案 Y，与 libwebrtc 同构）

OxMgr 进程管理器（host/server 簇运行时依赖）。**本目录 = 纯档案层**：只存上游 release 原始
`tar.gz + .sha256`（文件名即完整坐标 产品-v版本-rust-triple），**不存解包件**。

```
3rdparty/oxmgr/
├── oxmgr-v0.5.0-aarch64-unknown-linux-musl.tar.gz   + .sha256   # Jetson
├── oxmgr-v0.5.0-x86_64-unknown-linux-musl.tar.gz    + .sha256   # dev/CI（09-10 补下载，对账全等）
└── README.md
```

**消费链**：`msrtc.sh` 按 `uname -m` 选 triple → `.cache/oxmgr/<档案名>/` 解包（gitignored、
可再生、目录含版本=bump 自然失效）→ **解包前 sha256 对账，不符拒绝** → 注入 PATH →
CLI `shutil.which("oxmgr")` 打包进 `bin/`。未知架构（含 mac arm64——上游无 darwin 发布）
不注入并显式 WARN，由 PATH 自装兜底，绝不隐式回退错架构二进制。

| 项 | aarch64 | x86_64 |
|----|---------|--------|
| 上游 | github.com/Vladimir-Urik/OxMgr/releases/tag/v0.5.0 | 同左 |
| tarball sha256 | 9cf0a2aa… | e12466ef…（09-10 补下载复验全等） |

**bump/新平台配方**：下载上游 tar.gz+核对 sha256 → 两个文件放入本目录（保持上游实名）→
`msrtc.sh` 改 `OXMGR_PKG` 行版本号（+ 新 triple 则 case 加一行）→ `rm -rf .cache/oxmgr` 后
任一 msrtc.sh 命令自动重解包验证。
