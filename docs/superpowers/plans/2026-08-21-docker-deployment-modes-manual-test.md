# docker-deployment-modes 人工测试计划

> 配套 `docs/superpowers/plans/2026-08-21-docker-deployment-modes.md` 的验证文档。
> 目的：人工逐步验证改造产物（CLI 命令面 / entrypoint 自举 / prod compose / 回归），
> 覆盖自动化未覆盖的交互/时序/持久化面。
> 状态：✅ = 通过 / ❌ = 失败（记录现象）/ ⏳ = 待执行

## 前置

- [ ] dev server 容器可用（`./mediaservo.sh up --env dev`）
- [ ] runtime 镜像构建完成（`./mediaservo.sh build server --image runtime`——基线数字记录）
- [ ] 环境负载正常（`uptime`——load < 4，构建类任务结束后测回归组）

---

## A 组：CLI 命令面（不依赖镜像——先行）

| # | 命令 | 预期结果 | 实际 |
|---|---|---|---|
| A1 | `./mediaservo.sh -h` | 目标命令面：build/up/down/restart/logs/exec/e2e/test/ci/install/package/clean/config/data/doctor/version + `build --image` 与 `e2e <suite>` 帮助 | |
| A2 | `./mediaservo.sh doctor` | pixi/cargo/docker/node 版本输出（4 行）| |
| A3 | `./mediaservo.sh up --env dev server` | 幂等（已有容器 → Started；无报错）| |
| A4 | `./mediaservo.sh ps` | compose ps 表格——server 行含 9800/20000/40000-40100 映射 | |
| A5 | `./mediaservo.sh logs server --follow` | 日志跟踪（Ctrl-C 退出）| |
| A6 | `./mediaservo.sh exec server -- sh -c 'echo ok'` | 输出 `ok` | |
| A7 | `./mediaservo.sh config set signaling.psk test123` | 写 `config/host.conf` + 提示（dev 轨道）| |
| A8 | `./mediaservo.sh data inspect` | 卷列表（含 mediaservo_mediaservo-data）| |

## B 组：entrypoint 自举（需 runtime 镜像）

| # | 命令 | 预期结果 | 实际 |
|---|---|---|---|
| B1 | `docker run --rm -v ms-data-test:/opt/mediaservo/etc -v ms-rec-test:/opt/mediaservo/recordings -e MEDIASERVO_ADMIN_PASSWORD=test123 -p 9800:9800 mediaservo-server:runtime-baseline` | 日志顺序：`generated fresh secrets` → `created bootstrap admin` → `psk_set=true` → `Listening on 0.0.0.0:9800` → `Server ready` | |
| B2 | `docker exec <c> ls -la /opt/mediaservo/etc/` | `.env` + `server.yaml` 权限 600、属主 mediaservo；accounts.yaml/devices.yaml 存在 | |
| B3 | `docker exec <c> cat /opt/mediaservo/etc/.env` | `MEDIASERVO_PSK=<64hex>`（**非** mediaservo-dev）| |
| B4 | 浏览器/curl `http://127.0.0.1:9800/admin` + 登录 `admin`/`test123` | 200 + 登录成功（引导账号生效）| |
| B5 | `docker restart <c>` → `docker exec <c> cat /opt/mediaservo/etc/.env` | PSK **不变**（卷持久）；配置不覆盖（可在重启前改 devices.yaml 验证）| |
| B6 | `docker run --rm -v ms-data-test2:... -p 9801:9800 mediaservo-server:runtime-baseline` | **新密钥**（与 B3 不同——每实例独立）| |
| B7 | 负例：卷内 `.env` 缺失（`docker exec <c> rm .env` 后重启）| 日志 `init incomplete` → exit 1（不启 server——幂等保护）| |

## C 组：prod compose（需镜像）

| # | 命令 | 预期结果 | 实际 |
|---|---|---|---|
| C1 | `MEDIASERVO_ADMIN_PASSWORD=test123 ./mediaservo.sh up --env prod` | 根 compose 起 server（命名卷首启初始化）——admin 200 | |
| C2 | 登录 admin/test123 → Settings → Provision Device（note: smoke-manual）| 配发成功；`docker logs` **无** "falling back to direct write"（命名卷原子写）| |
| C3 | `./mediaservo.sh ps --env prod` + `./mediaservo.sh logs server --follow` | ps 显示 prod 容器；日志无 `mediaservo-dev`（密钥非默认）| |
| C4 | `./mediaservo.sh down --env prod` → `./mediaservo.sh up --env prod` | **卷持久化**：登录仍成功（账号在）+ devices.yaml 含 smoke-manual 设备 | |
| C5 | `./mediaservo.sh data backup /tmp/ms-backup` | `mediaservo_mediaservo-data.tar.gz` + `...recordings.tar.gz` 生成 | |
| C6 | `./mediaservo.sh data reset --force` → `./mediaservo.sh up --env prod` | 卷清空 → 重新初始化（新密钥 + 空注册表）| |

## D 组：回归（load 正常时）

| # | 命令 | 预期结果 | 实际 |
|---|---|---|---|
| D1 | `./mediaservo.sh e2e sfu` | 4 passed | |
| D2 | `./mediaservo.sh e2e push` | 6 passed（此前高负载环境干扰——构建后应恢复）| |
| D3 | `./mediaservo.sh e2e host` | e2e-install-host PASS | |
| D4 | `./mediaservo.sh e2e package` | e2e-package PASS | |
| D5 | `./mediaservo.sh e2e brand` | e2e-brand PASS | |
| D6 | `./mediaservo.sh e2e ui` | Playwright（device-provisioning spec）1 passed | |
| D7 | `./mediaservo.sh e2e smoke` | prod smoke PASS（= C 组自动化断言）| |

## E 组：清理

| # | 命令 | 预期 |
|---|---|---|
| E1 | `./mediaservo.sh down --env prod` | prod 容器停止（卷保留）|
| E2 | `docker volume rm ms-data-test ms-data-test2 ms-rec-test` | 测试卷删除 |
| E3 | `./mediaservo.sh up --env dev` | 恢复 dev 环境 |

---

## 结果记录

| 日期 | 组 | 结果 | 备注 |
|---|---|---|---|
| | | | |
| | | | |

## 失败排查提示

- **B 组 init 失败**：`docker logs <c>` 看 entrypoint 输出（生成/权限/模板路径）；检查卷属主（`docker run --rm -v ms-data-test:/d alpine ls -la /d`）
- **C2 配发 EBUSY**：命名卷下 rename 应成功——若出现 fallback 日志 → 卷挂载异常（`docker inspect` 确认卷类型）
- **D2 帧测试挂**：先查 `uptime`（load >5 时编码器停摆——环境干扰，stash 对照证非代码）；再查 `docker logs` 有无 RTP/worker 错误
- **smoke 登录失败**：MEDIASERVO_ADMIN_PASSWORD 需在**首次** up 时设置（引导只首启生效）
