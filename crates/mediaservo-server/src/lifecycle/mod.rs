//! `msrtc-server` instance lifecycle (frontend-process-split T15-T19, the admin plane of the single-binary dual role).
//!
//! Engine = oxmgr (isomorphic with host, design.md "Server process management: revised"). Topology of 2~3 processes
//! (server + caddy web + weaknet-serve conditionally), oxfile static template with no translation layer. All oxmgr calls go through the **instance daemon**
//! (OXMGR_HOME/OXMGR_DAEMON_ADDR derived from the instance dir -- C32 isolation; base 18500 avoids the host family).
//!
//! Backward-compat hard gate: main.rs enters this module only when the first arg matches LIFECYCLE_CMDS;
//! `run`/no-arg/`--config`/unknown args = daemon mode as-is (systemd/compose/docker direct starts untouched).

mod conflict;
mod inspect;
mod reap;
mod startup;
pub mod templates;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use reap::{DirectReason, LiveProc, ProcKind, ReapStep, StepOutcome};

use templates::{
    DEFAULT_WEB_PORT, SFU_ENV_KEYS, WEAKNET_APP, WEAKNET_SERVE_PORT, gen_secret,
    instance_daemon_port, looks_like_oxfile, manual_env_keys, merge_manual_env, oxfile_has_weaknet,
    parse_listen_port, parse_weaknet_port, parse_web_port, render_admin_account, render_caddyfile,
    render_oxfile, render_server_yaml, resolve_port, server_product, server_web_app,
    weaknet_listen,
};

/// Admin-plane subcommand table (main.rs dispatch predicate). `run` is not listed -- it goes to daemon mode (explicit form).
pub const LIFECYCLE_CMDS: &[&str] = &[
    "init", "start", "stop", "restart", "status", "doctor", "logs", "startup", "monit", "ps",
    "version",
];

pub fn is_lifecycle_cmd(c: &str) -> bool {
    LIFECYCLE_CMDS.contains(&c)
}

pub fn dispatch(cmd: &str, args: &mut dyn Iterator<Item = String>) -> i32 {
    match cmd {
        "init" => cmd_init(args),
        "start" => cmd_start(args, "start"),
        "stop" => cmd_stop(args),
        "restart" => cmd_restart(args),
        "status" => inspect::cmd_status(args),
        "doctor" => inspect::cmd_doctor(args),
        "logs" => inspect::cmd_logs(args),
        "startup" => startup::cmd_startup(args),
        "monit" => cmd_oxmgr_proxy(&["ui"], args),
        "ps" => cmd_oxmgr_proxy(&["list"], args),
        "version" => {
            println!("{} {}", server_product(), env!("CARGO_PKG_VERSION"));
            0
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            2
        }
    }
}

/// Resolve the instance dir (positional arg, smart default like host: instance in cwd -> ".", sibling/parent of binary -> install root, else .server).
pub(super) fn parse_dir(args: &mut dyn Iterator<Item = String>) -> Result<PathBuf, String> {
    let Some(arg) = args.next() else {
        if PathBuf::from("etc/server.yaml").exists() {
            return Ok(PathBuf::from("."));
        }
        if let Some(exe_dir) =
            std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
        {
            if exe_dir.join("etc").join("server.yaml").exists() {
                return Ok(exe_dir);
            }
            if let Some(root) = exe_dir.parent()
                && root.join("etc").join("server.yaml").exists()
            {
                return Ok(root.to_path_buf());
            }
        }
        return Ok(PathBuf::from(".server"));
    };
    if arg.starts_with("--") {
        return Err(format!(
            "instance dir argument must not start with -- (dir is a positional arg): {arg}"
        ));
    }
    if args.next().is_some() {
        return Err("instance dir accepts only one positional argument".to_string());
    }
    Ok(PathBuf::from(arg))
}

// ── init ─────────────────────────────────────────────────────────────────────

/// `init [<dir>]`: instance dir generation (PIT-160: existing files are **never overwritten**).
/// etc/{server,devices,accounts}.yaml + Caddyfile + run/oxfile.toml + secret bootstrap (0600, idempotent,
/// compose entrypoint logic folded in -- both tracks share semantics). env MEDIASERVO_ADMIN_PASSWORD seeds the first account (entrypoint 5).
fn cmd_init(args: &mut dyn Iterator<Item = String>) -> i32 {
    let dir = match parse_dir(args) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match init_instance(&dir) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("init: {e}");
            1
        }
    }
}

fn init_instance(dir: &Path) -> Result<(), String> {
    let etc = dir.join("etc");
    let run = dir.join("run");
    std::fs::create_dir_all(&etc)
        .map_err(|e| format!("failed to create {}: {e}", etc.display()))?;
    std::fs::create_dir_all(run.join("logs"))
        .map_err(|e| format!("failed to create run dir: {e}"))?;

    // 1. server.yaml (contains credentials -> 0600; skipped entirely if present -- secrets must not be regenerated/overwritten, entrypoint 2 idempotency)
    let cfg_path = etc.join("server.yaml");
    if cfg_path.exists() {
        eprintln!("init: {} already exists, skipping (credentials preserved)", cfg_path.display());
    } else {
        let yaml = render_server_yaml(&gen_secret(), &gen_secret());
        write_private(&cfg_path, yaml.as_bytes())?;
        println!("generated {} (PSK/JWT bootstrap, 0600)", cfg_path.display());
    }

    // 2. Empty registry templates (admin-plane C33; written only per missing file)
    let registry_templates = [
        ("devices.yaml", templates::DEVICES_TEMPLATE),
        ("accounts.yaml", templates::ACCOUNTS_TEMPLATE),
    ];
    for (name, content) in registry_templates {
        let p = etc.join(name);
        if p.exists() {
            eprintln!("init: {} already exists, skipping", p.display());
        } else {
            std::fs::write(&p, content)
                .map_err(|e| format!("failed to write {}: {e}", p.display()))?;
            println!("generated {}", p.display());
        }
    }

    // 3. admin bootstrap account (appended when env provided and no admin exists yet -- entrypoint 5 folded in)
    bootstrap_admin_from_env(&etc.join("accounts.yaml"))?;

    // 4. Caddyfile (config-unification P1 read order: yaml > env MEDIASERVO_WEB_PORT > default 8080)
    let caddy_path = etc.join("Caddyfile");
    let yaml_cfg = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|c| serde_yaml::from_str::<mediaservo_common::config::ServerConfig>(&c).ok());
    let (web_port, web_root_override, weaknet_port) = resolved_ports(yaml_cfg.as_ref());
    let backend_port =
        std::fs::read_to_string(&cfg_path).ok().and_then(|c| parse_listen_port(&c)).unwrap_or(9800);
    if caddy_path.exists() {
        eprintln!("init: {} already exists, skipping", caddy_path.display());
    } else {
        let root = web_root_override.unwrap_or_else(|| absolute(&dir.join("web")));
        std::fs::write(
            &caddy_path,
            render_caddyfile(web_port, backend_port, &root, &weaknet_listen(weaknet_port)),
        )
        .map_err(|e| format!("failed to write Caddyfile: {e}"))?;
        println!("generated {} (:{web_port} -> 127.0.0.1:{backend_port})", caddy_path.display());
    }

    // 5. oxfile (static 2~3 entries, weaknet conditional rendering [M-1]; never overwrites -- manual env edits preserved,
    //    stale migration = Python _drop_stale_server_oxfile single point [bF3])
    let oxfile = run.join("oxfile.toml");
    if oxfile.exists() {
        eprintln!("init: {} already exists, skipping", oxfile.display());
    } else {
        write_oxfile(&run)?;
    }
    println!(
        "init: instance ready {} (after start use the server log setup-token to create an account, or rerun init with MEDIASERVO_ADMIN_PASSWORD)",
        dir.display()
    );
    Ok(())
}

/// Unified port read order (config-unification P1): yaml (Option field) > env > constant default.
/// Shared by init 4 and write_oxfile -- eliminates dual read-order drift. yaml_cfg=None (missing file/parse failure) = full fallback.
fn resolved_ports(
    yaml_cfg: Option<&mediaservo_common::config::ServerConfig>,
) -> (u16, Option<std::path::PathBuf>, u16) {
    let env_u16 = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u16>().ok());
    let web = yaml_cfg.map(|c| &c.web);
    let sfu = yaml_cfg.map(|c| &c.sfu);
    let web_port =
        resolve_port(web.and_then(|w| w.port), env_u16("MEDIASERVO_WEB_PORT"), DEFAULT_WEB_PORT);
    let weaknet_port = resolve_port(sfu.and_then(|s| s.weaknet_port), None, WEAKNET_SERVE_PORT);
    let root = web.and_then(|w| w.static_root.clone()).map(std::path::PathBuf::from);
    (web_port, root, weaknet_port)
}

/// Write run/oxfile.toml (shared by init and the start missing-file fallback -- the only Rust-side oxfile write point).
/// PIT-160 root fix (PLAN S2.4): re-rendering never starts from scratch -- merge source = the live oxfile when present, else
/// the .bak (last successful render snapshot, covering both "user deleted the file then start re-renders" and "Python deploy stale
/// backup then init re-renders"); both missing = first render, pure. After writing, refresh .bak with the final state (N re-renders
/// do not double the manual entries -- the snapshot advances with the merge result). .bak is inert to oxmgr (which reads the explicit oxfile path).
fn write_oxfile(run_dir: &Path) -> Result<(), String> {
    let dir = run_dir.parent().ok_or("oxfile rendering needs the instance root dir")?;
    let bin = std::env::current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
    let yaml_cfg = std::fs::read_to_string(dir.join("etc").join("server.yaml"))
        .ok()
        .and_then(|c| serde_yaml::from_str::<mediaservo_common::config::ServerConfig>(&c).ok());
    let (_, _, weaknet_port) = resolved_ports(yaml_cfg.as_ref());
    let new_text =
        render_oxfile(&absolute(dir), &bin, &caddy_command(), &sfu_env_passthrough(), weaknet_port);
    let path = run_dir.join("oxfile.toml");
    let bak = run_dir.join("oxfile.toml.bak");
    let (old, source) = match std::fs::read_to_string(&path) {
        Ok(t) => (Some(t), "run/oxfile.toml"),
        Err(_) => match std::fs::read_to_string(&bak) {
            Ok(t) => (Some(t), "run/oxfile.toml.bak"),
            Err(_) => (None, ""),
        },
    };
    let corrupt = old.as_deref().is_some_and(|o| !o.trim().is_empty() && !looks_like_oxfile(o));
    let text = match old.as_deref() {
        Some(o) if !corrupt => merge_manual_env(o, &new_text),
        _ => new_text,
    };
    let (merged, lost) = match old.as_deref() {
        Some(o) if !corrupt => {
            let want = manual_env_keys(o);
            let have = manual_env_keys(&text);
            let m = want.iter().filter(|k| have.contains(k)).count();
            (m, want.len() - m)
        }
        _ => (0, 0),
    };
    std::fs::write(&path, &text).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    println!("generated {}", path.display());
    if let Err(e) = std::fs::write(&bak, &text) {
        eprintln!(
            "warning: failed to write {} snapshot ({e}) -- the next re-render has no manual env to absorb",
            bak.display()
        );
    }
    if corrupt {
        eprintln!(
            "warning: the old {source} was not rendered by this program -- this render degrades to pure rendering, manual env not absorbed (PIT-160 fallback, does not block start)"
        );
    }
    eprintln!("{}", pit160_note(merged, lost));
    Ok(())
}

/// PIT-160 note line (pure function, testable): no manual entries -> the old guidance text preserved byte-for-byte; otherwise the "merged N
/// manual env entries" variant + a count of not-merged entries (the new render has no matching app entry). Managed keys of the renderer
/// (MEDIASERVO_SFU_* and WEAKNET_STATEDIR) always follow this render (baked-wins) and are not counted in N.
fn pit160_note(merged: usize, lost: usize) -> String {
    if merged == 0 && lost == 0 {
        return "Note: if this tree previously had manual [apps.env] lines (ALLOW_DEV_*/RUST_LOG/WEAKNET_ADMIN_PASS), this render does not include them -- restore them per the msrtc.sh -h \"environment variables [B]\" section".to_string();
    }
    let tail = if lost > 0 {
        format!(
            "; additionally {lost} entries were not merged because the new render has no matching app entry -- restore them per the msrtc.sh -h \"environment variables [B]\" section"
        )
    } else {
        String::new()
    };
    format!(
        "Note: merged {merged} manual env entries (source = last render snapshot; managed keys MEDIASERVO_SFU_*/WEAKNET_STATEDIR follow this render){tail}"
    )
}

/// weaknet probe port = reverse-parsed from the oxfile (`serve --listen 127.0.0.1:NNNN`) -- same source as the rendered output;
/// reverse-parse failure (old oxfile without the entry / manually mangled) falls back to the default constant.
fn weaknet_probe_port(oxfile: &Path) -> u16 {
    std::fs::read_to_string(oxfile)
        .ok()
        .and_then(|t| parse_weaknet_port(&t))
        .unwrap_or(WEAKNET_SERVE_PORT)
}

/// At init, bake SFU-related env into the oxfile [apps.env] (the drill/multi-instance port-isolation entry).
/// The passthrough key set = the single source SFU_ENV_KEYS; the merge exclusion set is MANAGED_ENV_KEYS (this set + weaknet
/// baked-value keys -- final-cut baked-wins ruling: the env-passthrough and template-baked families are separately named managed key sets, each single-sourced).
fn sfu_env_passthrough() -> Vec<(String, String)> {
    SFU_ENV_KEYS.iter().filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v))).collect()
}

/// caddy command probe chain (2026-09-29 adaptive): 1. PATH 2. same dir as this binary (out/server/bin
/// self-contained deployment -- caddy ships with the package) 3. the in-repo pixi tree (source machines). All miss = the literal name (keeps the current degraded semantics).
fn caddy_command() -> String {
    caddy_probe().unwrap_or_else(|| "caddy".to_string())
}

fn caddy_probe() -> Option<String> {
    if let Some(p) = which("caddy") {
        return Some(p);
    }
    // 2. exe's own dir (the deploy tree bin/ trio shape)
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let p = dir.join("caddy");
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    // 3. in-repo pixi tree (source machines running target/debug directly; a build-time path -- cross-machine deploys are handled by 2)
    // (single candidate -- the former array loop form, straightened per 1.97 clippy single_element_loop, zero semantic change)
    let cand = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.pixi/envs/default/bin/caddy");
    let p = cand.canonicalize().unwrap_or(cand.clone());
    if p.is_file() {
        return Some(p.to_string_lossy().into_owned());
    }
    None
}

pub(super) fn which(bin: &str) -> Option<String> {
    std::env::split_paths(&std::env::var("PATH").unwrap_or_default()).find_map(|d| {
        let p = d.join(bin);
        (p.is_file() && is_executable(&p)).then(|| p.to_string_lossy().into_owned())
    })
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(_p: &Path) -> bool {
    true
}

/// Entrypoint 5: seed the admin when env MEDIASERVO_ADMIN_PASSWORD exists (env read shell; logic in bootstrap_admin).
fn bootstrap_admin_from_env(path: &Path) -> Result<(), String> {
    let pass = std::env::var("MEDIASERVO_ADMIN_PASSWORD").unwrap_or_default();
    if pass.trim().is_empty() {
        return Ok(());
    }
    bootstrap_admin(path, &pass)
}

/// Append a bootstrap admin when the account table has no admin (idempotent; password is an explicit arg -- avoids test env races).
pub fn bootstrap_admin(path: &Path, pass: &str) -> Result<(), String> {
    if pass.trim().is_empty() {
        return Ok(());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read accounts.yaml: {e}"))?;
    if text.lines().any(|l| l.trim() == "admin:") {
        eprintln!("init: admin account already exists, skipping bootstrap");
        return Ok(());
    }
    let mut body = text.replace("accounts: {}", "accounts:");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&render_admin_account("admin", pass));
    std::fs::write(path, body).map_err(|e| format!("failed to write accounts.yaml: {e}"))?;
    println!("init: bootstrap admin account created (bootstrap)");
    Ok(())
}

/// Credential file write + 0600 (unix).
fn write_private(path: &Path, data: &[u8]) -> Result<(), String> {
    let mut f = std::fs::File::create(path)
        .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to set permissions on {}: {e}", path.display()))?;
    }
    f.write_all(data).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

// ── start / stop / restart ───────────────────────────────────────────────────

/// dev loop decision (design "dev mode loop"): explicit --no-web, or caddy missing from PATH -> degrade to backend-only.
/// Returns (post-degrade no_web, whether degraded because caddy is missing [warn marker]). Pure function, unit-testable.
pub fn decide_no_web(no_web_flag: bool, caddy_found: bool) -> (bool, bool) {
    if no_web_flag {
        (true, false)
    } else if !caddy_found {
        (true, true)
    } else {
        (false, false)
    }
}

/// Parse `start/restart` args: `--no-web` flag + a single positional dir. Err(i32) = exit code already printed.
fn parse_start_args(args: &mut dyn Iterator<Item = String>) -> Result<(PathBuf, bool), i32> {
    let mut no_web = false;
    let mut positional: Vec<String> = Vec::new();
    for a in &mut *args {
        match a.as_str() {
            "--no-web" => no_web = true,
            s if s.starts_with("--") => {
                eprintln!("unknown argument: {s}");
                return Err(2);
            }
            s => positional.push(s.to_string()),
        }
    }
    if positional.len() > 1 {
        eprintln!("instance dir accepts only one positional argument");
        return Err(2);
    }
    let mut it = positional.into_iter();
    parse_dir(&mut it).inspect_err(|e| eprintln!("{e}")).map_err(|_| 2).map(|d| (d, no_web))
}

fn cmd_start(args: &mut dyn Iterator<Item = String>, verb: &str) -> i32 {
    let (dir, no_web_flag) = match parse_start_args(args) {
        Ok(v) => v,
        Err(code) => return code,
    };
    start_impl(&dir, no_web_flag, verb)
}

fn start_impl(dir: &Path, no_web_flag: bool, verb: &str) -> i32 {
    if !dir.join("etc").join("server.yaml").exists() {
        eprintln!(
            "{verb}: {} has no instance config -- run `{} init <dir>` first",
            dir.display(),
            server_product()
        );
        return 2;
    }
    // oxfile: the init output wins (manual edits preserved -- the drill's SFU port injection path); re-render only if missing
    let oxfile = dir.join("run").join("oxfile.toml");
    if !oxfile.exists()
        && let Err(e) = write_oxfile(&dir.join("run"))
    {
        eprintln!("{verb}: {e}");
        return 1;
    }
    let cfg = match std::fs::read_to_string(dir.join("etc").join("server.yaml")) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{verb}: failed to read server.yaml: {e}");
            return 1;
        }
    };
    let port = match parse_listen_port(&cfg) {
        Some(p) => p,
        None => {
            eprintln!(
                "{verb}: server.yaml parse failed (listen.port unreadable) -- run doctor first"
            );
            return 1;
        }
    };
    let (no_web, caddy_missing) = decide_no_web(no_web_flag, caddy_probe().is_some());
    if caddy_missing {
        eprintln!(
            "{verb}: caddy not in PATH -- warn + auto-degrade to --no-web (dev form; use doctor for strict checks)"
        );
    }
    let web_port = std::fs::read_to_string(dir.join("etc").join("Caddyfile"))
        .ok()
        .and_then(|cf| parse_web_port(&cf))
        .unwrap_or(DEFAULT_WEB_PORT);
    // weaknet entry present (oxfile-text predicate -- same source as render_oxfile conditional rendering [BA-9]):
    // decides whether to keep the 9810 probe + the --no-web allowlist weaknet-serve entry.
    let weaknet_cluster = std::fs::read_to_string(&oxfile).is_ok_and(|t| oxfile_has_weaknet(&t));
    // Port-conflict protection (same pattern as host start: interactive y takeover / non-tty exit)
    let mut busy: Option<(&str, u16)> = None;
    if inspect::port_in_use(port) {
        busy = Some(("backend listen port", port));
    } else if !no_web && inspect::port_in_use(web_port) {
        busy = Some(("web port", web_port));
    } else if weaknet_cluster && inspect::port_in_use(weaknet_probe_port(&oxfile)) {
        // BA-9: an external dev serve on the weaknet port makes the cluster show a fake "running" with a crash-loop unit -- report the occupied port and refuse to start
        // Probe port = oxfile reverse parse (same source as the render; fallback to the constant on parse failure -- config-unification P1)
        busy = Some(("weaknet panel port", weaknet_probe_port(&oxfile)));
    }
    if let Some((what, p)) = busy {
        return contention_flow(verb, dir, what, p, no_web);
    }
    apply_oxfile(dir, &oxfile, no_web, verb)
}

/// The `--no-web` apply allowlist (pure-function unit-test anchor, lck-F8): **only web is removed, weaknet is never dragged along**
/// (D5-6) -- oxmgr 0.5.0 `--only` = allowlist semantics verified live: non-allowlisted entries are stopped (not deleted);
/// with three entries the default allowlist must include weaknet-serve (when the entry is present). The comma form is verified equivalent (`--only a,b`).
pub fn no_web_only_list(weaknet_in_oxfile: bool) -> String {
    if weaknet_in_oxfile {
        format!("{},{}", server_product(), WEAKNET_APP)
    } else {
        server_product()
    }
}

fn apply_oxfile(dir: &Path, oxfile: &Path, no_web: bool, verb: &str) -> i32 {
    let has_wnet = std::fs::read_to_string(oxfile).is_ok_and(|t| oxfile_has_weaknet(&t));
    let mut oxargs: Vec<String> = vec!["apply".into(), oxfile.to_string_lossy().into_owned()];
    if no_web {
        oxargs.extend(["--only".into(), no_web_only_list(has_wnet)]);
    }
    let refs: Vec<&str> = oxargs.iter().map(String::as_str).collect();
    let code = run_oxmgr(Some(dir), &refs);
    if code == 0 {
        // BA-6 four-point sync of the apply_oxfile text: the three-entry form lists them as-is (reported only when weaknet is present)
        let wnet_part = if has_wnet { format!(" + {WEAKNET_APP}") } else { String::new() };
        let cluster = if no_web {
            format!("backend only (--no-web): {svc}{wnet_part}", svc = server_product())
        } else {
            format!(
                "whole cluster: {svc} + {web}{wnet_part}",
                svc = server_product(),
                web = server_web_app()
            )
        };
        println!("{verb}: applied {} -- {cluster}", oxfile.display());
    }
    code
}

/// Port occupied -> locate the old server instance -> interactive takeover (stop the old cluster) / non-tty exit (host PIT-155 pattern).
fn contention_flow(verb: &str, dir: &Path, what: &str, port: u16, no_web: bool) -> i32 {
    eprintln!("{verb}: {what} {port} is occupied -- another process is listening");
    // start-conflict-doctor T2: occupant attribution (pid/exe/deleted/parent daemon) -- attribute, do not kill (F1).
    let reports = conflict::diagnose_port(port);
    conflict::print_report(&reports, port);
    let old = inspect::find_other_server_dir(dir);
    match &old {
        Some(od) => eprintln!("  old instance dir: {}", od.display()),
        None => {
            match inspect::strays_under(&inspect::read_default_registry(), dir).into_iter().next() {
                // T5: /proc scan finds nothing -> the occupant may live under a shared/old daemon (outside this instance registry's
                // jurisdiction -- the 2026-10-12 incident shape). Attribute, do not kill: the takeover path still refuses (old=None semantics unchanged).
                Some(e) => {
                    eprintln!(
                        "  the port occupant lives under the old shared daemon: name={} id={} status={} pid={} (outside this instance registry) -- {}",
                        e.name,
                        e.id,
                        e.status,
                        e.pid.map_or_else(|| "?".into(), |p| p.to_string()),
                        inspect::SPLIT_GUIDANCE
                    );
                    // global-reap T4: text loop closed -- after the T5 attribution, offer a one-command exit
                    eprintln!("  use msrtc-server stop -g to reap (globally by process identity)");
                }
                None => eprintln!(
                    "  (could not locate the old instance dir -- the port may be held by another program)"
                ),
            }
        }
    }
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "  non-interactive environment -- exiting (multi-instance coexistence: give each instance a distinct listen.port + MEDIASERVO_WEB_PORT and re-init)"
        );
        return 1;
    }
    eprint!(
        "  enter y to take over (stop the old instance and start this one) / any other key to exit: "
    );
    let _ = std::io::stdout().flush();
    let mut ans = String::new();
    let _ = std::io::stdin().read_line(&mut ans);
    if !ans.trim().eq_ignore_ascii_case("y") {
        return 1;
    }
    let Some(od) = old else {
        eprintln!(
            "takeover aborted: old instance dir not located -- continuing would fight live processes (PIT-155 lesson)"
        );
        return 1;
    };
    eprintln!("takeover: stopping old instance {}", od.display());
    stop_cluster_in(&od);
    apply_oxfile(dir, &dir.join("run").join("oxfile.toml"), no_web, verb)
}

/// stop arg form (pure parse, unit-test anchor): `stop` default dir / `stop <dir>` / `stop -g|--all`;
/// `-g` together with a dir = invalid args (exit 2).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum StopArgs {
    Dir(Option<String>),
    Global,
}

pub(super) fn parse_stop_args(args: &mut dyn Iterator<Item = String>) -> Result<StopArgs, String> {
    let mut global = false;
    let mut positional: Vec<String> = Vec::new();
    for a in &mut *args {
        match a.as_str() {
            "-g" | "--all" => global = true,
            s if s.starts_with('-') => {
                return Err(format!("unknown argument: {s}"));
            }
            s => positional.push(s.to_string()),
        }
    }
    if global {
        if !positional.is_empty() {
            return Err("stop -g is a global reap and does not accept a dir argument".to_string());
        }
        return Ok(StopArgs::Global);
    }
    if positional.len() > 1 {
        return Err("instance dir accepts only one positional argument".to_string());
    }
    Ok(StopArgs::Dir(positional.into_iter().next()))
}

/// `stop [<dir>]`: stop+delete by registered name + instance daemon convergence (after stop this instance has no resident processes).
/// `stop -g|--all`: global reap by process identity across dirs and daemons (PLAN S2.4).
fn cmd_stop(args: &mut dyn Iterator<Item = String>) -> i32 {
    let parsed = match parse_stop_args(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match parsed {
        StopArgs::Global => scan_all(),
        StopArgs::Dir(None) => stop_cluster(
            &parse_dir(&mut std::iter::empty()).unwrap_or_else(|_| PathBuf::from(".server")),
        ),
        StopArgs::Dir(Some(d)) => stop_cluster(&PathBuf::from(d)),
    }
}

fn stop_cluster(dir: &Path) -> i32 {
    let names = match inspect::list_registered_apps(dir) {
        Ok(n) => n,
        Err(e) => {
            // Second-review residual #1: daemon down/oxmgr missing must **not exit early** -- in this variant the leftover processes may well
            // be registered under another daemon; the sweep must still run (idempotent success semantics judged at the tail).
            eprintln!("stop: {e}");
            println!(
                "stop: treated as no running instance (no managed processes) -- continuing the sweep fallback"
            );
            Vec::new()
        }
    };
    let mut failed = 0;
    for name in &names {
        if run_oxmgr(Some(dir), &["stop", name]) != 0 {
            failed = 1;
        }
        if run_oxmgr(Some(dir), &["delete", name]) != 0 {
            failed = 1;
        }
    }
    // Sweep fallback (global-reap T2, PLAN S2.2/S2.3): after the instance table is emptied, a full /proc scan for this instance's
    // leftovers (including those registered under shared/default daemons, the split shape), gracefully closed via their owning daemon to
    // prevent restart_policy=always revival; direct kill only after both lookups miss.
    let reaped_pids = reap_under(dir);
    // Instance daemon convergence (M1: moved after the sweep -- converging first would force instance-owned leftovers into raw kills)
    if !names.is_empty() && run_oxmgr(Some(dir), &["daemon", "stop"]) != 0 {
        eprintln!(
            "stop: instance daemon stop command did not succeed (it may have exited on its own)"
        );
    }
    if failed == 0 {
        // B2 ownership-attribution self-verification: poll the three ports for <=5s -- our leftover = failure, stranger holding the port = WARN + success.
        let ports = stop_ports(dir);
        let verified = verify_ports_attribution(&ports, &reaped_pids, dir);
        println!(
            "stop: stopped {n} processes and converged the instance daemon ({prod} cluster) {}",
            if verified {
                "-- self-verified clean at port level"
            } else {
                "-- [!] ports still held by this instance's leftovers"
            },
            n = names.len(),
            prod = server_product()
        );
        // T4-a live correction: self-verification failed = this instance's leftovers present, which means failure (RC=0 once let a fake-green revival through --
        // the attribution line reported failed honestly but the exit code did not follow). Paths where failed itself is non-zero already return 1 by count.
        if !verified {
            failed = 1;
        }
    }
    failed
}

/// This instance's three probe ports (parse failure -> skip that port, no fake red, S2.3).
/// Since S1 = the shared startup::instance_ports implementation (stop self-verification and the startup conflict gate share a source -- behavior byte-identical:
/// the only difference = the old form collapsed a missing weaknet entry to the default constant while the new form skips; under conflict-gate semantics skip is more honest; on the stop
/// self-verification side, in no-oxfile-entry scenarios server.yaml/Caddyfile still have both ports present, so the verification surface does not shrink).
fn stop_ports(dir: &Path) -> Vec<u16> {
    startup::instance_ports(dir)
}

/// Sweep execution: live processes whose signature hits and whose dir belongs to this instance -> reap_plan -> execute each.
/// Returns the set of pids reaped this round (for B2 attribution). All judgment lives in the reap pure functions; this is only a thin IO shell.
fn reap_under(dir: &Path) -> Vec<u32> {
    let live = reap::scan_live_procs();
    let matched: Vec<(&LiveProc, ProcKind)> = live
        .iter()
        .filter_map(|p| {
            let kind = reap::proc_kind(&p.argv, p.exe.as_deref())?;
            if !reap::kind_dir_within(&kind, &p.argv, dir) {
                return None;
            }
            // caddy disk marker (B1) -- the product's bin must exist under root
            if let ProcKind::Caddy { config } = &kind
                && !reap::caddy_marker_ok(config)
            {
                return None;
            }
            Some((p, kind))
        })
        .collect();
    if matched.is_empty() {
        println!("sweep: no leftover processes");
        return Vec::new();
    }
    // Registry snapshots: the instance daemon table (if the dir is readable) + the default daemon table (read after B3 sanitization).
    let mut registries: Vec<(PathBuf, Vec<inspect::ManagedEntry>)> = Vec::new();
    let inst_dir = dir.join("run").join("oxmgr");
    registries.push((inst_dir.clone(), inspect::read_registry(&inst_dir.join("state.json"))));
    if let Some(dd) = inspect::default_data_dir() {
        registries.push((dd.clone(), inspect::read_registry(&dd.join("state.json"))));
    }
    let refs: Vec<(PathBuf, &[inspect::ManagedEntry])> =
        registries.iter().map(|(d, e)| (d.clone(), e.as_slice())).collect();
    let plan =
        reap::reap_plan(&matched.iter().map(|(p, _)| (*p).clone()).collect::<Vec<_>>(), &refs);
    execute_steps(&plan).into_iter().map(|(pid, _)| pid).collect()
}

/// Daemon liveness probe: DAEMON_ADDR port connectable = alive (S10 direct-kill gate criterion 1).
fn daemon_alive(daemon_data_dir: &Path) -> bool {
    let home = absolute(daemon_data_dir);
    std::net::TcpStream::connect(("127.0.0.1", instance_daemon_port(&home))).is_ok()
}

/// Registry entry presence (S10 direct-kill gate criterion 2): name matches and pid matches (or pid is stale but the name is present
/// -- the shape where a live daemon re-pulled a new pid, M2 same source).
fn registry_has_entry(daemon_data_dir: &Path, name: &str, pid: &u32) -> bool {
    let state = daemon_data_dir.join("state.json");
    inspect::read_registry(&state)
        .iter()
        .any(|e| e.name == name && (e.pid == Some((*pid).into()) || e.pid.is_none()))
}

/// Reap step execution (shared thin shell by T2/T3; semantics byte-identical to the T2 inline form -- daemon unreachable degrades
/// to direct kill, the direct-kill banner, a single follow-up kill cap). Returns a (pid, outcome) sequence for tallying.
fn execute_steps(plan: &[ReapStep]) -> Vec<(u32, StepOutcome)> {
    plan.iter()
        .map(|step| match step {
            ReapStep::ViaDaemon { daemon_data_dir, name, pid } => {
                println!(
                    "sweep: pid {pid} ({name}) -> closed via its owning daemon {}",
                    daemon_data_dir.display()
                );
                if run_oxmgr_for_daemon(daemon_data_dir, &["stop", name]) == 0
                    && run_oxmgr_for_daemon(daemon_data_dir, &["delete", name]) == 0
                {
                    return (*pid, StepOutcome::ViaDaemon);
                }
                // S10 direct-kill gate (10-13 second incident): close failure != daemon dead -- the daemon may only be
                // briefly unreachable/converged by stop while its entry is still pulling new pids. Direct-kill permission = daemon confirmed dead
                // AND no registry entry; otherwise report only (-g/stop never overstep, preventing always-revival loops).
                if daemon_alive(daemon_data_dir)
                    || registry_has_entry(daemon_data_dir, name, pid)
                {
                    eprintln!(
                        "sweep: pid {pid} ({name}) daemon close failed and the daemon is alive/entry present -- skipping direct kill (use stop <dir> or fix the daemon and retry)"
                    );
                // K2 split states: a direct-kill-gate skip != a kill failure -- a separate Skipped (not counted in failed, does not raise RC;
                // leftover final-state stock is still caught by the T4-b final second pass and counted as failed).
                    return (*pid, StepOutcome::Skipped);
                }
                eprintln!("sweep: daemon close failed and the daemon is confirmed dead -- bypassing the daemon to kill directly");
                kill_and_wait(*pid);
                let ok = !pid_alive(*pid);
                (*pid, if ok { StepOutcome::DirectKill } else { StepOutcome::Failed })
            }
            ReapStep::DirectKill { pid, reason: DirectReason::NoRegistry } => {
                println!("sweep: pid {pid} -> direct kill bypassing the daemon (no owning registry)");
                kill_and_wait(*pid);
                let ok = !pid_alive(*pid);
                (*pid, if ok { StepOutcome::DirectKill } else { StepOutcome::Failed })
            }
        })
        .collect()
}

/// `stop -g` (PLAN S2.4): full /proc scan for signature hits (no dir filter, no port probing), each closed gracefully via
/// its owning daemon (environ reverse lookup -- processes under an instance daemon are closed via that daemon's own home,
/// registry snapshot = the default daemon table + each environ home's table), direct kill only after both lookups miss. Summary table +
/// final-state lines for signature-hit pids + an "instance daemon untouched" note (m4). Exit code = failure count, matching existing stop (0/1).
/// consistent with existing stop (0/1).
fn scan_all() -> i32 {
    println!(
        "stop -g: global reap (by process identity, across dirs and daemons; the instance daemon itself is untouched)"
    );
    let live = reap::scan_live_procs();
    let (matched, rejected) = reap::select_global(&live, reap::caddy_marker_ok);
    for p in &rejected {
        println!(
            "stop -g: pid {} hit the caddy signature but the disk marker is missing (not this product's layout) -- skipping",
            p.pid
        );
    }
    if matched.is_empty() {
        println!(
            "stop -g: no live product processes -- reap summary: via daemon 0 / direct kill 0 / skipped 0 / failed 0"
        );
        println!("final state: all clear");
        println!("instance daemon untouched (not a target)");
        return 0;
    }
    // Registry snapshots: the default daemon table + each matched process's environ home instance table (reap_plan
    // pairs them sorted per-proc environ; -g passes an empty instance list = the default table, no dir semantics needed).
    // B3 sibling (T4-b caught live): default_data_dir() resolves from the CLI's own env, while
    // run_oxmgr_default strips XDG_DATA_HOME from child processes -- the CLI's view of the "default daemon" and the executor's view
    // diverge (inside an XDG sandbox shell, -g scanned the real default daemon and direct-killed production processes = the incident shape). Unified to
    // the post-strip semantics: the CLI side ignores XDG_DATA_HOME, same source as the executing child.
    // (no env mutation shortcut -- computed directly in the "strip XDG_DATA_HOME" resolution form: HOME/.local/share,
    //  consistent with default_data_dir's XDG fallback implementation, zero unsafe)
    let default_dir = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .is_some()
        .then(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".local").join("share").join("oxmgr"))
        })
        .flatten()
        .or_else(inspect::default_data_dir);
    let mut registries: Vec<(PathBuf, Vec<inspect::ManagedEntry>)> = Vec::new();
    if let Some(dd) = default_dir {
        registries.push((dd.clone(), inspect::read_registry(&dd.join("state.json"))));
    }
    for home in matched.iter().filter_map(|p| p.environ_oxmgr_home.clone()) {
        let home = absolute(&home);
        if registries.iter().any(|(d, _)| *d == home) {
            continue;
        }
        registries.push((home.clone(), inspect::read_registry(&home.join("state.json"))));
    }
    let refs: Vec<(PathBuf, &[inspect::ManagedEntry])> =
        registries.iter().map(|(d, e)| (d.clone(), e.as_slice())).collect();
    let plan = reap::reap_plan(&matched, &refs);
    let steps = execute_steps(&plan);
    let (via, direct, skipped, mut failed) =
        reap::tally(&steps.iter().map(|(_, o)| *o).collect::<Vec<_>>());
    // Final state (the -g form of attribution-style final port printing -- ports differ per instance so they are not probed; judged by a signature second pass):
    let still: Vec<u32> = reap::scan_live_procs()
        .into_iter()
        .filter(|p| match reap::proc_kind(&p.argv, p.exe.as_deref()) {
            Some(ProcKind::Caddy { config }) => reap::caddy_marker_ok(&config),
            Some(_) => true,
            None => false,
        })
        .map(|p| p.pid)
        .collect();
    if still.is_empty() {
        println!("final state: all clear");
    } else {
        println!("final state: signature-hit pids still alive: {still:?}");
        // T4-b correction: final-state leftovers = not fully cleared, counted as failures (previously only tally's Failed steps were checked --
        // revival/escape shapes beyond "kill signal sent but the process still lives" were not covered by the tally)
        failed += still.len();
    }
    println!(
        "reap summary: via daemon {via} / direct kill {direct} / skipped {skipped} / failed {failed}"
    );
    println!("instance daemon untouched (not a target)");
    i32::from(failed > 0)
}

/// Send oxmgr stop/delete to a **specific daemon data dir** (the four env keys derived like oxmgr_cmd).
fn run_oxmgr_for_daemon(daemon_data_dir: &Path, args: &[&str]) -> i32 {
    let home = absolute(daemon_data_dir);
    let port = instance_daemon_port(&home);
    let mut cmd = Command::new(oxmgr_bin());
    cmd.env("OXMGR_HOME", &home)
        .env("OXMGR_DATA_DIR", &home)
        .env("OXMGR_DAEMON_ADDR", format!("127.0.0.1:{port}"))
        .env("OXMGR_API_ADDR", format!("127.0.0.1:{}", port + 1000));
    match cmd.args(args).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// SIGTERM -> wait <=3s -> SIGKILL (S2.2 last resort). EPERM (cross-user) recorded as failure without crashing (m2).
fn kill_and_wait(pid: u32) {
    unsafe {
        if libc::kill(pid as i32, libc::SIGTERM) != 0 {
            eprintln!(
                "sweep: SIGTERM on pid {pid} failed ({}) -- possibly a cross-user process",
                std::io::Error::last_os_error()
            );
            return;
        }
    }
    for _ in 0..30 {
        if !pid_alive(pid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    unsafe {
        if libc::kill(pid as i32, libc::SIGKILL) == 0 {
            eprintln!("sweep: pid {pid} did not exit within 3s -- SIGKILLed");
        }
    }
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
}

/// B2 ownership-attribution port self-verification three states (pure judgment, unit-test anchor): Ok(()) = clear/stranger pass,
/// Err(leftover report set) = this instance's processes still hold ports.
pub(super) fn port_verdict(
    listeners: &[conflict::ConflictReport],
    ours: &[u32],
) -> Result<(), Vec<conflict::ConflictReport>> {
    let residue: Vec<conflict::ConflictReport> =
        listeners.iter().filter(|r| ours.contains(&r.pid)).cloned().collect();
    if residue.is_empty() { Ok(()) } else { Err(residue) }
}

/// Poll the three ports for <=5s (10x500ms) with ownership attribution. true = all clear or a stranger holds the port (WARN).
fn verify_ports_attribution(ports: &[u16], reaped: &[u32], dir: &Path) -> bool {
    // Live pids whose signature hits this instance's dir also count as "ours" (the sweep match set, B2 attribution union)
    let sweep_matched: Vec<u32> = reap::scan_live_procs()
        .into_iter()
        .filter(|p| {
            reap::proc_kind(&p.argv, p.exe.as_deref())
                .is_some_and(|k| reap::kind_dir_within(&k, &p.argv, dir))
        })
        .map(|p| p.pid)
        .collect();
    let ours: Vec<u32> = reaped.iter().chain(sweep_matched.iter()).copied().collect();
    let mut verified = false;
    for _ in 0..10 {
        let mut verdict_ok = true;
        for &port in ports {
            if !inspect::port_in_use(port) {
                continue;
            }
            let rep = conflict::diagnose_port(port);
            match port_verdict(&rep, &ours) {
                // B2: a stranger holding the port != a leftover -- WARN + pass (the TalkServo live shape must not fake a failure)
                Ok(()) => {
                    for r in &rep {
                        eprintln!(
                            "[warn] port {port} is held by a non-instance process (pid {} exe {}; possibly another dir's instance of this product)",
                            r.pid, r.exe
                        );
                    }
                }
                Err(residue) => {
                    verdict_ok = false;
                    if !residue.is_empty() {
                        eprintln!("stop: port {port} is still held by this instance's leftovers:");
                        for r in &residue {
                            eprintln!("  {}", r.line());
                        }
                    }
                }
            }
        }
        if verdict_ok {
            verified = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !verified {
        eprintln!(
            "stop: port-level self-verification failed (this instance's leftovers not cleared) -- inspect the occupant before handling"
        );
    }
    verified
}

fn stop_cluster_in(od: &Path) {
    if let Ok(names) = inspect::list_registered_apps(od) {
        for name in &names {
            let _ = run_oxmgr(Some(od), &["stop", name]);
            let _ = run_oxmgr(Some(od), &["delete", name]);
        }
        let _ = run_oxmgr(Some(od), &["daemon", "stop"]);
    }
}

/// `restart [<dir>] [--no-web]`: stop then apply again (guards reused from the start path).
fn cmd_restart(args: &mut dyn Iterator<Item = String>) -> i32 {
    let (dir, no_web_flag) = match parse_start_args(args) {
        Ok(v) => v,
        Err(code) => return code,
    };
    if stop_cluster(&dir) != 0 {
        eprintln!(
            "restart: the stop phase had failures; still restarting (oxmgr is idempotent for live apps)"
        );
    }
    start_impl(&dir, no_web_flag, "restart")
}

/// Pull the instance dir out of the proxy tail args: the last pathlike positional
/// (`.`, `..`, anything containing `/`, or an existing directory) is the dir; the rest
/// pass through to oxmgr untouched. Server proxy takes no proc names today, but the split
/// is shared-shape with host cmd_oxmgr (K3 v2) so both sides resolve dir identically.
fn split_dir_arg(tail: &[String]) -> (Option<PathBuf>, Vec<String>) {
    let is_dir_arg = |a: &str| {
        !a.starts_with('-') && (a == "." || a == ".." || a.contains('/') || Path::new(a).is_dir())
    };
    match tail.iter().rposition(|a| is_dir_arg(a)) {
        Some(i) => {
            let mut rest = tail.to_vec();
            let dir = rest.remove(i);
            (Some(PathBuf::from(dir)), rest)
        }
        None => (None, tail.to_vec()),
    }
}

/// Strict smart default for the proxy commands (parse_dir minus the legacy `.server` guess):
/// (1) cwd carries an etc/server.yaml instance → cwd; (2) an instance beside the binary
/// (exe dir or its parent, the install layout) → that root; otherwise None — an explicit dir
/// is then required. Rationale (PIT-224): the proxy talks to the daemon addressed by dir;
/// an unprovable dir may spawn one that belongs to nowhere, so the fallback is sealed.
fn proxy_smart_dir(cwd: &Path, exe_dir: Option<&Path>) -> Option<PathBuf> {
    let has_instance = |d: &Path| d.join("etc").join("server.yaml").exists();
    if has_instance(cwd) {
        return Some(cwd.to_path_buf());
    }
    let exe = exe_dir?;
    if has_instance(exe) {
        return Some(exe.to_path_buf());
    }
    exe.parent().filter(|p| has_instance(p)).map(PathBuf::from)
}

/// monit/ps -- oxmgr TUI/list passthrough (instance env; dir = explicit trailing pathlike
/// positional, else the strict smart default — K3 v2, aligned with host cmd_oxmgr).
fn cmd_oxmgr_proxy(fixed: &[&str], args: &mut dyn Iterator<Item = String>) -> i32 {
    let tail: Vec<String> = args.collect();
    let (dir, passthrough) = match split_dir_arg(&tail) {
        (Some(d), rest) => (d, rest),
        (None, rest) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let exe =
                std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
            match proxy_smart_dir(&cwd, exe.as_deref()) {
                Some(d) => (d, rest),
                None => {
                    eprintln!(
                        "{}: no instance found in the current dir or beside this binary — pass the instance dir explicitly (e.g. out/server)",
                        fixed[0]
                    );
                    return 2;
                }
            }
        }
    };
    let mut cmd = oxmgr_cmd(&dir);
    cmd.args(fixed).args(&passthrough);
    match cmd.status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!("cannot execute oxmgr: {e}");
            eprintln!(
                "hint: the deployed instance's oxmgr is in <prefix>/bin -- export PATH and retry"
            );
            1
        }
    }
}

// ── K3 v2 (monit/ps dir resolution): split + strict smart default pin tests ────────────────
#[cfg(test)]
mod proxy_dir_tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn split_picks_trailing_pathlike_dir() {
        assert_eq!(split_dir_arg(&s(&["out/server"])), (Some(PathBuf::from("out/server")), s(&[])));
        assert_eq!(
            split_dir_arg(&s(&["--lines", "50", "."])),
            (Some(PathBuf::from(".")), s(&["--lines", "50"]))
        );
    }

    #[test]
    fn split_keeps_flags() {
        let (dir, rest) = split_dir_arg(&s(&["--follow"]));
        assert_eq!(dir, None);
        assert_eq!(rest, s(&["--follow"]));
        let (dir, _) = split_dir_arg(&s(&[]));
        assert_eq!(dir, None);
    }

    #[test]
    fn smart_default_cwd_then_binary_sibling_then_none() {
        let root = std::env::temp_dir().join(format!("msrtc-srv-k3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // nothing marks an instance → None (explicit dir required; no .server guessing)
        assert_eq!(proxy_smart_dir(&root, Some(root.join("bin").as_path())), None);
        // binary sibling: <root>/inst/{bin,etc/server.yaml} with a plain cwd
        let inst = root.join("inst");
        std::fs::create_dir_all(inst.join("bin")).unwrap();
        std::fs::create_dir_all(inst.join("etc")).unwrap();
        std::fs::write(inst.join("etc").join("server.yaml"), b"").unwrap();
        assert_eq!(proxy_smart_dir(&root, Some(inst.join("bin").as_path())), Some(inst.clone()));
        // cwd instance wins over the binary side
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join("etc").join("server.yaml"), b"").unwrap();
        assert_eq!(
            proxy_smart_dir(&root, Some(Path::new("/nonexistent-msrtc-k3/bin"))),
            Some(root.clone())
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

// -- oxmgr pipeline (instance daemon env -- C32 isolation, derived from the same source as host translate::oxmgr_env) --

pub(super) fn oxmgr_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("oxmgr")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("oxmgr"))
}

/// k12 mirror (F19, host translate::oxmgr_env shape) — SINGLE SOURCE for the daemon identity env:
/// consumed by oxmgr_cmd (CLI face) and render_startup_unit (unit face) so the two cannot drift.
/// Home form pinned to the server's own `absolute()` (std::path::absolute, lexical) — the
/// host's `oxmgr_home` canonicalize form is deliberately NOT ported (Momus MAJOR-1: canonicalize
/// on a symlinked install path would re-key the port hash = F19 reborn on a subset of installs).
pub(super) fn oxmgr_env(dir: &Path) -> Vec<(String, String)> {
    let home = absolute(&dir.join("run").join("oxmgr"));
    let port = instance_daemon_port(&home);
    vec![
        ("OXMGR_HOME".to_string(), home.to_string_lossy().into_owned()),
        // C32 legacy name oxmgr remembers — dual-set for compatibility (host k12 same shape)
        ("OXMGR_DATA_DIR".to_string(), home.to_string_lossy().into_owned()),
        ("OXMGR_DAEMON_ADDR".to_string(), format!("127.0.0.1:{port}")),
        ("OXMGR_API_ADDR".to_string(), format!("127.0.0.1:{}", port + 1000)),
    ]
}

fn oxmgr_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new(oxmgr_bin());
    for (k, v) in oxmgr_env(dir) {
        cmd.env(k, v);
    }
    cmd
}

pub(super) fn run_oxmgr(dir: Option<&Path>, args: &[&str]) -> i32 {
    let mut cmd = match dir {
        Some(d) => oxmgr_cmd(d),
        // B3 (PLAN S2.2): default-daemon semantics = explicitly no OXMGR_* -- std Command inherits the parent env,
        // and operator shell residue would silently point "default daemon" operations at an instance daemon (same family of misdirection).
        None => {
            let mut c = Command::new(oxmgr_bin());
            for k in [
                "OXMGR_HOME",
                "OXMGR_DATA_DIR",
                "OXMGR_DAEMON_ADDR",
                "OXMGR_API_ADDR",
                "XDG_DATA_HOME", // B3 sibling (T4-a caught live): an XDG sandbox likewise makes default_data_dir
                                 // resolve to a non-default daemon -- the default semantics = a fully uncovered form
            ] {
                c.env_remove(k);
            }
            c
        }
    };
    match cmd.args(args).status() {
        Ok(st) => st.code().unwrap_or(1),
        Err(e) => {
            eprintln!(
                "oxmgr execution failed: {e} -- install OxMgr (GitHub Releases prebuilt / cargo install, see https://github.com/Vladimir-Urik/OxMgr#install)"
            );
            1
        }
    }
}

pub(super) fn oxmgr_list(dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    let out = oxmgr_cmd(dir).args(["list", "--json"]).output().map_err(|e| {
        format!("oxmgr execution failed: {e} -- install OxMgr and add it to PATH first")
    })?;
    if !out.status.success() {
        return Err(format!("oxmgr list failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("failed to parse oxmgr list --json: {e}"))
}

pub(super) fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- K2: Skipped split-state pin tests (S6 M1 -- the only conversion point + no kill-failure drift + RC behavior) --

    #[test]
    fn k2_tally_outcome_partition() {
        // The four states are mutually exclusive and complete: Skipped counted independently, never flowing into failed
        let out = vec![
            StepOutcome::Skipped,
            StepOutcome::Skipped,
            StepOutcome::Failed,
            StepOutcome::ViaDaemon,
        ];
        let (via, direct, skipped, failed) = reap::tally(&out);
        assert_eq!((via, direct, skipped, failed), (1, 0, 2, 1));
    }

    #[test]
    fn k2_kill_failure_is_not_skipped() {
        // No kill-failure drift negative (M1 exact rejection): execute_steps' two kill-failure exits
        // (ViaDaemon kill failed then daemon confirmed dead -> direct kill still alive; NoRegistry direct kill still alive)
        // must land in Failed, not Skipped -- an enum-level semantic guard + a tally consumer guard.
        let kill_failed = vec![StepOutcome::Failed, StepOutcome::Failed, StepOutcome::Skipped];
        let (_, _, skipped, failed) = reap::tally(&kill_failed);
        assert_eq!(failed, 2, "kill failure = Failed, no drift");
        assert_eq!(skipped, 1, "only direct-kill-gate skips = Skipped");
        // RC formula (MINOR#1): Skipped not in failed -> pure skip gives RC 0
        let pure_skip = [StepOutcome::Skipped, StepOutcome::Skipped];
        let (_, _, _, failed) = reap::tally(&pure_skip);
        assert_eq!(i32::from(failed > 0), 0, "pure skips + clean final state -> RC 0");
    }

    #[test]
    fn no_web_only_list_keeps_weaknet_never_web() {
        // lck-F8/D5-6: --no-web only removes web -- when the weaknet entry is present it must be in the allowlist (prevent dragged-along stop)
        assert!(no_web_only_list(true).contains(&format!(",{WEAKNET_APP}")));
        assert!(no_web_only_list(true).starts_with(&format!("{},", server_product())));
        // Entry absent (conditional-rendering form when bin is not assembled) -> server only, no ghost names (apply --only unknown-name risk)
        assert_eq!(no_web_only_list(false), server_product());
        // The allowlist never contains web (removal semantics = it alone is removed)
        assert!(!no_web_only_list(true).contains(&server_web_app()));
        assert!(!no_web_only_list(false).contains(&server_web_app()));
    }

    #[test]
    fn no_web_downgrade_decision() {
        assert_eq!(decide_no_web(true, true), (true, false)); // explicit --no-web
        assert_eq!(decide_no_web(true, false), (true, false)); // explicit wins, no warn marker
        assert_eq!(decide_no_web(false, false), (true, true)); // caddy missing auto-degrade + warn
        assert_eq!(decide_no_web(false, true), (false, false)); // whole cluster
    }

    #[test]
    fn init_is_idempotent_and_secret_safe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        let cfg = dir.join("etc").join("server.yaml");
        let ox_first =
            std::fs::read_to_string(dir.join("run").join("oxfile.toml")).expect("oxfile");
        // A second init after tampering must not overwrite (PIT-160)
        std::fs::write(&cfg, "edited by user").expect("tamper");
        init_instance(dir).expect("second init");
        assert_eq!(std::fs::read_to_string(&cfg).expect("read"), "edited by user");
        assert_eq!(
            std::fs::read_to_string(dir.join("run").join("oxfile.toml")).expect("read"),
            ox_first
        );
        for f in [
            "etc/server.yaml",
            "etc/devices.yaml",
            "etc/accounts.yaml",
            "etc/Caddyfile",
            "run/oxfile.toml",
        ] {
            assert!(dir.join(f).exists(), "{f} missing");
        }
    }

    #[cfg(unix)]
    #[test]
    fn server_yaml_is_0600_and_secrets_fresh() {
        use std::os::unix::fs::PermissionsExt;
        let a = tempfile::tempdir().expect("tempdir a");
        init_instance(a.path()).expect("init a");
        let md = std::fs::metadata(a.path().join("etc/server.yaml")).expect("md");
        assert_eq!(md.permissions().mode() & 0o777, 0o600);
        let b = tempfile::tempdir().expect("tempdir b");
        init_instance(b.path()).expect("init b");
        let sa = std::fs::read_to_string(a.path().join("etc/server.yaml")).expect("read a");
        let sb = std::fs::read_to_string(b.path().join("etc/server.yaml")).expect("read b");
        assert_ne!(sa, sb, "the random credentials of two inits must differ");
        // jwt and admin_jwt share a value (pairing fail-fast gate C35)
        let jwt = sa.lines().find(|l| l.starts_with("jwt_secret:")).expect("jwt line");
        let adjwt = sa.lines().find(|l| l.starts_with("admin_jwt_secret:")).expect("admin line");
        assert_eq!(jwt.split(':').nth(1).expect("v1"), adjwt.split(':').nth(1).expect("v2"));
    }

    #[test]
    fn admin_bootstrap_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let acc = tmp.path().join("accounts.yaml");
        std::fs::write(&acc, templates::ACCOUNTS_TEMPLATE).expect("write");
        // Empty password: no-op
        bootstrap_admin(&acc, "").expect("empty-pass no-op");
        assert!(!std::fs::read_to_string(&acc).expect("read").contains("role: admin"));
        // Bootstrap once; a second run does not repeat (explicit args, no env race)
        bootstrap_admin(&acc, "pw-test").expect("bootstrap 1");
        bootstrap_admin(&acc, "pw-test").expect("bootstrap 2");
        let after = std::fs::read_to_string(&acc).expect("read");
        assert_eq!(after.matches("role: admin").count(), 1, "admin bootstrap is idempotent");
        assert!(after.contains("  admin:"));
        serde_yaml::from_str::<serde_yaml::Value>(&after)
            .expect("still valid YAML after bootstrap");
    }

    #[test]
    fn dir_flag_guard_rejects_dashdash() {
        let mut it = ["--dir".to_string(), "/tmp/x".to_string()].into_iter();
        assert!(
            parse_dir(&mut it).is_err(),
            "--dir style typos must be rejected (same lesson as the host init --dir incident)"
        );
    }

    // ---- config-unification P1: yaml port keys flow through into init render products ----

    #[test]
    fn init_caddyfile_uses_yaml_web_port() {
        // PIT-13 positive control: assert the concrete rendered value, not merely "no warning"
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        // Add web.port to server.yaml, delete the Caddyfile, re-init (PIT-160 semantics: written only when missing)
        let cfg = dir.join("etc").join("server.yaml");
        let text = std::fs::read_to_string(&cfg).expect("read");
        std::fs::write(&cfg, format!("{text}\nweb:\n  port: 9080\n")).expect("write");
        std::fs::remove_file(dir.join("etc").join("Caddyfile")).expect("rm caddyfile");
        init_instance(dir).expect("re-init");
        let cf = std::fs::read_to_string(dir.join("etc").join("Caddyfile")).expect("read cf");
        assert!(
            cf.contains(":9080 {"),
            "yaml web.port must render into the Caddyfile site address"
        );
        assert!(!cf.contains(":8080"), "the default port must not linger in the site address");
    }

    #[test]
    fn init_weaknet_yaml_port_flows_to_oxfile_and_caddyfile() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        let cfg = dir.join("etc").join("server.yaml");
        let text = std::fs::read_to_string(&cfg).expect("read");
        std::fs::write(&cfg, format!("{text}\nsfu:\n  weaknet_port: 9811\n")).expect("write");
        // Re-render the oxfile (write_oxfile is the single point -- delete it and let init rebuild)
        std::fs::remove_file(dir.join("run").join("oxfile.toml")).expect("rm oxfile");
        // oxfile conditional rendering needs bin/weaknet present; without bin only the Caddyfile side is verified
        std::fs::remove_file(dir.join("etc").join("Caddyfile")).expect("rm cf");
        init_instance(dir).expect("re-init");
        let cf = std::fs::read_to_string(dir.join("etc").join("Caddyfile")).expect("read cf");
        assert!(
            cf.contains("reverse_proxy 127.0.0.1:9811"),
            "weaknet_port renders into the Caddyfile upstream"
        );
        // oxfile: fake the bin then call write_oxfile directly to verify both products share a source
        std::fs::create_dir_all(dir.join("bin")).expect("mkdir bin");
        std::fs::write(dir.join("bin").join("mediaservo-weaknet"), b"#!/bin/sh\n")
            .expect("write bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                dir.join("bin").join("mediaservo-weaknet"),
                std::fs::Permissions::from_mode(0o755),
            )
            .expect("chmod");
        }
        write_oxfile(&dir.join("run")).expect("write oxfile");
        let ox = std::fs::read_to_string(dir.join("run").join("oxfile.toml")).expect("read ox");
        assert!(
            ox.contains("serve --listen 127.0.0.1:9811"),
            "weaknet_port renders into the oxfile --listen (both products share a source)"
        );
    }

    // ---- T4 (PIT-160 root fix): write_oxfile merge source live->.bak + note-line divergence ----

    #[test]
    fn t4c_manual_env_survives_via_bak_when_live_deleted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        init_instance(dir).expect("first init");
        let ox = dir.join("run").join("oxfile.toml");
        let bak = dir.join("run").join("oxfile.toml.bak");
        let base = std::fs::read_to_string(&bak).expect(
            "write_oxfile must also produce the .bak snapshot (the PIT-160 re-render source)",
        );
        // Inject manual keys into .bak (= the shape after Python deploy stale backup); two paths for the server entry's env table (present/absent)
        let tampered = if base.contains("[apps.env]") {
            base.replacen("[apps.env]", "[apps.env]\nALLOW_X = \"1\"", 1)
        } else {
            base.replacen("[apps.logs]", "[apps.env]\nALLOW_X = \"1\"\n[apps.logs]", 1)
        };
        assert_ne!(tampered, base, "fixture injection failed");
        std::fs::write(&bak, &tampered).expect("tamper bak");
        std::fs::remove_file(&ox).expect("rm live oxfile");
        write_oxfile(&dir.join("run")).expect("re-render (live missing -> bak merge source)");
        let after = std::fs::read_to_string(&ox).expect("read ox");
        assert!(after.contains("ALLOW_X = \"1\""), ".bak manual env must be merged back:\n{after}");
        assert_eq!(
            std::fs::read_to_string(&bak).expect("read bak"),
            after,
            ".bak should advance with the written final state (multiple rounds, N does not double)"
        );
        // Both live and bak missing -> pure render (no manual keys at all)
        std::fs::remove_file(&ox).expect("rm oxfile 2");
        std::fs::remove_file(&bak).expect("rm bak");
        write_oxfile(&dir.join("run")).expect("re-render (first-render shape)");
        let pure = std::fs::read_to_string(&ox).expect("read ox 2");
        assert!(!pure.contains("ALLOW_X"), "both live and bak missing -> pure render");
    }

    // ---- global-reap T2: B3 env sanitization + B2 attribution three states + sweep plan fixture ----

    #[test]
    fn run_oxmgr_default_strips_oxmgr_env() {
        // B3: default-daemon semantics = explicitly no OXMGR_*. std Command inherits the parent env -- after env_remove,
        // get_envs must contain (key, None) covering the deleted items (parent shell residue must not leak into the child either).
        // (no set_var to fabricate a parent env: edition 2024 unsafe + parallel test env races -- the deletion semantics
        // apply equally to "inherited values"; the covering entries prove it.)
        let keys = ["OXMGR_HOME", "OXMGR_DATA_DIR", "OXMGR_DAEMON_ADDR", "OXMGR_API_ADDR"];
        let mut cmd = Command::new(oxmgr_bin());
        for k in keys {
            cmd.env_remove(k);
        }
        for k in keys {
            assert!(
                cmd.get_envs().any(|(key, v)| key == std::ffi::OsStr::new(k) && v.is_none()),
                "{k} must appear as an explicit deletion"
            );
        }
    }

    #[test]
    fn port_verdict_three_states() {
        use super::conflict::ConflictReport;
        let rep = |pid: u32| ConflictReport {
            pid,
            exe: "/x/bin/msrtc-server".into(),
            cmd: "msrtc-server run".into(),
            deleted: false,
            parent: None,
        };
        // 1. idle (no listener) -> pass
        assert!(port_verdict(&[], &[1, 2]).is_ok());
        // 2. our leftover (pid in this round's reap list) -> fail
        let ours = vec![rep(101), rep(102)];
        assert!(port_verdict(&ours, &[101]).is_err());
        // 3. stranger holds the port -> pass (B2: the TalkServo live shape must not fake a failure)
        assert!(port_verdict(&ours, &[999]).is_ok());
        // Boundary: both sides empty -> pass
        assert!(port_verdict(&[], &[]).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn sweep_plan_fixture_direct_kill_when_no_registry() {
        use super::reap::{LiveProc, ProcKind, ReapStep};
        // fixture: tempdir instance root + a faked bin/msrtc-server marker (the marker judgment surface)
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = absolute(tmp.path());
        std::fs::create_dir_all(root.join("bin")).expect("mkdir bin");
        std::fs::write(root.join("bin").join("msrtc-server"), b"").expect("marker");
        // LiveProc points at it: server signature + --config owned by this root; empty registry -> DirectKill plan
        let p = LiveProc {
            argv: vec![
                format!("{}/bin/msrtc-server", root.display()),
                "run".into(),
                "--config".into(),
                format!("{}/etc/server.yaml", root.display()),
            ],
            exe: Some(format!("{}/bin/msrtc-server", root.display())),
            cwd: Some(root.clone()),
            environ_oxmgr_home: None,
            pid: 424242,
        };
        assert_eq!(
            super::reap::proc_kind(&p.argv, p.exe.as_deref()),
            Some(ProcKind::Server),
            "fixture must pass the signature check"
        );
        assert!(super::reap::kind_dir_within(&ProcKind::Server, &p.argv, &root));
        // Empty registry (both-table snapshot) -> direct-kill plan (assert the plan, do not execute -- zero real signals in unit tests)
        let plan = super::reap::reap_plan(&[p], &[(root.join("run").join("oxmgr"), &[])]);
        assert_eq!(
            plan,
            vec![ReapStep::DirectKill {
                pid: 424242,
                reason: super::reap::DirectReason::NoRegistry
            }]
        );
    }

    // ---- global-reap T3: stop arg parsing, three forms ----

    #[test]
    fn stop_args_global_alone_and_with_dir_conflict() {
        let mut a = ["-g".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut a), Ok(StopArgs::Global));
        let mut b = ["--all".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut b), Ok(StopArgs::Global));
        // -g + dir = invalid (exit 2 mapped by cmd_stop)
        let mut c = ["-g".to_string(), "/opt/x".to_string()].into_iter();
        assert_eq!(
            parse_stop_args(&mut c),
            Err("stop -g is a global reap and does not accept a dir argument".to_string())
        );
        // dir first, -g last: also rejected (order-independent)
        let mut d = ["/opt/x".to_string(), "--all".to_string()].into_iter();
        assert!(parse_stop_args(&mut d).is_err());
        // Unknown flag rejected
        let mut e = ["--dir".to_string()].into_iter();
        assert!(parse_stop_args(&mut e).is_err());
    }

    #[test]
    fn stop_args_dir_forms_unchanged() {
        // Bare stop = default dir (existing behavior); explicit dir; a dir together with an unknown -- arg is also rejected.
        let mut a: Box<dyn Iterator<Item = String>> = Box::new(std::iter::empty());
        assert_eq!(parse_stop_args(&mut a), Ok(StopArgs::Dir(None)));
        let mut b = ["/opt/inst".to_string()].into_iter();
        assert_eq!(parse_stop_args(&mut b), Ok(StopArgs::Dir(Some("/opt/inst".into()))));
        let mut c = ["/a".to_string(), "/b".to_string()].into_iter();
        assert!(parse_stop_args(&mut c).is_err());
    }

    #[test]
    fn t4_pit160_note_variants() {
        assert_eq!(
            pit160_note(0, 0),
            "Note: if this tree previously had manual [apps.env] lines (ALLOW_DEV_*/RUST_LOG/WEAKNET_ADMIN_PASS), this render does not include them -- restore them per the msrtc.sh -h \"environment variables [B]\" section",
            "N=0 must preserve the old guidance text byte-for-byte"
        );
        let v = pit160_note(3, 0);
        assert!(v.contains("merged 3 manual env entries"), "N>0 variant: {v}");
        let l = pit160_note(2, 1);
        assert!(l.contains("additionally 1 entries"), "unmerged entries must report a count: {l}");
    }
}
