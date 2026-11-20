#!/bin/bash
# server-entrypoint.sh (docker/server/) — tree-packaged prod image entry (container-build-rework B3.3)
#
# Boot contract (order matters; root runs this, the daemon drops privileges):
#   1. <tree>/bin/<brand>-server init <tree> — idempotent first-boot render
#      (PIT-160 first-writer-wins): fresh PSK/JWT into etc/server.yaml (0600),
#      prod-safe empty registries, admin bootstrap from MEDIASERVO_ADMIN_PASSWORD
#      (init step 3, folded in), etc/Caddyfile + run/oxfile.toml with the RUNTIME
#      SFU env baked into [apps.env] (sfu_env_passthrough). The image ships NO
#      etc/ render products and NO run/oxfile.toml (docker/server/Dockerfile strips them),
#      so init is the single first-boot writer and picks up the container env.
#   2. sshd (R19 maintenance port): key-only, no root login; host keys +
#      authorized_keys live in the etc volume (never baked). Root daemon —
#      port-22 bind + setuid need root, so sshd is entrypoint-managed, NOT an
#      oxfile app (the oxmgr daemon runs as mediaservo and cannot bind 22).
#   3. chown etc/run/recordings to mediaservo (root wrote the volume; the
#      privilege-dropped daemon + apps must read/write them).
#   4. OXMGR_* 4-key env parity with lifecycle::oxmgr_env(dir) — HOME=DATA_DIR=
#      <tree>/run/oxmgr, daemon port = 18500 + byte_sum(home) % 400, API = +1000
#      (templates.rs instance_daemon_port). Explicit ADDR pins the daemon to the
#      port the in-tree CLI (mediaservo-server start/stop/status) derives.
#   5. background apply helper: `oxmgr daemon run` boots with ZERO apps on a
#      fresh container (run/ is writable-layer state, not a persisted volume),
#      so wait for the daemon event socket then `oxmgr apply run/oxfile.toml`.
#      PoC-verified: oxmgr apps inherit the daemon env, and the baked [apps.env]
#      is the robust SFU path regardless.
#   6. exec setpriv mediaservo oxmgr daemon run — PID1 (R11), signal-faithful
#      (PoC: clean SIGTERM exit 143). App graceful-stop on container teardown is
#      PID-namespace semantics (R13 accepts PID1-death liveness).
set -euo pipefail

BRAND="${MEDIASERVO_BRAND:-mediaservo}"
TREE="/opt/$BRAND"
SERVER="$TREE/bin/$BRAND-server"
OXMGR="$TREE/bin/oxmgr"
MS_UID="$(id -u mediaservo)"
MS_GID="$(id -g mediaservo)"

[ -x "$SERVER" ] || { echo "FATAL: $SERVER missing (MEDIASERVO_BRAND=$BRAND vs image tree)" >&2; exit 1; }
[ -x "$OXMGR" ] || { echo "FATAL: $OXMGR missing (R14 deploy leg did not land oxmgr)" >&2; exit 1; }

# ── 1. init (root writes the etc volume; idempotent — existing files skipped) ──
"$SERVER" init "$TREE"
# Keep the bootstrap password out of the long-lived daemon/app environment.
unset MEDIASERVO_ADMIN_PASSWORD MEDIASERVO_PSK

# ── 2. sshd maintenance port (R19) ─────────────────────────────────────────────
SSHD_DIR="$TREE/etc/ssh"
mkdir -p "$SSHD_DIR"
for t in ed25519 rsa; do
    [ -f "$SSHD_DIR/ssh_host_${t}_key" ] || \
        ssh-keygen -q -t "$t" -N "" -f "$SSHD_DIR/ssh_host_${t}_key"
done
if [ ! -f "$SSHD_DIR/authorized_keys" ]; then
    echo "WARN: $SSHD_DIR/authorized_keys absent — ssh accepts no login until the site delivers a key into the etc volume" >&2
fi
# Image-owned config, regenerated each boot (host keys + authorized_keys above
# are the persisted state). UsePAM no: key-only auth needs no PAM stack.
cat > "$SSHD_DIR/sshd_config" <<EOF
Port 22
HostKey $SSHD_DIR/ssh_host_ed25519_key
HostKey $SSHD_DIR/ssh_host_rsa_key
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AuthorizedKeysFile $SSHD_DIR/authorized_keys
PidFile /run/sshd-tree.pid
UsePAM no
EOF

# ── 3. ownership handoff (volume copy-up kept image ownership; init wrote root) ──
chown -R mediaservo:mediaservo "$TREE/etc" "$TREE/run" "$TREE/recordings"
chmod 700 "$SSHD_DIR"
[ ! -f "$SSHD_DIR/authorized_keys" ] || chmod 600 "$SSHD_DIR/authorized_keys"
/usr/sbin/sshd -f "$SSHD_DIR/sshd_config" -e

# ── 4. OXMGR 4-key env parity (lifecycle/mod.rs oxmgr_env) ─────────────────────
export OXMGR_HOME="$TREE/run/oxmgr"
export OXMGR_DATA_DIR="$OXMGR_HOME"
# byte_sum of the absolute home path (ASCII path -> char code == byte), pure
# bash so the runtime needs no awk/od beyond coreutils.
_sum=0
for ((_i = 0; _i < ${#OXMGR_HOME}; _i++)); do
    printf -v _b '%d' "'${OXMGR_HOME:_i:1}"
    _sum=$((_sum + _b))
done
_port=$((18500 + _sum % 400))
export OXMGR_DAEMON_ADDR="127.0.0.1:$_port"
export OXMGR_API_ADDR="127.0.0.1:$((_port + 1000))"
mkdir -p "$OXMGR_HOME"
chown mediaservo:mediaservo "$OXMGR_HOME"

# ── 5. background apply helper (daemon boots app-less on a fresh container) ────
(
    _ready=0
    for _ in $(seq 1 120); do
        # readiness = the daemon event socket exists (the daemon opens its IPC
        # TCP port BEFORE listening on this socket, so socket-present implies
        # TCP-ready). A file probe connects to nothing — a /dev/tcp connect
        # would make the daemon log an IPC handshake ERROR for the probe.
        if [ -S "$OXMGR_HOME/events.sock" ]; then _ready=1; break; fi
        sleep 0.5
    done
    if [ "$_ready" = 1 ]; then
        setpriv --reuid="$MS_UID" --regid="$MS_GID" --clear-groups \
            "$OXMGR" apply "$TREE/run/oxfile.toml" \
            || echo "FATAL: oxmgr apply failed — apps not started (check run/logs)" >&2
    else
        echo "FATAL: oxmgr daemon event socket never appeared — apps not applied" >&2
    fi
) &

# ── 6. PID1: privilege-dropped oxmgr daemon (exec keeps PID-1 signal fidelity) ──
exec setpriv --reuid="$MS_UID" --regid="$MS_GID" --clear-groups "$OXMGR" daemon run