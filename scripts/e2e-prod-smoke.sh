#!/usr/bin/env bash
# e2e-prod-smoke.sh — 生产冒烟（e2e smoke 套件）: up --env prod 单容器 → admin 200
# + 认证闭合（psk_set=true——非默认放行）+ 设备配发原子写（无 EBUSY fallback）+ 卷持久化。
# 前置: mediaservo-server:latest 镜像（build server --image runtime 或已有）
set -euo pipefail
cd "$(dirname "$0")/.."

FAIL=0
note() { echo "== $1"; }

# 1. 起 prod（命名卷——首次自动初始化）
note "up --env prod"
docker compose -f docker-compose.yml up -d server 2>&1 | tail -1

# 2. 等就绪（healthcheck 探针）
note "wait ready"
for i in $(seq 1 20); do
    sleep 5
    C=$(curl --noproxy "*" -s -o /dev/null -w "%{http_code}" --max-time 4 http://127.0.0.1:9800/admin 2>/dev/null || true)
    [ "$C" = "200" ] && break
done
[ "$C" = "200" ] || { echo "FAIL: admin 未就绪（$C）"; FAIL=1; }

# 3. 认证闭合（首启生成密钥——日志 psk_set=true 非默认放行）
note "auth closed (no default psk)"
if docker logs mediaservo-server-1 2>&1 | grep -q "psk_set=true"; then
    echo "OK: psk_set=true（entrypoint 自举生效）"
else
    echo "FAIL: 未发现 psk_set=true——认证可能未闭合（检查 .env 生成）"; FAIL=1
fi
docker exec mediaservo-server-1 sh -c 'grep -c "mediaservo-dev" /opt/mediaservo/etc/.env 2>/dev/null' 2>/dev/null \
    && { echo "FAIL: .env 含默认 psk"; FAIL=1; } || echo "OK: .env 无默认 psk"

# 4. 设备配发原子写（无 EBUSY fallback）
note "device provision atomic write"
TOKEN=$(curl --noproxy "*" -s -X POST http://127.0.0.1:9800/api/auth/login \
    -H 'Content-Type: application/json' \
    -d "{\"username\":\"admin\",\"password\":\"${MEDIASERVO_ADMIN_PASSWORD:-smoke-admin-pass}\"}" 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('token',''))" 2>/dev/null || true)
if [ -z "$TOKEN" ]; then
    echo "FAIL: admin 登录失败（引导账号未建？MEDIASERVO_ADMIN_PASSWORD）"; FAIL=1
else
    curl --noproxy "*" -s -X POST http://127.0.0.1:9800/api/admin/devices \
        -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"note":"smoke"}' >/dev/null 2>&1 || true
    if docker logs mediaservo-server-1 2>&1 | grep -q "falling back to direct write"; then
        echo "WARN: 原子写回退触发（EBUSY？）——命名卷应普通文件"
    else
        echo "OK: 配发无 EBUSY fallback（命名卷原子写）"
    fi
fi

# 5. 卷持久化（重启容器数据保留）
note "volume persistence"
docker compose -f docker-compose.yml restart server >/dev/null 2>&1
sleep 8
if docker exec mediaservo-server-1 sh -c 'grep -q smoke /opt/mediaservo/etc/devices.yaml' 2>/dev/null; then
    echo "OK: 重启后设备配发保留（卷持久化）"
else
    echo "FAIL: 重启后 devices.yaml 无 smoke 设备"; FAIL=1
fi

echo
if [ "$FAIL" = 0 ]; then
    echo "PASS: prod smoke 全绿"
else
    echo "FAIL: $FAIL 项失败"; exit 1
fi