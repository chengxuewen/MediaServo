# 3rdparty GUI 依赖档案（p3-gui-viewer W0 · D281 oxmgr 同法）

**本目录 = 纯档案层**：上游 release **原始 zip + .sha256** 进 git，不存解包件。
消费方（sr8 P3 起）= `cmake/gui-deps/recipes/{sdl3,imgui}.json` 经 PolyOrch thirdparty face：
`./mediaservo.sh build gui-deps` 独立构建 SDL3 → `build/3rdparty/<profile>/sdl3/install-<cfg>`
（configure 期只 import，H2 不构建）；ImGui = `fetch_only` 配方 → `build/3rdparty/imgui/source`
（source wrapper，随消费方编译，F8 不进包缓存）。配方 JSON 里 `url`/`url_hash` 是本台账的机器可读副本，
改版本 = 两处同笔（台账表 + 配方）+ `sha256sum -c` 复验。

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

**bump 配方**：下载新 tag zip → `sha256sum > <同名>.sha256` → 更新本表 +
`cmake/gui-deps/recipes/<name>.json` 的 `url`/`url_hash`（哈希不符 = 配方门的 fetch 即红 =
台账与构建天然联动）。
