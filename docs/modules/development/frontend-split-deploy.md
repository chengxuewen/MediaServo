# 前端分离形态部署与运行手册（frontend-process-split 终态）

> 2026-08-31 | 关联：D262/D268、C24 修订、PIT-163~170 | 计划档 `.sisyphus/plans/frontend-process-split/`

## 1. 拓扑速览

```
浏览器 ──HTTPS──▶ caddy（web 层）──┬── /            静态 out/server/web（vite dist）
                                    └── /api /ws 等  反代 127.0.0.1:9800 → mediaservo-server
host-agent ──/ws 直连 9800（D-2=B 一期姿势，不经过 caddy）
媒体面 RTP/UDP 直连 server（不受 web 层影响，不可代理）
```

三模式：① native 分离（主路径）② 单容器 all-in-one（嵌入，`--image runtime`）③ compose dev（vite 即前端）。

## 2. 构建

```bash
./msrtc.sh build server        # 后端（不嵌入变体）+ 自动装配 out/server/{bin,etc,web}
./msrtc.sh build web           # 仅刷 out/server/web（改 TS 快速通道，秒级，无 cargo）
./msrtc.sh build server --image runtime   # 模式②镜像（Dockerfile 显式 admin-dashboard 嵌入）
```

前端改一行 TS → `build web` → 浏览器刷新即生效（无需重编译/重启 Rust——C24 收窄至模式②）。

## 3. 部署（有状态落地，D266 同构）

```bash
export MEDIASERVO_ADMIN_PASSWORD='***'      # 生产首启必设（dev 模板账号裸机 fail-fast，C35 守卫）
./msrtc.sh deploy server --prefix /opt/msrtc-server      # /opt 需 root；幂等重跑保 etc 凭据（PIT-160）
# 落地树：<prefix>/{bin/mediaservo-server,bin/oxmgr,etc/*,web/,run/{oxfile.toml,oxmgr/,logs/}}
<..>/bin/mediaservo-server startup on /opt/msrtc-server  # 开机锚点（systemd user unit → oxmgr daemon）
loginctl enable-linger                                   # 建议：无登录会话也存活
<..>/bin/mediaservo-server status /opt/msrtc-server      # 0=健康 1=降级 2=未运行（脚本可消费）
<..>/bin/mediaservo-server doctor /opt/msrtc-server      # 退出码=失败数（PATH/端口/yaml/web dist/announced IP）
```

端口非默认时：改 `etc/server.yaml listen.port` 后，**手工同步** `etc/Caddyfile` 的
`:8080` 站点与 `127.0.0.1:<port>` 上游、`run/oxfile.toml` `[apps.env]`（`MEDIASERVO_SFU_PORT`
等）——init 渲染是一次性快照（改进项：`init --port`）。

## 4. 日常运维（实例命令面 = msrtc-host 同款）

```bash
mediaservo-server start [dir] [--no-web]   # oxmgr 整簇/仅后端；端口占用交互接管；caddy 缺→自动降级
mediaservo-server stop|restart [dir]       # 全簇收敛（幂等；只按作用域地址操作本实例 daemon）
mediaservo-server status [dir]             # server/web 两行 + /ready 探针列
mediaservo-server logs [server|web|all] [dir]
mediaservo-server monit|ps [dir]           # oxmgr TUI / 资源列
```

Python CLI 的 `run/start/stop/restart server` 已退役→指引（C39）；`run web` 保留过渡。

## 5. 故障定位

| 现象 | 首查 | 参照 |
|------|------|------|
| /ready 503 但 /health 200 | mediasoup worker 线程死（C++ 崩溃=进程亡会被 oxmgr 拉起；线程优雅死需 `restart`） | PIT-167/165 |
| 有信令无画面 | sfu/rooms producer 计数 → streamer 日志 → C38 三层链；注意 **bytes_sent 增长≠链路健康** | PIT-168 |
| 浏览器 502 | caddy 在但后端死（`status`）| — |
| 部署重跑后 host 簇失踪 | 勿再手工 `oxmgr stop`；deploy 守卫已 inode 精确化 | PIT-170/166 |
| 账号 produce 未被拒 | psk 未配+JWT 已配组合的旁路已修，回归 e2e_sfu | PIT-163 |

## 6. 回滚

- web 层：`stop` 后改回 `run server` 旧姿势 = 直接跑**嵌入变体**二进制
  （`cargo build -p mediaservo-server --features admin-dashboard`）→ `:9800/admin` 本体出 SPA；
  根 Caddyfile（全透传）保持原样即模式② ingress。
- 整轨：模式②镜像一切如旧（本次变更未触碰其语义，仅修复构建法 PIT-169）。
