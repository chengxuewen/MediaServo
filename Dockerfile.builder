# Dockerfile.builder — msrtc-builder toolchain face (container-build-rework B2/R2)
#
# Pure toolchain: NO project source, NO wrap archives — materials ride the git
# tree (R2). Ubuntu 20.04 = the artifact glibc floor (2.31, host parity, R4).
# Mirrors switchable via --build-arg MIRROR=cn|upstream (P2-H7): cn bakes
# tuna/rsproxy/npmmirror/PIP-tuna; upstream leaves stock (CI posture).
#
# Build:  docker build -f Dockerfile.builder -t msrtc-builder:v1 \
#           --build-arg MIRROR=cn [--build-arg HTTPS_PROXY=http://...] .
# Use:    ssh-resident dev  -> docker run -d -p 2222:22 ... (CMD sshd -D)
#         CI / one-shot     -> docker run --rm ... <command> (entrypoint exec)
FROM ubuntu:20.04

ARG MIRROR=cn
ARG DEBIAN_FRONTEND=noninteractive
# proxy only consumed by the foreign-download RUNs below (nodesource replaced
# by npmmirror tarball; caddy github release); never baked into the image.
ARG HTTPS_PROXY=""

# ---- apt: base toolchain (P2-H1 classes; focal) ----
RUN if [ "$MIRROR" = "cn" ]; then \
        sed -i 's|archive.ubuntu.com|mirrors.tuna.tsinghua.edu.cn|g; s|security.ubuntu.com|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list; \
    fi && \
    apt-get update && apt-get install -y --no-install-recommends \
        build-essential g++-10 git curl ca-certificates openssh-server \
        pkg-config python3 python3-pip ninja-build lld \
        libssl-dev libuv1-dev libglib2.0-dev libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# gcc-9.4 (focal default) cannot compile mediasoup-sys 0.14.1 worker C++
# (Utils.hpp ReverseDiff template double-definition, PoC-measured) — R4
# fallback: gcc-10 from the focal repo (NOT back to 22.04). cc crate and
# meson both honor CC/CXX.
ENV CC=gcc-10 \
    CXX=g++-10

# ---- rustup (R2: >=1.85 stable; tuna mirror per product-Dockerfile precedent) ----
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
    RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
    PATH=/usr/local/cargo/bin:${PATH}
# rustfmt is REQUIRED: mediasoup-sys build.rs pipes generated code into it
# (planus-codegen; minimal profile omits it -> broken pipe at build.rs:39)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
        sh -s -- -y --profile minimal --component clippy --component rustfmt --no-modify-path

# ---- cargo registry (rsproxy sparse for cn; stock crates.io for upstream) ----
RUN if [ "$MIRROR" = "cn" ]; then \
        printf '[source.crates-io]\nreplace-with = "rsproxy-sparse"\n[source.rsproxy-sparse]\nregistry = "sparse+https://rsproxy.cn/index/"\n' \
            > "${CARGO_HOME}/config.toml"; \
    fi

# ---- cmake >=3.28,<4 via pip (P2-H1/H2: apt cmake=3.16 too old; Kitware focal
# is EOL-frozen and may serve 4.x which breaks vendored wraps; pip pin is
# deterministic). PIP_INDEX_URL is the switchable pypi mirror. ----
RUN if [ "$MIRROR" = "cn" ]; then \
        pip3 install -i https://pypi.tuna.tsinghua.edu.cn/simple "cmake>=3.28,<4"; \
    else \
        pip3 install "cmake>=3.28,<4"; \
    fi

# ---- node 20 + pnpm (npmmirror binary tarball — domestic, no nodesource/proxy) ----
RUN cd /tmp && \
    curl -fsSL -o node.tar.xz https://registry.npmmirror.com/-/binary/node/v20.18.1/node-v20.18.1-linux-x64.tar.xz && \
    tar -xJf node.tar.xz -C /usr/local --strip-components=1 && rm node.tar.xz && \
    if [ "$MIRROR" = "cn" ]; then \
        npm config --global set registry https://registry.npmmirror.com; \
    fi && \
    npm install --global pnpm@10.32.1 && \
    ln -s "$(command -v pnpm || echo /usr/local/bin/pnpm)" /usr/local/bin/pnpm 2>/dev/null || true
# pnpm home on a shared path so root and dev both resolve it
ENV PNPM_HOME=/usr/local/share/pnpm \
    npm_config_registry=${npm_config_registry:-}

# ---- caddy binary (tree staging needs it on PATH; foreign download -> proxyable) ----
ARG CADDY_VERSION=2.10.0
RUN curl ${HTTPS_PROXY:+-x "$HTTPS_PROXY"} -fsSL \
        -o /tmp/caddy.tar.gz \
        "https://github.com/caddyserver/caddy/releases/download/v${CADDY_VERSION}/caddy_${CADDY_VERSION}_linux_amd64.tar.gz" && \
    tar -xzf /tmp/caddy.tar.gz -C /usr/local/bin caddy && rm /tmp/caddy.tar.gz && \
    chmod 755 /usr/local/bin/caddy && caddy version

# ---- RUSTFLAGS parity with pixi-shell.sh:53-55 (lld + system glib for the
# vendored webrtc .a GLib UNDEFs; system lib path needs no -L) ----
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld -C link-arg=-lgio-2.0 -C link-arg=-lgobject-2.0 -C link-arg=-lglib-2.0"

# ---- sshd: key-only, no root login ----
RUN sed -i 's/^#*PermitRootLogin.*/PermitRootLogin no/' /etc/ssh/sshd_config && \
    sed -i 's/^#*PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config && \
    sed -i 's/^#*PubkeyAuthentication.*/PubkeyAuthentication yes/' /etc/ssh/sshd_config && \
    mkdir -p /var/run/sshd

# ---- dev user (uid 1000 = host bind-mount alignment) + git safe.directory ----
ARG DEV_UID=1000
RUN useradd -m -s /bin/bash -u "${DEV_UID}" dev && \
    git config --global --add safe.directory '*' && \
    su - dev -c 'git config --global --add safe.directory "*"' && \
    mkdir -p /home/dev/.ssh && chmod 700 /home/dev/.ssh && chown dev:dev /home/dev/.ssh

COPY docker/builder-entry.sh /usr/local/bin/builder-entry
RUN chmod 755 /usr/local/bin/builder-entry

# default = ssh-resident dev mode; CI passes its own command (entrypoint execs it)
CMD ["/usr/sbin/sshd", "-D"]
