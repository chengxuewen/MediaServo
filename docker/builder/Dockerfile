# docker/builder/Dockerfile — mediaservo-builder toolchain face (container-build-rework B2/R2)
#
# Pure toolchain: NO project source, NO wrap archives — materials ride the git
# tree (R2). Ubuntu 20.04 = the artifact glibc floor (2.31, host parity, R4).
# Mirrors switchable via --build-arg MIRROR=cn|upstream (P2-H7): cn bakes
# tuna/rsproxy/npmmirror/PIP-tuna; upstream leaves stock (CI posture).
#
# Build:  docker build -f docker/builder/Dockerfile -t mediaservo-builder:v1 \
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

# system-default compiler = gcc-10: the deploy goods face sanitizes CC/CXX
# (F25 anti-pixi-leak bridge), so with CC/CXX stripped the fallback is the
# system `cc` — the image must guarantee it resolves mediasoup-capable
RUN update-alternatives --install /usr/bin/gcc gcc /usr/bin/gcc-10 100 \
                         --slave /usr/bin/g++ g++ /usr/bin/g++-10 && \
    ln -sf /usr/bin/gcc-10 /usr/bin/cc && ln -sf /usr/bin/g++-10 /usr/bin/c++ && \
    cc --version | head -1 && gcc --version | head -1

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

# pypi mirror baked as default (P2-H7 switchable set): mediasoup-sys tasks.py
# self-installs meson/ninja via pip at ARTIFACT build time — without this env
# the in-container build hits files.pythonhosted.org directly (mainland read
# timeouts, PoC-measured). CI overrides by passing -e PIP_INDEX_URL=...
ENV PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple \
    PIP_DISABLE_PIP_VERSION_CHECK=1


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

COPY docker/builder/entrypoint.sh /usr/local/bin/builder-entrypoint
RUN chmod 755 /usr/local/bin/builder-entrypoint

# Toolchain-generation provenance (trailing on purpose: one label layer keeps
# the cached toolchain layers hot). BUILDER_TAG/GIT_REV arrive from
# `build image dev`. The tag is a THREE-POINT CONTRACT — this image,
# docker/server/Dockerfile stage1 `FROM mediaservo-builder:v1`, and the `dev up` default;
# bump v1->v2 ONLY on semantic toolchain changes (compiler / cmake pin / node /
# mirror posture), never per product release. Manual bump, all points together
# (oxmgr-archive precedent: the literal IS the version source).
ARG BUILDER_TAG=v1
ARG BUILDER_NAME=mediaservo-builder
ARG GIT_REV=unknown
LABEL org.opencontainers.image.title="${BUILDER_NAME}" \
      org.opencontainers.image.version="${BUILDER_TAG}" \
      org.opencontainers.image.revision="${GIT_REV}" \
      org.opencontainers.image.description="MediaServo native toolchain image (gcc-10 + rust + cmake 3.31 + node20/pnpm + caddy; container-build-rework B2)"

# default = ssh-resident dev mode; CI passes its own command (entrypoint execs it)
ENTRYPOINT ["/usr/local/bin/builder-entrypoint"]
CMD ["/usr/sbin/sshd", "-D"]
