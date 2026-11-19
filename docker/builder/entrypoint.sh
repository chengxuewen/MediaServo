#!/bin/bash
# builder-entrypoint.sh (docker/builder/) — msrtc-builder container entry (container-build-rework B2)
#
# root side: ssh host keys (fresh containers; persist via mounting a volume at
# /etc/ssh for stable host identity), privilege-separation dir; then exec the
# requested command. Default CMD = sshd -D (ssh-resident dev mode); CI runs
# `docker run --rm <image> bash -c "...build..."` which execs instead.
#
# Key injection (dev mode, done by the runner — not baked in the image):
#   docker run -v ~/.ssh/id_ed25519.pub:/home/dev/.ssh/authorized_keys:ro ...
#   docker exec <c> chown dev:dev /home/dev/.ssh/authorized_keys
set -e
mkdir -p /var/run/sshd
if [ ! -f /etc/ssh/ssh_host_rsa_key ]; then
    ssh-keygen -A
fi
exec "$@"
