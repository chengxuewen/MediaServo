use futures_util::FutureExt;
use mediaservo_common::auth::JwtAuth;
use mediaservo_server::admin;
use mediaservo_server::config;
use mediaservo_server::monitor;
use mediaservo_server::signaling;
use std::time::Duration;
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::timeout::TimeoutLayer;

mod lifecycle;

/// 用法表——品牌 replace 复用 host.rs USAGE 模式（T15；默认品牌 mediaservo-server）。
const USAGE: &str = "用法: mediaservo-server <init|start|stop|restart|status|doctor|logs|startup|monit|ps|run|version> [<dir>]（目录为位置参数，默认 .server/）
子命令:
  init [<dir>]       生成实例：etc/{server,devices,accounts}.yaml + Caddyfile + run/oxfile.toml
                     + PSK/JWT secret 自举（0600 幂等；compose entrypoint 逻辑归位——双轨同语义）
  start [<dir>] [--no-web]
                     oxmgr apply 拉起进程簇（mediaservo-server + caddy web）；端口竞争=交互接管/
                     非交互退出；--no-web=仅后端（dev 形态）；caddy 不在 PATH → warn 自动降级
  stop [<dir>]       全簇停止（oxmgr stop+delete + 实例 daemon 收敛）
  restart [<dir>] [--no-web]  stop 后重新 apply
  status [<dir>]     进程表 + /ready 探针列（退出码 0 健康 / 1 降级 / 2 未运行或目录非法）
  doctor [<dir>]     环境诊断（oxmgr/caddy PATH、yaml、web dist、announced IP；退出码=失败数）
  logs [server|web|all] [<dir>]  oxmgr logs 转发（支持 -f/--lines 透传）
  startup on|off|status [<dir>]  开机锚点（systemd user unit 拉实例 daemon，全局唯一）
  monit              oxmgr TUI（进程/CPU/RAM/日志流）
  ps                 oxmgr 进程列表
  run                【守护模式】现 mediaservo-server 启动行为原样（--config 等）；
                     无参/未知参数 → 回落本模式——既有 systemd/compose/脚本直启零破坏
  version            版本信息

示例:
  mediaservo-server init /opt/mediaservo && mediaservo-server start /opt/mediaservo
  mediaservo-server --config /etc/mediaservo/server.yaml    守护直启（兼容旧链）
  mediaservo-server -h                            本帮助";

fn print_usage() {
    println!("{}", USAGE.replace("mediaservo-server", &lifecycle::templates::server_product()));
}

/// Entry point — 单二进制双角色派发（T15）。管理面子命令 → lifecycle；
/// `run` → 守护（显式形态）；无参/`--config`/未知 → 守护回落（向后兼容硬门：
/// systemd/compose/docker 直启链行为逐字节不变）。
fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let first = argv.get(1).map(String::as_str);
    if matches!(first, Some("-h") | Some("--help")) {
        print_usage();
        return;
    }
    if let Some(cmd) = first.filter(|c| lifecycle::is_lifecycle_cmd(c)) {
        let code = lifecycle::dispatch(cmd, &mut argv.iter().skip(2).cloned());
        std::process::exit(code);
    }
    // `run` = 显式守护形态：剔除该 token，其余 argv 与直启同构（--config 落回 args[1]）
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

/// 守护模式 = 原 main 本体（panic hook + runtime + run_server）。argv 透传
/// （`--config` 判定从 env::args() 改为显式入参，其余逻辑零改动）。
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
    // 默认：相对二进制路径 bin/../etc/server.yaml（build server 组装时生成）
    // 容器 fallback：/opt/mediaservo/etc/server.yaml（旧路径兜底）
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

    // 相对路径解析: devices/accounts 路径相对于 config 文件所在目录（非 CWD）
    let config_dir = std::path::Path::new(&config_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
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

    // ── G2 设备注册表加载（devices.yaml; 缺省路径与 server.yaml 同目录）────────
    // 文件缺失/解析失败 → 空注册表 + 警告（PSK 路径不受影响，不阻断启动）。
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
    // unified-device-admin: 单一 Arc 实例，signaling（接入鉴权）与 admin（管理写回）共享。
    let device_registry = std::sync::Arc::new(device_registry);

    // ── G3 舱端账号注册表加载（accounts.yaml; 缺省路径与 server.yaml 同目录）────
    // 文件缺失/解析失败 → 空注册表 + 警告（PSK/设备路径不受影响，不阻断启动）。
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
        // I4 review: 文件存在但损坏 → fail-fast（静默空注册表 = 授权强制被静默禁用）。
        Err(e) => {
            panic!("账号注册表 {accounts_path} 损坏，拒绝以授权禁用状态启动: {e}");
        }
    };
    // I2 review: 已知开发占位账号 fail-fast（config/accounts.yaml 仅限本地 dev）—
    // 显式覆盖 env MEDIASERVO_ALLOW_DEV_CREDENTIALS=1（dev compose 已设）豁免。
    if let Err(e) = accounts.check_dev_credentials(
        std::env::var("MEDIASERVO_ALLOW_DEV_CREDENTIALS").as_deref() == Ok("1"),
    ) {
        panic!("{e}");
    }
    let accounts = std::sync::Arc::new(accounts);

    // I2 review: 账号 token 经 admin_jwt_secret 签发、/ws 经 jwt_secret 验签 —
    // 分歧 = 账号认证静默回退 PSK（矩阵绕过），启动期 fail-fast。
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
                // psk-admin-management: config 优先 + env 兜底（死配置修复 — server.yaml psk 正式接入鉴权）
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
        // psk-admin-management: config 优先 + env 兜底（死配置修复 — server.yaml psk 正式接入鉴权）
        let merged_psk = config.psk.clone().or_else(|| std::env::var("MEDIASERVO_PSK").ok());
        srv.psk_state = std::sync::Arc::new(std::sync::RwLock::new(merged_psk));
        srv
    };

    // Build axum router
    let signaling_router = signaling::signaling_router(signaling_server.clone());
    let monitor_router = monitor::monitor_router(signaling_server.clone());

    // ── Admin API state ────────────────────────────────────────────────────
    // 与 SignalingServer 内建频道同源（流上/下线事件由信令路径推送）。
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
        #[cfg(feature = "sfu-mediasoup")]
        sfu_manager: std::sync::Arc::clone(&signaling_server.sfu_manager),
    };

    let admin_router = admin::admin_router(admin_state.clone());
    // G3: 登录端点独立 router（不被 admin auth middleware 拦截 — 它就是发证入口）。
    let login_router = admin::login_router(admin_state.clone());

    let app = axum::Router::new()
        .merge(signaling_router)
        .merge(monitor_router)
        .merge(login_router)
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
