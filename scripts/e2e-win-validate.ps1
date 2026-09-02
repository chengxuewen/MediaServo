# e2e-win-validate.ps1 — CARLA 机（x86-Windows）双包验证清单（I2 交付物, 暂为文档化 stub）
# -----------------------------------------------------------------------------
# 状态: 未执行（开发环境无 Windows 机）。CARLA 机可用时逐项执行, 每项通过后标记 [x]。
# 前置事实（platform.md）: Windows 为 best-effort — pixi.toml platforms 需含 win-64;
# webrtc-sys/libwebrtc Windows 构建未全面验证。若 win-64 构建受阻, 本清单降级为
# "仅打包产物结构验证"（步骤 1-3, 跳过 4-6 运行闭环）。
# -----------------------------------------------------------------------------
#
# 环境: CARLA 机（x86-Windows）+ 局域网内 Docker server（mediasoup, 见 compose）
# 目标: 验证 mediaservo-host-<ver>.tar.gz 在 Windows 上构建→打包→解包→最小闭环推流

# ── 0. 前置 ──────────────────────────────────────────────────────────────────
# [ ] pixi 已安装（https://pixi.sh）; `pixi run cargo --version` 可用
# [ ] pixi.toml `[workspace.platforms]` 含 win-64（当前仅 Linux/macOS → 需先添加,
#     并在 [target.win-64] 补 system-requirements 若需要）
# [ ] Docker server 已在局域网主机运行: 容器 9800(WS) + 20000(WebRTC) 端口对 LAN 开放;
#     记录 <SERVER_LAN_IP>（CARLA 机可达）
# [ ] 本仓库已检出到 CARLA 机（git clone / 拷贝）

# ── 1. 构建 + 打包（C22 镜像原则: host 在真机原生构建/运行, 非容器）──────────
# [ ] pixi run cargo build -p mediaservo-host          # 8 进程二进制（host + host-*）
# [ ] pixi run python scripts/mediaservo_cli.py package host
# [ ] 断言 dist/mediaservo-host-<ver>.tar.gz 存在
#     备注: Windows 下二进制为 .exe（_exe_name 已处理）; oxmgr 需 npm install -g oxmgr

# ── 2. 解包 + doctor ─────────────────────────────────────────────────────────
# [ ] mkdir C:\mediaservo-host; tar -xzf dist/mediaservo-host-<ver>.tar.gz -C C:\mediaservo-host --strip-components=1 （Win10+ 自带 tar.exe；包内顶层为 mediaservo-host-<ver>/）
# [ ] C:\mediaservo-host\bin\host.exe doctor C:\mediaservo-host          # 期望: 全部通过
#     备注: 包内 identity.json 为打包机生成的新鲜身份 — 多设备部署时每台删除后
#     重跑 `host init <prefix>` 生成独立设备身份（G4; 见包内 host-version.txt）

# ── 3. 最小闭环: capturer 采集 → FrameBus → streamer 推流 → LAN Docker server ──
#     对照 streamer_e2e（Linux 双进程, C2）与 e2e_audio_conf 的 Windows 等价物
# [ ] 启动 capturer:  bin\host-capturer.exe --camera cam0 --config C:\mediaservo-host\etc\host.toml
#     （后台; 期望: 周期发布 I420 帧到 FrameBus topic camera/cam0, 日志无错误）
# [ ] 启动 streamer:  bin\host-streamer.exe --stream cam0-stream --config ... \
#         --token C:\mediaservo-host\etc\link\cam0-stream.token \
#         --signal ws://<SERVER_LAN_IP>:9800/ws --psk <PSK> --room <vehicle-id>
#     （前台; 期望: 协商成功 → 出站 stats 日志 bytes_sent>0 / frames_encoded>0）
# [ ] server 侧佐证: docker logs mediaservo-server-1 | grep "ReceiveRtpPacket.*key frame" ≥1
# [ ] SIGTERM（Ctrl+C）→ streamer 优雅退出码 0

# ── 4. host 生命周期冒烟 ─────────────────────────────────────────────────────
# [ ] bin\host.exe start C:\mediaservo-host     # oxmgr 拉起全部进程（注意: oxmgr Windows
#     支持面有限 — 若受阻, 回退步骤 3 的 capturer/streamer 手动双进程最小闭环）
# [ ] bin\host.exe status C:\mediaservo-host    # host-agent 在列
# [ ] bin\host.exe stop C:\mediaservo-host      # 无已管理进程

# ── 5. SDK 包（仅结构验证, 消费方为 Linux 车端时跳过运行）────────────────────
# [ ] pixi run python scripts/mediaservo_cli.py package bindings
# [ ] tar tzf dist/mediaservo-sdk-<ver>.tar.gz 断言关键文件:
#     lib/libmediaservo_*.so*（Windows 为 .dll 时另行评估）— 实际消费以 Linux 为主（D240）

# ── 6. 结果上报 ──────────────────────────────────────────────────────────────
# 完成 1-5 后将实际输出（构建错误/闭环证据）贴回 task-I2-report.md 的 Windows 节

Write-Host "e2e-win-validate.ps1: 文档化清单 stub — 未在 Windows 机执行。"
Write-Host "按注释清单在 CARLA 机逐项验证, 完成后将证据回填 task-I2-report.md。"
exit 0
