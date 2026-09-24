# mediaservo C++ SDK 例子（聚合构建）

Rust SDK 三家族之一（`mediaservo::client` / C shim / 本目录 C++17 header-only）的
可运行例子面。聚合根 `CMakeLists.txt` 以 `MEDIASERVO_SDK_DIR` 守卫消费
`target/{debug,release}/` 的 `libmediaservo_client_c.*` + `include/mediaservo/client.h`；
SDL3/ImGui 走仓内档案（`3rdparty/*.zip` file:// FetchContent，CI 零联网）。
ImGui 档案 = `v1.92.9b-docking`（第一方 docking 分支，imgui_viewer 的四区 IDE 布局依赖）。

## IN（本目录保证的）

| 例子 | 形态 | 判据 |
|---|---|---|
| `imgui_viewer` | 舱端 GUI 验收件（**docking 四区**，见下） | `ctest -R core` + dummy 无头 `[frame] cb=/tex=` 双计数 |
| `imgui_shell` | 渲染壳（SDL3+ImGui 帧循环，`ShellOptions{docking, ini_path}`）+ `viewer_core`（RateEstimator/JSON 提取）+ `FrameStaging`/`VideoTexture`（I420→纹理零搬移） | 无显示环境 fail-soft exit 0；dummy 驱动下 ini 禁写 |
| `control_demo` | 无头遥控闭环（login→join→open_control→steer→ack） | `build example` 编译门 + device-day 活体 |

## imgui_viewer 布局（docking 四区，IDE 形）

```
┌───────────┬──────────────────────────┬──────────────┐
│ Streams   │      Video Grid          │ Stream Info  │
│ 设备▸流树  │   （1-3 列，等比 fit）     │ (聚焦流详情)  │
│ 点选→Pull  │                          ├──────────────┤
│           │                          │   Control    │
├───────────┴──────────────────────────┴──────────────┤
│ Log（进程日志环 500 行 · follow 自动滚 · reset layout）│
└──────────────────────────────────────────────────────┘
```

拖分栏=抓边界；双击标题=该窗最大化/还原；拖标题出画布=浮动窗；位置与分栏宽
存 `imgui_viewer.ini`（删文件或 Log 区「reset layout」=恢复四区出厂形）。
dummy 无头模式下 ini 不写盘（CI 构建目录零污染）。

### 当模板抄时的导读

1. **`imgui_viewer/src/app_model.hpp` 头注** = 分层铁律（main 节拍/模型/sessions 动作/panels 无状态渲染，依赖单向）。
2. **`imgui_viewer/src/dock_layout.hpp` 头注** = docking 五要素速查（开关/画布/DockBuilder/首版守卫/持久化），三条实战坑全标：`DockBuilder*` 在 `imgui_internal.h`；`SetNodeSize` 必先于 Split；首版守卫用 **ini 探测**（`App::had_saved_layout()`）而非网传 WasActive（那是窗口的不是节点的）。
3. **`imgui_viewer/src/app_log.{hpp,cpp}`** = GUI 接多线程日志的最小形（mutex+环+add_fmt；stdout 机器判据行与 UI 日志行分工）。

### 手测单（首轮验收，5 分钟）

- [ ] 登录 → 左树出现设备分组；**点行/勾盒多选**（选中行黄高亮）→ 点「Pull selected ▶」→ 中格出画、右上详情有 fps/kbps、树内转 live 灰缀
- [ ] Pull 流房自动并入整车控制房（树里 `●` 组），Control 右下 steer→ack RTT 现值
- [ ] 抓任意分栏边界拖动 → 退出重开 → **宽度保持**（ini 生效）
- [ ] 双击 Video Grid 标题=最大化，再双击=还原；拖 Streams 标题出画布=浮动
- [ ] Log 区「reset layout」→ 四区回出厂切分

## 运行

```bash
# 构建（CLI 自动补 build-c；改过 Rust 绑定面需先 cargo build -p mediaservo-client-c）
./mediaservo.sh build example && ./mediaservo.sh test example

# GUI（真显示；Xwayland 会话内 DISPLAY 已带）
./target/examples/bin/imgui_viewer
#   口令走 UI/stdin（G13 永不 argv）；勾选流房（kind=video）自动并入整车控制房

# 无头验收（CI/服务器机）：dummy 驱动 + 定时自退 + env 注入
SDL_VIDEODRIVER=dummy MSRTC_ROOM=vehicle_test MSRTC_PASS=dev \
  MSRTC_WS_URL=ws://<server>:9800 MSRTC_HTTP_BASE=http://<server>:9800 \
  MSRTC_USER=admin MSRTC_RUN_SECS=20 ./target/examples/bin/imgui_viewer
#   判据行：[frame] t=..s cb=<回调帧> tex=<纹理帧> WxH —— 稳态两者相等且 ~30fps 增长

# 急停签名（可选）：预共享密钥文件（0600，车舱同值；无文件 = 不签名形）
export MSRTC_ESTOP_KEY_FILE=/path/to/control-hmac.key
```

## 舱端双房约定（读 API 前先读这段）

媒体面与控制面**不同房间**（PIT-140 v2 + W4c 定性）：

| 面 | 房间 | 发现方式 |
|---|---|---|
| 视频/音频流 | `<整车房>_<流id>`（如 `vehicle_test`） | `GET /api/rooms` 派生条目，`kind:"video"/"audio"` |
| 遥控/急停 | `<整车房>`（如 `vehicle`） | 同列表 `kind:"control"` |

单房间打两边 = 一边必空（流房无 chassis producer / 整车房无视频 producer）。
本 viewer 已按双房配对；自研舱端请照此接线。

## OUT（刻意不做/未做）

- 触屏/竖屏专属布局与 GPU 纹理后端（sdlgpu3）——docking 后拖拽已覆盖桌面手感需求，触屏另轮
- Android/Windows 装配——device-day（CI = Linux `test-gui` job + mac 待建）
- 音频会议面（`audio-` 房 kind 已可发现，消费未接）

## CI

`test-gui` job（Linux）= `build example` + `ctest -R core`；活体出画与 mac/win/Android
= self-hosted/device-day（需活 server + producer）。见 `.github/workflows/ci.yml`。
