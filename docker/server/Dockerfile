# docker/server/Dockerfile — tree-packaged production image (container-build-rework B3.3)
#
# Single artifact chain (R3/R11): the mediaservo-builder toolchain (B2) runs the SAME
# native CLI/cmake path as the host IDE; `deploy server --release --prefix` assembles
# the deploy tree (bin + web + etc scaffolding + oxmgr); this image packages that
# tree. Runtime = debian:bookworm-slim (R20: glibc 2.36 >= the 2.31 build floor,
# backward-compat direction). oxmgr daemon run = container PID1 (R11, PoC-passed);
# caddy lives in-container under oxmgr (R12, compose proxy retires).
#
# Build (context = this submodule root; dual build-contexts from the parent workspace):
#   docker build -f docker/server/Dockerfile -t mediaservo-server:0.1.3 \
#     --build-arg GIT_REV="$(git rev-parse HEAD)" \
#     --build-context wrapcache=../../3rdparty/mediasoup-worker \
#     --build-context oxmgrpkg=../../3rdparty/oxmgr .
#
# Materials ride the git tree (R2): wrapcache = the pinned meson wrap layer
# (offline worker build); oxmgrpkg = the vendored musl-static tarball + .sha256.
# Without the wrapcache context the sentinel stays empty and upstream wrap
# download behavior stays in charge (CI/GitHub posture, product-Dockerfile parity).

# Brand + builder image are CLI-injected (R15 dual-mode: `build image` passes
# --build-arg BRAND=<resolved brand> + BUILDER_IMAGE=<brand>-builder:v1 — env
# from the parent shell > out-tree derivation > upstream mediaservo). Defaults
# keep a bare `docker build -f docker/server/Dockerfile .` working.
# NOTE: must precede the FIRST FROM — FROM-line interpolation only sees args
# declared above the first FROM (measured: after a scratch-FROM -> blank base).
ARG BUILDER_IMAGE=mediaservo-builder:v1
ARG BRAND=mediaservo

# ---- build-context sentinels (overridden by --build-context; empty otherwise) ----
FROM scratch AS wrapcache
FROM scratch AS oxmgrpkg

# ---- stage 1: assemble a FRESH deploy tree inside the builder toolchain ----
# Toolchain-generation tag (three-point contract with `build image dev` + the
# `dev up` default; manual bump v1->v2 on semantic toolchain changes only —
# never per product release; see mediaservo.sh _ms_build_image governance).
FROM ${BUILDER_IMAGE} AS treebuild
ARG BRAND
WORKDIR /workspace
COPY . /workspace

# oxmgr unpack leg (the parent shell's oxmgr-unpack precedent, self-contained here): select the
# musl-static tarball -> sha256 verify (corrupt archive = refuse, never silent) ->
# untar onto PATH. The deploy face resolves oxmgr via find_program(PATH) and the
# R14 server-legs row copies it into the tree bin/.
COPY --from=oxmgrpkg oxmgr-v0.5.0-x86_64-unknown-linux-musl.tar.gz oxmgr-v0.5.0-x86_64-unknown-linux-musl.tar.gz.sha256 /tmp/oxmgr/
RUN cd /tmp/oxmgr && \
    sha256sum -c oxmgr-v0.5.0-x86_64-unknown-linux-musl.tar.gz.sha256 && \
    tar xzf oxmgr-v0.5.0-x86_64-unknown-linux-musl.tar.gz -C /usr/local/bin && \
    chmod 755 /usr/local/bin/oxmgr && oxmgr --version

# admin + voice dists are HARD build deps of the tree web legs (P2-C1: the deploy
# face does not invoke _ms_ensure_*_dist; MediaServoAppTree FATALs on a missing
# dist). CI=true: pnpm refuses to write node_modules without a TTY otherwise.
RUN cd www && CI=true pnpm install --frozen-lockfile && \
    CI=true pnpm build:admin && CI=true pnpm build:voice

# deploy server: fresh-goods cargo build (Release hard-coded per PLAN §2.4) +
# cmake install rows + managed-binary init. The wrapcache mount feeds the
# worker's meson wraps offline; MEDIASOUP_LOCAL_DEV=1 is the mandatory pairing
# (without it the post-build purge DELETES the layer archives —
# MediaServoMesonCache.cmake, measured). CMAKE_BIN satisfies the deploy face's
# pixi-cmake requirement natively (B1 bypass); MEDIASERVO_NO_PIXI=1 skips the
# pixi front door. The goods leg sanitizes CC/CXX (F25) — the builder's cc ->
# gcc-10 symlink is the fallback that keeps mediasoup-sys compilable (R4).
# weaknet WARN-skip is EXPECTED: the goods leg builds mediaservo-server only,
# so the prod topology is server + web (2 oxfile apps), no weaknet panel.
RUN --mount=from=wrapcache,target=/wrapcache,ro \
    if [ -n "$(ls -A /wrapcache 2>/dev/null)" ]; then \
        export MESON_PACKAGE_CACHE_DIR=/wrapcache MEDIASOUP_LOCAL_DEV=1; \
    fi; \
    MEDIASERVO_NO_PIXI=1 CMAKE_BIN=/usr/local/bin/cmake MEDIASERVO_BRAND=${BRAND} \
        ./mediaservo.sh deploy server --release --prefix /opt/${BRAND}

# Secrets + render-state OUT of the image (fresh-staging rule, PLAN §2.4④).
# The deploy-time init seeded etc/ from config/ — dev PSK/JWT + dev announced
# IPs (server.docker.yaml), dev placeholder accounts admin123/dispatch123/
# operator123 (accounts.yaml), real dev device entries (devices.yaml) — and
# rendered run/oxfile.toml with BUILD-time env. The container entrypoint re-runs
# init at first boot: fresh random secrets (0600), prod-safe empty registries,
# admin bootstrap from MEDIASERVO_ADMIN_PASSWORD, and oxfile re-rendered with
# the RUNTIME env (MEDIASERVO_SFU_ANNOUNCED_IP baked into [apps.env]). A shipped
# oxfile would never pick up the runtime env — PIT-160 first-writer-wins skips
# an existing file — so run/oxfile.toml{,.bak} must not ride the image. etc/ is
# left as empty scaffolding: init is the single first-boot writer.
RUN rm -f /opt/${BRAND}/etc/server.yaml /opt/${BRAND}/etc/.env \
          /opt/${BRAND}/etc/accounts.yaml /opt/${BRAND}/etc/devices.yaml \
          /opt/${BRAND}/etc/Caddyfile \
          /opt/${BRAND}/run/oxfile.toml /opt/${BRAND}/run/oxfile.toml.bak && \
    mkdir -p /opt/${BRAND}/etc /opt/${BRAND}/run/logs /opt/${BRAND}/recordings && \
    test ! -e /opt/${BRAND}/etc/server.yaml && \
    test ! -e /opt/${BRAND}/run/oxfile.toml

# ---- stage 2: runtime (R20 debian:bookworm-slim) ----
FROM debian:bookworm-slim
ARG GIT_REV=unknown
# Switchable apt mirror (P2-H7 parity with docker/builder/Dockerfile): cn bakes tuna
# (deb.debian.org throttles mainland pulls to a stall, measured); upstream
# leaves stock (CI posture). bookworm uses deb822 debian.sources.
ARG MIRROR=cn
LABEL org.opencontainers.image.title="mediaservo-server" \
      org.opencontainers.image.version="0.1.3" \
      org.opencontainers.image.revision="${GIT_REV}" \
      org.opencontainers.image.description="MediaServo deploy-tree production image (server + caddy + oxmgr PID1)"
ENV DEBIAN_FRONTEND=noninteractive
# Runtime lib face: the server binary links only glibc-family libs (ldd-verified;
# the mediasoup worker is a STATIC lib thread, vendored openssl/libuv static).
# openssh-server = R19 maintenance port; curl = R17 healthcheck probe (retained
# in the runtime whitelist); openssl = ops/debug; ca-certificates = TLS roots.
# usermod -p '*': useradd leaves the shadow field '!' (locked) which sshd with
# UsePAM no rejects even for pubkey; '*' = password login disabled but account
# usable for key auth (sshd_config PasswordAuthentication no is the real gate).
RUN if [ "$MIRROR" = "cn" ]; then \
        sed -i 's|deb.debian.org/debian-security|mirrors.tuna.tsinghua.edu.cn/debian-security|g; s|deb.debian.org/debian|mirrors.tuna.tsinghua.edu.cn/debian|g' /etc/apt/sources.list.d/debian.sources; \
    fi && \
    apt-get update && apt-get install -y --no-install-recommends \
        openssh-server curl ca-certificates openssl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -s /bin/bash -u 1000 mediaservo \
    && usermod -p '*' mediaservo \
    && mkdir -p /var/run/sshd
# Brand identity of THIS image (R15): the entrypoint derives the tree prefix
# /opt/${MEDIASERVO_BRAND} from it; compose may override for a standalone build.
ARG BRAND
ENV MEDIASERVO_BRAND=${BRAND}
COPY --from=treebuild /opt/${BRAND} /opt/${BRAND}
# Role-facing manuals baked into the image (the image alone is self-documenting;
# the package artifact set also ships them as DEPLOY-MANUAL.md / OPS-MANUAL.md).
COPY --from=treebuild /workspace/docs/modules/34-site-deployment.md /usr/local/share/doc/${BRAND}/DEPLOY-MANUAL.md
COPY --from=treebuild /workspace/docs/modules/35-container-ops-manual.md /usr/local/share/doc/${BRAND}/OPS-MANUAL.md
# Brand-render the baked manuals to match this image's brand (the source docs
# are mediaservo-authored and every lowercase mediaservo token is brand-related;
# the title-case MediaServo product name is preserved). Identity for mediaservo.
RUN if [ "${BRAND}" != mediaservo ]; then \
        sed -i "s/mediaservo/${BRAND}/g" /usr/local/share/doc/${BRAND}/DEPLOY-MANUAL.md \
                                   /usr/local/share/doc/${BRAND}/OPS-MANUAL.md; \
    fi
COPY docker/server/entrypoint.sh /usr/local/bin/server-entrypoint.sh
# Named-volume copy-up preserves image ownership: pre-create + chown the tree
# so the mediaservo daemon can write etc/run after the entrypoint drops privs.
RUN chmod 755 /usr/local/bin/server-entrypoint.sh && \
    chown -R mediaservo:mediaservo /opt/${BRAND}
# root runs the entrypoint (volume writes + sshd port-22 bind); the daemon is
# privilege-dropped via setpriv at the entrypoint's exec (PID-1 signal fidelity).
USER root
EXPOSE 8080 9800 22 20000/udp 40000-40100/udp
# R17 compound probe: caddy + routing + server in one curl; start_period=60s
# unifies the startup budget; retries=3 (90s self-heal tolerance). oxmgr
# liveness = PID1-death semantics (R13).
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1
# oxmgr daemon run maps SIGINT -> graceful shutdown ("stopping managed
# processes", exit 0, ~1s) but its SIGTERM path hangs with apps present
# (B3.3 smoke-measured: SIGTERM -> 10s grace -> SIGKILL 137). STOPSIGNAL SIGINT
# makes `docker stop` take the graceful path while oxmgr stays PID1 (R11).
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/server-entrypoint.sh"]