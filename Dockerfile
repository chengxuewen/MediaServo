# ---- Base: Ubuntu 22.04 LTS + Rust + system deps ----
# Ubuntu 22.04 is mediasoup's recommended prebuild base (widest glibc compatibility)
FROM ubuntu:22.04 AS base
ENV DEBIAN_FRONTEND=noninteractive

# CN mirror acceleration: apt switched to the Tsinghua mirror (PIT-31/36 lessons — mainland network)
RUN sed -i 's|archive.ubuntu.com|mirrors.tuna.tsinghua.edu.cn|g; s|security.ubuntu.com|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates build-essential pkg-config cmake ninja-build git \
    libssl-dev libuv1-dev \
    libglib2.0-dev libclang-dev \
    libglib2.0-dev \
    python3 python3-pip \
    && rm -rf /var/lib/apt/lists/*

# CN mirror acceleration: rustup switched to the Tsinghua mirror
# Build-time proxy — in-container processes (mediasoup-sys meson wrapdb / tasks.py pip) need a dedicated proxy (PIT-19/20/33)
# Passed via docker build --build-arg or compose args, never hardcoded (PIT-20); CI (GitHub) needs no proxy — leave empty
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
ENV RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
    RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
    HTTP_PROXY=${HTTP_PROXY:-} \
    HTTPS_PROXY=${HTTPS_PROXY:-} \
    # direct wrapdb connection (proxy unreachable — root cause of meson wrap download failures, PIT-127)
    NO_PROXY=${NO_PROXY:-},wrapdb.mesonbuild.com \
    PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple \
    PIP_DISABLE_PIP_VERSION_CHECK=1

# Install Rust via rustup (matches rust-toolchain.toml: stable channel)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

# CN mirror acceleration: cargo crates.io switched to the rsproxy sparse mirror (D208: tuna mirrors no binaries; rsproxy serves both index and binaries)
RUN mkdir -p /root/.cargo && printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n' > /root/.cargo/config.toml

# Meson for the mediasoup C++ Worker (range pin guards against surprise major bumps, consistent with pixi.toml; pypi Tsinghua mirror prevents timeouts)
RUN pip3 install -i https://pypi.tuna.tsinghua.edu.cn/simple 'meson>=1.1.0,<2'

# ---- Dev: full toolchain + source ----
# D208: manifests-first — the fetch layer only invalidates on Cargo.lock/Cargo.toml changes (source edits no longer re-trigger fetch)
FROM base AS dev
RUN apt-get update && apt-get install -y --no-install-recommends gdb && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
COPY Cargo.toml Cargo.lock ./
COPY crates/mediaservo-common/Cargo.toml crates/mediaservo-common/
COPY crates/mediaservo-media/Cargo.toml crates/mediaservo-media/
COPY crates/mediaservo-webrtc/Cargo.toml crates/mediaservo-webrtc/
COPY crates/mediaservo-codec/Cargo.toml crates/mediaservo-codec/
COPY crates/mediaservo-server/Cargo.toml crates/mediaservo-server/
COPY crates/mediaservo-host/Cargo.toml crates/mediaservo-host/
COPY crates/mediaservo-client/Cargo.toml crates/mediaservo-client/
COPY crates/mediaservo-link/Cargo.toml crates/mediaservo-link/
COPY crates/mediaservo-deck/Cargo.toml crates/mediaservo-deck/
COPY crates/mediaservo-field/Cargo.toml crates/mediaservo-field/
COPY bindings/c/mediaservo-field-c/Cargo.toml bindings/c/mediaservo-field-c/
COPY bindings/c/mediaservo-link-c/Cargo.toml bindings/c/mediaservo-link-c/
COPY bindings/c/mediaservo-deck-c/Cargo.toml bindings/c/mediaservo-deck-c/
COPY bindings/cxx/mediaservo-link-cxx/Cargo.toml bindings/cxx/mediaservo-link-cxx/
COPY bindings/cxx/mediaservo-deck-cxx/Cargo.toml bindings/cxx/mediaservo-deck-cxx/
COPY bindings/cxx/mediaservo-field-cxx/Cargo.toml bindings/cxx/mediaservo-field-cxx/
COPY bindings/node/rust/mediaservo-node/Cargo.toml bindings/node/rust/mediaservo-node/
# PIT-76: vendored [patch] dependencies need their manifests (fetch stage)
COPY vendor/webrtc-sys/Cargo.toml vendor/webrtc-sys/
# Build all dummy srcs — cargo fetch requires every dependency crate to have targets (missing src → 'no targets specified')
# and the media crate declares [[example]] (square-gen-egui/viewer/square-gen) → those files must be touched too
RUN mkdir -p crates/mediaservo-common/src && touch crates/mediaservo-common/src/lib.rs && \
    mkdir -p crates/mediaservo-server/src && echo 'fn main() {}' > crates/mediaservo-server/src/main.rs && \
    mkdir -p crates/mediaservo-media/src crates/mediaservo-webrtc/src crates/mediaservo-codec/src \
             crates/mediaservo-host/src crates/mediaservo-client/src \
             crates/mediaservo-link/src crates/mediaservo-deck/src crates/mediaservo-field/src && \
    touch crates/mediaservo-media/src/lib.rs crates/mediaservo-webrtc/src/lib.rs \
          crates/mediaservo-codec/src/lib.rs crates/mediaservo-host/src/lib.rs crates/mediaservo-client/src/lib.rs \
          crates/mediaservo-link/src/lib.rs crates/mediaservo-deck/src/lib.rs crates/mediaservo-field/src/lib.rs && \
    mkdir -p crates/mediaservo-media/examples crates/mediaservo-webrtc/examples && touch crates/mediaservo-media/examples/square-gen-egui.rs \
          crates/mediaservo-media/examples/viewer.rs crates/mediaservo-media/examples/square-gen.rs \
          crates/mediaservo-webrtc/examples/webrtc_loopback_egui.rs && \
    mkdir -p bindings/c/mediaservo-field-c/src bindings/c/mediaservo-link-c/src bindings/c/mediaservo-deck-c/src \
             bindings/cxx/mediaservo-link-cxx/src bindings/cxx/mediaservo-deck-cxx/src bindings/cxx/mediaservo-field-cxx/src \
             bindings/node/rust/mediaservo-node/src && \
    touch bindings/c/mediaservo-field-c/src/lib.rs bindings/c/mediaservo-link-c/src/lib.rs bindings/c/mediaservo-deck-c/src/lib.rs \
          bindings/cxx/mediaservo-link-cxx/src/lib.rs bindings/cxx/mediaservo-deck-cxx/src/lib.rs bindings/cxx/mediaservo-field-cxx/src/lib.rs \
          bindings/node/rust/mediaservo-node/src/lib.rs
RUN cargo fetch
RUN rm -rf crates/*/src
COPY . .
CMD ["bash"]

# ---- Builder: release build with layer caching ----
FROM base AS builder
WORKDIR /workspace

# 1. Copy dependency manifests first (layer caching)
COPY Cargo.toml Cargo.lock ./
COPY crates/mediaservo-common/Cargo.toml crates/mediaservo-common/
COPY crates/mediaservo-media/Cargo.toml crates/mediaservo-media/
COPY crates/mediaservo-webrtc/Cargo.toml crates/mediaservo-webrtc/
COPY crates/mediaservo-codec/Cargo.toml crates/mediaservo-codec/
COPY crates/mediaservo-server/Cargo.toml crates/mediaservo-server/
COPY crates/mediaservo-host/Cargo.toml crates/mediaservo-host/
COPY crates/mediaservo-client/Cargo.toml crates/mediaservo-client/
COPY crates/mediaservo-link/Cargo.toml crates/mediaservo-link/
COPY crates/mediaservo-deck/Cargo.toml crates/mediaservo-deck/
COPY crates/mediaservo-field/Cargo.toml crates/mediaservo-field/
COPY bindings/c/mediaservo-field-c/Cargo.toml bindings/c/mediaservo-field-c/
COPY bindings/c/mediaservo-link-c/Cargo.toml bindings/c/mediaservo-link-c/
COPY bindings/c/mediaservo-deck-c/Cargo.toml bindings/c/mediaservo-deck-c/
COPY bindings/cxx/mediaservo-link-cxx/Cargo.toml bindings/cxx/mediaservo-link-cxx/
COPY bindings/cxx/mediaservo-deck-cxx/Cargo.toml bindings/cxx/mediaservo-deck-cxx/
COPY bindings/cxx/mediaservo-field-cxx/Cargo.toml bindings/cxx/mediaservo-field-cxx/
COPY bindings/node/rust/mediaservo-node/Cargo.toml bindings/node/rust/mediaservo-node/
# PIT-76: vendored [patch] dependencies need their manifests (fetch stage)
COPY vendor/webrtc-sys/Cargo.toml vendor/webrtc-sys/

# 2. Create dummy sources to build & cache dependencies (all members + media [[example]] declared files)
RUN mkdir -p crates/mediaservo-common/src && touch crates/mediaservo-common/src/lib.rs && \
    mkdir -p crates/mediaservo-server/src && echo 'fn main() {}' > crates/mediaservo-server/src/main.rs && \
    mkdir -p crates/mediaservo-media/src crates/mediaservo-webrtc/src crates/mediaservo-codec/src \
             crates/mediaservo-host/src crates/mediaservo-client/src \
             crates/mediaservo-link/src crates/mediaservo-deck/src crates/mediaservo-field/src && \
    touch crates/mediaservo-media/src/lib.rs crates/mediaservo-webrtc/src/lib.rs \
          crates/mediaservo-codec/src/lib.rs crates/mediaservo-host/src/lib.rs crates/mediaservo-client/src/lib.rs \
          crates/mediaservo-link/src/lib.rs crates/mediaservo-deck/src/lib.rs crates/mediaservo-field/src/lib.rs && \
    mkdir -p crates/mediaservo-media/examples crates/mediaservo-webrtc/examples && touch crates/mediaservo-media/examples/square-gen-egui.rs \
          crates/mediaservo-media/examples/viewer.rs crates/mediaservo-media/examples/square-gen.rs \
          crates/mediaservo-webrtc/examples/webrtc_loopback_egui.rs && \
    mkdir -p bindings/c/mediaservo-field-c/src bindings/c/mediaservo-link-c/src bindings/c/mediaservo-deck-c/src \
             bindings/cxx/mediaservo-link-cxx/src bindings/cxx/mediaservo-deck-cxx/src bindings/cxx/mediaservo-field-cxx/src \
             bindings/node/rust/mediaservo-node/src && \
    touch bindings/c/mediaservo-field-c/src/lib.rs bindings/c/mediaservo-link-c/src/lib.rs bindings/c/mediaservo-deck-c/src/lib.rs \
          bindings/cxx/mediaservo-link-cxx/src/lib.rs bindings/cxx/mediaservo-deck-cxx/src/lib.rs bindings/cxx/mediaservo-field-cxx/src/lib.rs \
          bindings/node/rust/mediaservo-node/src/lib.rs

# 3. Fetch and build dependencies (cached — only re-runs on Cargo.toml changes)
RUN cargo fetch && \
    cargo build --release --bin mediaservo-server --features sfu-mediasoup,admin-dashboard
# 4. Remove dummy sources
RUN rm -rf crates/*/src

# 4b. PIT-23: the admin dist must be built before cargo build (build.rs requires www/apps/admin/dist to exist).
#     dist is a gitignored artifact (not in the repo) → must be built inside the container; otherwise build.rs falls
#     back to ADMIN_DIST_DIR=/nonexistent → /admin serves 404 at runtime. Node 20 + pnpm 10.
#     Layer-cache fix (team build-reviewer F4): node/pnpm install + pnpm install moved ahead of COPY . . —
#     source changes no longer re-run network installs (same shape as the cargo dependency cache).
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    npm install -g pnpm@10.32.1
# 4c. www manifests layer (package.json/lockfile/workspace) — dependency-install cache layer
COPY www/package.json www/pnpm-lock.yaml www/pnpm-workspace.yaml www/turbo.json www/
RUN cd www && CI=true pnpm install --frozen-lockfile

# 5. Copy real source code
COPY . .

# 5b. Force rebuild of workspace crates — COPY keeps host mtimes (older than the dummy build) → cargo fingerprint
#     wrongly concludes sources are unchanged and links the empty common rlib → cascading 'cannot find protocol' errors. touch fixes mtime.
RUN find crates -name '*.rs' -exec touch {} +

# 5c. admin build (deps already installed — re-runs build:admin only on source changes)
RUN cd www && CI=true pnpm build:admin && \
    cd / && rm -rf /workspace/www/node_modules

# 6. Final build — only recompiles changed source (with a correct ADMIN_DIST_DIR)
RUN cargo build --release --bin mediaservo-server --features sfu-mediasoup,admin-dashboard

# ---- Runtime: minimal Ubuntu 22.04 ----
FROM ubuntu:22.04 AS runtime
# Privilege drop uses setpriv (util-linux, present in the base image) — su-exec has no Ubuntu package (Debian-only);
# the runtime layer referenced it since e56650c, making the image unbuildable since then (T9 pre-existing defect first exposed, 2026-08-31)
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 libuv1 ca-certificates curl openssl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -s /bin/bash mediaservo
COPY --from=builder /workspace/target/release/mediaservo-server /usr/local/bin/
# Named-volume initialization (team review C5/H8): root writes the volume → entrypoint drops privileges via setpriv at the end.
# Directory ownership must be set before USER — named-volume copy-up preserves image ownership (root-cause fix for empty-volume EACCES).
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
COPY docker/templates/ /usr/local/etc/templates/
RUN chmod 755 /usr/local/bin/entrypoint.sh \
    && mkdir -p /usr/local/etc /opt/mediaservo/recordings \
    && chown -R mediaservo:mediaservo /usr/local/etc /opt/mediaservo
# Run the entrypoint as root (volume writes must not rely on copy-up behavior) — the server is privilege-dropped by setpriv at the entrypoint's end (exec keeps PID-1)
USER root
EXPOSE 9800 40000-40100/udp
HEALTHCHECK --interval=30s --timeout=3s CMD curl -f http://localhost:9800/health || exit 1
# exec-form keeps PID-1 (replaces launching mediaservo-server directly — after the entrypoint bootstraps, exec drops privileges in the same process)
# Binary in /usr/local/bin/ → relative path bin/../etc/server.yaml = /usr/local/etc/server.yaml
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
