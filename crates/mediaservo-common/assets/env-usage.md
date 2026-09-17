环境变量（部署侧 env 总表——发布壳与 server/host 实例帮助共用本文件渲染，单一真源 assets/env-usage.md）:
  [A] 脚本自动注入（不用管）: MSRTC_BRAND(→MEDIASERVO_BRAND=${MEDIASERVO_BRAND}) / MSRTC_OUT_ROOT / OXMGR_DATA_DIR / MSRTC_PY(子模块 CLI 解释器，未设=python3)
  [B] server 簇手工行——持久于 run/oxfile.toml 对应 [[apps]] 的 [apps.env] 段。
      ⚠ 整树重建或从零渲染（init / start 兜底）时手工行会丢——须照此回补（迁移链的回吸收只补缺不覆盖）:
      MEDIASERVO_ALLOW_DEV_CREDENTIALS=1   放行开发占位账号（生产不设）
      ALLOW_DEV_ENROLL=1                   公钥指纹自动入册陌生设备（D283；专网/开发档，生产不设）
      RUST_LOG=mediaservo_server=info      日志级别
      WEAKNET_ADMIN_PASS=…                 弱网面板写凭证（凭证不入模板，PIT-160 通道）
  [C] 进程启动 env（systemd/oxmgr/shell 注入均可）:
      MEDIASERVO_PSK                       信令共享密钥（server.yaml 优先，env 兜底）
      MEDIASERVO_SFU_ANNOUNCED_IP          ICE 公告地址（多网卡/容器 NAT 必设）
      MEDIASERVO_SFU_PORT                  媒体固定端口（缺省 ${MEDIASERVO_SFU_PORT}）
      MEDIASERVO_WEB_PORT                  web 入口 8080 让位时用
      WEAKNET_BIN                          弱网入口解析首选（簇常驻机不设即命中 out/server/bin）
      MEDIASERVO_CONTROL_HMAC_KEY_FILE     车端急停验签密钥文件（推荐；0600 门，D-HMAC；
                                           controller 专属——注入位=unit Environment= 或 shell export）
      MEDIASERVO_CONTROL_HMAC_KEY          急停密钥明文形（迁移兼容，文件形优先）
  注: host 树运行 env 每次 start 由 etc/host.yaml 全量渲染进 run/oxfile.toml——手工编该 oxfile 加 env 行会被覆盖，不被支持
