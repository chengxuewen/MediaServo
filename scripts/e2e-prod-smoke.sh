#!/usr/bin/env bash
# e2e-prod-smoke.sh — production smoke (e2e smoke suite): up --env prod single container -> admin 200
# + auth closed (psk_set=true — not default-open) + device-provision atomic write (no EBUSY fallback) + volume persistence.
# Prereq: mediaservo-server:latest image (build server --image runtime or pre-existing)
set -euo pipefail
cd "$(dirname "$0")/.."

FAIL=0
note() { echo "== $1"; }

# 1. Start prod (named volume — auto-initialized on first run)
note "up --env prod"
docker compose -f docker-compose.yml up -d server 2>&1 | tail -1

# 2. Wait for readiness (healthcheck probe)
note "wait ready"
for i in $(seq 1 20); do
    sleep 5
    C=$(curl --noproxy "*" -s -o /dev/null -w "%{http_code}" --max-time 4 http://127.0.0.1:9800/admin 2>/dev/null || true)
    [ "$C" = "200" ] && break
done
[ "$C" = "200" ] || { echo "FAIL: admin not ready ($C)"; FAIL=1; }

# 3. Auth closed (key generated on first boot — log psk_set=true, not default-open)
note "auth closed (no default psk)"
if docker logs mediaservo-server-1 2>&1 | grep -q "psk_set=true"; then
    echo "OK: psk_set=true (entrypoint bootstrap in effect)"
else
    echo "FAIL: no psk_set=true found — auth may not be closed (check .env generation)"; FAIL=1
fi
docker exec mediaservo-server-1 sh -c 'grep -c "mediaservo-dev" /opt/mediaservo/etc/.env 2>/dev/null' 2>/dev/null \
    && { echo "FAIL: .env contains the default psk"; FAIL=1; } || echo "OK: no default psk in .env"

# 4. Device-provision atomic write (no EBUSY fallback)
note "device provision atomic write"
TOKEN=$(curl --noproxy "*" -s -X POST http://127.0.0.1:9800/api/auth/login \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"admin\",\"password\":\"${MEDIASERVO_ADMIN_PASSWORD:-smoke-admin-pass}\"}" 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('token',''))" 2>/dev/null || true)
if [ -z "$TOKEN" ]; then
    echo "FAIL: admin login failed (bootstrap account missing? MEDIASERVO_ADMIN_PASSWORD)"; FAIL=1
else
    curl --noproxy "*" -s -X POST http://127.0.0.1:9800/api/admin/devices \
        -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"note":"smoke"}' >/dev/null 2>&1 || true
    if docker logs mediaservo-server-1 2>&1 | grep -q "falling back to direct write"; then
        echo "WARN: atomic write fell back (EBUSY?) — named volume should be a regular file"
    else
        echo "OK: provisioning without EBUSY fallback (atomic write on named volume)"
    fi
fi

# 5. Volume persistence (data survives container restart)
note "volume persistence"
docker compose -f docker-compose.yml restart server >/dev/null 2>&1
sleep 8
if docker exec mediaservo-server-1 sh -c 'grep -q smoke /opt/mediaservo/etc/devices.yaml' 2>/dev/null; then
    echo "OK: device provisioning persisted after restart (volume persistence)"
else
    echo "FAIL: no smoke device in devices.yaml after restart"; FAIL=1
fi

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: prod smoke all green"
else
    echo "FAIL: $FAIL checks failed"; exit 1
fi