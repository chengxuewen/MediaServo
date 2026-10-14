# docker-deployment-modes Manual Test Plan

> Companion verification document for `docs/superpowers/plans/2026-08-21-docker-deployment-modes.md`.
> Purpose: manually verify the rework deliverables step by step (CLI command surface / entrypoint bootstrap / prod compose / regression),
> covering interaction/timing/persistence aspects not covered by automation.
> Status: ✅ = pass / ❌ = fail (record symptoms) / ⏳ = pending

## Prerequisites

- [ ] dev server container available (`./mediaservo.sh up --env dev`)
- [ ] runtime image built (`./mediaservo.sh build server --image runtime` — record baseline numbers)
- [ ] Normal environment load (`uptime` — load < 4; run the regression group after build-type tasks finish)

---

## Group A: CLI command surface (no image needed — run first)

| # | Command | Expected | Actual |
|---|---------|----------|--------|
| A1 | `./mediaservo.sh -h` | Target command surface: build/up/down/restart/logs/exec/e2e/test/ci/install/package/clean/config/data/doctor/version + `build --image` and `e2e <suite>` help | |
| A2 | `./mediaservo.sh doctor` | pixi/cargo/docker/node version output (4 lines) | |
| A3 | `./mediaservo.sh up --env dev server` | Idempotent (existing container → Started; no errors) | |
| A4 | `./mediaservo.sh ps` | compose ps table — server row shows 9800/20000/40000-40100 mappings | |
| A5 | `./mediaservo.sh logs server --follow` | Log follow (Ctrl-C to exit) | |
| A6 | `./mediaservo.sh exec server -- sh -c 'echo ok'` | Prints `ok` | |
| A7 | `./mediaservo.sh config set signaling.psk test123` | Writes `config/host.conf` + prompt (dev track) | |
| A8 | `./mediaservo.sh data inspect` | Volume list (includes mediaservo_mediaservo-data) | |

## Group B: entrypoint bootstrap (needs runtime image)

| # | Command | Expected | Actual |
|---|---------|----------|--------|
| B1 | `docker run --rm -v ms-data-test:/opt/mediaservo/etc -v ms-rec-test:/opt/mediaservo/recordings -e MEDIASERVO_ADMIN_PASSWORD=test123 -p 9800:9800 mediaservo-server:runtime-baseline` | Log order: `generated fresh secrets` → `created bootstrap admin` → `psk_set=true` → `Listening on 0.0.0.0:9800` → `Server ready` | |
| B2 | `docker exec <c> ls -la /opt/mediaservo/etc/` | `.env` + `server.yaml` mode 600, owner mediaservo; accounts.yaml/devices.yaml present | |
| B3 | `docker exec <c> cat /opt/mediaservo/etc/.env` | `MEDIASERVO_PSK=<64hex>` (**not** mediaservo-dev) | |
| B4 | Browser/curl `http://127.0.0.1:9800/admin` + login `admin`/`test123` | 200 + login succeeds (bootstrap account works) | |
| B5 | `docker restart <c>` → `docker exec <c> cat /opt/mediaservo/etc/.env` | PSK **unchanged** (volume persistence); config not overwritten (optionally edit devices.yaml before restart to verify) | |
| B6 | `docker run --rm -v ms-data-test2:... -p 9801:9800 mediaservo-server:runtime-baseline` | **New keys** (different from B3 — per-instance isolation) | |
| B7 | Negative: `.env` missing from the volume (restart after `docker exec <c> rm .env`) | Log `init incomplete` → exit 1 (server does not start — idempotent guard) | |

## Group C: prod compose (needs image)

| # | Command | Expected | Actual |
|---|---------|----------|--------|
| C1 | `MEDIASERVO_ADMIN_PASSWORD=test123 ./mediaservo.sh up --env prod` | Root compose starts server (named volume initialized on first start) — admin 200 | |
| C2 | Login admin/test123 → Settings → Provision Device (note: smoke-manual) | Provisioning succeeds; `docker logs` has **no** "falling back to direct write" (named-volume atomic write) | |
| C3 | `./mediaservo.sh ps --env prod` + `./mediaservo.sh logs server --follow` | ps shows the prod container; logs have no `mediaservo-dev` (secret not default) | |
| C4 | `./mediaservo.sh down --env prod` → `./mediaservo.sh up --env prod` | **Volume persistence**: login still succeeds (account present) + devices.yaml contains the smoke-manual device | |
| C5 | `./mediaservo.sh data backup /tmp/ms-backup` | `mediaservo_mediaservo-data.tar.gz` + `...recordings.tar.gz` generated | |
| C6 | `./mediaservo.sh data reset --force` → `./mediaservo.sh up --env prod` | Volumes cleared → re-initialized (new keys + empty registry) | |

## Group D: regression (when load is normal)

| # | Command | Expected | Actual |
|---|---------|----------|--------|
| D1 | `./mediaservo.sh e2e sfu` | 4 passed | |
| D2 | `./mediaservo.sh e2e push` | 6 passed (previously polluted by high load — should recover after builds) | |
| D3 | `./mediaservo.sh e2e host` | e2e-install-host PASS | |
| D4 | `./mediaservo.sh e2e package` | e2e-package PASS | |
| D5 | `./mediaservo.sh e2e brand` | e2e-brand PASS | |
| D6 | `./mediaservo.sh e2e ui` | Playwright (device-provisioning spec) 1 passed | |
| D7 | `./mediaservo.sh e2e smoke` | prod smoke PASS (= Group C automated assertions) | |

## Group E: cleanup

| # | Command | Expected |
|---|---------|----------|
| E1 | `./mediaservo.sh down --env prod` | prod containers stopped (volumes kept) |
| E2 | `docker volume rm ms-data-test ms-data-test2 ms-rec-test` | Test volumes removed |
| E3 | `./mediaservo.sh up --env dev` | dev environment restored |

---

## Results Log

| Date | Group | Result | Notes |
|------|-------|--------|-------|
| | | | |
| | | | |

## Failure Troubleshooting Tips

- **Group B init failure**: check `docker logs <c>` for entrypoint output (generation/permissions/template path); check volume ownership (`docker run --rm -v ms-data-test:/d alpine ls -la /d`)
- **C2 provisioning EBUSY**: rename under a named volume should succeed — if fallback logs appear → volume mount issue (confirm volume type via `docker inspect`)
- **D2 frame test hangs**: check `uptime` first (encoder stalls when load >5 — environment interference, stash comparison proved not code); then check `docker logs` for RTP/worker errors
- **smoke login failure**: MEDIASERVO_ADMIN_PASSWORD must be set at the **first** `up` (bootstrap only takes effect on first start)
