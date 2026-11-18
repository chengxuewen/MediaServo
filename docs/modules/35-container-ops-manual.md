# MediaServo 容器镜像运维手册

> 随 `msrtc-server` 镜像 / `package image` 产物集发布。面向**现场运维人员**：
> 拿到包后如何加载、配置、启动、验证、日常运维、排障、升级回滚。
> **首次装机（CentOS 8 vault 源 / 内核调优 / SELinux / 离线演练）见同包《部署手册》
> `DEPLOY-MANUAL.md`**；本手册只给运维日常所需的自包含步骤。

## 0. 包里有什么

| 文件 | 说明 |
|---|---|
| `msrtc-server-<tag>.tar` | 镜像包（`docker save`；含 `<tag>` + `latest` 双 tag） |
| `SHA256SUMS` | 镜像 tar 的 sha256 清单（交付对账） |
| `docker-compose.yml` | 生产 compose（单容器：server + caddy + oxmgr） |
| `.env.example` | 配置模板（拷贝成 `.env` 填写） |
| `DEPLOY-MANUAL.md` | **部署工程师手册**（装机/调优/防火墙/离线演练/凭据交付） |
| `OPS-MANUAL.md` | 本手册（**运维日常**） |

**单容器拓扑**：一个容器跑整棵树——`msrtc-server`（信令 + admin + SFU）+
`caddy`（8080 网页/反代）+ `oxmgr`（进程管理，容器 PID1）。无独立 proxy 容器。

## 1. 前置检查

```bash
docker --version                      # 需要 docker-ce 20.10+
docker compose version                # 需要 compose v2 插件
```

- **端口**：8080/tcp（网页）、9800/tcp（host 直连）、20000/udp（媒体）、
  2222/tcp（ssh 维护，可改）。首次安装见《部署手册》。
- **资源**：`mem_limit` 默认 512m（Small 档压测实测，见 §4）；磁盘按录像量预留。

## 2. 快速启动（5 步）

```bash
# ① 校验镜像包完整性
sha256sum -c SHA256SUMS

# ② 加载镜像（无需外网）
docker load -i msrtc-server-<tag>.tar

# ③ 准备配置
cp .env.example .env
vi .env            # 填 §3 的必填项

# ④ 起栈
docker compose --env-file .env up -d

# ⑤ 看状态（等 healthy，start-period 60s）
docker compose --env-file .env ps
```

## 3. 配置（.env）

**必填**

| 变量 | 含义 | 注意 |
|---|---|---|
| `TAG` | 镜像 tag（回滚用） | 与加载的 tar tag 一致 |
| `MEDIASERVO_SFU_ANNOUNCED_IP` | SFU 对外宣告 IP | **多网卡必须显式填客户端可达 IP**（自动探测取首 IP，多网卡会静默媒体不通） |
| `MEDIASERVO_ADMIN_PASSWORD` | 首启 admin 密码 | 首启写入账户表；之后经 admin 页面改 |
| `SSH_PORT` | 宿主 ssh 口（默认 2222） | 与宿主已有 sshd 冲突时改 |

**可选**

| 变量 | 默认 | 含义 |
|---|---|---|
| `MEDIASERVO_BRAND` | `msrtc` | 树前缀 `/opt/<brand>` + 卷路径；改它须配套换镜像 |
| `MEM_LIMIT` | `512m` | 容器内存上限（仅内存限，CPU 放开）；大并发按 §4 调 |
| `RUST_LOG` | `info` | 日志级别 |
| `WEB_PORT`/`AGENT_PORT`/`SFU_PORT` | 8080/9800/20000 | 仅移**宿主侧**端口；容器目标固定 |

> 别改容器内部端口（`MEDIASERVO_WEB_PORT`/`MEDIASERVO_SFU_PORT`），除非你同时改
> compose 目标端并保持 1:1（见 §4.1 SFU 坑）。

## 4. 端口映射与卷

| 容器 | 宿主默认 | 协议 | 用途 |
|---|---|---|---|
| 8080 | 8080 | tcp | 网页入口：`/admin` `/voice/` `/api` `/ws` `/health` |
| 9800 | 9800 | tcp | host-agent 直连 `/ws` |
| 22 | 2222 (`SSH_PORT`) | tcp | ssh 维护（key-only） |
| 20000 | 20000 (`SFU_PORT`) | udp | SFU 媒体（单端口复用） |
| 40000-40100 | 同左 | udp | 遗留范围，当前模型不用，可删 |

**卷**（持久化，必须挂）：

| 容器路径 | 内容 |
|---|---|
| `/opt/msrtc/etc` | 密钥（PSK/JWT）、账户/设备表、ssh host key、Caddyfile（首启生成） |
| `/opt/msrtc/recordings` | 录像 |

不挂卷 = 重建丢数据、密钥重生。

### 4.1 SFU 端口坑（重要）

客户端连的是**宣告 IP : 容器内 SFU 端口（20000）**，不是宿主偏移口。默认
`20000:20000` 是 1:1，正确。若宿主侧换成 21000，必须同时
`-e MEDIASERVO_SFU_PORT=21000` **且** `-p 21000:21000/udp`（1:1），否则宣告 20000
与宿主 21000 不一致 → 媒体不通（信令正常、画面黑屏）。

> 资源档位：Small（10 房/20 人 720p）实测峰值 56.81MiB，`mem_limit` 取 512m
> （约 9× 余量）。更大并发请在测试环境按同样方法压测后上调 `MEM_LIMIT`。

## 5. 验证

```bash
IP=127.0.0.1; P=8080   # 换成现场 IP/端口

# 健康（caddy+路由+server 复合探针）
curl --noproxy '*' -f http://$IP:$P/health

# 网页内容判别（经 8080）
curl --noproxy '*' -s http://$IP:$P/admin  | grep -o '<title>[^<]*</title>'   # MediaServo Admin
curl --noproxy '*' -s http://$IP:$P/voice/ | grep -o '<title>[^<]*</title>'   # MediaServo Voice

# host 直连
curl --noproxy '*' -f http://$IP:9800/health

# admin 首登
curl --noproxy '*' -s -X POST http://$IP:$P/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"<密码>"}'    # 期望返回 token
```

浏览器：`http://<IP>:8080/admin` 登录；`http://<IP>:8080/voice/` 对讲页。

> 宿主 curl 打 localhost 若配了 `http_proxy` 会被劫持返 502 —— 统一 `--noproxy '*'`。

## 6. 日常运维

```bash
# 状态
docker compose --env-file .env ps
docker inspect <container> --format '{{.State.Health.Status}}'

# 日志（服务日志在容器内 oxmgr 文件面 run/logs/*.out.log，不是 docker logs）
docker compose --env-file .env exec server sh -c 'tail -100 /opt/msrtc/run/logs/*.out.log'

# 备份（两个卷打包；用镜像自身 tar，无需外网）
docker run --rm --entrypoint sh \
  -v <project>_msrtc-data:/data:ro \
  -v "$PWD/backup:/backup" msrtc-server:<tag> \
  -c 'tar czf /backup/msrtc-data.tar.gz -C /data .'
# recordings 卷同理

# 优雅停机（镜像 STOPSIGNAL SIGINT；约 1s，exit 0）
docker compose --env-file .env stop
# 注意：docker kill（SIGTERM/SIGKILL）不优雅停 app；优先用 stop

# 重启 / 重起
docker compose --env-file .env restart
docker compose --env-file .env up -d        # 配置变更后 recreate
```

## 7. 升级与回滚

```bash
# 升级：加载新 tar -> 改 .env 的 TAG -> recreate
docker load -i msrtc-server-<新tag>.tar
sed -i 's/^TAG=.*/TAG=<新tag>/' .env
docker compose --env-file .env up -d
curl --noproxy '*' -f http://<IP>:8080/health

# 回滚：保留上一版 tar -> 重钉旧 TAG -> recreate（数据卷不动）
docker load -i msrtc-server-<旧tag>.tar
sed -i 's/^TAG=.*/TAG=<旧tag>/' .env
docker compose --env-file .env up -d
# 回滚后 admin 登录仍可用（账户表在命名卷持久化）
```

## 8. 故障排查

| 现象 | 排查 |
|---|---|
| `ps` 不 healthy | `docker inspect` 看 Health.Log；`tail run/logs/*.out.log`；确认 8080 未被占 |
| 网页打不开 | 确认 8080 映射 + 防火墙 8080/tcp；`curl :8080/health` |
| **信令通、画面黑屏/无媒体** | SFU 媒体不通：① `MEDIASERVO_SFU_ANNOUNCED_IP` 是否客户端可达（多网卡必填公网/局域网 IP）；② 20000/udp 是否放行且 1:1 映射（§4.1）；③ 防火墙 20000/udp |
| admin 登不上 | 首启 `MEDIASERVO_ADMIN_PASSWORD` 是否设；容器日志 setup-token 兜底 |
| ssh 连不上 | `etc/ssh/authorized_keys` 是否投了公钥（日志 WARN）；`SSH_PORT` 是否对 |
| 容器起不来 | `docker compose ... logs`；端口占用（8080/9800/2222）；磁盘满 |
| 数据丢失/密钥变 | 是否挂了 `etc`/`recordings` 卷（§4）；无卷 = 每次重建重生 |

## 9. 首次装机要点（详见《部署手册》`DEPLOY-MANUAL.md`）

- **docker-ce**：CentOS 8 已 EOL，基础源需切 `vault.centos.org`，再装
  `docker-ce docker-ce-cli containerd.io docker-compose-plugin`。
- **UDP 内核参数**：`net.core.rmem_max=16777216`、`net.core.netdev_max_backlog=5000`
  （`/etc/sysctl.d/99-msrtc.conf` + `sysctl --system`）。
- **防火墙**（firewalld）：放行 8080/tcp、9800/tcp、20000/udp、2222/tcp。
- **SELinux**：命名卷通常 enforcing 可跑；遇阻查 `ausearch -m avc -ts recent` 精修，
  **不要直接关 SELinux**。

## 10. 凭据与安全

- **admin 密码**：`.env` 的 `MEDIASERVO_ADMIN_PASSWORD` 仅首启引导；交付后建议经
  admin 页面轮换（旧密码不再影响已存在账户）。
- **ssh**：key-only（密码登录禁用），`mediaservo` 用户（禁 root）。host key +
  `authorized_keys` 在 `etc` 卷（**不烤进镜像**）。投公钥：

  ```bash
  docker compose --env-file .env exec -T server sh -c \
    'cat >> /opt/msrtc/etc/ssh/authorized_keys' < ops_id_ed25519.pub
  docker compose --env-file .env exec server sh -c \
    'chown mediaservo:mediaservo /opt/msrtc/etc/ssh/authorized_keys; chmod 600 /opt/msrtc/etc/ssh/authorized_keys'
  ssh -p 2222 mediaservo@<IP>
  ```

## 11. 命令速查卡

```bash
sha256sum -c SHA256SUMS                          # 对账
docker load -i msrtc-server-<tag>.tar            # 加载
cp .env.example .env && vi .env                  # 配置
docker compose --env-file .env up -d             # 起
docker compose --env-file .env ps                # 状态
curl --noproxy '*' -f http://<IP>:8080/health    # 健康
docker compose --env-file .env exec server sh -c 'tail -50 /opt/msrtc/run/logs/*.out.log'  # 日志
docker compose --env-file .env stop              # 优雅停
docker compose --env-file .env up -d             # 升级/回滚后 recreate（先改 TAG）
```

---

**故障升级路径**：现场运维 → 保留 `run/logs/*.out.log` + `docker inspect` 输出 +
`docker compose config` 结果 → 交付方研发。**不要**在生产现场直接改容器内文件
（改配置请经 admin 页面或卷内改后 recreate）。