# MediaServo Container Image Operations Manual

> Ships with the delivery folder. Audience: **site operations staff** — how to
> load, configure, start, verify, operate day-to-day, troubleshoot, and
> upgrade/rollback after receiving the package.
> **First-time install (docker install / kernel tuning / LSM / the
> offline drill) is in the companion `DEPLOY-MANUAL.md`**; this manual only
> carries the self-contained day-2 steps.
> Applies to the image tag shipped in this folder (`msrtc-server-<tag>.tar`).

## 0. What Is in the Package

The deliverable is **one directory** (named `msrtc-server-image-<tag>/`,
containing these 6 items):

| File | Description |
|---|---|
| `msrtc-server-<tag>.tar` | image archive (`docker save`; dual tags `<tag>` + `latest`) |
| `SHA256SUMS` | sha256 manifest of the tar (handover reconciliation) |
| `docker-compose.yml` | production compose (single container: server + caddy + oxmgr) |
| `.env.example` | configuration template (copy to `.env` and fill) |
| `DEPLOY-MANUAL.md` | **deployment engineer manual** (install/tuning/firewall/offline drill/credentials) |
| `OPS-MANUAL.md` | this manual (**day-2 operations**) |

**Single-container topology**: one container runs the whole tree —
`msrtc-server` (signaling + admin + SFU) + `caddy` (8080 web/reverse proxy) +
`oxmgr` (process manager, container PID1). No separate proxy container.

## 1. Preflight

```bash
docker --version                      # needs docker-ce 20.10+
docker compose version                # needs the compose v2 plugin
```

- **Ports**: 8080/tcp (web), 9800/tcp (host direct), 20000/udp (media),
  2222/tcp (ssh maintenance, changeable). First-time install: see DEPLOY-MANUAL.
- **Resources**: `mem_limit` defaults to 512m (measured at the standard tier,
  see §4); reserve disk according to recording volume.

## 2. Quick Start (5 Steps)

```bash
# 1) Verify the image archive integrity
sha256sum -c SHA256SUMS

# 2) Load the image (no internet needed)
docker load -i msrtc-server-<tag>.tar

# 3) Prepare the configuration
cp .env.example .env
vi .env            # fill the required values from §3

# 4) Start the stack
docker compose --env-file .env up -d

# 5) Check status (wait for healthy; start-period is 60s)
docker compose --env-file .env ps
```

## 3. Configuration (.env)

**Required**

| Variable | Meaning | Note |
|---|---|---|
| `TAG` | image tag (for rollback) | must match the loaded tar's tag |
| `MEDIASERVO_SFU_ANNOUNCED_IP` | SFU announced IP | **on multi-NIC hosts you MUST set the client-reachable IP explicitly** (auto-detection takes the first IP and silently breaks media) |
| `MEDIASERVO_ADMIN_PASSWORD` | first-boot admin password | written to the account table on first boot; change it via the admin UI afterwards |
| `SSH_PORT` | host ssh port (default 2222) | change when it collides with an existing host sshd |

**Optional**

| Variable | Default | Meaning |
|---|---|---|
| `MEDIASERVO_BRAND` | `msrtc` | tree prefix `/opt/<brand>` + volume paths; changing it requires a matching image |
| `MEM_LIMIT` | `512m` | container memory limit (memory only; CPU unlimited); raise per §4 for larger loads |
| `RUST_LOG` | `info` | log level |
| `WEB_PORT`/`AGENT_PORT`/`SFU_PORT` | 8080/9800/20000 | shift the **host-side** ports only; container targets are fixed |

> Do not change the in-container ports (`MEDIASERVO_WEB_PORT` /
> `MEDIASERVO_SFU_PORT`) unless you also change the compose targets and keep
> them 1:1 (see the SFU pitfall in §4.1).

## 4. Port Map and Volumes

| Container | Host default | Proto | Purpose |
|---|---|---|---|
| 8080 | 8080 | tcp | web entry: `/admin` `/voice/` `/api` `/ws` `/health` |
| 9800 | 9800 | tcp | host-agent direct `/ws` |
| 22 | 2222 (`SSH_PORT`) | tcp | ssh maintenance (key-only) |
| 20000 | 20000 (`SFU_PORT`) | udp | SFU media (single-port mux) |

**Volumes** (persistence — required):

| Container path | Contents |
|---|---|
| `/opt/msrtc/etc` | secrets (PSK/JWT), account/device tables, ssh host keys, Caddyfile (generated on first boot) |
| `/opt/msrtc/recordings` | recordings |
| `/opt/msrtc/run/logs` | service logs (oxmgr app `*.out.log`) — persist across recreates (upgrades/rollbacks) |

No volumes = data lost and secrets regenerated on every recreate.

> `run/oxfile.toml` is intentionally NOT persisted: init re-renders it from the
> current `.env` on every first boot, so env changes (e.g. ANNOUNCED_IP) always
> take effect. Persistent logs grow over time — truncate or archive
> `run/logs/*.out.log` periodically.

### 4.1 The SFU Port Pitfall (Important)

Clients connect to the **announced IP : the in-container SFU port (20000)**,
not the host-shifted port. The default `20000:20000` is 1:1 and correct. If you
move the host side to 21000 you must ALSO set `-e MEDIASERVO_SFU_PORT=21000`
**and** `-p 21000:21000/udp` (1:1); otherwise the announced 20000 mismatches
the host 21000 → media breaks (signaling fine, black screen).

> Capacity ladder (`MEM_LIMIT`): **512m** = standard tier (10 rooms / 20 peers
> @720p, MEASURED peak 56.81 MiB, ~9× headroom); **1g** ≈ 2× standard (e.g. 20
> rooms / 40 peers — extrapolated, verify on site); **2g** ≈ 4× standard (e.g.
> 50 rooms / 100 peers — extrapolated, re-measure before relying on it). Memory
> scales with the room/peer object graph, not bitrate; re-measure when doubling
> tiers.

## 5. Verification

```bash
IP=127.0.0.1; P=8080   # replace with the site IP/port

# Health (composite probe: gateway + routing + service)
curl --noproxy '*' -f http://$IP:$P/health

# Web content check (via 8080)
curl --noproxy '*' -s http://$IP:$P/admin  | grep -o '<title>[^<]*</title>'   # MediaServo Admin
curl --noproxy '*' -s http://$IP:$P/voice/ | grep -o '<title>[^<]*</title>'   # MediaServo Voice

# Host direct face
curl --noproxy '*' -f http://$IP:9800/health

# admin first login
curl --noproxy '*' -s -X POST http://$IP:$P/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"<pass>"}'    # expect a token back
```

Browser: log in at `http://<IP>:8080/admin`; the intercom page is
`http://<IP>:8080/voice/`.

> curl from a host that exports `http_proxy` gets hijacked to a 502 when
  targeting localhost — always pass `--noproxy '*'`.

## 6. Day-2 Operations

```bash
# Status
docker compose --env-file .env ps
docker inspect <container> --format '{{.State.Health.Status}}'

# Logs (oxmgr file face run/logs/*.out.log, NOT docker logs; volume-backed so
# they survive recreates — post-mortem evidence stays after upgrades/rollbacks)
docker compose --env-file .env exec server sh -c 'tail -100 /opt/msrtc/run/logs/*.out.log'

# Backup (pack the two volumes; uses the image's own tar, no internet needed)
docker run --rm --entrypoint sh \
  -v <project>_data:/data:ro \
  -v "$PWD/backup:/backup" msrtc-server:<tag> \
  -c 'tar czf /backup/msrtc_data.tar.gz -C /data .'
# same for the recordings volume
# (<project> = the compose project name; default msrtc -> msrtc_data)

# Graceful stop (image STOPSIGNAL SIGINT; ~1s, exit 0)
docker compose --env-file .env stop
# Note: docker kill (SIGTERM/SIGKILL) does NOT stop the apps gracefully;
# prefer stop

# Restart / recreate
# (restart: unless-stopped is set in the compose file — after a host reboot
#  the stack returns automatically once docker.service is enabled)
docker compose --env-file .env restart
docker compose --env-file .env up -d        # recreate after a config change
```

## 7. Upgrade and Rollback

```bash
# Upgrade: load the new tar -> bump TAG in .env -> recreate
docker load -i msrtc-server-<new-tag>.tar
sed -i 's/^TAG=.*/TAG=<new-tag>/' .env
docker compose --env-file .env up -d
curl --noproxy '*' -f http://<IP>:8080/health
docker image prune -f               # optional: reclaim superseded image layers

# Rollback: keep the previous tar -> re-pin the old TAG -> recreate (volumes untouched)
docker load -i msrtc-server-<old-tag>.tar
sed -i 's/^TAG=.*/TAG=<old-tag>/' .env
docker compose --env-file .env up -d
# admin login still works after rollback (the account table persists in the
# named volume)
```

## 8. Troubleshooting

| Symptom | Where to look |
|---|---|
| `ps` not healthy | `docker inspect` Health.Log; `tail run/logs/*.out.log`; is 8080 already taken? |
| Web page unreachable | check the 8080 mapping + firewall 8080/tcp; `curl :8080/health` |
| **Signaling fine, black screen / no media** | SFU media path broken: ① is `MEDIASERVO_SFU_ANNOUNCED_IP` client-reachable (mandatory on multi-NIC)? ② is 20000/udp allowed and mapped 1:1 (§4.1)? ③ firewall 20000/udp |
| admin login fails | was `MEDIASERVO_ADMIN_PASSWORD` set on first boot? container logs have the setup-token fallback |
| ssh unreachable | was a public key delivered to `etc/ssh/authorized_keys` (WARN in logs)? is `SSH_PORT` right? |
| **After a volume re-init, the UI shows 401s and admin WS closes** | expected: a fresh etc volume re-runs init and rotates the JWT secret — the browser's old token is invalid. Clear site data (or log out) and sign in again with MEDIASERVO_ADMIN_PASSWORD |
| Container won't start | `docker compose ... logs`; port conflicts (8080/9800/2222); disk full |
| Console shows `/weaknet/v1/capabilities 502` | expected in production: the image ships no weaknet app; the Caddyfile route is a known no-op (console noise only, no impact) |
| startup log shows **"dev placeholder accounts active"** | a dev-only toggle is enabled (`MEDIASERVO_ALLOW_DEV_CREDENTIALS`) or dev accounts were mounted into the etc volume — production MUST unset it and use the admin bootstrap instead (DEPLOY-MANUAL §5) |
| Data lost / secrets changed | are the `etc`/`recordings`/`run/logs` volumes mounted (§4)? no volumes = regenerated on every recreate |

## 9. First-Install Essentials (details + per-distro recipes: DEPLOY-MANUAL §1-§2, Appendix A)

- **docker-ce 20.10+ / compose v2 / systemd** — any mainstream distro; install
  recipes (CentOS 8 vault / Rocky / Alma / RHEL / Ubuntu / Debian) live in
  DEPLOY-MANUAL Appendix A.
- **UDP kernel params, firewall ports, LSM notes** (SELinux / AppArmor): see
  DEPLOY-MANUAL §2 (distro-neutral core) and Appendix A (distro specifics).

## 10. Credentials and Security

- **admin password**: `MEDIASERVO_ADMIN_PASSWORD` in `.env` only bootstraps
  the first boot; rotate it via the admin UI after handover (the old value no
  longer affects existing accounts).
- **ssh**: key-only (password login disabled), user `mediaservo` (root
  disabled). Host keys + `authorized_keys` live in the etc volume (**never
  baked into the image**). Public-key delivery and login-verification steps:
  **see DEPLOY-MANUAL.md §5** (single source — the drill-tested commands live there).

## 11. Command Cheat Card

```bash
sha256sum -c SHA256SUMS                          # reconcile
docker load -i msrtc-server-<tag>.tar            # load
cp .env.example .env && vi .env                  # configure
docker compose --env-file .env up -d             # start
docker compose --env-file .env ps                # status
curl --noproxy '*' -f http://<IP>:8080/health    # health
docker compose --env-file .env exec server sh -c 'tail -50 /opt/msrtc/run/logs/*.out.log'  # logs
docker compose --env-file .env stop              # graceful stop
docker compose --env-file .env up -d             # recreate after upgrade/rollback (bump TAG first)
```

---

**Escalation path**: site ops → collect `run/logs/*.out.log` + `docker
inspect` output + `docker compose config` result → hand to the delivering
engineering team. Do **not** edit files inside the container on site (change
configuration via the admin UI, or edit inside the volume and recreate).
