# 更新日志

本文件记录面向使用方（部署、运维、座舱/车端团队）的行为变化；
开发过程细节请直接查看 git log。最新版在最前。

## Unreleased

### 新增

- [server] 消费者面房间发现 `GET /api/rooms`：账号 JWT 即可见房间列表（admin/dispatcher
  全量在线房；viewer/operator 按 allowlist，改单热生效）；离线/无主房间不列出；响应仅含
  room_id 与 kind（video/audio），替代"手输房间名"接入方式。p3 GUI 例子首刀，C++/Rust SDK
  将提供 `list_rooms` 便捷封装（后续刀）。
- [sdk-client] `mediaservo_client::list_rooms(http_base, jwt)`：房间发现便捷函数（GET /api/rooms
  → `Vec<RoomInfo>{room_id, kind}`，自由函数会话前可用）；登录/发现共用同一手写 HTTP 面（零新增依赖）；
  授权面拒绝返回 `ClientError::RestRejected{code, message}`。
- [sdk-client] C/C++ 绑定 `ms_client_list_rooms`（C ABI，JSON 数组透传 + `needed` 溢出反馈——
  新缓冲合同，`control_producer_ids` 的 cap 盲点不复制）与 C++ `client::list_rooms()`
  （header-only，溢出自动扩容重试一次，上限 64KiB）。ABI 表 12→13 符号对账绿。
- [sdk-client] C++ 例子命令面 `./mediaservo.sh list/build/run/test example [名]`（p3 W0）：
  产物 `target/examples/bin/`（不进交付树）；GUI 依赖档案 SDL3 release-3.4.16 + Dear ImGui
  v1.92.9b（3rdparty/ 本地 zip+sha256 台账）；新例子 `imgui_viewer`（空窗骨架）与
  `imgui_shell`（渲染壳）目录就位，`control_demo` 迁入统一聚合。
- [sdk-client] 会话视频统计面 `ms_client_session_video_stats` / C++ `Session::video_stats()`：
  inbound-rtp 折叠扁平 JSON（bytes/frames/分辨率/fps/丢包），mini-stats 渲染数据源
  （Rust 侧 fold 规则单测钉；C 面 needed 溢出合同同形）。
- [sdk-client] 例子壳 `viewer_core`：RateEstimator（Δt=0 保持/计数倒退重锚，绝不负速率）
  与无依赖 JSON 标量提取，ctest `-R core` 判据就位。
- [sdk-client] C 绑定视频消费崩溃修复：帧泵线程 reactor panic（block_on 外侧实参构造）
  ——修复前 C/cxx 面 consume_video 收帧线程静默死亡（多路画面必现；spike 实锤四连 panic）。
- [sdk-client] **修复：非浏览器 SDK 消费视频 ~1 秒后画面停止刷新**（已知问题结清）。
  根因 = sink 注册所在的轨道包装对象出作用域即析构，析构连坐摘除刚注册的 sink——
  此前所有"轨道重建/线程上下文"假设均证伪，实测包装每次取用新建、指针恒变。
  修复 = 注册时与 sink 成对持有包装；90 秒双源对拍（回调帧数 vs libwebrtc 解码计数）
  2700/2699 咬合零衰减。附带删除救援/追踪重挂补丁机制（其前提假设已死且致重复交付）。
- [sdk-client] GUI 例子 `imgui_viewer` W3b 首刀：登录→入房→消费→**I420 纹理上屏全链**
  （新增壳层件 FrameStaging 泵线程→主线程最新帧槽 + VideoTexture SDL IYUV 工位）。
  无头判据 `SDL_VIDEODRIVER=dummy` + `MSRTC_RUN_SECS` 自退 + `[frame] cb=/tex=` 双计数
  ——CI 无显示环境亦可验收视频链路。
- [sdk-client] GUI 例子 W4 合并轮：登录面板（口令输入框，G13）→ `/api/rooms` 勾选多路
  （每房一会话）→ Tiles 页 1-3 列网格 + mini-stats 行（kbps/fps/分辨率，W3a 资产消费端）
  → Control 页（chassis steer 滑条 + ack 回显 + RTT；急停按钮为未签名形，签名刀在案）。
  无头 CI 通道：MSRTC_PASS+MSRTC_ROOM 自动登录入房，`[tile]` 异常显式打印。

- [sdk-client][host][protocol] 急停命令链路（遥控安全）：座舱 SDK `emergency_stop` 双路投递——数据通道快路径携带 HMAC-SHA256
  签名（部署预共享密钥 MEDIASERVO_CONTROL_HMAC_KEY，车舱同值）+ 信令通道审计副本；车端执行器
  验签闸门（密钥已配置时，无签名/错签的急停一律拒执并回执 estop_signature_rejected），执行结果
  逐条落 actuation 审计（jsonl 或日志行，审计永不阻塞执行）。密钥未配置 = 行为与旧版一致（迁移期）。
- [deploy] 兼容矩阵脚本 scripts/e2e-compat.sh：方言四探针（缺字段→v1/声明钳制/拒低 4101）+ 新 SDK 全链
  回环 + dispatcher 4012 拒控负例；旧 server 象限需 b34f3a1 产物（缺省 SKIP 记账）。
- [deploy] 弱网还账（loss × Cmd/ACK）：remote-burst 5/5 全通 RTT 254-301ms；degraded 失联级零脆断，
  结论与证据见主仓 docs/plans/client-dual-form/evidence/s4-weaknet.md。

### 新增
- [sdk-client] 座舱 SDK 第四家族（C ABI + C++）：新库 mediaservo-client-c 提供 ms_client_* 稳定 C 接口
  （登录/入房/视频回调/遥控通道），新头文件 mediaservo/client.hpp 提供 C++11 兼容 RAII 包装；
  附纯 C++ 遥控样例 control_demo 与 ROS2 桥接样例节点（device-day 构建）。CI 新增 test-cxx
  作业：四 SDK C++ 测试编译运行 + C ABI 符号表巡检（含 client 新面 12 导出对表）。

### 修复
- [server][host][sdk-client] 遥控数据通道资源泄漏（生产级）：座舱端断开连接后其数据通道注册与对端消费句柄此前
  不回收，长期运行会耗尽可用通道号导致新座舱无法建链（并曾表现为仪表盘数据源计数
  单调增长）；现断链即定向收割并通知车端拆除本地通道。仪表盘同步修正：数据通道
  关闭事件不再误触发视频流重启。
- [host] 车端控制器冷启动不再因服务端未就绪而崩溃重启风暴：信令连接加入指数退避重试
  （覆盖整簇拉起窗口），耗尽后仍由守护策略兜底。
- [server][host][sdk-client] 遥控数据通道消息体单侧开（车端/舱端互发收不到）根修：数据通道消费回执现在携带服务端
  分配的 SCTP 流参数，两端以带外协商通道接收转发消息（mediasoup-client 官方契约）——
  此前等待的带内握手在代理形态下永不发生。兼容旧版线形（字段缺省可读）。

### 新增
- [sdk-client] 新增 mediaservo-client v2 消费端 SDK（Rust 库）：账号登录(JWT)、入房、SFU 视频消费、遥控数据通道（4012 拒控/协议过低为显式类型化错误）；旧客户端演示形态（内置 WS/:9101 转发/HMAC 通道）退役。
- [host][protocol] 车端遥控接收链路换代：host-controller 从（自 2026-08-25 起即已死路的）点对点协商改为服务端 SFU 数据通道——上电即完成 DTLS/SCTP 注册（chassis/gimbal/light/ack 四通道），指令镜像发布到本机总线 control/cmd（回执 control/ack）供 ROS/回放节点零改动消费。
- [protocol][server][host] 信令通道硬化：服务器主动心跳探测断链（约 10-15 秒回收僵尸连接；网络抖动期连接不再被误杀）；车端短暂失联后凭一次性重挂票快速重挂（认证始终全量重跑，吊销即刻生效）；心跳与超时参数经环境变量 MEDIASERVO_WS_PING_SECS / WS_PONG_MISS / WS_PSK_WAIT_SECS / WS_JOIN_WAIT_SECS / WS_RESUME_HOLD_SECS 可调。
- [protocol][server] 信令拥塞时高频状态上报限损（丢弃计数自报），认证/控制/终态消息不再被状态洪流排队拖死（优先级双队列）。协议方言升至 v3（新增会话续期域；v1/v2 端点行为逐字节不变）。
- [protocol] 信令协议版本协商：端点在加入房间时声明方言版本，服务器协商后回显生效值；过旧版本显式拒绝（错误码 4101）而非静默降级。旧客户端不声明即按 v1 处理——线上报文逐字节不变，升级顺序无约束。控制数据通道（遥控）要求协商版本 ≥2。
- 设备公钥指纹准入：host 用初始化时已生成的设备私钥应答服务器挑战，注册只需在管理台「待批准设备」点批准；专网/开发环境设 `ALLOW_DEV_ENROLL=1` 后新设备接入零人工（不再抄发/配置任何密钥）。
- 管理台设备页新增「待批准设备」队列（一键批准 + 可选命名）。
- Web 播放器协商内核改用官方 mediasoup-client（手拼 SDP/硬编码负载类型技术债清偿；对外行为与界面不变，弱网韧性语义原样保留）。
- 部署帮助新增「环境变量总表」：`msrtc.sh -h` 与 `msrtc-server -h` / `msrtc-host -h` 三面共用单一真源 `crates/mediaservo-common/assets/env-usage.md`（[A] 脚本注入 / [B] oxfile 手工行 / [C] 启动 env），整树重部署丢手工 env 时按表回补。

### ⚠ 升级注意
- 设备准入换代：旧版按「设备密钥」注册的车辆，升级 host 后首次连接会被拒绝——请在管理台删除旧条目，再让设备重新接入（自动档秒收录；默认档点一次批准）。设备密钥通路保留一个版本周期，下版删除。
- `host init` 生成的 `identity.json` 不再包含密钥字段（旧文件仍可读）。

## v0.1.1（2026-09-10）

### 新增
- 弱网模拟全套工具：管理后台侧栏新增「弱网面板」（管理员可见），浏览器即可
  施加/查看网络损伤；命令行 `./msrtc.sh weaknet apply|set|scenario|status|clear`
  同步可用；弱网服务随 server 部署簇自动起停，无需手动启动进程。
- 推流弱网策略三档（host 配置 etc/host.yaml 每条流新增 stream_mode）：
  smooth 保帧率（遥控推荐）/ quality 保清晰度（取证推荐）/ balanced 默认均衡，
  弱网下帧率不再断崖式下跌。
- 播放韧性增强：车端或服务重启后网页端自动恢复出画面（免手动刷新）；
  源停止时如实显示「源离线」而非误报连接失败。
- 弱网模拟车端独立形态：可在 Jetson 车端本机对物理网口施加网络损伤。

### 变更
- server 部署簇包含 3 个服务（主服务 / Web 前端 / 弱网模拟），
  start、stop、status 一条命令统一管理整簇。
- 版本号全局统一：所有组件随单一 workspace 版本发布；发布包
  {品牌}-{组件}-{版本}.tar.gz 顶层为版本目录，内附本文件（CHANGES.md）
  与版本契约 version.txt。
- 发布包输出统一收敛到 out/packages/。

### 升级注意
- 旧部署实例的 etc/host.yaml 注释不含 stream_mode 用法，请参考最新发布包
  内注释或 docs 补充。
- server 簇升级后首次启动会重写运行清单（旧文件自动备份 .bak），
  此前手工添加的运行环境变量会自动保留。
- 弱网面板需管理员账号登录；对指定视频流施加损伤要求该流正在推送。

## v0.1.0（2026-08-20 ～ 09-09，未版本化内部基线）
- 首个内部集成基线：多类型视频源（相机/生成器/桌面）→ SFU 转发 → 网页播放
  全链路；设备/账号统一管理后台（变更热生效，免手工改配置重启）；
  PSK/JWT 双沿鉴权与密钥轮换；音频会议房间；H.264/VP8/VP9/AV1 编码可选；
  断线自愈与弱网友好播放；原生 / 单容器 / compose 三种部署形态。
