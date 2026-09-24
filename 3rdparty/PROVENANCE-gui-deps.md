# 3rdparty GUI 依赖档案（p3-gui-viewer W0 · D281 oxmgr 同法）

**本目录 = 纯档案层**：上游 release **原始 zip + .sha256** 进 git，不存解包件。
消费方 = `bindings/cxx/examples/CMakeLists.txt` 的 FetchContent（`file://` 本地 URL +
`URL_HASH` 对账，不符拒绝展开）。解包目录 `target/examples/build/_deps/`（gitignored、可再生）。

| 项 | SDL3 | Dear ImGui |
|----|------|-----------|
| 档案 | `sdl3-release-3.4.16.zip`（~17MB） | `imgui-v1.92.9b-docking.zip`（~2.4MB） |
| 上游 | github.com/libsdl-org/SDL tag `release-3.4.16` | github.com/ocornut/imgui tag `v1.92.9b-docking`（= commit `b48d1afbe8ee8b238e2961dc363a949dd7304e23`；第一方 docking 分支随版 tag，`IMGUI_HAS_DOCK` 指纹=分支判据，`IMGUI_VERSION` 宏与 master 同串不作数） |
| sha256 | `547af2e721e8fc1a60f17acbb01cf6b65a6fa2022d885746b776e2d867adc2b1` | `0434445157a575f452ff0f2d1681fdd90ea8939e0c6983e6f8e47c51fba1bccd` |
| 锚定语义 | 3.4.x stable（PLAN 版本锚 F-T-13）；**鸿蒙线解锁钩子 = 3.6.0 tag 发布后换档案**（PLAN §12 开放项） | **commit-pin**（F-A-12：禁 master 移动快照；zip 按 commit 全 sha 下载，tag 名仅档案命名） |
| 哈希生成人 | Sisyphus agent（2026-09-16，下载会话内 `sha256sum` 现算） | 同左 |
| 第二人复验 | sdl 行：待用户复验（09-16 档）；**imgui 行：09-23 换 docking 档复验义务重置**——`cd 3rdparty && sha256sum -c sdl3-release-3.4.16.zip.sha256 imgui-v1.92.9b-docking.zip.sha256` + 对上游 tag `v1.92.9b-docking` 页（F-S-5） | 同左 |

**体积判例**：SDL3 源码 zip 17MB 进 git = oxmgr tar（D281）先例，PLAN 明示接受。

**"三档案"勘误（2026-09-16 W0 执行实录）**：PLAN 落笔时写"三档案"，执行核实
SDL3 + ImGui（自带 `imgui_impl_sdl3` + `imgui_impl_sdlrenderer3` 后端）两档案已覆盖
构建依赖全集，第三件（字体档案如 Noto CJK）= W3 纹理/字体需要时再入，届时同法补账。

**bump 配方**：下载新 tag zip → `sha256sum > <同名>.sha256` → 更新本表 + examples/CMakeLists.txt
的 `URL_HASH`（哈希不符 = FetchContent configure 即红 = 台账与构建天然联动）。
