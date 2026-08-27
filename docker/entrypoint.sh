#!/usr/bin/env bash
# MediaServo server entrypoint — 命名卷初始化 + 密钥自举（团队审核 C1/C2/C5/H8 吸收）
#
# 安全契约（顺序不可乱）:
#   ① root 运行（镜像默认 USER=root）——写卷
#   ② 密钥单源生成: PSK → 卷 .env（chmod 600）; jwt/admin_jwt 同值 → server.yaml
#      （server 认证路径: PSK 只读 env（signaling.rs）; JWT 读 yaml（admin.rs））
#   ③ 模板落位（prod-safe——禁 dev 占位）: accounts:{} + devices:{} + server.yaml 渲染
#   ④ 未完成检查（任一缺失 exit 1——首启幂等——不启 server）
#   ⑤ 可选 admin 引导（MEDIASERVO_ADMIN_PASSWORD）
#   ⑥ set -a 注入 .env → exec su-exec 降权（PID-1 信号保真）
set -euo pipefail

ETC=/usr/local/etc
TEMPLATES=/usr/local/etc/templates
ENV_FILE="$ETC/.env"
PASSWD_FILE="$ETC/accounts.yaml"

# ── ① 卷目录（镜像层已 chown mediaservo——copy-up 保留属主）──────────────
mkdir -p "$ETC"

# ── ② 密钥单源生成（先于模板——首启幂等）─────────────────────────────
if [ ! -f "$ENV_FILE" ]; then
    umask 177
    PSK=$(openssl rand -hex 32)
    JWT=$(openssl rand -hex 32)
    printf 'MEDIASERVO_PSK=%s\n' "$PSK" > "$ENV_FILE"
    # server.yaml 渲染（jwt/admin_jwt 同值——accounts.rs fail-fast 一致性）
    sed "s|__JWT_SECRET__|${JWT}|g" "$TEMPLATES/server.yaml.template" > "$ETC/server.yaml"
    # root 生成 → 属主移交 mediaservo（su-exec 降权后 server 可读）
    chown mediaservo:mediaservo "$ENV_FILE" "$ETC/server.yaml"
    chmod 600 "$ENV_FILE" "$ETC/server.yaml"
    echo "generated fresh secrets (psk/jwt) into $ETC" >&2
fi

# ── ③ 模板落位（逐文件缺失拷贝——已有跳过：持久化语义）──────────────
for f in accounts.yaml devices.yaml; do
    [ -f "$ETC/$f" ] || { cp "$TEMPLATES/$f" "$ETC/$f"; chown mediaservo:mediaservo "$ETC/$f"; }
done

# ── ④ 未完成检查（幂等——任一缺失 exit 1——容器内不依赖 DEFAULT fallback）──
for f in "$ENV_FILE" "$ETC/server.yaml" "$ETC/accounts.yaml" "$ETC/devices.yaml"; do
    [ -f "$f" ] || { echo "FATAL: init incomplete — missing $f" >&2; exit 1; }
done

# ── ⑤ admin 引导（首启创建——空账号表否则无法登录）────────────────────
if [ -n "${MEDIASERVO_ADMIN_PASSWORD:-}" ]; then
    if ! grep -q "^  admin:" "$PASSWD_FILE"; then
        HASH=$(printf '%s:%s' "admin" "$MEDIASERVO_ADMIN_PASSWORD" | openssl dgst -sha256 | awk '{print $2}')
        cat >> "$PASSWD_FILE" <<EOF
  admin:
    password_hash: "sha256:${HASH}"
    role: admin
EOF
        echo "created bootstrap admin account (password from MEDIASERVO_ADMIN_PASSWORD)" >&2
    fi
    unset MEDIASERVO_ADMIN_PASSWORD
fi

# ── ⑥ 注入 env + 降权 exec（PID-1 信号保真——su-exec 而非 su）──────────
set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a
# 二进制 /usr/local/bin/ → 相对路径 bin/../etc/server.yaml = /usr/local/etc/server.yaml（entrypoint 已生成）
exec su-exec mediaservo mediaservo-server "$@"
