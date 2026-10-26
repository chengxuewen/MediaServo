//! mediaservo-weaknet -- tc/netem weak-network emulation agent (dual-surface single binary).
//!
//! T4 = CLI assembly: the `apply|set|status|clear` full chain + `up|down` aliases via engine/state;
//! T12 = vehicle surface arrives: `--config weaknet.yaml` (defaults supplied by config.rs) + `status --watch` (watch.rs single-screen redraw).
//! Wording/exit-code truth = `scripts/weaknet.sh` (until retirement day); contract source = parent-repo docs/plans/weaknet-agent/.
//! Exit codes: 0 OK / 2 environment insufficient or apply failed / 3 state conflict (lock, foreign qdisc, state divergence) / 4 invalid params.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};

use mediaservo_weaknet::config;
use mediaservo_weaknet::engine::{self, ApplyRequest, Env, Fail, Replay, Verify, Wn};
use mediaservo_weaknet::measure;
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scenario;
use mediaservo_weaknet::scope::{self, Targeting};
use mediaservo_weaknet::server;
use mediaservo_weaknet::spec::{self, Dir as SpecDir, IfaceKind, ImpairSpec, LossSpec, ScopeSel};
use mediaservo_weaknet::state::{self, Dirs, State};
use mediaservo_weaknet::watch;

/// bash DEFAULT_DURATION -- apply's sole liveness promise.
const DEFAULT_DURATION: u64 = 300;

#[derive(Debug, Parser)]
#[command(
    name = "mediaservo-weaknet",
    version,
    about = "weak-network emulation agent: server surface (CLI+serve+panel) and vehicle surface (up/down/watch+yaml) in one binary"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Apply impairment (default dir=both = bash two-leg equivalent; default duration=300s fuse)
    Apply(Box<ApplyArgs>),
    /// Override fields (merged over the state base, full replay; no lifetime extension; no active spec -> exit4)
    Set(Box<ApplyArgs>),
    /// Timeline scenario
    Scenario {
        #[command(subcommand)]
        step: ScenarioCmd,
    },
    /// Current state (--watch = ANSI single-screen redraw, top-style)
    Status {
        #[arg(long)]
        watch: bool,
    },
    /// Clear all impairment (lock-free reachable, firefighting channel)
    Clear,
    /// Control plane: REST+SSE+embedded panel (default 127.0.0.1:9810)
    Serve(ServeArgs),
    /// = apply <profile> (vehicle alias sugar, default duration 300)
    Up {
        profile: String,
        #[arg(long)]
        iface: Option<String>,
        #[arg(long)]
        duration: Option<u64>,
    },
    /// = clear (vehicle alias sugar)
    Down,
    /// = status --watch
    Watch,
    /// Verdict table for a probe/timeline pair (judge port; frozen goldens = truth; rc 1 = any FAIL)
    Measure {
        /// probe.jsonl (browser rows) path
        probe: String,
        /// timeline.jsonl (scenario events) path
        timeline: String,
        /// retention tolerance for the PASS verdict (default 0.95)
        #[arg(long)]
        tol: Option<f64>,
    },
    /// Hidden subcommand: the watchdog's self re-exec executor (design §fuse; spawner passes current_exe + this command)
    #[command(name = "__watchdog", hide = true)]
    Watchdog {
        /// state file path containing the expiry moment (expires_at_ms)
        state_path: String,
    },
}

#[derive(Debug, Subcommand)]
enum ScenarioCmd {
    /// Run a scenario (job exclusivity + transient lock per step; ports the bash flags --no-baseline/--keep)
    Run {
        #[arg(long)]
        file: Option<String>,
        /// scenario root override (default per the vehicle-surface resolution)
        #[arg(long)]
        dir: Option<String>,
        /// Skip the baseline window (the judge window degrades to the start+10s default -- bash synonym)
        #[arg(long, default_value_t = false)]
        no_baseline: bool,
        /// Keep the site at the end (no clear; bash synonym)
        #[arg(long, default_value_t = false)]
        keep: bool,
        /// Ctrl-C/SIGTERM equivalently triggers the existing stop-flag path (no second damage-control channel invented),
        /// wrapped up by the runner's end-of-step check (scenario-end aborted="user" + the existing teardown, exit0)
        #[arg(long, default_value_t = false)]
        auto_clear: bool,
    },
    /// List available scenarios
    List,
    /// Stop a running scenario (cancel flag / stale flags self-clear; always exit0 idempotent)
    Stop,
}

#[derive(Debug, clap::Args)]
struct ApplyArgs {
    /// Named profile (profiles/<name>.yaml; explicit flag > profile, same priority as bash)
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    rtt: Option<u64>,
    #[arg(long)]
    jitter: Option<u64>,
    /// Loss ("2%" or "2", % stripped in gemodel mode)
    #[arg(long)]
    loss: Option<String>,
    /// simple (default) | gemodel (requires --gemodel)
    #[arg(long)]
    loss_mode: Option<String>,
    /// Gilbert-Elliott param group (r h k, % auto-stripped)
    #[arg(long, num_args = 3, value_names = ["R", "H", "K"])]
    gemodel: Option<Vec<String>>,
    #[arg(long)]
    reorder: Option<String>,
    #[arg(long)]
    rate: Option<f64>,
    #[arg(long)]
    seed: Option<u64>,
    /// Fuse duration (seconds, default 300 -- apply's sole liveness promise; <5 not accepted)
    #[arg(long)]
    duration: Option<u64>,
    /// Target interface (default lo = documented constant, C20 exemption; overridden by env WEAKNET_DEV)
    #[arg(long)]
    iface: Option<String>,
    /// Direction (default both; leg definitions = design §dir table; set default doesn't override the base dir)
    #[arg(long, value_enum)]
    dir: Option<Dir>,
    /// Targeting: room names (=streams), comma-separated; "all" = explicit back to room level (implemented by T5 scope.rs)
    #[arg(long)]
    stream: Option<String>,
    /// Targeting: device IDs (all streams in the owner group; older servers fall back to peer_id with WARN) (T5)
    #[arg(long)]
    device: Option<String>,
    /// Escape hatch: explicit RTP port set, comma-separated (works with zero server; highest priority)
    #[arg(long)]
    rtp_port: Option<String>,
    /// Signaling TCP leg port (optional, carries the bash --signaling slot)
    #[arg(long)]
    signaling_port: Option<u16>,
    /// stats control-plane URL (default chain: flag > env WEAKNET_SERVER_URL > probing out/server/etc/server.yaml)
    #[arg(long)]
    server_url: Option<String>,
    /// Param file weaknet.yaml (vehicle surface; default chain flag > env WEAKNET_CONFIG > probe next to the binary/cwd)
    #[arg(long)]
    config: Option<String>,
    /// Print only the equivalent command sequence (incl. channel prefixes, zero kernel/docker touching), exit0
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(Debug, clap::Args)]
struct ServeArgs {
    /// Bind address (default 127.0.0.1:9810; non-loopback needs --lan or startup refuses)
    #[arg(long)]
    listen: Option<String>,
    /// Relax to LAN: Origin allowlist + mandatory-token hard gate + banner emits token only
    #[arg(long, default_value_t = false)]
    lan: bool,
    /// token override (WEAKNET_TOKEN env wins; default CSPRNG >=128bit at startup)
    #[arg(long)]
    token: Option<String>,
    /// server-surface stats/admin URL (default flag > env WEAKNET_SERVER_URL > probing out/server/etc/)
    #[arg(long)]
    server_url: Option<String>,
    /// token file: read if present (0600 enforced), generate if absent (0600) -- priority --token flag > env WEAKNET_TOKEN
    /// > this file > CSPRNG (design §D2 sF5; the unit passes {dir}/run/weaknet.token = T4)
    #[arg(long)]
    token_file: Option<std::path::PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum Dir {
    Out,
    In,
    Both,
}

impl From<Dir> for SpecDir {
    fn from(d: Dir) -> Self {
        match d {
            Dir::Out => SpecDir::Out,
            Dir::In => SpecDir::In,
            Dir::Both => SpecDir::Both,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = dispatch(cli.cmd) {
        eprintln!("weaknet: {}", e.msg);
        std::process::exit(i32::try_from(e.code).unwrap_or(2));
    }
}

fn dispatch(cmd: Cmd) -> Wn<()> {
    let dirs = Dirs::from_env();
    let env = Env::from_env();
    match cmd {
        // bash parity: flock covers apply|set|scenario; clear/status lock-free reachable (firefighting).
        // T8 exclusivity gate (rev-2.2 Momus-B2): while a scenario job is alive, external apply/set 409/exit3 first (independent of the lock).
        Cmd::Apply(a) => {
            scenario::job_gate(&dirs)?;
            let _lk = engine::take_write_lock(&dirs)?;
            do_apply(&a, &dirs, &env)
        }
        Cmd::Set(a) => {
            scenario::job_gate(&dirs)?;
            let _lk = engine::take_write_lock(&dirs)?;
            engine::autoheal_if_expired(&dirs, &env)?; // apply/clear skip the check, everything else verifies up front (bash)
            do_set(&a, &dirs, &env)
        }
        Cmd::Scenario { step } => {
            engine::autoheal_if_expired(&dirs, &env)?; // bash: scenario also verifies expiry up front
            do_scenario(step, &dirs, &env)
        }
        Cmd::Status { watch: is_watch } => {
            engine::autoheal_if_expired(&dirs, &env)?;
            if is_watch {
                return watch::run(&dirs, &env);
            }
            do_status(&dirs, &env)
        }
        Cmd::Clear => {
            println!("{}", engine::clear(&dirs, &env, false)?);
            Ok(())
        }
        Cmd::Measure { probe, timeline, tol } => {
            let mut argv = vec![probe, timeline];
            if let Some(t) = tol {
                argv.push("--tol".to_string());
                argv.push(format!("{t}"));
            }
            measure::measure_cmd(&argv)
        }
        Cmd::Serve(args) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| Fail::env(format!("failed to build the tokio runtime: {e}")))?;
            rt.block_on(server::serve_main(
                args.listen.as_deref(),
                args.lan,
                args.token.as_deref(),
                args.token_file.as_deref(),
                args.server_url.as_deref(),
            ))
        }
        Cmd::Up { profile, iface, duration } => {
            let a = ApplyArgs {
                profile: Some(profile),
                rtt: None,
                jitter: None,
                loss: None,
                loss_mode: None,
                gemodel: None,
                reorder: None,
                rate: None,
                seed: None,
                duration,
                iface,
                dir: None,
                stream: None,
                device: None,
                rtp_port: None,
                signaling_port: None,
                server_url: None,
                config: None,
                dry_run: false,
            };
            let _lk = engine::take_write_lock(&dirs)?;
            do_apply(&a, &dirs, &env)
        }
        Cmd::Down => {
            println!("{}", engine::clear(&dirs, &env, false)?);
            Ok(())
        }
        Cmd::Watch => {
            engine::autoheal_if_expired(&dirs, &env)?;
            watch::run(&dirs, &env)
        }
        Cmd::Watchdog { state_path } => {
            fuse::watchdog_run(&PathBuf::from(state_path), watchdog_expire).map_err(Fail::env)
        }
    }
}

// ---------- apply / set ----------

fn do_apply(a: &ApplyArgs, dirs: &Dirs, env: &Env) -> Wn<()> {
    // T12 vehicle surface: weaknet.yaml = the defaults supplier (priority CLI flag > env > yaml > documented default).
    let cfg = config::load(a.config.as_deref())?;
    let iface =
        resolve_iface(a.iface.as_deref().or_else(|| cfg.as_ref().and_then(|c| c.iface_or())));
    let base = match a.profile.as_deref() {
        Some(p) => load_profile(p)?,
        None => cfg.as_ref().and_then(|c| c.spec.clone()).unwrap_or_default(),
    };
    let spec = merge_spec(&base, a)?;
    let duration =
        a.duration.or_else(|| cfg.as_ref().and_then(|c| c.duration)).unwrap_or(DEFAULT_DURATION);
    spec::validate_duration(duration).map_err(Fail::bad_param)?;
    let mut ports_flag = parse_ports(a.rtp_port.as_deref())?;
    if ports_flag.is_empty()
        && let Some(p) = cfg.as_ref().and_then(|c| c.ports.clone())
    {
        ports_flag = p;
    }
    if a.dry_run {
        return print_dry_run(
            env,
            &spec,
            &iface,
            &ports_flag,
            a.signaling_port,
            &format!("{duration}s (lazy self-heal + watchdog; no --forever exemption exposed)"),
        );
    }
    let kind = engine::iface_kind_of(&iface);
    let targeting = match resolve_targeting(a, ports_flag.clone()) {
        Ok(t) => t,
        Err(mut e) => {
            // anti-deadlock rule layer: physical port + no explicit port supply (flag/yaml both empty, stats unresolved either) -> enumerate guidance cause.
            if let Some(hint) =
                config::physical_ports_hint(kind == IfaceKind::Physical, !ports_flag.is_empty())
            {
                e.msg.push_str(hint);
            }
            return Err(e);
        }
    };
    // operator-surface anti-deadlock (design §vehicle 3): print the impact surface before applying on a physical port.
    if kind == IfaceKind::Physical {
        let impact = if targeting.pairs.is_empty() {
            targeting.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
        } else {
            format!(
                "pairs {}",
                targeting
                    .pairs
                    .iter()
                    .map(|(l, r)| format!("{l}:{r}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        println!(
            "impact surface: {iface}/UDP/{impact}/countdown {duration}s (tear down: weaknet down)"
        );
    }
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
        names: targeting.scope_names(),
        iface: iface.clone(),
        ports: targeting.ports.clone(),
        pairs: targeting.pairs.clone(),
        sig_port: a.signaling_port,
    };
    let out =
        engine::replay(&req, Replay::Apply { duration_secs: duration }, dirs, env, Verify::Inline)?;
    let scope_detail = match req.scope {
        ScopeSel::Media => format!(
            "ports=[{}]",
            req.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
        ),
        _ => format!(
            "pairs=[{}]",
            req.pairs.iter().map(|(l, r)| format!("{l}:{r}")).collect::<Vec<_>>().join(" "),
        ),
    };
    println!(
        "applied: dev={iface} spec=[{}] {scope_detail} filters={} channel={} read-back fingerprint passed",
        out.spec_string, out.filter_count, out.channel,
    );
    println!(
        "verify leaf {}->{} packets OK effective RTT~{}ms (halved when iface=lo) fuse={duration}s (lazy self-heal + watchdog)",
        out.leaf.0, out.leaf.1, spec.rtt_ms
    );
    Ok(())
}

fn do_set(a: &ApplyArgs, dirs: &Dirs, env: &Env) -> Wn<()> {
    if a.config.is_some() {
        return Err(Fail::bad_param(
            "set does not consume --config (weaknet.yaml is apply's defaults supplier; set's base = the active state)",
        ));
    }
    let prior = State::read_from(&dirs.state_json())
        .map_err(Fail::env)?
        .ok_or_else(|| Fail::bad_param("no state (apply first; set does not create a new site)"))?;
    if !any_override(a) {
        return Err(Fail::bad_param("set needs at least one override param (see -h)"));
    }
    let iface = a.iface.clone().unwrap_or_else(|| prior.iface.clone());
    let spec = merge_spec(&prior.spec, a)?;
    let dry_ports = if a.rtp_port.is_some() {
        parse_ports(a.rtp_port.as_deref())?
    } else {
        prior.ports.clone()
    };
    let sig = a.signaling_port.or(prior.sig_port);
    if a.dry_run {
        return print_dry_run(
            env,
            &spec,
            &iface,
            &dry_ports,
            sig,
            "unchanged (set does not extend lifetime)",
        );
    }
    // redirection (--rtp-port/--stream/--device, any) = re-resolve; otherwise keep the base state's
    // scope/ports/pairs (T5: resolution already on disk, replay skips re-pulling stats).
    let targeting = if a.rtp_port.is_some() || a.stream.is_some() || a.device.is_some() {
        resolve_targeting(a, parse_ports(a.rtp_port.as_deref())?)?
    } else {
        Targeting {
            scope: prior.scope,
            ports: prior.ports.clone(),
            pairs: prior.pairs.clone(),
            names: match prior.scope {
                ScopeSel::Stream => prior.rooms.clone(),
                ScopeSel::Device => prior.devices.clone(),
                ScopeSel::Media => vec![],
            },
        }
    };
    if targeting.ports.is_empty() && targeting.pairs.is_empty() {
        return Err(Fail::env(
            "no usable media ports/pairs (base state empty and no --rtp-port/--stream/--device given) -- re-apply",
        ));
    }
    let before = state::param_summary(&prior.spec);
    let req = ApplyRequest {
        spec: spec.clone(),
        scope: targeting.scope,
        names: targeting.scope_names(),
        iface,
        ports: targeting.ports,
        pairs: targeting.pairs,
        sig_port: sig,
    };
    let out = engine::replay(
        &req,
        Replay::Set { prior: Box::new(prior), before: before.clone() },
        dirs,
        env,
        Verify::Inline,
    )?;
    println!(
        "set: [{before}] -> [{}] (counters_reset={}, root not rolled back, fuse not extended)",
        state::param_summary(&spec),
        if matches!(out.counters_reset, Some(true)) { "yes" } else { "no" }
    );
    Ok(())
}

fn any_override(a: &ApplyArgs) -> bool {
    a.rtt.is_some()
        || a.jitter.is_some()
        || a.loss.is_some()
        || a.gemodel.is_some()
        || a.reorder.is_some()
        || a.rate.is_some()
        || a.seed.is_some()
        || a.dir.is_some()
        || a.signaling_port.is_some()
        || a.profile.is_some()
        || a.rtp_port.is_some()
        || a.stream.is_some()
        || a.device.is_some()
        || a.jitter.is_some()
        || a.loss.is_some()
        || a.gemodel.is_some()
        || a.reorder.is_some()
        || a.rate.is_some()
        || a.seed.is_some()
        || a.dir.is_some()
        || a.signaling_port.is_some()
        || a.profile.is_some()
}

/// Base + explicit flag overrides (bash priority: flag > profile; range validation mirrors the exit-4 table).
fn merge_spec(base: &ImpairSpec, a: &ApplyArgs) -> Wn<ImpairSpec> {
    let mut s = base.clone();
    if let Some(v) = a.rtt {
        s.rtt_ms = v;
    }
    if let Some(v) = a.jitter {
        s.jitter_ms = v;
    }
    if let Some(v) = &a.reorder {
        s.reorder_pct = Some(v.clone());
    }
    if let Some(v) = a.rate {
        s.rate_mbps = Some(v);
    }
    if let Some(v) = a.seed {
        s.seed = Some(v);
    }
    if let Some(v) = a.dir {
        s.dir = v.into();
    }
    let mode = a.loss_mode.as_deref().unwrap_or("simple");
    if let Some(g) = &a.gemodel {
        let loss = a.loss.clone().or_else(|| match &s.loss {
            Some(LossSpec::GeModel { loss, .. }) => Some(loss.clone()),
            Some(LossSpec::Simple(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        });
        let Some(loss) = loss else {
            return Err(Fail::bad_param("gemodel needs --loss (p13; profile or flag both fine)"));
        };
        s.loss =
            Some(LossSpec::GeModel { loss, r: g[0].clone(), h: g[1].clone(), k: g[2].clone() });
    } else if let Some(p) = &a.loss {
        match mode {
            "simple" | "uniform" => s.loss = Some(LossSpec::Simple(p.clone())),
            "gemodel" => {
                return Err(Fail::bad_param(
                    "loss_mode=gemodel needs --gemodel r h k three params",
                ));
            }
            other => {
                return Err(Fail::bad_param(format!(
                    "loss_mode must be simple|gemodel, got {other:?}"
                )));
            }
        }
    }
    s.validate().map_err(Fail::bad_param)?;
    Ok(s)
}

fn resolve_iface(cli: Option<&str>) -> String {
    engine::resolve_iface(cli)
}

fn parse_ports(s: Option<&str>) -> Wn<Vec<u16>> {
    let Some(raw) = s.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(vec![]);
    };
    raw.split(',')
        .map(|p| {
            p.trim()
                .parse::<u16>()
                .map_err(|e| Fail::bad_param(format!("--rtp-port port invalid {p:?}: {e}")))
        })
        .collect()
}

/// Targeting adjudication (extracted at T6: the core goes into scope::resolve_targeting shared with the serve REST; only field hauling here).
fn resolve_targeting(a: &ApplyArgs, ports_flag: Vec<u16>) -> Wn<Targeting> {
    scope::resolve_targeting(
        a.server_url.as_deref(),
        a.stream.as_deref(),
        a.device.as_deref(),
        &ports_flag,
    )
}

fn print_dry_run(
    env: &Env,
    spec: &ImpairSpec,
    iface: &str,
    ports: &[u16],
    sig: Option<u16>,
    duration_note: &str,
) -> Wn<()> {
    // T9: leg split structurally identical to replay (physical port in/both = egress keeps only Out legs, the ingress leg goes to the ifb display section).
    let kind = engine::iface_kind_of(iface);
    let use_ifb = engine::ifb_needed(kind, spec.dir);
    let legs = if !use_ifb {
        spec::build_filter_legs(ScopeSel::Media, kind, spec.dir, ports, &[]).unwrap_or_default()
    } else if spec.dir == SpecDir::Both {
        spec::build_filter_legs(ScopeSel::Media, kind, SpecDir::Out, ports, &[]).unwrap_or_default()
    } else {
        Vec::new()
    };
    let ingress_legs = if use_ifb {
        spec::build_filter_legs(ScopeSel::Media, kind, SpecDir::In, ports, &[]).unwrap_or_default()
    } else {
        Vec::new()
    };
    let spec_string = spec::render_netem_spec_leg(spec);
    let rate = spec::render_rate_arg(spec.rate_mbps);
    let install_skeleton = !use_ifb || !legs.is_empty() || sig.is_some();
    let mut steps = Vec::new();
    if install_skeleton {
        steps.extend(engine::plan_skeleton(iface, true, &spec_string, &rate, &legs, sig));
    }
    if use_ifb {
        // read-before-probe must have no side effects -- show the full chain form (need_link=true incl. link-create + up steps).
        steps.extend(engine::plan_ifb(iface, &spec_string, &ingress_legs, true));
    }
    let prefix = engine::dryrun_prefix(env);
    let ip_prefix = engine::dryrun_prog_prefix(env, engine::Prog::Ip);
    println!(
        "[dry] equivalent command sequence (root absent shows the add form; change || add = 2015 compat; # = best-effort):"
    );
    for s in &steps {
        let alt = s.alt.as_ref().map_or_else(String::new, |a| format!(" || {}", a.join(" ")));
        let note = if s.best_effort { "   # best-effort" } else { "" };
        let pfx = if s.prog == engine::Prog::Ip { &ip_prefix } else { &prefix };
        println!("{pfx} {}{alt}{note}", s.run.join(" "));
    }
    if legs.is_empty() && ingress_legs.is_empty() {
        println!(
            "(media legs/pairs resolved at execution: --stream/--device/--rtp-port or stats observation)"
        );
    }
    println!("[dry] fuse: {duration_note} auto-clear=0 (signal layer off by default)");
    Ok(())
}

// ---------- status ----------

fn do_status(dirs: &Dirs, env: &Env) -> Wn<()> {
    let st = State::read_from(&dirs.state_json()).map_err(Fail::env)?;
    let dev = st.as_ref().map(|s| s.iface.clone()).unwrap_or_else(|| resolve_iface(None));
    // green-wash defense (bash do_status): tc channel unreadable != no qdisc -- probe/exec failure carries its own exit2 cause.
    let chan = engine::probe_channel(env, &dev)?;
    // T9 read-back routing: an ifb ingress-only session's site is at ifb0 root (iface root untouched).
    let (rd_dev, rd_form) = match &st {
        Some(s) if s.ifb_used && s.dir == SpecDir::In && s.sig_port.is_none() => {
            (mediaservo_weaknet::ifb::IFB_DEV.to_string(), engine::LeafForm::Root)
        }
        _ => (dev.clone(), engine::LeafForm::Parent110),
    };
    let show = engine::tc_exec(&chan, &["qdisc", "show", "dev", &rd_dev])?;
    let observed: Vec<&str> =
        show.lines().filter(|l| l.contains("netem") || l.contains("htb")).take(3).collect();
    match (&st, observed.is_empty()) {
        (None, true) => println!("inactive: no weaknet qdisc on {dev}"),
        (Some(_), true) => {
            return Err(Fail::conflict(
                "state present but qdisc absent (external cleanup? watchdog?) -- reset first with weaknet clear",
            ));
        }
        (None, false) => {
            return Err(Fail::conflict(format!(
                "qdisc residue but state missing (external/manual or overdue death): {} -- self-prove before clear",
                observed[0]
            )));
        }
        (Some(s), false) => {
            // cross-check (design §Error handling): state x tc fingerprint; divergence = externally rewritten, WARN.
            if let Err(e) = engine::assert_fingerprint_form(
                &show,
                &spec::render_netem_spec_leg(&s.spec),
                rd_form,
            ) {
                eprintln!(
                    "weaknet: WARN qdisc rewritten externally (another instance/manual tc/another clone): {}",
                    e.msg
                );
            }
            let rem = if s.is_forever() {
                "forever".to_string()
            } else {
                let now = fuse::now_epoch_ms();
                if now >= s.expires_at_ms {
                    "expired (lazy self-heal will clear it)".to_string()
                } else {
                    format!("{}s", (s.expires_at_ms - now) / 1000)
                }
            };
            println!(
                "== status (state summary, channel={} iface={dev} remaining {rem}, watchdog {}) ==",
                chan.describe(),
                watchdog_state(dirs)
            );
            println!("{}", state::param_summary(&s.spec));
            let scope_detail = if s.pairs.is_empty() {
                format!(
                    "ports=[{}]",
                    s.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
                )
            } else {
                format!(
                    "pairs=[{}]",
                    s.pairs.iter().map(|(l, r)| format!("{l}:{r}")).collect::<Vec<_>>().join(" "),
                )
            };
            println!(
                "scope={:?} dir={:?} {scope_detail} teardown {} steps",
                s.scope,
                s.dir,
                s.teardown.steps.len()
            );
            println!("== tc live ({dev}) ==");
            for l in &observed {
                println!("{l}");
            }
            if let Ok(classes) = engine::tc_exec(&chan, &["class", "show", "dev", &dev]) {
                for l in classes.lines().take(3) {
                    println!("{l}");
                }
            }
            if let Ok(v) = engine::tc_exec(&chan, &["-V"]) {
                println!("== tc version ==\n{}", v.trim());
            }
            if let Ok(raw) = std::fs::read_to_string(dirs.timeline()) {
                let lines: Vec<&str> = raw.lines().collect();
                println!("== timeline tail 3 ==");
                for l in lines.iter().rev().take(3).rev() {
                    println!("{l}");
                }
            }
        }
    }
    Ok(())
}

/// watchdog liveness summary (truth source lifted into watch.rs, shared with --watch).
fn watchdog_state(dirs: &Dirs) -> String {
    watch::watchdog_value(dirs).into()
}

// ---------- scenario ----------

fn do_scenario(sub: ScenarioCmd, dirs: &Dirs, env: &Env) -> Wn<()> {
    match sub {
        ScenarioCmd::Run { file, dir, no_baseline, keep, auto_clear } => {
            let file = file.ok_or_else(|| {
                Fail::bad_param(
                    "scenario run needs --file (<name|*.yaml> [--no-baseline|--keep]; see -h)",
                )
            })?;
            let (raw, name, disp) = scenario::read_run_yaml(&file, dir.as_deref())?;
            let plan = scenario::parse_plan_yaml(&raw).map_err(Fail::bad_param)?;
            // targeting = room-level stats (bash do_scenario has no --stream; default same as do_apply)
            let targeting = scope::resolve_targeting(None, None, None, &[])?;
            let rs = scenario::RunSpec {
                name,
                file_disp: disp,
                plan,
                no_baseline,
                keep,
                targeting,
                iface: engine::resolve_iface(None),
                sig_port: None,
                recovery_dwell_secs: None,
            };
            let exec: Arc<dyn scenario::StepExec> =
                Arc::new(scenario::EngineExec::new(dirs.clone(), env.clone()));
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| Fail::env(format!("failed to build the tokio runtime: {e}")))?;
            // serve-side /v1/scenario/run does not go through the signal layer (long-lived process, its stop surface already has the /v1/scenario/stop
            // endpoint) -- --auto-clear only applies to this CLI foreground path (out-of-scope note, adjudication 4 in the plan).
            let out = if auto_clear {
                rt.block_on(scenario::run_full_auto_clear(rs, exec, dirs.clone()))
            } else {
                rt.block_on(scenario::run_full(rs, exec, dirs.clone()))
            };
            println!("{}", out.summary);
            if out.exit_code != 0 {
                return Err(Fail {
                    code: out.exit_code,
                    msg: out.aborted.unwrap_or_else(|| "scenario failed (see above)".into()),
                });
            }
            Ok(())
        }
        ScenarioCmd::List => scenario_list(),
        ScenarioCmd::Stop => {
            println!("{}", scenario::stop(dirs)?);
            Ok(())
        }
    }
}

fn scenario_list() -> Wn<()> {
    // T13 vehicle surface: fs probe chain union compile-time embedded (single source = scope::list_assets, same shape as serve /v1/scenarios).
    let names = scope::list_assets("scenarios");
    if names.is_empty() {
        return Err(Fail::env(
            "no scenario assets (WEAKNET_ASSETS_DIR > weaknet.d next to the binary > ancestor scripts/weaknet.d > embedded, all empty)",
        ));
    }
    for n in names {
        println!("{n}");
    }
    Ok(())
}

/// profile/scenario asset resolution (T6 lifted to the lib surface scope::weaknet_d_root -- single truth source for CLI/serve).
fn weaknet_d_root() -> Option<PathBuf> {
    scope::weaknet_d_root()
}

// ---------- profile (serde_yaml port of emit.py whitelist semantics) ----------

fn load_profile(name: &str) -> Wn<ImpairSpec> {
    let parse = |raw: &str, disp: &str| -> Wn<ImpairSpec> {
        let doc: Value = serde_yaml::from_str(raw)
            .map_err(|e| Fail::bad_param(format!("invalid profile YAML {disp}: {e}")))?;
        config::spec_from_profile_doc(&doc).map_err(Fail::bad_param)
    };
    // 1) file form (with extension): read fs directly (bash same slot).
    if Path::new(name).extension().is_some_and(|e| e == "yaml" || e == "yml") {
        let p = PathBuf::from(name);
        if !p.is_file() {
            return Err(Fail::bad_param(format!("no profile file: {name}")));
        }
        let raw = std::fs::read_to_string(&p)
            .map_err(|e| Fail::env(format!("failed to read profile {}: {e}", p.display())))?;
        return parse(&raw, &p.display().to_string());
    }
    // 2) name form: fs asset dir first (the vehicle-surface resolution single function = scope::read_asset).
    let fs_hit = weaknet_d_root()
        .map(|root| root.join("profiles").join(format!("{name}.yaml")))
        .filter(|p| p.is_file());
    if let Some(p) = fs_hit {
        let raw = std::fs::read_to_string(&p)
            .map_err(|e| Fail::env(format!("failed to read profile {}: {e}", p.display())))?;
        return parse(&raw, &p.display().to_string());
    }
    // 3) embedded fallback (T13: vehicle scp single file `up smoke` with zero asset deps; name form only, path traversal never enters embedded).
    if scenario::is_safe_asset_name(name) {
        let rel = scenario::asset_rel("profiles", name);
        if let Some(raw) = scope::read_asset(&rel)? {
            return parse(&raw, &format!("embedded {rel}"));
        }
    }
    Err(Fail::bad_param(format!(
        "no profile: {name} (available: {}; sources: WEAKNET_ASSETS_DIR / weaknet.d next to the binary / scripts/weaknet.d / embedded)",
        scope::list_assets("profiles").join(" ")
    )))
}

// ---------- __watchdog executor (design §fuse rev-2.2 cross-channel contract) ----------

/// On expiry, execute per the plan: read state.teardown.channel/sidecar -> run_teardown (a dead sidecar takes the three-layer fallback) ->
/// rm state -> timeline `watchdog-clear`; failure -> `watchdog-clear-failed`+err (C15, never silent);
/// not-exist class already counted as idempotent success inside run_teardown (noise guard for two watchdogs firing concurrently).
fn watchdog_expire(state_path: &Path) {
    let dirs = Dirs { statedir: state_path.parent().unwrap_or(Path::new(".")).to_path_buf() };
    match State::read_from(state_path) {
        Ok(Some(st)) => {
            let failures = engine::run_teardown(&st);
            for f in &failures {
                eprintln!("weaknet watchdog: WARN teardown: {f}");
            }
            if let Err(e) = std::fs::remove_file(state_path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("weaknet watchdog: WARN failed to delete state: {e}");
            }
            let ev = if failures.is_empty() {
                json!({"ev": "watchdog-clear"})
            } else {
                json!({"ev": "watchdog-clear-failed", "err": failures.join("; ")})
            };
            if let Err(e) = state::timeline_append(&dirs, ev) {
                eprintln!("weaknet watchdog: WARN timeline failed: {e}");
            }
        }
        Ok(None) => {
            eprintln!("weaknet watchdog: state missing (already cleared?) -- idempotent exit");
        }
        Err(e) => {
            eprintln!("weaknet watchdog: WARN {e}");
            if let Err(te) =
                state::timeline_append(&dirs, json!({"ev": "watchdog-clear-failed", "err": e}))
            {
                eprintln!("weaknet watchdog: WARN timeline failed: {te}");
            }
        }
    }
}
