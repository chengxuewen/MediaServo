#!/usr/bin/env bash
# e2e-prod-smoke.sh — R8 parameterized production smoke (container-build-rework B3.5)
#
# One script, mounted twice (CI PR leg always + all-gates --full). Isolated compose
# project + random admin password + shifted host ports so it never fights a live
# instance. Assertions go through the caddy 8080 entry (R16/R8 post-R11 redirect):
# /health, /admin + /voice content-discriminated, admin first-boot login, auth
# closed, device-provision atomic write, down->up persistence, MemLimit inspect.
#
# Log-source re-point (R8): server logs live in run/logs/*.out.log under oxmgr —
# the old docker-logs scraping (psk_set / "direct write" probes) re-points at the
# file face. The psk_set=true marker is obsolete (current server logs structured
# JSON); auth-closed is asserted from the config file (random PSK, not the dev
# placeholder) + the "Server ready" boot marker.
#
# Prereq: the tree prod image present (Dockerfile.tree / `build image prod`).
# Env knobs (all optional): TAG MEDIASERVO_BRAND WEB_PORT AGENT_PORT SSH_PORT
#   SFU_PORT SFU_PORT_RANGE MEDIASERVO_SFU_ANNOUNCED_IP MEM_LIMIT.
set -euo pipefail
cd "$(dirname "$0")/.."

BRAND="${MEDIASERVO_BRAND:-msrtc}"
TAG="${TAG:-0.1.3}"
IMAGE="$BRAND-server:$TAG"
PROJECT="smoke-$$-$RANDOM"
WEB_PORT="${WEB_PORT:-18080}"
AGENT_PORT="${AGENT_PORT:-19800}"
SSH_PORT="${SSH_PORT:-12222}"
SFU_PORT="${SFU_PORT:-21000}"
SFU_PORT_RANGE="${SFU_PORT_RANGE:-41000-41100}"
ANNOUNCED="${MEDIASERVO_SFU_ANNOUNCED_IP:-127.0.0.1}"
PASS="$(openssl rand -hex 12)"
TREE="/opt/$BRAND"

FAIL=0
note() { echo "== $1"; }
ok()   { echo "OK: $1"; }
bad()  { echo "FAIL: $1"; FAIL=1; }
CURL=(curl --noproxy '*' -s --max-time 5)

ENVF="$(mktemp)"
cat > "$ENVF" <<EOF
TAG=$TAG
MEDIASERVO_BRAND=$BRAND
MEDIASERVO_SFU_ANNOUNCED_IP=$ANNOUNCED
MEDIASERVO_ADMIN_PASSWORD=$PASS
SSH_PORT=$SSH_PORT
WEB_PORT=$WEB_PORT
AGENT_PORT=$AGENT_PORT
SFU_PORT=$SFU_PORT
SFU_PORT_RANGE=$SFU_PORT_RANGE
EOF
COMPOSE=(docker compose -p "$PROJECT" -f docker-compose.yml --env-file "$ENVF")
cleanup() { "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true; rm -f "$ENVF"; }
trap cleanup EXIT

docker image inspect "$IMAGE" >/dev/null 2>&1 || { echo "FAIL: image $IMAGE absent — build it first (Dockerfile.tree)"; exit 1; }

wait_health() {
    local h
    for _ in $(seq 1 24); do
        sleep 5
        h=$("${CURL[@]}" -o /dev/null -w '%{http_code}' "http://127.0.0.1:$WEB_PORT/health" 2>/dev/null || true)
        [ "$h" = 200 ] && return 0
    done
    return 1
}

note "up (isolated project $PROJECT, image $IMAGE)"
"${COMPOSE[@]}" up -d >/dev/null 2>&1
CID="$("${COMPOSE[@]}" ps -q server)"
[ -n "$CID" ] || { echo "FAIL: no server container"; exit 1; }

note "health via caddy 8080 entry"
wait_health && ok "/health = 200 (8080)" || bad "/health not ready"

note "content discrimination (/admin vs /voice through 8080)"
AT=$("${CURL[@]}" "http://127.0.0.1:$WEB_PORT/admin" | grep -oE '<title>[^<]*</title>' | head -1)
VT=$("${CURL[@]}" "http://127.0.0.1:$WEB_PORT/voice/" | grep -oE '<title>[^<]*</title>' | head -1)
[ "$AT" = "<title>MediaServo Admin</title>" ] && ok "/admin = $AT" || bad "/admin = '$AT'"
[ "$VT" = "<title>MediaServo Voice</title>" ] && ok "/voice = $VT" || bad "/voice = '$VT'"
[ -n "$AT" ] && [ "$AT" != "$VT" ] && ok "admin/voice discriminated" || bad "admin/voice NOT discriminated"

note "host-agent direct face (9800)"
N=$("${CURL[@]}" -o /dev/null -w '%{http_code}' "http://127.0.0.1:$AGENT_PORT/health")
[ "$N" = 200 ] && ok "9800 /health = 200" || bad "9800 = $N"

note "admin first-boot login"
login() { "${CURL[@]}" -X POST "http://127.0.0.1:$WEB_PORT/api/auth/login" -H 'Content-Type: application/json' -d "{\"username\":\"admin\",\"password\":\"$PASS\"}" | grep -oE '"token":"[^"]+"' | head -1; }
TOK="$(login)"
[ -n "$TOK" ] && ok "admin JWT issued" || bad "admin login failed"

note "auth closed (random PSK, not dev placeholder) + boot marker (run/logs file face)"
PSK=$(docker exec "$CID" sh -c "grep '^psk:' $TREE/etc/server.yaml" | sed 's/.*"\(.*\)".*/\1/')
[ "$PSK" != "mediaservo-dev" ] && [ "${#PSK}" = 64 ] && ok "PSK random 64-hex (auth closed)" || bad "PSK = '$PSK'"
docker exec "$CID" sh -c "grep -q 'Server ready' $TREE/run/logs/*.out.log" && ok "run/logs Server ready marker" || bad "no Server ready log"
docker exec "$CID" sh -c "grep -q 'mediaservo-dev' $TREE/etc/server.yaml" && bad "dev PSK present" || ok "no dev PSK in config"

note "device-provision atomic write (no EBUSY fallback — run/logs file face)"
if [ -n "$TOK" ]; then
    TOKEN="${TOK#\"token\":\"}"; TOKEN="${TOKEN%\"}"
    "${CURL[@]}" -X POST "http://127.0.0.1:$WEB_PORT/api/admin/devices" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"note":"smoke"}' >/dev/null 2>&1 || true
    if docker exec "$CID" sh -c "grep -q 'falling back to direct write' $TREE/run/logs/*.out.log" 2>/dev/null; then
        bad "atomic write fell back (EBUSY?)"
    else
        ok "provisioning without EBUSY fallback"
    fi
fi

note "resource limits inspect (R9/R13: MemLimit present, cpus absent)"
MEM=$(docker inspect "$CID" --format '{{.HostConfig.Memory}}')
NANO=$(docker inspect "$CID" --format '{{.HostConfig.NanoCpus}}')
[ "$MEM" -gt 0 ] 2>/dev/null && ok "MemLimit = $MEM bytes" || bad "MemLimit = $MEM"
[ "$NANO" = 0 ] && ok "cpus unset (NanoCpus=0, RTP bursts free)" || bad "NanoCpus = $NANO"

note "PID1 = oxmgr daemon run (R11), privilege-dropped"
P1=$(docker exec "$CID" sh -c 'tr "\0" " " < /proc/1/cmdline')
echo "$P1" | grep -q "oxmgr daemon run" && ok "PID1 = $P1" || bad "PID1 = $P1"

note "persistence (down -> up, same named volumes)"
docker exec "$CID" sh -c "grep '^psk:' $TREE/etc/server.yaml" > "$ENVF.psk-before"
"${COMPOSE[@]}" down >/dev/null 2>&1
"${COMPOSE[@]}" up -d >/dev/null 2>&1
CID="$("${COMPOSE[@]}" ps -q server)"
wait_health || bad "health after re-up"
TOK2="$(login)"
[ -n "$TOK2" ] && ok "admin login survives down->up" || bad "persistence login failed"
docker exec "$CID" sh -c "grep '^psk:' $TREE/etc/server.yaml" > "$ENVF.psk-after"
diff "$ENVF.psk-before" "$ENVF.psk-after" >/dev/null && ok "PSK not regenerated (secrets persisted)" || bad "PSK regenerated"
rm -f "$ENVF.psk-before" "$ENVF.psk-after"

note "graceful stop (STOPSIGNAL SIGINT baked)"
docker stop "$CID" >/dev/null 2>&1
EC=$(docker inspect "$CID" --format '{{.State.ExitCode}}')
[ "$EC" = 0 ] && ok "docker stop graceful exit = 0" || bad "stop exit = $EC"

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: prod smoke all green (project $PROJECT)"
else
    echo "FAIL: $FAIL checks failed"; exit 1
fi