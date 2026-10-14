# CI/CD Pipeline — Continuous Integration/Deployment

> Status: pre-Phase 2 design | Related decisions: D-CI-01, D112 | Created per doc-audit M8

## 3-Stage Pipeline (D-CI-01)

A single GitHub Actions workflow, three chained stages, artifacts handed between stages:

```
Check → Test → Build (Package)
```

### Stage 1 — Check

Fast, no external dependencies, fail-early feedback:

- **rustfmt**: `cargo fmt --all -- --check`
- **clippy**: `cargo clippy --workspace --all-targets -- -D warnings`
- **cargo deny**: `cargo deny check bans licenses sources`

Toolchain versions pinned by `rust-toolchain.toml`; no matrix needed.

### Stage 2 — Test

Parallel matrix covering the whole workspace:

- `cargo test --workspace` (default target)
- Coverage collection: `cargo tarpaulin --workspace --out xml`
- Coverage report uploaded to Codecov, gate at 80%

### Stage 3 — Build & Package

- `cargo build --workspace --release`
- The 3 crates packaged into tarballs individually:
  - `mediaservo-host-{version}-{target}.tar.gz`
  - `mediaservo-client-{version}-{target}.tar.gz`
  - `mediaservo-server-{version}-{target}.tar.gz`
- Artifacts uploaded as workflow run attachments

## Phase 2 Extensions

- **Cross-compile**: aarch64-unknown-linux-gnu (Jetson Orin) matrix
- **Docker**: `mediaservo-server` builds multi-arch images (linux/amd64 + linux/arm64)
- **Integration test**: boots docker-compose (server + 2 mock hosts) and runs E2E scenarios
- **Release automation**: tag push → publishes a GitHub Release + crates.io automatically

## Trigger Rules

| Trigger | Stages 1-2 | Stage 3 |
|--------|-----------|---------|
| PR → main | ✅ | ❌ |
| push → main | ✅ | ✅ |
| tag v* | ✅ | ✅ (release mode) |

> See `.github/workflows/ci.yml` for details
