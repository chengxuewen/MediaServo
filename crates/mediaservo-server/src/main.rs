use futures_util::FutureExt;
use mediaservo_common::auth::JwtAuth;
use mediaservo_server::admin;
use mediaservo_server::config;
use mediaservo_server::monitor;
use mediaservo_server::signaling;
use std::time::Duration;
use tower_http::timeout::TimeoutLayer;

mod lifecycle;

/// Usage table -- brand replace reuses the host.rs USAGE pattern (T15; default brand mediaservo-server).
const USAGE: &str = "Usage: mediaservo-server <init|start|stop|restart|status|doctor|logs|startup|monit|ps|run|version> [<dir>] (dir is a positional arg, default .server/)
Subcommands:
  init [<dir>]       Create an instance: etc/{server,devices,accounts}.yaml + Caddyfile + run/oxfile.toml
                     + PSK/JWT secret bootstrap (0600, idempotent; compose entrypoint logic folded in -- both tracks share semantics)
  start [<dir>] [--no-web]
                     oxmgr apply brings up the process cluster (mediaservo-server + caddy web); port conflict = interactive takeover /
                     non-interactive exit; --no-web = backend only (dev form); caddy missing from PATH -> warn + automatic degradation
  stop [<dir>] [--all|-g]  Stop the whole cluster (oxmgr stop+delete + instance daemon convergence); -g = global reap by process identity, across dirs and daemons
  restart [<dir>] [--no-web]  stop then apply again
  status [<dir>]     Process table + /ready probe column (exit 0 healthy / 1 degraded / 2 not running or invalid dir)
  doctor [<dir>]     Environment diagnostics (oxmgr/caddy PATH, yaml, web dist, announced IP; exit code = failure count)
  logs [server|web|all] [<dir>]  oxmgr logs passthrough (supports -f/--lines)
  startup on|off|status [<dir>]  Boot anchor (systemd user unit brings up the instance daemon, globally unique)
  monit              oxmgr TUI (processes/CPU/RAM/log stream)
  ps                 oxmgr process list
  run                [DAEMON MODE] current mediaservo-server daemon startup behavior as-is (--config etc.);
                     no args/unknown args -> falls back to this mode -- existing systemd/compose/script direct starts untouched
  version            Version info

Examples:
  mediaservo-server init /opt/mediaservo && mediaservo-server start /opt/mediaservo
  mediaservo-server --config /etc/mediaservo/server.yaml    daemon direct start (legacy path)
  mediaservo-server -h|-H|--help                  this help";

fn print_usage() {
    // The env table comes from the common asset (single source, same source as msrtc.sh/host); brand word substituted per product.
    println!(
        "{}",
        format!("{}\n\n{}", USAGE, mediaservo_common::env_usage_text())
            .replace("mediaservo-server", &lifecycle::templates::server_product())
    );
}

/// Entry point -- single binary dual-role dispatch (T15). Admin-plane subcommands -> lifecycle;
/// `run` -> daemon (explicit form); no args/`--config`/unknown -> daemon fallback (hard backward-compat gate:
/// systemd/compose/docker direct-start chain behavior is byte-identical).
fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let first = argv.get(1).map(String::as_str);
    if matches!(first, Some("-h") | Some("-H") | Some("--help")) {
        print_usage();
        return;
    }
    if let Some(cmd) = first.filter(|c| lifecycle::is_lifecycle_cmd(c)) {
        let code = lifecycle::dispatch(cmd, &mut argv.iter().skip(2).cloned());
        std::process::exit(code);
    }
    // Option-argument tightening (PIT-171 UX defect): the daemon branch actually only accepts --config (run_server
    // only reads args[1]=="--config"); any other leading "-x" falls back to the daemon = all-default config + a misleading
    // panic against the C35 guard. Unknown option -> USAGE + exit 2 (no-arg/`--config` prefix/run legacy chain untouched).
    if let Some(f) = first
        && f.starts_with('-')
        && f != "--config"
    {
        eprintln!("unknown option: {f}");
        print_usage();
        std::process::exit(2);
    }
    // `run` = explicit daemon form: drop that token; the remaining argv is isomorphic to direct start (--config lands back at args[1])
    let daemon_argv = if first == Some("run") {
        let mut v: Vec<String> = Vec::with_capacity(argv.len().saturating_sub(1));
        v.extend(argv.first().cloned());
        v.extend(argv.iter().skip(2).cloned());
        v
    } else {
        argv
    };
    run_daemon(daemon_argv);
}

/// Daemon mode = the original main body (panic hook + runtime + run_server). argv passthrough
/// (the `--config` check moved from env::args() to an explicit argument; everything else unchanged).
fn run_daemon(argv: Vec<String>) {
    // ── Panic boundary ───────────────────────────────────────────────────────
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        // Log before the process aborts — tracing may not flush in time,
        // so also emit to stderr as a fallback.
        eprintln!("FATAL PANIC at {location}: {msg}");
        tracing::error!(panic.location = %location, panic.message = %msg, "Server panic");
    }));

    // Wrap async body in catch_unwind so panics are logged before exit.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let result =
        rt.block_on(async { std::panic::AssertUnwindSafe(run_server(argv)).catch_unwind().await });

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::error!("Server error: {}", e);
            std::process::exit(1);
        }
        Err(_panic) => {
            // Already logged by panic hook
            tracing::error!("Server terminated due to panic");
            std::process::exit(1);
        }
    }
}

async fn run_server(argv: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("MediaServo Server v{} starting", env!("CARGO_PKG_VERSION"));

    // Parse config — collect args once for bounds-safe access
    // Default: relative to the binary path bin/../etc/server.yaml (generated by the build server assembly)
    // Container fallback: /opt/mediaservo/etc/server.yaml (legacy path fallback)
    let config_path = {
        let args: Vec<String> = argv;
        if args.len() > 2 && args[1] == "--config" {
            args[2].clone()
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|p| p.join("../etc/server.yaml")))
                .filter(|p| p.exists())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/opt/mediaservo/etc/server.yaml".to_string())
        }
    };
    let config = match config::load(&config_path) {
        Ok(c) => {
            tracing::info!("Config loaded from {config_path}");
            c
        }
        Err(e) => {
            tracing::warn!("Config {config_path}: {e}, using defaults");
            serde_yaml::from_str(DEFAULT_SERVER_CONFIG).unwrap()
        }
    };

    // Relative path resolution: devices/accounts paths are relative to the config file's directory (not CWD)
    let config_dir =
        std::path::Path::new(&config_path).parent().unwrap_or_else(|| std::path::Path::new("."));
    let resolve_path = |p: &str| -> String {
        let path = std::path::Path::new(p);
        if path.is_absolute() {
            p.to_string()
        } else {
            config_dir.join(path).to_string_lossy().into_owned()
        }
    };

    // Build JWT authenticator from config (optional)
    let jwt_auth = config.jwt_secret.as_ref().map(|s| JwtAuth::new(s.as_str()));

    // -- G2 device registry load (devices.yaml; default path = same dir as server.yaml) ----------------
    // Missing file/parse failure -> empty registry + warning (PSK path unaffected, does not block startup).
    let devices_path = config
        .devices_file
        .as_deref()
        .map(resolve_path)
        .unwrap_or_else(|| resolve_path("devices.yaml"));
    let device_registry = match mediaservo_server::devices::DeviceRegistry::load(&devices_path) {
        Ok(reg) => {
            tracing::info!("Device registry loaded from {devices_path}: {} devices", reg.len());
            reg
        }
        Err(e) => {
            tracing::warn!(
                "Device registry {devices_path}: {e}; running with empty registry (PSK path only)"
            );
            mediaservo_server::devices::DeviceRegistry::empty()
        }
    };
    // unified-device-admin: a single Arc instance shared by signaling (access auth) and admin (management write-back).
    let device_registry = std::sync::Arc::new(device_registry);

    // -- accountless-client-auth T1: API-key registry (api_keys.yaml; default = same dir as server.yaml)
    // Missing file/parse failure -> empty registry + warning (exchange always 401; account/PSK/device paths unaffected).
    let api_keys_path = config
        .api_keys_file
        .as_deref()
        .map(resolve_path)
        .unwrap_or_else(|| resolve_path("api_keys.yaml"));
    let api_registry = match mediaservo_server::apikeys::ApiKeyRegistry::load(&api_keys_path) {
        Ok(reg) => {
            tracing::info!("API key registry loaded from {api_keys_path}: {} keys", reg.len());
            reg
        }
        Err(e) => {
            tracing::warn!("API key registry {api_keys_path}: {e}; running with empty registry");
            mediaservo_server::apikeys::ApiKeyRegistry::empty()
        }
    };
    let api_registry = std::sync::Arc::new(api_registry);

    // device-enroll S7: ALLOW_DEV_ENROLL (env, default false = production manual pending queue;
    // =1 controlled network/dev auto-registers on verify success, zero manual steps). Parsing semantics same family (ALLOW_DEV_CREDENTIALS): literal "1" only.
    let allow_dev_enroll = std::env::var("ALLOW_DEV_ENROLL").as_deref() == Ok("1");
    if allow_dev_enroll {
        tracing::warn!(
            "ALLOW_DEV_ENROLL=1: unknown devices auto-register after pubkey verify passes (controlled private network/dev environments only)"
        );
    }

    // -- G3 cockpit account registry load (accounts.yaml; default path = same dir as server.yaml) --
    // Missing file/parse failure -> empty registry + warning (PSK/device paths unaffected, does not block startup).
    let accounts_path = config
        .accounts_file
        .as_deref()
        .map(resolve_path)
        .unwrap_or_else(|| resolve_path("accounts.yaml"));
    let accounts = match mediaservo_server::accounts::AccountRegistry::load(&accounts_path) {
        Ok(reg) => {
            tracing::info!("Account registry loaded from {accounts_path}: {} accounts", reg.len());
            reg
        }
        // I4 review: file exists but is corrupt -> fail-fast (silently empty registry = authorization enforcement silently disabled).
        Err(e) => {
            panic!(
                "account registry {accounts_path} is corrupt; refusing to start with authorization disabled: {e}"
            );
        }
    };
    // I2 review: known dev placeholder accounts fail-fast (config/accounts.yaml is local dev only) --
    // explicit override env MEDIASERVO_ALLOW_DEV_CREDENTIALS=1 (set in dev compose) exempts.
    if let Err(e) = accounts.check_dev_credentials(
        std::env::var("MEDIASERVO_ALLOW_DEV_CREDENTIALS").as_deref() == Ok("1"),
    ) {
        panic!("{e}");
    }
    let accounts = std::sync::Arc::new(accounts);

    // I2 review: account tokens are signed with admin_jwt_secret, /ws verifies with jwt_secret --
    // a mismatch = account auth silently falls back to PSK (matrix bypass); fail-fast at startup.
    if let Err(e) = mediaservo_server::accounts::validate_secret_pairing(
        !accounts.is_empty(),
        config.jwt_secret.as_deref(),
        config.admin_jwt_secret.as_deref(),
    ) {
        panic!("{e}");
    }

    // Print admin setup token if configured
    if let Some(ref secret) = config.admin_jwt_secret {
        admin::print_setup_token(secret);
    }

    // Create the signaling server (shared state for WebSocket rooms)
    #[cfg(feature = "sfu-mediasoup")]
    let signaling_server = {
        use mediaservo_server::sfu;
        use std::sync::Arc;

        match sfu::SfuManager::new(Some(&config)).await {
            Ok(m) => {
                tracing::info!("SFU manager initialized (mediasoup)");
                let mut srv = signaling::SignalingServer::new(
                    Arc::new(m),
                    config.ws_max_message_size,
                    jwt_auth.clone(),
                );
                srv.device_registry = std::sync::Arc::clone(&device_registry);
                srv.allow_dev_enroll =
                    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(allow_dev_enroll));
                srv.devices_path = std::sync::Arc::from(devices_path.as_str());
                // psk-admin-management: config first + env fallback (dead-config fix -- server.yaml psk formally wired into auth)
                let merged_psk =
                    config.psk.clone().or_else(|| std::env::var("MEDIASERVO_PSK").ok());
                srv.psk_state = std::sync::Arc::new(std::sync::RwLock::new(merged_psk));
                srv
            }
            Err(e) => {
                tracing::info!("SFU manager skipped: {e}");
                panic!("sfu-mediasoup feature enabled but worker failed: {e}");
            }
        }
    };
    #[cfg(not(feature = "sfu-mediasoup"))]
    let mut signaling_server = {
        let mut srv = signaling::SignalingServer::new(config.ws_max_message_size, jwt_auth);
        srv.device_registry = std::sync::Arc::clone(&device_registry);
        srv.allow_dev_enroll =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(allow_dev_enroll));
        srv.devices_path = std::sync::Arc::from(devices_path.as_str());
        // psk-admin-management: config first + env fallback (dead-config fix -- server.yaml psk formally wired into auth)
        let merged_psk = config.psk.clone().or_else(|| std::env::var("MEDIASERVO_PSK").ok());
        srv.psk_state = std::sync::Arc::new(std::sync::RwLock::new(merged_psk));
        srv
    };

    // Build axum router
    let signaling_router = signaling::signaling_router(signaling_server.clone());
    let monitor_router = monitor::monitor_router(signaling_server.clone());

    // ── Admin API state ────────────────────────────────────────────────────
    // Same source as the built-in SignalingServer channel (stream up/down events pushed via the signaling path).
    let admin_tx = signaling_server.admin_events();
    let admin_state = admin::AdminState {
        event_tx: admin_tx,
        signaling: signaling_server.clone(),
        admin_jwt_secret: config.admin_jwt_secret.clone(),
        listen_host: config.listen.host.clone(),
        listen_port: config.listen.port,
        rate_limit: config.rate_limit,
        room_capacity: config.room_capacity,
        consumer_limit_per_stream: config.consumer_limit_per_stream,
        accounts: std::sync::Arc::clone(&accounts),
        accounts_path: accounts_path.clone(),
        psk_state: std::sync::Arc::clone(&signaling_server.psk_state),
        config_path: config_path.clone(),
        device_registry: std::sync::Arc::clone(&device_registry),
        devices_path: devices_path.clone(),
        api_registry: std::sync::Arc::clone(&api_registry),
        api_keys_path: api_keys_path.clone(),
        api_token_ttl_secs: config.api_token_ttl_secs,
        #[cfg(feature = "sfu-mediasoup")]
        sfu_manager: std::sync::Arc::clone(&signaling_server.sfu_manager),
    };

    let admin_router = admin::admin_router(admin_state.clone());
    // G3: login endpoint on its own router (not intercepted by the admin auth middleware -- it is the credential issuer).
    let login_router = admin::login_router(admin_state.clone());

    let app = axum::Router::new()
        .merge(signaling_router)
        .merge(monitor_router)
        .merge(login_router)
        // p3 W2-A: consumer-facing room discovery (separate router, isolated from the admin privileged plane).
        .merge(mediaservo_server::rooms::rooms_router(admin_state.clone()))
        .merge(admin_router);
    let app = mediaservo_server::static_files::add_admin_routes(app);

    // Rate limiting: per-IP governor using config.rate_limit requests/sec
    // ponytail: rate limiter disabled for testing
    // let governor_conf = ...

    // Timeout: hard cap request processing at 30s to prevent resource exhaustion
    let app = app.layer(TimeoutLayer::new(Duration::from_secs(30)));

    // Bind address
    let bind_addr = format!("{}:{}", config.listen.host, config.listen.port);

    // Run server with graceful shutdown + connection draining
    if let Some(ref tls_cfg) = config.tls {
        // ── TLS mode ──
        let rustls_config = mediaservo_server::tls::build_rustls_config(tls_cfg)
            .await
            .unwrap_or_else(|e| panic!("TLS setup failed: {e}"));
        tracing::info!("Listening on {} (TLS)", bind_addr);
        tracing::info!("Server ready on {} (TLS)", bind_addr);

        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();

        let server = axum_server::bind_rustls(bind_addr.parse().unwrap(), rustls_config)
            .handle(handle)
            .serve(app.into_make_service());

        let shutdown_future = async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutdown signal received, initiating graceful shutdown...");
            signaling_server.shutdown();
            shutdown_handle.shutdown();

            let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                let remaining = signaling_server.active_connections();
                if remaining == 0 {
                    tracing::info!("All connections drained, shutting down");
                    break;
                }
                if tokio::time::Instant::now() >= drain_deadline {
                    tracing::warn!(
                        "Shutdown timeout reached (30s) with {} active connections, forcing exit",
                        remaining
                    );
                    break;
                }
                tracing::info!("Draining: {} active connections remaining", remaining);
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        };

        tokio::select! {
            result = server => {
                if let Err(e) = result {
                    tracing::error!("Server error: {}", e);
                }
            }
            () = shutdown_future => {}
        }
    } else {
        // ── Plain TCP mode ──
        let listener = tokio::net::TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| format!("Failed to bind {}: {}", bind_addr, e))?;
        tracing::info!("Listening on {}", bind_addr);
        tracing::info!("Server ready on {}", bind_addr);

        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutdown signal received, initiating graceful shutdown...");
            signaling_server.shutdown();

            let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                let remaining = signaling_server.active_connections();
                if remaining == 0 {
                    tracing::info!("All connections drained, shutting down");
                    break;
                }
                if tokio::time::Instant::now() >= drain_deadline {
                    tracing::warn!(
                        "Shutdown timeout reached (30s) with {} active connections, forcing exit",
                        remaining
                    );
                    break;
                }
                tracing::info!("Draining: {} active connections remaining", remaining);
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });

        if let Err(e) = server.await {
            tracing::error!("Server error: {}", e);
        }
    }
    tracing::info!("Shutdown complete");
    Ok(())
}

/// Default server config for headless/E2E fallback.
const DEFAULT_SERVER_CONFIG: &str = r#"
listen:
  host: "0.0.0.0"
  port: 9800
room_capacity: 10
rate_limit: 100
psk: "mediaservo-dev"
"#;
