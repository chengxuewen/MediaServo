# MediaServo Container Image Operations Manual

> Ships with the delivery folder. Audience: **site operations staff** — how to
> load, configure, start, verify, operate day-to-day, troubleshoot, and
> upgrade/rollback after receiving the package.
> **First-time install (CentOS 8 vault repos / kernel tuning / SELinux / the
> offline drill) is in the companion `DEPLOY-MANUAL.md`**; this manual only
> carries the self-contained day-2 steps.

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
| 40000-40100 | same | udp | legacy range, unused by the current model — safe to drop |

**Volumes** (persistence — required):

| Container path | Contents |
|---|---|
| `/opt/msrtc/etc` | secrets (PSK/JWT), account/device tables, ssh host keys, Caddyfile (generated on first boot) |
| `/opt/msrtc/recordings` | recordings |

No volumes = data lost and secrets regenerated on every recreate.

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

# Logs (service logs are the oxmgr file face run/logs/*.out.log, NOT docker logs)
docker compose --env-file .env exec server sh -c 'tail -100 /opt/msrtc/run/logs/*.out.log'

# Backup (pack the two volumes; uses the image's own tar, no internet needed)
docker run --rm --entrypoint sh \
  -v <project>_msrtc-data:/data:ro \
  -v "$PWD/backup:/backup" msrtc-server:<tag> \
  -c 'tar czf /backup/msrtc-data.tar.gz -C /data .'
# same for the recordings volume

# Graceful stop (image STOPSIGNAL SIGINT; ~1s, exit 0)
docker compose --env-file .env stop
# Note: docker kill (SIGTERM/SIGKILL) does NOT stop the apps gracefully;
# prefer stop

# Restart / recreate
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
| Container won't start | `docker compose ... logs`; port conflicts (8080/9800/2222); disk full |
| Data lost / secrets changed | are the `etc`/`recordings` volumes mounted (§4)? no volumes = regenerated on every recreate |

## 9. First-Install Essentials (details in DEPLOY-MANUAL.md)

- **docker-ce**: CentOS 8 is EOL — switch the base repos to `vault.centos.org`
  first, then install `docker-ce docker-ce-cli containerd.io
  docker-compose-plugin`.
- **UDP kernel params**: `net.core.rmem_max=16777216`,
  `net.core.netdev_max_backlog=5000` (`/etc/sysctl.d/99-msrtc.conf` +
  `sysctl --system`).
- **Firewall** (firewalld): allow 8080/tcp, 9800/tcp, 20000/udp, 2222/tcp.
- **SELinux**: named volumes usually run fine under enforcing; on denials
  check `ausearch -m avc -ts recent` and refine — **do not disable SELinux
  wholesale**.

## 10. Credentials and Security

- **admin password**: `MEDIASERVO_ADMIN_PASSWORD` in `.env` only bootstraps
  the first boot; rotate it via the admin UI after handover (the old value no
  longer affects existing accounts).
- **ssh**: key-only (password login disabled), user `mediaservo` (root
  disabled). Host keys + `authorized_keys` live in the etc volume (**never
  baked into the image**). Deliver a public key:

  ```bash
  docker compose --env-file .env exec -T server sh -c \
    'cat >> /opt/msrtc/etc/ssh/authorized_keys' < ops_id_ed25519.pub
  docker compose --env-file .env exec server sh -c \
    'chown mediaservo:mediaservo /opt/msrtc/etc/ssh/authorized_keys; chmod 600 /opt/msrtc/etc/ssh/authorized_keys'
  ssh -p 2222 mediaservo@<IP>
  ```

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
