# MediaServo

MediaServo — 实时媒体伺服平台（MediaServo Platform）。独立部署的视频/媒体服务平台，以"精确、低延迟、响应式"的伺服驱动为核心，涵盖监控相机接入与录制回放（NVR）、视频会议、远程桌面、遥操作、直播推拉流等能力。可单独部署，也可作为 Docker 模块嵌入第三方平台。

## 功能范围

- **远程桌面**：屏幕捕获、GPU 编码（H.264/H.265）、输入注入、<100ms 延迟
- **视频会议**：多方音视频通话、SFU/MCU、屏幕共享
- **推拉流**：RTMP/HLS/SRT 接入与分发、直播转码
- **监控接入**：ONVIF/GB28181 相机发现与流管理
- **WebRTC 遥操作**：低延迟视频 + DataChannel 控制（车辆/机器人）
- **车端推流**：车辆摄像头推流到云端 / 舱内拉流

## 架构

```
┌──────────────────────────────────────────────┐
│           MediaServo 后台服务                 │
│   用户管理 · 权限控制 · License · 信令       │
└──────────────────┬───────────────────────────┘
                   │ gRPC / REST
    ┌──────────────┼──────────────┐
    ▼              ▼              ▼
┌──────────┐ ┌──────────┐ ┌──────────────┐
│  Client  │ │   Host   │ │ 嵌入/模块     │
│ (GUI)    │ │(headless)│ │ 嵌入/平台模块 │
│ 操作端   │ │ 远端     │ │              │
└──────────┘ └──────────┘ └──────────────┘
```

- **Client**：桌面 GUI 全功能应用（Tauri v2），可控制他人也可被控制
- **Host**：无 GUI 守护进程，适合边缘设备/服务器/车端，纯产出媒体流
- **微内核 + 插件**：mediaservo-common 微内核，领域功能以插件形式加载
- **Auth 双模式**：独立部署自带账户系统；作为第三方平台模块时委托平台 RBAC/LDAP

详见 [`docs/architecture.md`](docs/architecture.md)。

## 技术栈

- **Native 层**：Rust (edition 2024)，libwebrtc (主) / str0m / webrtc-rs 三后端
- **绑定层**：napi-rs（Node.js）、C FFI（静态链接到宿主）
- **信令**：WebSocket (Phase 1) + MQTT 5.0 (Phase 2+)
- **传输**：RTP/RTCP、SRT、WebRTC DataChannel
- **内部协议**：FlatBuffers（零拷贝，多语言）

## 开发状态

当前处于 Phase 3 完成阶段。7 crate workspace，webrtc triple-backend，codec 三后端 (stub+FFmpeg+GStreamer)，400+ tests 全部通过，Docker/CI/DevContainer 就位。

## 构建与开发

统一 CLI 入口（bootstrap 后可用）：

```bash
./mediaservo.sh -h            # 全部子命令（Windows: mediaservo.bat）

# 动词 × 目标矩阵（target: server | host | client | all）
./mediaservo.sh build         # 构建 all（host/client 原生 + server Docker）
./mediaservo.sh build host    # 仅宿主侧（别名: build-host）
./mediaservo.sh build server  # 仅 server（别名: build-server）
./mediaservo.sh up --env dev  # 启动 dev server 容器（幂等，自动注入 ANNOUNCED_IP）；prod: up --env prod
./mediaservo.sh up --announced-ip 10.144.0.3  # 显式指定容器公告地址（覆盖自动探测；多值逗号分隔——
                                  # 容器仅单公告生效取首 IP，多地址需裸机运行）
./mediaservo.sh build server --native  # 裸机编译 server（pixi 工具链，需联网拉 meson wrap；多 IP 公告在 run 阶段生效）
./mediaservo.sh run server --native [--foreground]  # 裸机运行 server（后台: target/server-native.pid + .log）
./mediaservo.sh start host    # 启动 host 推流（oxmgr 多进程; 杀旧进程）；start server=裸机 native（默认）\| --mode compose/
#                           --env dev 容器
./mediaservo.sh stop server   # 停止 server（先杀裸机 pid——若在跑; 再 compose stop 保留容器——秒级再启）
./mediaservo.sh stop host     # 停止 host 进程（pkill host-legacy + mediaservo-host stop——幂等）
./mediaservo.sh logs server    # server 日志（默认裸机 target/server-native.log；--env dev/--mode compose 容器日志；--native 显式）
./mediaservo.sh down --env dev && ./mediaservo.sh up --env dev   # 重启 dev server（清旧再启，配置变更生效）
./mediaservo.sh logs host     # host 日志（/tmp/mediaservo-host.log）
./mediaservo.sh e2e           # e2e_sfu 回归（前置: server + host + vite 运行中）
./mediaservo.sh status server  # 健康探测（默认裸机 native——pid+端口+announced；--mode compose/--env dev 容器 health；exit 0/1/2）
./mediaservo.sh status host    # 推流进程+网关 17980+实例日志（C38 ②层）；环境诊断见 doctor
./mediaservo.sh config validate   # 配置校验（pyyaml）
./mediaservo.sh clean         # 清构建产物（保留 cargo-cache 卷）; clean --all 全清
./mediaservo.sh install host  # 安装到前缀（默认 <root>/install/host）; install bindings 同
./mediaservo.sh package host|bindings  # dist/ 双包发布（D-H13）: mediaservo-host-<ver>.tar.gz + mediaservo-sdk-<ver>.tar.gz
```

> client 目标已注册（build 可用）；up/down/restart/logs client 待骨架完成后实现。

首次使用：`source bootstrap.sh`（Linux/macOS）或 `bootstrap.bat`（Windows）。
Windows 为 best-effort（pixi.toml platforms 需含 win-64）。

## 许可

Apache 2.0
