# Site Deployment Manual (Offline Production Image) — Deployment Engineer

> **This manual = the deployment engineer's handbook** (first install and
> go-live: system tuning / firewall / SELinux / offline load → start → verify /
> credential handover / rollback drill). It ships in the delivery folder as
> `DEPLOY-MANUAL.md`.
> **Day-2 operations (start/verify/logs/backup/troubleshooting/upgrade/rollback/
> cheat card) live in the companion `OPS-MANUAL.md`.**
> Scope = the offline delivery folder (one directory); target environment =
> restricted intranet Linux sites (no internet, no image registry) — any
> distro with docker-ce 20.10+, the compose v2 plugin, and systemd; distro
> specifics live in Appendix A.
> Applies to the image tag shipped in this folder (`msrtc-server-<tag>.tar`).

## 0. Delivery Set and Boundaries

The deliverable is one directory `msrtc-server-image-<tag>/` (one folder per
release; copying that folder IS the handover):

| File | Purpose |
|---|---|
| `msrtc-server-<tag>.tar` | `docker save` image archive (dual tags: `<tag>` + `latest`) |
| `SHA256SUMS` | sha256 manifest of the tar (handover reconciliation) |
| `docker-compose.yml` | production compose (single container: service + gateway + process manager) |
| `.env.example` | four-variable contract template (copy to `.env` and fill) |
| `DEPLOY-MANUAL.md` | this manual |
| `OPS-MANUAL.md` | site operations manual (day-2: verify/logs/backup/upgrade/troubleshooting) |

**Single-container topology**: `msrtc-server` (server + caddy + oxmgr as PID1).
oxmgr shepherds the server/web processes; caddy serves browsers on 8080, while
9800 is the direct host-agent face. **No separate proxy container.** Runtime
base: debian:bookworm-slim.

**Rollback boundary = image + configuration level** (volume-level restore is a
tracked follow-up): rollback = re-load the old tar + re-pin `TAG` in `.env`;
data volumes are untouched.

## 1. Prerequisite: docker-ce (any mainstream Linux)

The stack ships as an offline image + compose; the host only needs:

- **docker-ce 20.10+** with the **compose v2 plugin**
- **systemd** (docker.service enabled at boot)
- root/sudo for the install-time steps

Install per your distribution — see **Appendix A** for the per-distro recipes
(CentOS 8 vault / Rocky / Alma / RHEL / Ubuntu / Debian, incl. offline rpm/deb
fetch). Verify after install:

```bash
systemctl enable --now docker
docker compose version   # must print a v2.x
```

## 2. System Tuning

### 2.1 UDP sysctl (SFU RTP receive path)

```bash
cat > /etc/sysctl.d/99-msrtc.conf <<'EOF'
net.core.rmem_max = 16777216
net.core.netdev_max_backlog = 5000
EOF
sysctl --system
```

### 2.2 Firewall Ports (firewalld — RHEL family; ufw equivalents in Appendix A)

| Port | Proto | Purpose |
|---|---|---|
| 8080 | tcp | browser / caddy entry (admin/voice/API/WS reverse proxy) |
| 9800 | tcp | host-agent direct /ws |
| 20000 | udp | SFU RTC |
| 2222 | tcp | ssh maintenance (`${SSH_PORT:-2222}:22`) |

```bash
firewall-cmd --permanent --add-port=8080/tcp
firewall-cmd --permanent --add-port=9800/tcp
firewall-cmd --permanent --add-port=20000/udp
firewall-cmd --permanent --add-port=2222/tcp
firewall-cmd --reload
```

### 2.3 SELinux (RHEL family; Ubuntu/Debian use AppArmor — usually no action)

Named volumes get their labels managed by docker; enforcing mode usually just
works. Bind mounts may need the `:z` suffix. **Do NOT blanket-disable
SELinux**; on denials check `ausearch -m avc -ts recent` first and refine the
policy from the audit log.

## 3. Offline Deployment

```bash
# 1) Reconcile + load the image (no internet needed)
sha256sum -c SHA256SUMS
docker load -i msrtc-server-<tag>.tar      # loads both <tag> and latest

# 2) Fill in .env (four-variable contract)
cp .env.example .env
vi .env
```

Required `.env` values:

```ini
TAG=<tag>                          # image pin (rollback = re-pin the old tag)
MEDIASERVO_SFU_ANNOUNCED_IP=<IP>   # see §3.1 multi-NIC hard rule
MEDIASERVO_ADMIN_PASSWORD=<pass>    # first-boot admin bootstrap (see §5)
SSH_PORT=2222                      # host-side ssh maintenance port
```

```bash
# 3) Start the stack (bare docker compose; no CLI/pixi dependency)
docker compose --env-file .env up -d
docker compose --env-file .env ps        # expect healthy
```

### 3.1 Multi-NIC Hard Rule

**On site you MUST set `MEDIASERVO_SFU_ANNOUNCED_IP` explicitly to an IP that
is reachable by clients** (public/LAN). Auto-detection joins all NIC IPs with
commas, but the SFU only takes the **first one** — on multi-NIC hosts the
first IP is often an unreachable internal interface → **silent ICE failure**
(signaling works, media does not). Auto-detection is a development convenience
only; do not rely on it on site. The compose `${VAR:?}` empty-value fail-fast
is the bottom line.

## 4. Deployment Verification

```bash
# Health (composite probe on 8080: gateway + routing + service)
curl --noproxy '*' -f http://<IP>:8080/health
# admin / voice content check (via 8080)
curl --noproxy '*' -s http://<IP>:8080/admin  | grep -o '<title>[^<]*</title>'   # MediaServo Admin
curl --noproxy '*' -s http://<IP>:8080/voice/ | grep -o '<title>[^<]*</title>'   # MediaServo Voice
# host-agent direct face
curl --noproxy '*' -f http://<IP>:9800/health
# admin first login
curl --noproxy '*' -s -X POST http://<IP>:8080/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"<pass>"}'    # expect a token back
```

> curl from the host to localhost can be hijacked by `http_proxy` (502) —
  always pass `--noproxy '*'`.

Browser: `http://<IP>:8080/admin` to log in (admin/password), and
`http://<IP>:8080/voice/` for the intercom page. Media publish verification:
see the drill in §7.

## 5. Credential Handover

- **admin password**: `MEDIASERVO_ADMIN_PASSWORD` in `.env` only bootstraps the
  first boot (init writes it into `etc/accounts.yaml`, persisted in the named
  volume). **Rotate after handover via the admin API**
  (`POST /api/admin/accounts*`, hot-effective). After rotation the old value in
  `.env` no longer affects existing accounts (init is the sole first-boot
  writer; later password changes are owned by the account table). The password
  never appears in the container process env (the entrypoint unsets it after
  init).
- **ssh key**: key-only (password login disabled), user `mediaservo` (uid 1000,
  root login disabled). Host keys + `authorized_keys` persist in the etc volume
  (`etc/ssh/`, **never baked into the image**). Deliver the public key:

  ```bash
  # Append the ops public key into the etc volume (container path
  # /opt/msrtc/etc/ssh/authorized_keys)
  docker compose --env-file .env exec -T server sh -c \
    'cat >> /opt/msrtc/etc/ssh/authorized_keys' < ops_id_ed25519.pub
  docker compose --env-file .env exec -T server sh -c \
    'chown mediaservo:mediaservo /opt/msrtc/etc/ssh/authorized_keys; chmod 600 /opt/msrtc/etc/ssh/authorized_keys'
  ssh -p 2222 mediaservo@<IP>    # verify key login
  ```

  Optional hardening — restrict the ssh port to the ops subnet (examples):

  ```bash
  firewall-cmd --permanent --add-rich-rule='rule family=ipv4 source address=<ops-cidr> port port=2222 protocol=tcp accept'
  ufw allow from <ops-cidr> to any port 2222 proto tcp
  ```

  > The account shadow field is `*` (password disabled but account usable) —
    the `!` locked state left by `useradd` makes sshd with `UsePAM no` reject
    pubkey auth; the image fixes the shadow field (`usermod -p '*'`).

## 6. Operations

```bash
# Backup (bare docker on site; uses the delivered image's own tar, no internet
# dependency)
docker run --rm --entrypoint sh -v msrtc_data:/data:ro \
  -v "$PWD/backup:/backup" msrtc-server:<tag> \
  -c 'tar czf /backup/msrtc_data.tar.gz -C /data .'
# recordings and logs volumes: same pattern (msrtc_recordings / msrtc_logs;
# named <project>_<role>, project = the compose name, msrtc by default). Logs persist across recreates but grow — archive or
# truncate run/logs/*.out.log periodically.

# Logs (oxmgr file face, run/logs/*.out.log; NOT docker logs)
docker compose --env-file .env exec server sh -c 'tail -50 /opt/msrtc/run/logs/*.out.log'

# Graceful stop (image STOPSIGNAL is SIGINT; oxmgr on SIGINT stops apps
# gracefully, while SIGTERM would hang with apps attached)
docker compose --env-file .env stop      # takes the SIGINT path, ~1s exit 0
```

Resources: only `mem_limit` is set (CPU unlimited for RTP bursts); the default
comes from a standard-tier (10 rooms / 20 peers at 720p) load measurement
(peak × 1.3 rounded up to 512MB) — see the MEM_LIMIT ladder in `.env.example`
before raising it.

## 7. Rollback Drill (image + configuration level)

```bash
# The previous release's tar stays on site (delivery keeps one version back)
docker load -i msrtc-server-<old-tag>.tar
sed -i 's/^TAG=.*/TAG=<old-tag>/' .env       # re-pin the old tag
docker compose --env-file .env up -d         # recreates onto the old image; volumes untouched
curl --noproxy '*' -f http://<IP>:8080/health
# admin login still works after rollback (accounts.yaml persists in the named
# volume and survives version changes)
```

## 8. Appendix: Full Offline Drill Record (Acceptance)

> Acceptance = one complete offline drill record. The table below is the drill
> run on 2026-11-18 on the dev host (Ubuntu 20.04, simulating a restricted
> site: only the delivery tar, no external image pulls). Ports and IPs in the
> table are drill-environment values (isolated offsets); the site uses the
> standard ports from §2.2.

| # | Action | Result |
|---|---|---|
| 1 | `sha256sum -c SHA256SUMS` | `msrtc-server-0.1.3.tar: OK` |
| 2 | `docker load -i msrtc-server-0.1.3.tar` | loaded dual tags `msrtc-server:0.1.3` + `:latest` |
| 3 | `cp .env.example .env`, fill TAG/ANNOUNCED_IP=127.0.0.1/ADMIN_PASSWORD/SSH_PORT | four variables ready |
| 4 | `docker compose --env-file .env up -d` (offset ports for isolation) | `msrtc-server-1 Up (healthy)` |
| 5 | `curl --noproxy '*' :18080/health` | 200 |
| 6 | /admin vs /voice title check | MediaServo Admin / MediaServo Voice (distinct ✓) |
| 7 | admin first login POST /api/auth/login | JWT token ✓ |
| 8 | ssh key injection + `ssh -p 12222 mediaservo@127.0.0.1` | SSH_OK uid=1000 ✓ |
| 9 | `data backup` (prod image tar) | two volume tar.gz files incl. server.yaml/accounts/Caddyfile, **no alpine pull** ✓ |
| 10 | down → up (same volumes) | admin login survives, PSK not regenerated ✓ |
| 11 | `docker compose stop` (STOPSIGNAL SIGINT) | exit 0 in ~1s ✓ |
| 12 | Rollback: re-load tar + re-pin TAG + up | healthy, login survives ✓ |

**Conclusion**: the full offline chain (load → up → browser → login → backup →
persistence → graceful stop → rollback) is green; the explicit multi-NIC
ANNOUNCED_IP rule and the admin/ssh credential handover flows are verified.


## Appendix A: Per-Distribution Recipes (docker-ce / firewall / LSM / offline fetch)

| Distro | Install docker-ce | Firewall | LSM | Offline fetch |
|---|---|---|---|---|
| CentOS 8 (EOL) | vault repo fix + docker-ce repo (below) | firewalld (§2.2) | SELinux (§2.3) | `dnf download --resolve ...` -> `dnf localinstall *.rpm` |
| Rocky / Alma 8-9 | docker-ce repo (dnf) | firewalld (§2.2) | SELinux (§2.3) | same as CentOS 8 |
| RHEL 8-9 | docker-ce repo (dnf) | firewalld (§2.2) | SELinux (§2.3) | same |
| Ubuntu 20.04+ | docker-ce repo (apt) | ufw | AppArmor (no action) | `apt-get download ...` -> `apt install ./*.deb` |
| Debian 11+ | docker-ce repo (apt) | ufw / nftables | AppArmor (no action) | same as Ubuntu |

**CentOS 8 vault fix** (EOL — `mirror.centos.org` is dead):

```bash
sed -i 's|mirror.centos.org|vault.centos.org|g; s|^#baseurl|baseurl|g; s|^mirrorlist|#mirrorlist|g' \
    /etc/yum.repos.d/CentOS-*.repo
dnf install -y dnf-utils
dnf config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
dnf install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
```

**RHEL / Rocky / Alma** (live repos):

```bash
dnf config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
dnf install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
```

**Ubuntu / Debian** (replace `<ubuntu|debian>` with your series):

```bash
apt-get update && apt-get install -y ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/<ubuntu|debian>/gpg -o /etc/apt/keyrings/docker.asc
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] \
  https://download.docker.com/linux/<ubuntu|debian> $(. /etc/os-release && echo $VERSION_CODENAME) stable" \
  > /etc/apt/sources.list.d/docker.list
apt-get update && apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
```

**Firewall equivalents** (port set from §2.2): firewalld `firewall-cmd
--permanent --add-port=<port>/<proto>` + `--reload`; ufw `ufw allow
<port>/<proto>`.

If a recipe fails on your distro version, fall back to the official Docker
docs for that distro (fetch them on an internet-connected machine); everything
else in this manual is distro-neutral.

## Appendix B: Legacy UDP Range Revival (only for legacy-image rollback)

The 40000-40100/udp mapping was removed from the delivered compose (unused by
the current single-port SFU model on 20000/udp). Only if you roll back to a
legacy image that needs the range, re-add under `services.server.ports`:

    - "${SFU_PORT_RANGE:-40000-40100}:40000-40100/udp"

and open it in the host firewall (`firewall-cmd --permanent --add-port=40000-40100/udp`
+ `--reload`, or the Appendix A equivalent for your distro).
