#!/bin/bash
# R18 load-test server setup: host networking (reliable WebRTC ICE, no docker NAT)
# + pre-seeded server.yaml (init first-writer-wins skips it) on ports that avoid
# the live dev stack (listen 19800 / SFU 21000 / web 18080). Writes the PSK to
# r18-psk.txt next to this script for the driver.
#
# Usage: IMAGE=msrtc-server:0.1.3 bash setup-server.sh
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE="${IMAGE:-msrtc-server:0.1.3}"
NAME="${NAME:-msrtc-r18}"
VOL="${NAME}-etc"
PSK=$(openssl rand -hex 32); JWT=$(openssl rand -hex 32)
echo "$PSK" > "$SCRIPT_DIR/r18-psk.txt"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker volume rm "$VOL" >/dev/null 2>&1 || true
docker volume create "$VOL" >/dev/null
docker run --rm -v "$VOL:/etc/msrtc" -e PSK="$PSK" -e JWT="$JWT" --entrypoint sh "$IMAGE" -c '
cat > /etc/msrtc/server.yaml <<EOF
version: 1
listen:
  host: "0.0.0.0"
  port: 19800
room_capacity: 50
rate_limit: 10000
consumer_limit_per_stream: 50
psk: "$PSK"
jwt_secret: "$JWT"
admin_jwt_secret: "$JWT"
devices_file: "devices.yaml"
accounts_file: "accounts.yaml"
sfu:
  port: 21000
  announced_ips:
    - "127.0.0.1"
web:
  port: 18080
EOF
chown 1000:1000 /etc/msrtc/server.yaml; chmod 600 /etc/msrtc/server.yaml'
docker run -d --name "$NAME" --network host \
  -e MEDIASERVO_SFU_ANNOUNCED_IP=127.0.0.1 -e MEDIASERVO_SFU_PORT=21000 \
  -e MEDIASERVO_ADMIN_PASSWORD=r18-load-pass -e RUST_LOG=info \
  -v "$VOL:/opt/msrtc/etc" "$IMAGE" >/dev/null
for i in $(seq 1 24); do sleep 5; H=$(curl --noproxy '*' -s -o /dev/null -w '%{http_code}' --max-time 4 http://127.0.0.1:18080/health 2>/dev/null||true); [ "$H" = 200 ] && break; done
echo "health=$H container=$NAME WS=ws://127.0.0.1:19800/ws"
[ "$H" = 200 ] || { echo "FAIL: server not healthy"; exit 1; }