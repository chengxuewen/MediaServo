# MediaServo Build Optimization Strategy

**Generated**: 2026-08-03 | **Source**: team-mode analysis (4 parallel analysts: docker-cache / mirror / prebake / workflow) + team-review corrections (fact-check / tech-review / risk-review / consistency-review, 4 reviewers) | **Status**: reviewed and corrected, pending implementation (related D208)

> This document captures the complete analysis of the "container builds take too long" problem: time breakdown, audit findings, option comparison (C1 format), and the execution roadmap. All mirror-reachability conclusions were verified by live curl tests.

## 1. Background

mediaservo-server depends on mediasoup-sys (a C++ Worker built with meson/ninja, Linux x86_64 only); the first Docker build takes **15-30 minutes**. Local development (macOS) builds the server via `docker compose`; the pain concentrates on:

- The first build (new machine / after `docker volume rm`) is a full compile
- The dev image contains no precompiled dependencies; filling the `cargo-cache` volume the first time = a full compile
- CI's gha cache is unreachable locally (`type=gha` can only be consumed by GitHub Actions)

## 2. Time Breakdown (dev path)

| Stage | Layer | Est. time | Share |
|-------|-------|-----------|-------|
| apt install (tuna mirror) | Dockerfile L7-12 | 1-2 min | ~7% |
| rustup install (tuna mirror) | Dockerfile L19 | 1-2 min | ~7% |
| pip meson | Dockerfile L26 | ~0.5 min | ~2% |
| COPY . . + cargo fetch | dev L31-32 | 1-3 min | ~8% |
| mediasoup-sys C++ Worker (meson subprojects + ninja ~100 C++ files + flatbuffers codegen) | first in-container compile | **8-15 min** | **~45%** |
| Rust dependency compile (~300 crates: axum/tokio/mediasoup-rs etc.) | first in-container compile | **5-10 min** | **~35%** |
| The application crates themselves | same | 0.5-1 min | ~3% |

**Key conclusion**: the builder stage already bakes all dependencies into image layers (CI incremental builds are only 1-2 min); the 15-30 min of pain concentrates entirely on (a) the dev container's first `cargo run` (empty volume, full compile) and (b) after the volume is wiped. CI's gha cache is fully disconnected from local builds.

## 3. Audit Findings (live curl tests)

### 3.1 Configuration bugs (P0 — the mirror layers never actually took effect)

| # | Problem | Finding | Impact |
|---|---------|---------|--------|
| 1 | `scripts/pixi-init.sh`'s rsproxy sparse URL is broken: `sparse+https://rsproxy.cn/crates.io-index/` → 404 | the rsproxy sparse protocol requires `sparse+https://rsproxy.cn/index/` (200); `/crates.io-index/` is the **git-protocol** address (not deprecated; pixi-init.sh L34 `[registries.rsproxy]` remains valid) — it just doesn't serve the sparse layout | the cargo mirror was never actually in effect |
| 2 | Using tuna as the cargo mirror in the Dockerfile is inadequate: tuna mirrors only the index; `.crate` binaries still go to static.crates.io | tuna dl 404; static.crates.io measured at 52KB/s; rsproxy/ustc index+binaries both pass | slow dependency downloads |
| 3 | pixi has no China mirror configured: `channels = ["conda-forge"]` goes straight to conda.anaconda.org | ustc/tuna conda mirrors measured at 8.2MB/s | the slowest layer and the only unmirrored one |

### 3.2 Configuration bugs (P1 — scripts/orchestration broken)

| # | Problem | Location |
|---|---------|----------|
| 4 | `docker-cargo.sh` references the service name `dev`, but compose actually has only `server`/`proxy` → the script always fails | scripts/docker-cargo.sh L10/L13 |
| 5 | The production compose mounts `cargo-cache:/workspace/target` — the runtime image has no /workspace; a leftover from pure-dev compose | docker-compose.yml L25 + L38-39 |
| 6 | devcontainer.json points at the production compose (no `build:`) → devcontainer pulls the runtime image, no toolchain | .devcontainer/devcontainer.json L3 |
| 7 | `mirror.ghproxy.com` is defunct (project terminated 2024); the pixi-install.sh fallback is broken | scripts/pixi-install.sh L85 |

### 3.3 Structural Inconsistencies

| Problem | Impact |
|---------|--------|
| The builder builds with `--features sfu-mediasoup` (without `--no-default-features`) | cargo `--features` is **additive**: the builder actually builds defaults + sfu-mediasoup = **identical to dev** (server Cargo.toml L59 default = sfu-mediasoup + admin-dashboard). No feature mismatch. **The real gap**: the admin **dist artifacts** aren't built — build.rs L27 falls back to `ADMIN_DIST_DIR=/nonexistent/admin/dist` when `www/apps/admin/dist` is missing; it compiles but /admin 404s at runtime (PIT-23) |
| The builder dummy src covers only 2/7 crates (common+server) | once Phase 2+ server depends on media/webrtc/codec the build will explode |
| The dev stage `COPY . .` comes before `cargo fetch` | any source change invalidates the fetch layer, repeating dozens of pointless downloads daily |
| Cargo.toml L12-13 `lto=true + codegen-units=1` | **affects only release builds** (builder/CI/production image paths), slowing them 2-3x; dev's debug path (`[profile.release]` has zero effect on debug) is unaffected |
| Dockerfile L19 rustup version unpinned + L26 pip meson unpinned | toolchain upgrades unexpectedly invalidate the base layer |

## 4. Option Comparison & Recommendation

### 4.1 Pre-Baked Container Images — recommended, the biggest single win

| Option | Approach | Pros | Cons | Recommendation |
|--------|----------|------|------|:--------------:|
| A. Push only the dev image | dev stage bakes target/debug + CI pushes | smallest change; covers 90% of local development | runtime builds gain nothing | |
| **B. dev + builder dual images** | dev bakes debug deps; builder pushes a registry cache | covers both local dev and production builds; the registry cache is locally reachable | +2.5-3.5GB image size; ~15-25 min more CI per commit | ✅ |
| C. Bake only the registry cache | cargo fetch artifacts into the image | small (~300-400MB) | saves download but not compile (compile is the bulk) | |
| D. Rely on gha cache only | current state | zero change | locally unreachable; no help for local pre-baking | |

**Source**: docker/build-push-action + `type=registry` cache standard practice; OpenVidu's prod/dev compose split (same as the PIT-32 lesson).

**Option B concrete changes**:

1. **Dockerfile dev stage rewrite** (L28-33):
   - manifests-first (all 7 crates listed) + dummy src fully created
   - `cargo fetch && cargo build --bin mediaservo-server` (**debug**; features exactly matching the dev compose command: `sfu-mediasoup,admin-dashboard`)
   - Remove the dummy src then `COPY . .` (bind mount overlays; target/ excluded by .dockerignore)
   - The mediasoup-sys C++ artifacts are cached by build-script input hash → **workspace source changes don't retrigger the C++ build**; it reruns (~10 min) only when the mediasoup-sys version, its dependency resolution, or features change — backstopped by the CI's new-sha images (build.rs has no rerun-if-changed → cargo tracks by package directory files; registry sources are immutable → no reruns in steady state)
2. **docker-compose.dev.yml**: `build:` → `image: ghcr.io/org/mediaservo-server-dev:latest` + `pull_policy: always` (⚠️ use `always` rather than `missing`: missing is compose's default and won't refresh an existing `:latest` — CI pushes a new image per commit, so a stale local image would be used forever until an explicit pull; only `always` or `newer` (compose 2.30+) guarantees the latest baked artifacts)
   - ⚠️ **Volume refresh semantics (H2)**: the `cargo-cache:/workspace/target` named volume's copy-on-first-use **only populates image target/debug into the volume the first time it is created and empty**. It does **not trigger for an existing non-empty volume** — on the upgrade path the baked cache would be silently ignored. Landing this must explicitly `docker volume rm mediaservo_cargo-cache` (or use a new volume name); write the command into the runbook rather than relying on auto-population. Docker copies image content into empty volumes only; after that the volume's content is fully independent of the image
   - The command must stay consistent with the baked feature set, or the cache misses
3. **New `docker-compose.dev.build.yml`** (fallback override, honoring PIT-32):
   ```yaml
   services:
     server:
       build: { context: ., target: dev }
   ```
   Usage: `docker compose -f docker-compose.dev.yml -f docker-compose.dev.build.yml up -d --build`
4. **docker.yml pushes both images** (push order: **dev first, runtime last**, so a mid-way failure doesn't leave the two images with drifted features/deps):
   - the builder step `push: true`, tags `ghcr.io/org/mediaservo-builder:latest` + `:sha-${{ github.sha }}`, cache-to adds `type=registry,ref=...,mode=max`
   - the runtime step's cache-from adds the registry ref; cache-to also needs `type=registry` (otherwise the runtime's own new layers never enter the registry cache)
   - a new dev push step (`target: dev`)
   - the PIT-32 gate gains one rule: dev.yml must not contain `build:`
5. **Tags/refresh**: `latest` (rolling) + `sha-${{ github.sha }}` (exact); refreshed on every main push (keeping the existing trigger)
6. **Storage & CI quota governance (H4)**:
   - **A GHCR cleanup workflow**: sha tags are never removed → each commit adds dev(2.5-3.5GB)+builder+runtime ≈ 5-7GB; the free tier (500MB-2GB) blows up within days. Add a cleanup job (`gh api` deletes old sha tags, keeping the most recent N=10)
   - **path-filter**: the dev image push runs only when `Cargo.lock` / `Cargo.toml` / `Dockerfile` / `crates/**` / `pixi.toml` change (most commits skip it, also cutting egress)
   - Total CI time budget: docker.yml is now ~15-20 min/commit; with the dev bake it's 35-45 min; combined with the ci.yml matrix, check against the free tier's 2000 min/month
   - The fallback override (docker-compose.dev.build.yml) can add `cache_from: [type=registry, ref=ghcr.io/org/mediaservo-server-builder:latest]` so the local --build path also benefits from the registry cache
7. **Precondition (H3 — ghcr reachability)**: the option's value chain depends on pulling a 2.5-3.5GB pre-baked image locally. In this project's environment direct GitHub/ghcr is unreliable (PIT-14/31; the daemon needs a mirror accelerator + proxy fallback):
   - Before implementing, measure ghcr pull reachability and speed (assuming the daemon proxy is configured)
   - Private repos: the first local pull needs `gh auth token | docker login ghcr.io -u <user> --password-stdin`; image visibility = repo visibility
   - If ghcr proves unreachable: switch to a domestically reachable registry (Ali ACR / Tencent TCR) to host the dev image and the registry cache ref
8. **Platform (M4)**: the baked image builds on an ubuntu-22.04 runner = **linux/amd64 only**. Apple Silicon developers pull and run under qemu emulation (no regression vs today's local-build amd64 emulation, but the bake's benefit is discounted under emulation). The dev service should declare `platform: linux/amd64` explicitly; document the emulation precondition in the README
9. **Decision continuity (M5)**: this plan **continues and extends D207** (pre-built dev images), changing D207's FROM-prebuilt-base mechanism to **compose `image:` + pull** (zero local builds); image names unified as `mediaservo-server-dev` / `mediaservo-server-builder` (consistent with the existing prod `mediaservo-server` prefix); D207 revised accordingly (see also D208)

**Time comparison (estimated)**:

| Scenario | Today (local build) | Pre-baked (pull) |
|----------|---------------------|------------------|
| First time | 15-30 min | pull 2.5-3.5GB (**ghcr reachability needs testing; possibly 30min+ behind a proxy**) + recompile workspace crates 1-3 min |
| Daily incremental | 1-5 min (volume cached) | same (the first time needs `docker volume rm` so the baked artifacts populate; incremental after) |
| Cargo.lock change | full recompile 15-30 min | **an existing volume won't be refreshed by a new image** (copy-on-first-use only fills empty volumes) → `docker volume rm` then pull the new sha image |

**Image size estimate**: base ~1.2-1.5GB + registry ~300-400MB + target/debug ~800MB-1.2GB ≈ **2.5-3.5GB**.

### 4.2 China Mirrors — fix immediately, lowest cost

**pixi/conda** (eliminates the slowest layer, no lockfile change) — `~/.pixi/config.toml`:

```toml
# The official pixi [mirrors] table — transparent substitution at fetch time; the lockfile is unchanged
[mirrors]
"https://conda.anaconda.org/conda-forge" = [
    "https://mirrors.ustc.edu.cn/anaconda/cloud/conda-forge",   # primary, 8.2MB/s (repodata uses the first mirror)
    "https://mirrors.tuna.tsinghua.edu.cn/anaconda/cloud/conda-forge",  # backup
    "https://conda.anaconda.org/conda-forge",  # original-URL fallback (official rule: if not listed explicitly, the original channel is fully replaced)
]
```

Alternative (not recommended): point pixi.toml channels at mirror URLs → requires `pixi update` to regenerate the lockfile and everyone to commit it; mirrors are byte-identical syncs so sha256 is unchanged — safe but produces a lock change.

**cargo** (rsproxy primary + ustc backup) — unified between local and the Dockerfile:

```toml
[source.crates-io]
replace-with = "rsproxy-sparse"

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"    # note: not /crates.io-index/ (the old URL 404s)

[source.ustc-sparse]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

> cargo `replace-with` doesn't support automatic fallback lists; rsproxy is itself a multi-node CDN (synced every minute) with ustc as the manual-switch backup.

**rustup local**:

```bash
export RUSTUP_DIST_SERVER="https://rsproxy.cn"
export RUSTUP_UPDATE_ROOT="https://rsproxy.cn/rustup"
```

**Measured mirror matrix**:

| Layer | Working mirrors (measured) | Not working (measured) |
|-------|----------------------------|-------------------------|
| conda | ustc 8.2MB/s, tuna 8.2MB/s | sjtu 403, aliyun 404 |
| cargo | rsproxy (CDN, fastest), ustc 765KB/s, sjtu 242KB/s | **tuna (index only; binaries 404)** |
| rustup | tuna, ustc, rsproxy | sjtu 301 unreachable |

**CI vs local boundary**: GitHub Actions runs on GitHub infrastructure, unaffected by China's network; CI acceleration = the gha cache; mirror config serves only local/self-hosted Docker.

### 4.3 Custom Build Image — implement merged with 4.1

- The builder stage effectively is the "custom build image" (manifest-first + dummy build + deps compiled)
- **Feature unification is no longer necessary (H1)**: cargo `--features` is additive; the builder already includes admin-dashboard (defaults = both features on), matching dev. **The real fix**: add `pnpm build:admin` + a COPY dist step to the Docker build flow (see §7; the PIT-23 ordering constraint: dist must be built before cargo build, since embedding happens at compile time)
- **Merge CI's two build steps into one** (docker.yml L33-41 + L42-51): `target: runtime, push: true, cache-to: type=gha + type=registry` builds and caches all intermediate layers at once; the second step is just a repeated cache restore
- Generalize the builder dummy src: `for d in crates/*/; do mkdir -p $d/src; touch $d/src/lib.rs; done` (idempotent)
- Push `ghcr.io/org/mediaservo-builder:buildcache` as a registry cache → **local first-build 15-30 min → minutes** (locally reachable, replacing gha)

### 4.4 Other Optimizations (ROI-ordered)

| # | Item | Est. saving | Effort | Risk | Status (doc-audit L8, 2026-08-03) |
|---|------|-------------|--------|------|-----------------------------------|
| 1 | dev stage manifests-first (drop the pre-COPY fetch) | 30-60s per build | S | low | ✅ done |
| 2 | Docker resource quota ≥8GB RAM / 6+ CPU (documented) | 2-3x full builds | S | none | ✅ done (operations.md §Docker resource limits) |
| 3 | warm-dev.sh background warm-up script (run each morning) | the first 15-28 min moved to background | S | low | ✅ exists |
| 4 | `lto="thin" + codegen-units=16` (**release-only**: builder/CI/production image paths; no benefit on dev's debug path — the debug profile is independent) | release dependency compile 2-3x (at the cost of 1-2% size/perf) | S | low | ✅ done |
| 5 | sccache dual-path (native + container volume) | cold dependency changes 8-15 → 2-4 min | M | medium — the daemon holds ~1GB; limited benefit for the mediasoup C++ part | ❌ on demand (trigger: Cargo.lock change frequency rising) |
| 6 | Precompiled mediasoup workers (the OpenVidu idea) | 4-8 min per build | M-L | medium — mediasoup-sys needs patching; version/glibc tightly bound | ❌ not done |
| 7 | BuildKit cache mounts (registry + meson OUT_DIR) | fetch 3 min → 5s; meson rebuilds in seconds | S-M | low | ❌ not done |
| 8 | .dockerignore adds `.pixi/ .omo/ .sisyphus/ .codegraph/` | smaller context upload | S | none | ✅ done (+ .env*/*.key) |
| 9 | pixi env slimming: move gst-plugins-ugly/gst-libav/llvm into a test feature | 1-2 min per env creation | S-M | low | ❌ not done |
| 10 | CI: docker.yml gains a PR-triggered build-only check | catches Dockerfile breakage earlier | S | low | ✅ done (with the PIT-39 gate) |

**Explicitly not doing / low value**:
- **Cross-platform target sharing is infeasible**: macOS native (aarch64-apple-darwin) and container (aarch64/x86_64-unknown-linux-gnu) artifacts key by triple; the mediasoup worker compiles per arch+OS; sccache keys by compiler+target too. The only cross-platform opportunity = precompiled worker binaries (Linux x64)
- The analysts disagreed on sccache priority: docker-cache argued today's benefit is low (layer cache already covers it) and suggested deferring to P3; workflow ranked it this month. **Recommend trigger-based adoption**: introduce when Cargo.lock change frequency rises or CI adds a feature matrix

## 5. Execution Roadmap

### This Week (pure fixes, all <1 day)

1. Fix `pixi-init.sh`'s rsproxy URL (`/crates.io-index/` → `/index/`)
2. Add `~/.pixi/config.toml` `[mirrors]` (ustc primary + tuna backup + the original URL as fallback)
3. Dockerfile cargo mirror tuna → rsproxy (index+binaries both pass); **revise D206** (keep apt/rustup on Tsinghua, switch cargo to rsproxy)
4. Fix `docker-cargo.sh`'s service name `dev` → `server`; fix devcontainer.json to point at the dev compose
5. Remove the production docker-compose.yml's misuse of the cargo-cache volume
6. dev stage manifests-first + `lto="thin"` + `codegen-units=16` (release-only) + .dockerignore additions (incl. `.env*`, `*.key` and other secret patterns)
7. Fix `pixi-install.sh`'s ghproxy fallback (ghproxy.com is defunct) → switch to `gh-proxy.com` or install pixi from the conda mirror
8. warm-dev.sh + record the baseline measurement
9. Test ghcr.io reachability (H3 precondition); pin rustup/meson versions (preventing unexpected base-layer invalidation)

### This Month (structural)

10. Dockerfile dev stage bakes the debug dependencies (4.1-1)
11. CI pushes the dev/builder dual images (4.1-4) + merge the two build steps + the registry cache (4.3) + **the GHCR cleanup workflow + path-filter (H4)**
12. Compose image-ization (`pull_policy: always`) + the `docker-compose.dev.build.yml` fallback override (with a registry `cache_from`)
13. The CI PR build-check job + a soft gate on build duration
14. **The admin dist fix**: the Docker build flow adds `pnpm build:admin` + COPY dist (the PIT-23 ordering constraint)

### Next Month (as needed)

15. sccache (trigger conditions in 4.4)
16. Precompiled mediasoup workers
17. pixi env slimming, BuildKit cache mounts

**Expected total benefit**: local first build 15-28 min → 2-5 min (pre-bake + mirrors); daily incremental saves 30-60s per round (manifests-first); cold dependency-change days 8-15 → 2-4 min (sccache).

## 6. Measurement & Regression Gates

**Baseline method** (measure once before and after each optimization; record in status.md):

```bash
# per-stage times (buildx per-layer DONE x.xs)
docker compose -f docker-compose.dev.yml build --progress=plain server

# dependency vs own-code compile split
docker compose exec server cargo build --timings -p mediaservo-server --features sfu-mediasoup

# end-to-end (median of 3 consecutive runs)
time scripts/docker-cargo.sh check -p mediaservo-server --features sfu-mediasoup
```

**Gates**:
- Soft gate: CI builds the builder with `--progress=plain` and parses the per-stage times; >12 min with gha hits → a warning comment (network jitter causes high false-positive rates; no hard failure)
- Per-stage times land in the build summary/artifact; reviewed manually weekly
- Rule: any Dockerfile/pixi.toml change must attach a measured build duration

## 7. Incidental Findings (not build optimization)

- **The prod image feature gap (H1 correction)**: the builder's and dev's feature sets are actually identical (both include admin-dashboard; cargo `--features` is additive); the real gap is that the admin **dist artifacts** aren't built — build.rs L27 falls back to `ADMIN_DIST_DIR=/nonexistent/admin/dist` when dist is missing; it compiles but /admin 404s at runtime (PIT-23). Fixing requires adding `pnpm build:admin` (**before cargo build**) + a COPY dist step to the Docker build flow (outside this document's scope; recorded only)
- `docs/modules/development/docker-workflow.md`'s `docker volume rm mediaservo_cargo-cache` command needs a sync update once this plan lands

## References

- [docker-workflow.md](../modules/development/docker-workflow.md) — the Docker development workflow
- [ffmpeg-static-build-strategy.md](./ffmpeg-static-build-strategy.md) — a build strategy reference (same kind)
- Project memory: PIT-11/12/13/19/31/32/33 (mediasoup builds, proxies, Docker lessons)
- Decisions: D206 (cargo mirror revision), D207 (pre-built image continuation), D208 (build-optimization strategy implementation)
