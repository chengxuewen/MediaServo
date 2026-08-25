# ---- Base: Ubuntu 22.04 LTS + Rust + system deps ----
# Ubuntu 22.04 is mediasoup's recommended prebuild base (widest glibc compatibility)
FROM ubuntu:22.04 AS base
ENV DEBIAN_FRONTEND=noninteractive

# 国内镜像加速: apt 换清华源 (PIT-31/36 教训 — 国内网络)
RUN sed -i 's|archive.ubuntu.com|mirrors.tuna.tsinghua.edu.cn|g; s|security.ubuntu.com|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list \
    && apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates build-essential pkg-config cmake ninja-build git \
    libssl-dev libuv1-dev \
    libglib2.0-dev libclang-dev \
    libglib2.0-dev \
    python3 python3-pip \
    && rm -rf /var/lib/apt/lists/*

# 国内镜像加速: rustup 换清华镜像
# 构建期代理 — 容器内进程（mediasoup-sys meson wrapdb / tasks.py pip）需独立代理 (PIT-19/20/33)
# 经 docker build --build-arg 或 compose args 传入，不硬编码 (PIT-20)；CI (GitHub) 无需代理，留空即可
ARG HTTP_PROXY
ARG HTTPS_PROXY
ARG NO_PROXY
ENV RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
    RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
    HTTP_PROXY=${HTTP_PROXY:-} \
    HTTPS_PROXY=${HTTPS_PROXY:-} \
    # wrapdb 直连（代理不可达——meson wrap 下载失败根因，PIT-127）
    NO_PROXY=${NO_PROXY:-},wrapdb.mesonbuild.com \
    PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple \
    PIP_DISABLE_PIP_VERSION_CHECK=1

# Install Rust via rustup (matches rust-toolchain.toml: stable channel)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

# 国内镜像加速: cargo crates.io 换 rsproxy sparse 镜像 (D208: tuna 不镜像二进制, rsproxy index+二进制全通)
RUN mkdir -p /root/.cargo && printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n' > /root/.cargo/config.toml

# Meson for mediasoup C++ Worker (范围约束防意外 major 升级, 与 pixi.toml 一致; pypi 清华镜像防超时)
RUN pip3 install -i https://pypi.tuna.tsinghua.edu.cn/simple 'meson>=1.1.0,<2'

# ---- Dev: full toolchain + source ----
# D208: manifests-first — fetch 层只在 Cargo.lock/Cargo.toml 变更时失效（源码变更不再触发重复 fetch）
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
# PIT-76: vendored [patch] 依赖需要 manifest（fetch 阶段）
COPY vendor/webrtc-sys/Cargo.toml vendor/webrtc-sys/
# dummy src 全建 — cargo fetch 要求依赖 crate 有 targets（缺 src 报 no targets specified）
# 且 media crate 声明了 [[example]]（square-gen-egui/viewer/square-gen）→ 需 touch 对应文件
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
# PIT-76: vendored [patch] 依赖需要 manifest（fetch 阶段）
COPY vendor/webrtc-sys/Cargo.toml vendor/webrtc-sys/

# 2. Create dummy sources to build & cache dependencies (全部 member + media [[example]] 声明文件)
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
    cargo build --release --bin mediaservo-server --features sfu-mediasoup
# 4. Remove dummy sources
RUN rm -rf crates/*/src

# 4b. PIT-23: admin dist 必须在 cargo build 前构建（build.rs 依赖 www/apps/admin/dist 存在）
#     dist 是 gitignore 产物（不在仓库）→ 容器内必须现场构建；否则 build.rs 回退
#     ADMIN_DIST_DIR=/nonexistent → /admin 运行时 404。Node 20 + pnpm 10。
#     层缓存修正（团队 build-reviewer F4）: node/pnpm 安装 + pnpm install 拆到 COPY . . 前——
#     源码变更不再重跑网络安装（cargo 依赖缓存同构）。
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    npm install -g pnpm@10.32.1
# 4c. www 清单层（package.json/lockfile/workspace）——依赖安装缓存层
COPY www/package.json www/pnpm-lock.yaml www/pnpm-workspace.yaml www/turbo.json www/
RUN cd www && CI=true pnpm install --frozen-lockfile

# 5. Copy real source code
COPY . .

# 5b. 强制重编 workspace crates — COPY 保留宿主 mtime（早于 dummy 构建）→ cargo fingerprint 误判
#     源码未变，链接空 common rlib 导致 cannot find protocol 连锁错误。touch 更新 mtime 解决。
RUN find crates -name '*.rs' -exec touch {} +

# 5c. admin 构建（依赖已装——仅源码变更时重跑 build:admin）
RUN cd www && CI=true pnpm build:admin && \
    cd / && rm -rf /workspace/www/node_modules

# 6. Final build — only recompiles changed source（含正确 ADMIN_DIST_DIR）
RUN cargo build --release --bin mediaservo-server --features sfu-mediasoup

# ---- Runtime: minimal Ubuntu 22.04 ----
FROM ubuntu:22.04 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 libuv1 ca-certificates curl openssl su-exec \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -s /bin/bash mediaservo
COPY --from=builder /workspace/target/release/mediaservo-server /usr/local/bin/
# 命名卷初始化（团队审核 C5/H8）: root 写卷 → entrypoint 末尾 su-exec 降权。
# 目录属主必须在 USER 前设定——命名卷 copy-up 保留镜像属主（空卷 EACCES 根治）。
COPY docker/entrypoint.sh /opt/mediaservo/entrypoint.sh
COPY docker/templates/ /opt/mediaservo/templates/
RUN chmod 755 /opt/mediaservo/entrypoint.sh \
    && mkdir -p /opt/mediaservo/etc /opt/mediaservo/recordings \
    && chown -R mediaservo:mediaservo /opt/mediaservo
# 以 root 跑 entrypoint（写卷不依赖 copy-up 行为）——server 由 entrypoint 末尾 su-exec 降权（PID-1 保真）
USER root
EXPOSE 9800 40000-40100/udp
HEALTHCHECK --interval=30s --timeout=3s CMD curl -f http://localhost:9800/health || exit 1
# exec-form 保 PID-1（原 mediaservo-server 直启被替换——entrypoint 自举后 exec 降权同进程）
ENTRYPOINT ["/opt/mediaservo/entrypoint.sh"]
