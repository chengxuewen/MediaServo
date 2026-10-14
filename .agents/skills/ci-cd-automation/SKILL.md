---
name: ci-cd-automation
description: "MediaServo CI/CD pipeline management: Docker compose (server + SFU), GitHub Actions (fmt/check/clippy/test/mediasoup/benchmark), pixi tasks (check/build/lint/test/audit/coverage), cargo-deny audit, and platform-specific builds (macOS native + Linux Docker). Use for CI troubleshooting, pipeline changes, or pre-merge verification."
---

# ci-cd-automation — MediaServo CI/CD Pipeline

> The pipeline IS the gate. Every check is a contract. Don't merge red.

## Pipeline Architecture

```
GitHub Actions (on push/PR to main)
│
├── fmt          : cargo fmt --all --check
├── check        : cargo check --workspace  (ubuntu + macOS)
├── clippy       : cargo clippy --workspace -- -D warnings
├── test         : cargo test --workspace   (ubuntu + macOS)
├── test-gstreamer   : pixi run test-gstreamer  (ubuntu)
├── test-mediasoup   : SFU tests (ubuntu-22.04 only)
├── benchmark        : cargo bench -p mediaservo-server
└── openapi-validate : Python OpenAPI 3.0.3 validation
```

## Local Development Workflow

### Quick (no SFU, no mediasoup)

```bash
pixi run check-fast     # cargo check --workspace --no-default-features
pixi run build-fast     # cargo build --workspace --no-default-features
pixi run format         # cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

### Full (with mediasoup)

```bash
pixi run check          # workspace (excludes mediaservo-server)
pixi run build          # workspace (excludes mediaservo-server)
pixi run lint           # clippy workspace
pixi run test           # workspace (excludes mediaservo-server)
pixi run check-server   # mediaservo-server via Docker (mediasoup)
pixi run test-sfu       # mediaservo-server with sfu-mediasoup feature (Docker)
```

## Docker Compose Workflow

### Server Dev Container

```bash
# Build and start
docker compose up -d

# View logs
docker compose logs -f server

# Execute commands inside
docker compose exec server cargo check -p mediaservo-server --features sfu-mediasoup

# SFU E2E test inside container
docker compose exec server bash scripts/test-sfu-e2e.sh

# Stop
docker compose down
```

### Critical Constraints

| Constraint | Detail |
|-----------|--------|
| mediasoup = Linux x86_64 only | macOS: `check` works, `build`/`test` fails — use Docker |
| macOS volume mounts 3-5x slower | Use `cargo-cache` named volume for target |
| First Docker build: 15-30 min | mediasoup C++ Worker + Rust deps from scratch |
| UDP ports 40000-40100 | Must be mapped for SFU RTP/RTCP |
| Server TLS uses rustls | No OpenSSL dependency needed |

## pixi Task Reference

### Cargo Tasks (mediasoup separated)

mediaservo-server goes through Docker (C13) — `scripts/docker-cargo.sh` / `docker compose exec server`;
the remaining crates compile natively.

| Task | Command | When |
|------|---------|------|
| `pixi run check` | `cargo check --workspace --exclude mediaservo-server` | After any code change |
| `pixi run build` | `cargo build --workspace --exclude mediaservo-server` | Before running bins |
| `pixi run lint` | `cargo clippy --workspace --all-targets -- -D warnings` | Before commit |
| `pixi run test` | `cargo test --workspace --exclude mediaservo-server` | Before PR |
| `pixi run check-server` | `scripts/docker-cargo.sh check -p mediaservo-server --features sfu-mediasoup` | Server changes |
| `pixi run test-server` | `scripts/docker-cargo.sh test -p mediaservo-server --features sfu-mediasoup` | SFU changes only |

### Vanilla Tasks (no wrapper, faster)

| Task | Command | When |
|------|---------|------|
| `pixi run check-fast` | `cargo check --workspace --no-default-features` | Quick iteration (no mediasoup) |
| `pixi run build-fast` | `cargo build --workspace --no-default-features` | Quick build |
| `pixi run format` | `cargo fmt --all -- --check` | Pre-commit |
| `pixi run format-fix` | `cargo fmt --all` | Auto-fix formatting |
| `pixi run audit` | `cargo deny check` | Pre-merge security audit |
| `pixi run coverage` | `cargo tarpaulin --workspace --out Html --out Lcov` | Coverage report |
| `pixi run test-gstreamer` | GStreamer codec tests (pixi env) | Codec changes |

## cargo-deny Audit

Run before EVERY merge:

```bash
pixi run audit   # cargo deny check
```

### What It Checks

| Check | Config | Threshold |
|-------|--------|-----------|
| Security advisories | `deny.toml [advisories]` | Deny yanked crates |
| License compliance | `deny.toml [licenses]` | Allow: Apache-2.0, MIT, BSD-*, ISC, Unicode-3.0, Zlib, LGPL-3.0 |
| Duplicate deps | `deny.toml [bans]` | Warn on multiple versions |
| Source registry | `deny.toml [sources]` | Deny unknown registries/git |
| Ignored advisories | RUSTSEC-2022-0004, RUSTSEC-2025-0025 | Reviewed and accepted |

### Audit Failure Protocol

1. **Security advisory (yanked)**: Find replacement version, update Cargo.toml
2. **License violation**: Check if new dep's license is in allow-list; if not, discuss with team
3. **Duplicate dependency**: Run `cargo tree -d` to find duplicates, deduplicate if possible
4. **Unknown source**: Remove git dependency or add to `allow-git`

## Pre-Merge Verification Sequence

Run in order. Do NOT skip steps.

```bash
# 1. Format check (fastest, catches most style issues)
pixi run format

# 2. Clippy (finds logic errors)
pixi run lint

# 3. Compile check (catches type errors)
pixi run check

# 4. Unit + integration tests (catches regressions)
pixi run test

# 5. Security audit (catches vulnerable deps)
pixi run audit

# 6. SFU tests (if touching server/media)
pixi run test-sfu        # Linux only
docker compose exec server cargo test --features sfu-mediasoup  # macOS workaround

# 7. GStreamer tests (if touching codec)
pixi run test-gstreamer  # Linux only

# 8. Coverage check (target: 80%+)
pixi run coverage | grep -E "mediaservo|TOTAL"
```

## CI Troubleshooting

### CI Failed: macOS but Passes Locally (Linux)

```bash
# Check for Linux-specific code paths
grep -r "cfg.*target_os.*linux" crates/
grep -r "cfg.*target_os.*macos" crates/

# Common causes:
# - Missing #[cfg(target_os = "macos")] guards
# - macOS-specific API usage without conditional compilation
# - Hardcoded Linux paths (/usr/lib, /etc)
```

### CI Failed: test-mediasoup

```bash
# mediasoup only builds on Linux x86_64
# CI uses ubuntu-22.04 for widest glibc compatibility

# Check required system deps:
sudo apt-get install -y meson ninja-build libuv1-dev libssl-dev

# Check MESON env var (must be absolute path — PIT-12)
# Check buildtype conflict (PIT-11: remove --buildtype from tasks.py)
# Clear build cache if build.rs changed (PIT-13):
rm -rf target/debug/build/mediasoup-sys-*
```

### CI Failed: openapi-validate

```bash
# Validate locally:
python3 -c "
import yaml
with open('docs/openapi.yaml') as f:
    spec = yaml.safe_load(f)
assert spec.get('openapi') == '3.0.3', 'Not OpenAPI 3.0.3'
for path, methods in spec['paths'].items():
    for method, detail in methods.items():
        assert 'responses' in detail, f'{path} {method}: missing responses'
print('OK')
"
```

## macOS-Specific Workflow

```bash
# Host and Client: NATIVE (no mediasoup dep)
cargo build -p mediaservo-host
cargo build -p mediaservo-client

# Server: check only (mediasoup won't build)
cargo check -p mediaservo-server --features sfu-mediasoup

# Full server build + test: use Docker
docker compose up -d
docker compose exec server cargo test -p mediaservo-server --features sfu-mediasoup
```

## Adding a New CI Job

Template:

```yaml
new-job-name:
  runs-on: ubuntu-latest   # or macos-latest, ubuntu-22.04 for mediasoup
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2     # always include
    - run: <command>
```

Rules:
- All jobs must use `rust-cache@v2` (saves ~5 min per run)
- mediasoup jobs: `runs-on: ubuntu-22.04`
- GStreamer jobs: use `prefix-dev/setup-pixi` + `pixi run`
- No hardcoded secrets in workflow files
- New jobs → update this skill's pipeline diagram

## Common Pitfalls

| Pitfall | Reference | Fix |
|---------|-----------|-----|
| mediasoup-sys buildtype conflict | PIT-11 | Resolved (official registry original) — local builds go through Docker (C13) |
| MESON must be absolute path | PIT-12 | `pixi run -- which meson` |
| cargo clean -p doesn't clear build cache | PIT-13 | `rm -rf target/debug/build/<pkg>-*` |
| macOS Docker volume mount slow | constraints.md | Use cargo-cache named volume |
| Sending test on macOS with sfu-mediasoup | constraints.md | Use Docker or check-only |
| First Docker build takes 15-30 min | constraints.md | Cache everything possible |

## Related Skills

| Skill | Relationship |
|-------|-------------|
| `review-hardcode` | Run BEFORE audit — catch hardcoded secrets in CI config |
| `test-harness` | Generate test skeletons that CI will run |
| `lesson-memory` (C9) | CI failures → write to pitfalls.md |
| `think-before-act` | Check CI status BEFORE implementing fix |
