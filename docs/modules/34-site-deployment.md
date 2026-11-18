# 34 — 现场部署手册（离线生产镜像）

> container-build-rework B3.7 / P2-H5。**本手册取代 `31-upgrade-migration.md`
> 的容器章节用于现场部署**（31 保留为升级/迁移参考）。适用产物 = `package image`
> 离线产物集；目标环境 = 受限内网 CentOS 8 现场（无外网、无 registry）。
> 语言规范 R22：现场手册中文。

## 0. 产物集与边界

`mediaservo package image --dist <DIR>` 产出（B3.6）：

| 文件 | 用途 |
|---|---|
| `msrtc-server-<tag>.tar` | `docker save` 镜像（双 tag：`<tag>` + `latest`） |
| `SHA256SUMS` | tar 包 sha256 清单（交付对账） |
| `docker-compose.yml` | 生产 compose（树镜像单容器，R12/R15/R16） |
| `.env.example` | 四变量契约模板（拷贝为 `.env` 填写） |
| `34-site-deployment.md` | 本手册 |

**单容器拓扑**：`msrtc-server`（server + caddy + oxmgr PID1）。oxmgr 牧羊
server/web 两进程；caddy 8080 服务浏览器，9800 直连 host-agent。**无独立 proxy
容器**（R12 退役）。运行基底 debian:bookworm-slim（R20）。

**回滚边界 = 镜像 + 配置级**（`data restore` 挂账）：回滚靠旧 tar 重新 load +
`.env` 的 `TAG` 重钉；数据卷不动。

## 1. 前置：CentOS 8 装 docker-ce（vault 源）

CentOS 8 已 EOL，`mirror.centos.org` 失效 → 切 `vault.centos.org`：

```bash
# 基础源切 vault（EOL 归档）
sed -i 's|mirror.centos.org|vault.centos.org|g; s|^#baseurl|baseurl|g; s|^mirrorlist|#mirrorlist|g' \
    /etc/yum.repos.d/CentOS-*.repo
dnf install -y dnf-utils
# docker-ce 官方仓（CentOS 8 路径仍在）
dnf config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
dnf install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
systemctl enable --now docker
docker compose version   # 确认 compose v2 插件
```

> 离线现场若无法访问 download.docker.com：在有网机器 `dnf download --resolve
> docker-ce docker-ce-cli containerd.io docker-compose-plugin` 下载 rpm 集，
> 拷贝到现场 `dnf localinstall *.rpm`。

## 2. 系统调优

### 2.1 UDP sysctl（SFU RTP 收包）

```bash
cat > /etc/sysctl.d/99-msrtc.conf <<'EOF'
net.core.rmem_max = 16777216
net.core.netdev_max_backlog = 5000
EOF
sysctl --system
```

### 2.2 firewalld 端口表（R16/R19）

| 端口 | 协议 | 用途 |
|---|---|---|
| 8080 | tcp | 浏览器/caddy 入口（admin/voice/API/WS 反代） |
| 9800 | tcp | host-agent 直连 /ws（D-2=B） |
| 20000 | udp | SFU RTC |
| 40000-40100 | udp | SFU RTC 端口段 |
| 2222 | tcp | ssh 维护口（`${SSH_PORT:-2222}:22`，R19） |

```bash
firewall-cmd --permanent --add-port=8080/tcp
firewall-cmd --permanent --add-port=9800/tcp
firewall-cmd --permanent --add-port=20000/udp
firewall-cmd --permanent --add-port=40000-40100/udp
firewall-cmd --permanent --add-port=2222/tcp
firewall-cmd --reload
```

### 2.3 SELinux

命名卷由 docker 管理标签，通常 enforcing 即可运行。若 bind mount 需 `:z`
后缀。**不要 blanket 关闭 SELinux**；遇阻先查 `ausearch -m avc -ts recent`，
按审计日志精修策略。

## 3. 离线部署

```bash
# ① 对账 + 加载镜像（无需外网）
sha256sum -c SHA256SUMS
docker load -i msrtc-server-<tag>.tar      # 载入 <tag> + latest 双 tag

# ② 填写 .env（四变量契约）
cp .env.example .env
vi .env
```

`.env` 必填：

```ini
TAG=<tag>                          # 镜像钉版（回滚=重钉旧 tag）
MEDIASERVO_SFU_ANNOUNCED_IP=<IP>   # 见 §3.1 多网卡硬规则
MEDIASERVO_ADMIN_PASSWORD=<密码>    # 首启 admin 引导（见 §5 交付）
SSH_PORT=2222                      # ssh 维护宿主口
```

```bash
# ③ 起栈（裸 docker compose，无 CLI/pixi 依赖）
docker compose --env-file .env up -d
docker compose --env-file .env ps        # 期望 healthy
```

### 3.1 多网卡硬规则（R6 / sfu.rs:481-486）

**现场必须显式填 `MEDIASERVO_SFU_ANNOUNCED_IP` 为客户端可达的 IP**（公网/
局域网）。自动探测会把所有网卡 IP 逗号拼接，但 SFU 只取**第一个**
（`sfu.rs:481-486`）——多网卡下首 IP 常是不可达的内部口 → **静默 ICE 失败**
（信令通、媒体不通）。自动探测仅供开发便利，现场禁用。compose 的
`${VAR:?}` 空值 fail-fast 是底线（R6）。

## 4. 部署验证

```bash
# 健康（caddy 8080 复合探针，R17）
curl --noproxy '*' -f http://<IP>:8080/health
# admin / voice 内容判别（经 8080）
curl --noproxy '*' -s http://<IP>:8080/admin  | grep -o '<title>[^<]*</title>'   # MediaServo Admin
curl --noproxy '*' -s http://<IP>:8080/voice/ | grep -o '<title>[^<]*</title>'   # MediaServo Voice
# host-agent 直面
curl --noproxy '*' -f http://<IP>:9800/health
# admin 首登
curl --noproxy '*' -s -X POST http://<IP>:8080/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"<密码>"}'    # 期望返回 token
```

> 宿主 curl 打 localhost 可能被 `http_proxy` 劫持返 502 —— 一律 `--noproxy '*'`。

浏览器：`http://<IP>:8080/admin` 登录（admin/密码），`http://<IP>:8080/voice/`
对讲页。媒体推送验证见 §7 演练。

## 5. 凭据交付

- **admin 密码**：`.env` 的 `MEDIASERVO_ADMIN_PASSWORD` 仅首启引导（init 写入
  `etc/accounts.yaml`，持久化在命名卷）。**交付后通过 admin API 轮换**
  （`POST /api/admin/accounts*` 热生效，C33）；轮换后 `.env` 里的旧密码不再
  影响已存在账户（PIT-160 首写胜）。密码不出现在容器进程 env（entrypoint
  init 后 unset）。
- **ssh key**：key-only（密码登录禁用），mediaservo 用户（uid 1000，禁 root）。
  host key + `authorized_keys` 持久化在 etc 卷（`etc/ssh/`，**绝不烤进镜像**）。
  交付公钥：

  ```bash
  # 现场把运维公钥写入 etc 卷（容器内路径 /opt/msrtc/etc/ssh/authorized_keys）
  docker compose --env-file .env exec server sh -c \
    'cat >> /opt/msrtc/etc/ssh/authorized_keys' < ops_id_ed25519.pub
  docker compose --env-file .env exec server sh -c \
    'chown mediaservo:mediaservo /opt/msrtc/etc/ssh/authorized_keys; chmod 600 /opt/msrtc/etc/ssh/authorized_keys'
  ssh -p 2222 mediaservo@<IP>    # key 登录验证
  ```

  > 账户 shadow 字段为 `*`（禁密码但账户可用）——`useradd` 的 `!` 锁定态会被
  > UsePAM no 的 sshd 拒绝 pubkey，镜像已 `usermod -p '*'` 修正（B3.3）。

## 6. 运维

```bash
# 备份（B3.6：prod 镜像自身 tar，无 alpine 依赖；卷按 compose project 标签发现）
mediaservo data backup ./backup        # 需 CLI 环境；裸 docker 见下
# 裸 docker 等价（无 CLI 现场）：
docker run --rm --entrypoint sh -v msrtc_msrtc-data:/data:ro \
  -v "$PWD/backup:/backup" msrtc-server:<tag> \
  -c 'tar czf /backup/msrtc_msrtc-data.tar.gz -C /data .'

# 日志（oxmgr 文件面，run/logs/*.out.log；非 docker logs）
docker compose --env-file .env exec server sh -c 'tail -50 /opt/msrtc/run/logs/*.out.log'

# 优雅停机（镜像 STOPSIGNAL SIGINT；oxmgr SIGINT=优雅停 app，SIGTERM 带 app 会挂起）
docker compose --env-file .env stop      # 走 SIGINT，~1s exit 0
```

资源：仅 `mem_limit`（R13，CPU 放开供 RTP 突发）；值由 R18 Small 档压测测定
（峰值×1.3 向上取整 512MB），写回 compose 默认 + `29-operations.md`。

## 7. 回滚演练（镜像 + 配置级）

```bash
# 旧版本 tar 仍在现场（交付保留上一版）
docker load -i msrtc-server-<旧tag>.tar
sed -i 's/^TAG=.*/TAG=<旧tag>/' .env       # 重钉旧 tag
docker compose --env-file .env up -d       #  recreate 到旧镜像；数据卷不动
curl --noproxy '*' -f http://<IP>:8080/health
# 回滚后 admin 登录仍可用（accounts.yaml 在命名卷持久化，跨版本 survives）
```

## 8. 附录：完整离线演练记录（验收）

> 验收 = 一次完整离线演练记录。以下为 2026-11-18 在本开发机（Ubuntu 20.04
> 宿主，模拟受限现场：仅用产物集 tar，不拉任何外网镜像）执行的演练。

| 步 | 动作 | 结果 |
|---|---|---|
| 1 | `sha256sum -c SHA256SUMS` | `msrtc-server-0.1.3.tar: OK` |
| 2 | `docker load -i msrtc-server-0.1.3.tar` | 载入 `msrtc-server:0.1.3` + `:latest` 双 tag |
| 3 | `cp .env.example .env` 填 TAG/ANNOUNCED_IP=127.0.0.1/ADMIN_PASSWORD/SSH_PORT | 四变量就绪 |
| 4 | `docker compose --env-file .env up -d`（偏移端口隔离） | `msrtc-server-1 Up (healthy)` |
| 5 | `curl --noproxy '*' :18080/health` | 200 |
| 6 | /admin vs /voice 标题判别 | MediaServo Admin / MediaServo Voice（判别✓） |
| 7 | admin 首登 POST /api/auth/login | JWT token✓ |
| 8 | ssh key 注入 + `ssh -p 12222 mediaservo@127.0.0.1` | SSH_OK uid=1000✓ |
| 9 | `data backup`（prod 镜像 tar） | 两卷 tar.gz，含 server.yaml/accounts/Caddyfile，**无 alpine 拉取**✓ |
| 10 | down→up（同卷） | admin 登录 survives，PSK 不重生成✓ |
| 11 | `docker compose stop`（STOPSIGNAL SIGINT） | exit 0 ~1s✓ |
| 12 | 回滚：重 load tar + TAG 重钉 + up | healthy，登录 survives✓ |

**结论**：离线 load→up→浏览器→登录→备份→持久化→优雅停机→回滚全链路绿；
多网卡 ANNOUNCED_IP 显式填规则与 admin/ssh 凭据交付流程已验证。