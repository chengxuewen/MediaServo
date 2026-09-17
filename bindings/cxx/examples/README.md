# mediaservo C++ SDK 例子（聚合构建）

Rust SDK 三家族之一（`mediaservo::client` / C shim / 本目录 C++17 header-only）的
可运行例子面。聚合根 `CMakeLists.txt` 以 `MEDIASERVO_SDK_DIR` 守卫消费
`target/{debug,release}/` 的 `libmediaservo_client_c.*` + `include/mediaservo/client.h`；
SDL3/ImGui 走仓内档案（`3rdparty/*.zip` file:// FetchContent，CI 零联网）。

## IN（本目录保证的）

| 例子 | 形态 | 判据 |
|---|---|---|
| `imgui_viewer` | 舱端 GUI 验收件：登录 → `list_rooms` 勾选多房 → Tiles 网格（等比 fit + mini-stats）→ Control 页（steer/ack·RTT/急停） | `ctest -R core` + dummy 无头 `[frame] cb=/tex=` 双计数 |
| `imgui_shell` | 渲染壳（SDL3+ImGui 帧循环）+ `viewer_core`（RateEstimator/JSON 提取）+ `FrameStaging`/`VideoTexture`（I420→纹理零搬移） | 无显示环境 fail-soft exit 0（X11/音频后端聚合根显式 OFF） |
| `control_demo` | 无头遥控闭环（login→join→open_control→steer→ack） | `build example` 编译门 + device-day 活体 |

## OUT（刻意不做/未做）

- 触屏/竖屏布局、双档响应式、GPU 纹理后端（sdlgpu3）——布局精修另轮
- Android/Windows 装配——device-day（CI = Linux `test-gui` job + mac 待建）
- 音频会议面（`audio-` 房 kind 已可发现，消费未接）

## 运行

```bash
# 构建（CLI 自动补 build-c；改过 Rust 绑定面需先 cargo build -p mediaservo-client-c）
./mediaservo.sh build example && ./mediaservo.sh test example

# GUI（真显示）
./target/examples/bin/imgui_viewer
#   口令走 stdin（G13 永不 argv）；UI 里勾选流房（kind=video）会**自动并入整车控制房**

# 无头验收（CI/服务器机）：dummy 驱动 + 定时自退 + env 注入
SDL_VIDEODRIVER=dummy MSRTC_ROOM=vehicle_test MSRTC_PASS=dev \
  MSRTC_WS_URL=ws://<server>:9800/ws MSRTC_HTTP_BASE=http://<server>:9800 \
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

## CI

`test-gui` job（Linux）= `build example` + `ctest -R core`；活体出画与 mac/win/Android
= self-hosted/device-day（需活 server + producer）。见 `.github/workflows/ci.yml`。
