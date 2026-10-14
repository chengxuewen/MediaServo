Environment variables (deployment env master table - the release shell and the server/host instance help render this same file; single source of truth: assets/env-usage.md):
  [A] auto-injected by scripts (no action needed): MSRTC_BRAND(->MEDIASERVO_BRAND=${MEDIASERVO_BRAND}) / MSRTC_OUT_ROOT / OXMGR_DATA_DIR / MSRTC_PY(submodule CLI interpreter, unset=python3)
  [B] server-cluster manual lines - persisted in the [apps.env] section of each [[apps]] entry in run/oxfile.toml.
      WARNING: full-tree rebuilds or render-from-scratch (init / start fallback) drop manual lines - re-add them per this table (the migration re-absorption only fills gaps, never overwrites):
      MEDIASERVO_ALLOW_DEV_CREDENTIALS=1   allow dev placeholder accounts (unset in production)
      ALLOW_DEV_ENROLL=1                   auto-enroll unknown devices by public-key fingerprint (D283; private-network/dev profile, unset in production)
      RUST_LOG=mediaservo_server=info      log level
      WEAKNET_ADMIN_PASS=...                 weaknet panel write credential (credential never templated, PIT-160 channel)
  [C] process-start env (injected via systemd/oxmgr/shell, any of them):
      MEDIASERVO_PSK                       signaling shared secret (server.yaml first, env fallback)
      MEDIASERVO_SFU_ANNOUNCED_IP          ICE announced address (required with multi-NIC / container NAT)
      MEDIASERVO_SFU_PORT                  fixed media port (default ${MEDIASERVO_SFU_PORT})
      MEDIASERVO_WEB_PORT                  set when the web entrypoint must yield port 8080
      WEAKNET_BIN                          top preference for weaknet entrypoint resolution (cluster host: unset already resolves out/server/bin)
      MEDIASERVO_CONTROL_HMAC_KEY_FILE     vehicle e-stop signature key file (recommended; 0600 gate, D-HMAC;
                                           controller-only - inject via unit Environment= or shell export)
      MEDIASERVO_CONTROL_HMAC_KEY          e-stop key in plaintext (migration compatibility; file form takes precedence)
  Note: the host tree's runtime env is fully re-rendered into run/oxfile.toml from etc/host.yaml at every start - hand-editing that oxfile to add env lines gets overwritten and is unsupported
