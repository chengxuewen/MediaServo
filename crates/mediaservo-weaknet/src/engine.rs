//! engine.rs -- TcEngine: channel probing (local-root / docker NET_ADMIN sidecar) + full skeleton argv chain
//! (verbatim parity with bash build_skeleton, add-or-change 2015-compatible) + spec-distinguishing read-back fingerprint +
//! leaf Sent double-sampling verify / counter-reset detection + fail-closed guard + teardown plan/execution.
//!
//! Source of truth: scripts/weaknet.sh (mechanism) + design.md §dir table (legs) + §ifb contract (T9: physical-port
//! ingress goes through the ifb0 mirror; chain steps in plan_ifb; capability verdict single-sourced in ifb.rs).
//!
//! Exit codes inherited: 2 environment insufficient / apply failed, 3 state conflict (foreign qdisc/lock), 4 invalid params (C15: every branch reports cause).

use std::process::Command;
use std::time::Duration;

use crate::fuse;
use crate::ifb;
use crate::spec::{
    self, Dir, IfaceKind, ImpairSpec, LegFilter, MatchKind, ScopeSel, build_filter_legs,
    render_netem_spec_leg, render_rate_arg,
};
use crate::state::{self, ChannelSer, Dirs, SidecarRef, State, Teardown};

/// Unified failure payload (code = bash exit-code semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: u32,
    pub msg: String,
}

impl Fail {
    #[must_use]
    pub fn env(msg: impl Into<String>) -> Self {
        Self { code: 2, msg: msg.into() }
    }
    #[must_use]
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self { code: 3, msg: msg.into() }
    }
    #[must_use]
    pub fn bad_param(msg: impl Into<String>) -> Self {
        Self { code: 4, msg: msg.into() }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl From<String> for Fail {
    fn from(msg: String) -> Self {
        Self::env(msg)
    }
}

pub type Wn<T> = Result<T, Fail>;

// ---------- Runtime environment (env read once, centralized; tests can construct manually) ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Env {
    /// WEAKNET_CHANNEL = auto|local|sidecar (design §engine vocabulary)
    pub channel_pref: String,
    /// sidecar container name (bash SIDE=weaknet-ctrl default; C20 = documented constant + env override)
    pub sidecar: String,
    /// iproute2 privileged image (gaiadocker/iproute2 -- the 2015-semantics build; pitfalls in the bash header note)
    pub image: String,
}

pub const DEF_SIDECAR: &str = "weaknet-ctrl";
pub const DEF_IMAGE: &str = "gaiadocker/iproute2";
/// Mirrors bash `DEV="${WEAKNET_DEV:-lo}"` default (C20 exemption shape = documented constant + env/flag override).
pub const DEFAULT_IFACE: &str = "lo";

/// Unified iface resolution chain (shared by CLI and serve; lifted from main.rs at T6, single source of truth).
#[must_use]
pub fn resolve_iface(cli: Option<&str>) -> String {
    cli.map(str::to_owned)
        .or_else(|| std::env::var("WEAKNET_DEV").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| DEFAULT_IFACE.to_string())
}

impl Default for Env {
    fn default() -> Self {
        Self { channel_pref: "auto".into(), sidecar: DEF_SIDECAR.into(), image: DEF_IMAGE.into() }
    }
}

impl Env {
    #[must_use]
    pub fn from_env() -> Self {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Self {
            channel_pref: var("WEAKNET_CHANNEL", "auto").to_lowercase(),
            sidecar: var("WEAKNET_SIDECAR", DEF_SIDECAR),
            image: var("WEAKNET_IMAGE", DEF_IMAGE),
        }
    }
}

// ---------- Process execution ----------

fn run(argv: &[&str]) -> Wn<String> {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| Fail::env(format!("failed to start `{}`: {e}", argv[0])))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Fail::env(format!(
            "`{}` failed (rc={:?}): {}",
            argv.join(" "),
            out.status.code(),
            stderr.trim()
        )));
    }
    Ok(stdout)
}

/// root privilege detection (/proc/self/status Uid line euid field, zero libc). Err = predicate unavailable (not root).
fn euid_is_root() -> Result<bool, String> {
    let raw = std::fs::read_to_string("/proc/self/status")
        .map_err(|e| format!("/proc/self/status unreadable: {e}"))?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let euid = rest
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| format!("Uid line missing fields: {line}"))?;
            return Ok(euid == "0");
        }
    }
    Err("Uid line not found".to_string())
}

// ---------- Channel ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    LocalRoot,
    Sidecar { container: String, image: String },
}

impl Channel {
    /// tc subcommand argv (without program name) -> full argv including program name.
    fn tc_argv(&self, args: &[&str]) -> Vec<String> {
        let mut full = vec!["tc".to_string()];
        full.extend(args.iter().map(|s| (*s).to_string()));
        self.prefixed(&full)
    }

    /// Channel real-execution prefix: local as-is, sidecar via `docker exec <c>` (same shape for any iproute2 program).
    fn prefixed(&self, argv: &[String]) -> Vec<String> {
        match self {
            Channel::LocalRoot => argv.to_vec(),
            Channel::Sidecar { container, .. } => {
                let mut v = vec!["docker".to_string(), "exec".to_string(), container.clone()];
                v.extend(argv.iter().cloned());
                v
            }
        }
    }

    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Channel::LocalRoot => "local-root".into(),
            Channel::Sidecar { container, .. } => format!("sidecar({container})"),
        }
    }
}

/// tc real execution (args = argv without the `tc` prefix).
pub fn tc_exec(channel: &Channel, args: &[&str]) -> Wn<String> {
    let argv = channel.tc_argv(args);
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    run(&refs)
}

/// Any iproute2 program real execution (argv[0] = program name, e.g. `["ip","link","show","ifb0"]`;
/// the T9 ifb path uses `ip`; the sidecar channel prefixes docker exec the same way).
pub fn exec_prog(channel: &Channel, argv: &[&str]) -> Wn<String> {
    let owned: Vec<String> = argv.iter().map(|s| (*s).to_string()).collect();
    let full = channel.prefixed(&owned);
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    run(&refs)
}

/// Probe/establish channel (auto: local predicate = euid0 AND tc -V available; else sidecar ensure; both failing = exit2 with both causes).
pub fn probe_channel(env: &Env, iface: &str) -> Wn<Channel> {
    match env.channel_pref.as_str() {
        "local" => local_channel().map_err(|cause| {
            Fail::env(format!(
                "WEAKNET_CHANNEL=local requested but the local root channel is unavailable: {cause}"
            ))
        }),
        "sidecar" => ensure_sidecar(env, iface).map_err(Fail::env).map(|()| Channel::Sidecar {
            container: env.sidecar.clone(),
            image: env.image.clone(),
        }),
        "auto" => {
            let local_cause = match local_channel() {
                Ok(c) => return Ok(c),
                Err(e) => e,
            };
            match ensure_sidecar(env, iface) {
                Ok(()) => Ok(Channel::Sidecar {
                    container: env.sidecar.clone(),
                    image: env.image.clone(),
                }),
                Err(side_cause) => Err(Fail::env(format!(
                    "no usable privileged channel: local({local_cause}) / sidecar({side_cause}) -- retry once host root+iproute2 or docker (docker group) is in place"
                ))),
            }
        }
        other => Err(Fail::bad_param(format!(
            "WEAKNET_CHANNEL must be auto|local|sidecar, got {other:?}"
        ))),
    }
}

fn local_channel() -> Result<Channel, String> {
    if !euid_is_root().unwrap_or_else(|e| {
        eprintln!("weaknet(engine): root predicate: {e}");
        false
    }) {
        return Err("not root (tc needs CAP_NET_ADMIN)".to_string());
    }
    run(&["tc", "-V"]).map(|_| Channel::LocalRoot).map_err(|e| e.msg)
}

/// sidecar lifecycle (branch-by-branch port of bash ensure_sidecar: running -> start -> pull -> run -> instant-exit forensics ->
/// tc reachable -> netem module check/load only, never test-mount; PIT-184 guard).
pub fn ensure_sidecar(env: &Env, iface: &str) -> Result<(), String> {
    run(&["docker", "--version"]).map_err(|e| {
        format!("docker unavailable (weaknet privileged channel missing): {}", e.msg)
    })?;
    let running = run(&["docker", "ps", "-q", "-f", &format!("name=^{}$", env.sidecar)])
        .map_err(|e| e.msg)?;
    if !running.trim().is_empty() {
        return Ok(());
    }
    let existed = run(&["docker", "ps", "-aq", "-f", &format!("name=^{}$", env.sidecar)])
        .map_err(|e| e.msg)?;
    if !existed.trim().is_empty() {
        run(&["docker", "start", &env.sidecar])
            .map_err(|e| format!("sidecar start failed (docker permissions/daemon?): {}", e.msg))?;
        return sidecar_ready_check(env, iface);
    }
    if run(&["docker", "image", "inspect", &env.image]).is_err() {
        eprintln!("weaknet(engine): image missing, attempting pull: docker pull {}", env.image);
        run(&["docker", "pull", &env.image]).map_err(|e| {
            format!(
                "failed to pull {} (internal proxy? pull manually and retry): {}",
                env.image, e.msg
            )
        })?;
    }
    run(&[
        "docker",
        "run",
        "-d",
        "--name",
        &env.sidecar,
        "--cap-add",
        "NET_ADMIN",
        "--net",
        "host",
        "--entrypoint",
        "sh",
        &env.image,
        "-c",
        "while :; do sleep 3600; done",
    ])
    .map_err(|e| format!("sidecar creation failed: {}", e.msg))?;
    sidecar_ready_check(env, iface)
}

fn sidecar_ready_check(env: &Env, iface: &str) -> Result<(), String> {
    // bash `sleep 0.5` then instant-exit forensics (image entrypoint while-loop keepalive is the port for 2015 versions lacking sleep infinity)
    std::thread::sleep(Duration::from_millis(500));
    let running = run(&["docker", "ps", "-q", "-f", &format!("name=^{}$", env.sidecar)])
        .map_err(|e| e.msg)?;
    if running.trim().is_empty() {
        let logs = run(&["docker", "logs", "--tail", "2", &env.sidecar])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|e| format!("<failed to read logs: {}>", e.msg));
        return Err(format!("sidecar exited immediately (docker logs tail): {logs}"));
    }
    let chan = Channel::Sidecar { container: env.sidecar.clone(), image: env.image.clone() };
    tc_exec(&chan, &["qdisc", "show", "dev", iface])
        .map_err(|e| format!("tc unreachable inside sidecar for {iface}: {}", e.msg))?;
    // module liveness probe: check/load only, never test-mount (the root slot is often held by us -- PIT-184-family guard)
    run(&[
        "docker",
        "exec",
        &env.sidecar,
        "sh",
        "-c",
        "lsmod | grep -q netem || modprobe sch_netem",
    ])
    .map_err(|e| {
        format!(
            "sch_netem not loadable (run modprobe sch_netem as host root, then retry): {}",
            e.msg
        )
    })?;
    Ok(())
}

/// seed version gate (bash seed_gate: iproute2 >= 6.6 for seed to be semantically valid; date-coded versions = non-semantic, report cause directly).
pub fn version_gate_seed(channel: &Channel) -> Wn<()> {
    let v = tc_exec(channel, &["-V"])?;
    let Some((maj, min)) = parse_iproute2_semver(&v) else {
        return Err(Fail::bad_param(format!(
            "tc is not a semantic version ({}), seed needs iproute2 >= 6.6",
            v.trim()
        )));
    };
    if (maj, min) >= (6, 6) {
        Ok(())
    } else {
        Err(Fail::bad_param(format!(
            "tc={maj}.{min} < 6.6 does not support seed (seed is also only reproducible for GE loss; bash header note 5)"
        )))
    }
}

pub(crate) fn parse_iproute2_semver(out: &str) -> Option<(u32, u32)> {
    // Two real forms: `iproute2-6.11.0` (new, hyphen) / `iproute2 ss150831` (old, space) -- both separators tolerated.
    let after = out.split("iproute2").nth(1)?;
    let mut it = after.trim_start_matches(['-', ' ', ':']).split('.');
    let maj = it.next()?.parse().ok()?;
    let min = it.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((maj, min))
}

// ---------- Pure planning (dry-run / snapshot-test predicate source, zero kernel touching) ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootAction {
    /// root absent/default class qdisc -> del (best-effort) + add htb
    Add,
    /// htb 1: already present (our own site continuing) -> skip the root step, incremental change
    Change,
    /// bare root netem (--all compat form, not exposed by the M0 CLI) -> replace root netem
    Replace,
}

/// First root-line predicate (bash: no htb -> add; `^qdisc htb 1:` present -> change-only).
#[must_use]
pub fn root_action(qdisc_show: &str) -> RootAction {
    let first_root = qdisc_show.lines().find(|l| l.starts_with("qdisc "));
    match first_root {
        Some(l) if l.starts_with("qdisc htb") => RootAction::Change,
        Some(l) if l.starts_with("qdisc netem") && l.contains("root") => RootAction::Replace,
        _ => RootAction::Add,
    }
}

/// Execution plan (T9: the ifb chain contains `ip link` steps, sharing StepPlan/channel prefix machinery with tc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prog {
    Tc,
    Ip,
}

impl Prog {
    #[must_use]
    pub fn str(self) -> &'static str {
        match self {
            Prog::Tc => "tc",
            Prog::Ip => "ip",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPlan {
    /// main argv (without program name; assembled with prog at execution/display time)
    pub run: Vec<String>,
    pub prog: Prog,
    /// 2015 compat: change failure falls back to add (bash `change 2>/dev/null || add`)
    pub alt: Option<Vec<String>>,
    /// best-effort step failure is noted but not fatal (del root / filter flush)
    pub best_effort: bool,
    pub what: String,
}

impl StepPlan {
    /// dry-run display lines (bash do_apply --dry-run shape: main form shown, best_effort steps omitted).
    #[must_use]
    pub fn display_lines(steps: &[StepPlan]) -> Vec<String> {
        steps
            .iter()
            .filter(|s| !s.best_effort)
            .map(|s| format!("{} {}", s.prog.str(), s.run.join(" ")))
            .collect()
    }
}

/// Skeleton step sequence (bash build_skeleton exact order): root (add form includes a preceding best-effort del) ->
/// 1:99 default class sentinel -> 1:10 class rate_arg -> netem leaf spec -> filters full rebuild (flush+legs) ->
/// optional signaling leg (prio 2 / protocol 6).
#[must_use]
pub fn plan_skeleton(
    dev: &str,
    add_root: bool,
    netem_spec: &str,
    rate_arg: &str,
    legs: &[LegFilter],
    sig_port: Option<u16>,
) -> Vec<StepPlan> {
    let mut steps = Vec::new();
    if add_root {
        steps.push(StepPlan {
            run: vec!["qdisc".into(), "del".into(), "dev".into(), dev.into(), "root".into()],
            prog: Prog::Tc,
            alt: None,
            best_effort: true,
            what: "pre-clear root (tolerates --all residue)".into(),
        });
        steps.push(StepPlan {
            run: vec![
                "qdisc".into(),
                "add".into(),
                "dev".into(),
                dev.into(),
                "root".into(),
                "handle".into(),
                "1:".into(),
                "htb".into(),
                "default".into(),
                "99".into(),
            ],
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: "htb root establish".into(),
        });
    }
    steps.push(change_or_add(
        "default class 1:99 sentinel",
        dev,
        &["parent", "1:", "classid", "1:99", "htb", "rate", spec::SENTINEL_RATE],
    ));
    steps.push(change_or_add(
        &format!("netem carrying class 1:10 rate={rate_arg}"),
        dev,
        &["parent", "1:", "classid", "1:10", "htb", "rate", rate_arg, "ceil", spec::SENTINEL_RATE],
    ));
    let mut leaf: Vec<String> = vec![
        "qdisc".into(),
        "change".into(),
        "dev".into(),
        dev.into(),
        "parent".into(),
        "1:10".into(),
        "handle".into(),
        "10:".into(),
        "netem".into(),
    ];
    leaf.extend(netem_spec.split_whitespace().map(str::to_string));
    let add_leaf: Vec<String> = {
        let mut v = leaf.clone();
        v[1] = "add".into();
        v
    };
    steps.push(StepPlan {
        run: leaf,
        prog: Prog::Tc,
        alt: Some(add_leaf),
        best_effort: false,
        what: "netem leaf 10:".into(),
    });
    steps.push(StepPlan {
        run: vec![
            "filter".into(),
            "del".into(),
            "dev".into(),
            dev.into(),
            "parent".into(),
            "1:".into(),
        ],
        prog: Prog::Tc,
        alt: None,
        best_effort: true,
        what: "pre-flush before full filter rebuild (idempotent)".into(),
    });
    for leg in legs {
        steps.push(StepPlan {
            run: filter_add_argv(dev, "1", leg),
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: format!("media leg filter {:?}", leg.matches),
        });
    }
    if let Some(port) = sig_port {
        let leg = LegFilter {
            protocol: 6,
            matches: vec![(MatchKind::Dport, port)],
            flowid: spec::MEDIA_FLOWID.to_string(),
        };
        steps.push(StepPlan {
            run: filter_add_argv(dev, "2", &leg),
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: format!("signaling leg filter dport={port} (TCP)"),
        });
    }
    steps
}

fn change_or_add(what: &str, dev: &str, rest: &[&str]) -> StepPlan {
    let mk = |verb: &str| -> Vec<String> {
        let mut v = vec!["class".to_string(), verb.to_string(), "dev".to_string(), dev.to_string()];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    };
    StepPlan {
        run: mk("change"),
        prog: Prog::Tc,
        alt: Some(mk("add")),
        best_effort: false,
        what: what.into(),
    }
}

fn filter_add_argv(dev: &str, prio: &str, leg: &LegFilter) -> Vec<String> {
    let mut fixed: Vec<String> = vec![
        "filter".into(),
        "add".into(),
        "dev".into(),
        dev.into(),
        "parent".into(),
        "1:".into(),
        "protocol".into(),
        "ip".into(),
        "prio".into(),
        prio.into(),
        "u32".into(),
        "match".into(),
        "ip".into(),
        "protocol".into(),
        leg.protocol.to_string(),
        "0xff".into(),
    ];
    for (kind, port) in &leg.matches {
        let k = match kind {
            MatchKind::Sport => "sport",
            MatchKind::Dport => "dport",
        };
        fixed.extend(["match".into(), "ip".into(), k.into(), port.to_string(), "0xffff".into()]);
    }
    fixed.extend(["flowid".into(), leg.flowid.clone()]);
    fixed
}

/// Bare root netem (--all compat form, not exposed by the M0 CLI -- used to compare against the bash state before T10).
#[must_use]
pub fn plan_all_root(dev: &str, netem_spec: &str) -> Vec<StepPlan> {
    let mut run: Vec<String> = vec![
        "qdisc".into(),
        "replace".into(),
        "dev".into(),
        dev.into(),
        "root".into(),
        "netem".into(),
    ];
    run.extend(netem_spec.split_whitespace().map(str::to_string));
    vec![StepPlan {
        run,
        prog: Prog::Tc,
        alt: None,
        best_effort: false,
        what: "--all bare root netem".into(),
    }]
}

// ---------- T9: ifb ingress mirror (design §ifb contract) ----------

/// physical port x dir in {In,Both} -> the ingress leg must go through the ifb0 mirror path (§dir table: physical NIC with dir=in:
/// the port leg moves to ifb). lo is always a pure port leg (dir_lo capability always true, §capability table).
#[must_use]
pub fn ifb_needed(kind: IfaceKind, dir: Dir) -> bool {
    kind == IfaceKind::Physical && matches!(dir, Dir::In | Dir::Both)
}

/// fail-closed gate (error classification predicate): ifb required but probe fails -> exit2 with cause, never silently degrade to
/// direct ingress without a mirror (the original black-hole-triple sin). Pure predicate, feeds the unit-test matrix.
pub fn guard_ifb_required(kind: IfaceKind, dir: Dir, probe: (bool, String)) -> Wn<bool> {
    if !ifb_needed(kind, dir) {
        return Ok(false);
    }
    if probe.0 {
        return Ok(true);
    }
    Err(Fail::env(format!(
        "physical-port ingress (dir={:?}) needs {IFB_NOTE}, capability probe failed: {} -- fix = modprobe ifb as host root (netns cannot load modules unprivileged)",
        dir, probe.1
    )))
}

const IFB_NOTE: &str = "ifb0 mirror";

/// ifb mirror chain steps (§ifb contract apply order, incl. link up -- missing the up step = mirred redirect onto a DOWN device
/// = ENETDOWN dropping everything while read-back shows the line exists: a green-wash source). Leaf decision (recorded this round):
/// **netem directly on ifb0 root** (contract text "tc qdisc add dev ifb0 root netem...", minimal form),
/// not mirroring the htb skeleton onto ifb0 -- cost = the rate wall does not apply on the ingress leg, WARNed at replay;
/// upgrade path = port plan_skeleton's root/class section when ifb-side rate limiting is needed.
/// `need_link` is decided by read-before-probe in apply (ifb0 already exists = created by someone else/the system: no re-add, teardown won't delete it).
#[must_use]
pub fn plan_ifb(dev: &str, netem_spec: &str, legs: &[LegFilter], need_link: bool) -> Vec<StepPlan> {
    let mut steps = Vec::new();
    if need_link {
        steps.push(StepPlan {
            run: vec![
                "link".into(),
                "add".into(),
                ifb::IFB_DEV.into(),
                "type".into(),
                "ifb".into(),
            ],
            prog: Prog::Ip,
            alt: None,
            best_effort: false,
            what: format!("{} establish (this session is the owner)", ifb::IFB_DEV),
        });
    }
    steps.push(StepPlan {
        run: vec!["link".into(), "set".into(), ifb::IFB_DEV.into(), "up".into()],
        prog: Prog::Ip,
        alt: None,
        best_effort: false,
        what: format!(
            "{} link up (missing this step = ENETDOWN black hole, §ifb contract)",
            ifb::IFB_DEV
        ),
    });
    let mut root: Vec<String> = vec![
        "qdisc".into(),
        "change".into(),
        "dev".into(),
        ifb::IFB_DEV.into(),
        "root".into(),
        "netem".into(),
    ];
    root.extend(netem_spec.split_whitespace().map(str::to_string));
    let add_root: Vec<String> = {
        let mut v = root.clone();
        v[1] = "add".into();
        v
    };
    steps.push(StepPlan {
        run: root,
        prog: Prog::Tc,
        alt: Some(add_root),
        best_effort: false,
        what: format!("{} root netem leaf", ifb::IFB_DEV),
    });
    steps.push(StepPlan {
        run: vec!["qdisc".into(), "add".into(), "dev".into(), dev.into(), "ingress".into()],
        prog: Prog::Tc,
        alt: None,
        best_effort: true, // already exists (set replay) = File exists tolerated; a real create failure is reported by the next filter step
        what: "iface ingress qdisc (mirror attach point)".into(),
    });
    steps.push(StepPlan {
        run: vec!["filter".into(), "del".into(), "dev".into(), dev.into(), "ingress".into()],
        prog: Prog::Tc,
        alt: None,
        best_effort: true,
        what: "pre-flush before full ingress filter rebuild (idempotent)".into(),
    });
    for leg in legs {
        steps.push(StepPlan {
            run: mirred_filter_argv(dev, leg),
            prog: Prog::Tc,
            alt: None,
            best_effort: false,
            what: format!("ingress mirror leg filter {:?} -> {}", leg.matches, ifb::IFB_DEV),
        });
    }
    steps
}

/// ingress mirror filter: same conjunctive form as the egress leg (protocol 17 + port AND, single filter), action swapped to
/// `mirred egress redirect dev ifb0` (§ifb contract; no parent 1:/flowid -- the ingress qdisc has no classes).
fn mirred_filter_argv(dev: &str, leg: &LegFilter) -> Vec<String> {
    let mut fixed: Vec<String> = vec![
        "filter".into(),
        "add".into(),
        "dev".into(),
        dev.into(),
        "ingress".into(),
        "protocol".into(),
        "ip".into(),
        "u32".into(),
        "match".into(),
        "ip".into(),
        "protocol".into(),
        leg.protocol.to_string(),
        "0xff".into(),
    ];
    for (kind, port) in &leg.matches {
        let k = match kind {
            MatchKind::Sport => "sport",
            MatchKind::Dport => "dport",
        };
        fixed.extend(["match".into(), "ip".into(), k.into(), port.to_string(), "0xffff".into()]);
    }
    fixed.extend([
        "action".into(),
        "mirred".into(),
        "egress".into(),
        "redirect".into(),
        "dev".into(),
        ifb::IFB_DEV.into(),
    ]);
    fixed
}

pub fn iface_kind_of(dev: &str) -> IfaceKind {
    if dev == "lo" { IfaceKind::Loopback } else { IfaceKind::Physical }
}

// ---------- fail-closed guard (pure functions over read-back text) ----------

/// bash guard_foreign_root: root-slot allowlist (noqueue/fq_codel/"fq "/pfifo_fast) passes;
/// htb/netem requires our own state to prove ownership; anything else = foreign occupant, exit3.
pub fn guard_foreign_root(qdisc_show: &str, state_exists: bool) -> Wn<()> {
    let Some(line) = qdisc_show.lines().find(|l| l.starts_with("qdisc ")) else {
        return Ok(());
    };
    if ["noqueue", "fq_codel", "pfifo_fast"].iter().any(|p| line.contains(p))
        || line.contains(" fq ")
    {
        return Ok(());
    }
    if line.starts_with("qdisc htb") {
        return if state_exists {
            Ok(())
        } else {
            Err(Fail::conflict(
                "lo has an htb root but no weaknet state (foreign/residue) -- reset first with weaknet clear",
            ))
        };
    }
    if line.starts_with("qdisc netem") {
        return Err(Fail::conflict(format!(
            "root's netem is not ours (foreign/--all residue/manual) -- refusing to touch it: {line}"
        )));
    }
    Err(Fail::conflict(format!(
        "root slot occupied by a foreign party: {line} (weaknet refuses to touch another party's qdisc)"
    )))
}

// ---------- Read-back fingerprint ----------

fn norm_words(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|w| {
            let w = w.trim_matches(|c: char| c == '(' || c == ')' || c == '%');
            normalize_num_token(w)
        })
        .collect()
}

/// Numeric token normalization: old tc echoes "delay 40.0ms" (float with trailing zero) while the spec form is "delay 40ms" -- strip
/// decimal trailing zeros the same way on both sides ("40.0ms"->"40ms", "2.5%"->"2.5%", "60.00"->"60"). Non-numeric segments verbatim.
#[must_use]
fn normalize_num_token(w: &str) -> String {
    let unit_at = w.find(|c: char| !c.is_ascii_digit() && c != '.');
    let (num, unit) = match unit_at {
        Some(i) => (&w[..i], &w[i..]),
        None => (w, ""),
    };
    if !num.contains('.') {
        return w.to_string();
    }
    let trimmed = num.trim_end_matches('0').trim_end_matches('.');
    format!("{trimmed}{unit}")
}

/// Spec-distinguishing token set: numeric/unit tokens + pattern keywords (gemodel/loss/reorder/seed/delay/limit).
/// "distribution normal" deliberately excluded -- old tc never echoes the distribution word (2015 mirror proven), jitter is carried by the "{j}ms"
/// numeric token. Normalization synced with [`normalize_num_token`] on both sides (the old mirror echoes delay as float).
#[must_use]
pub fn fingerprint_tokens(netem_spec: &str) -> Vec<String> {
    netem_spec
        .split_whitespace()
        .map(|w| normalize_num_token(w.trim_end_matches('%')))
        .filter(|w| w.chars().any(|c| c.is_ascii_digit()) || SPEC_KEYWORDS.contains(&w.as_str()))
        .collect()
}

const SPEC_KEYWORDS: &[&str] = &["limit", "delay", "loss", "gemodel", "reorder", "seed"];

/// Leaf-line form (T9: on the ifb path netem sits on ifb0 root, no parent 1:10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafForm {
    /// egress skeleton: `qdisc netem ... parent 1:10`.
    Parent110,
    /// ifb0 ingress path (§ifb contract minimal form): `qdisc netem ... root`.
    Root,
}

fn leaf_line_matches(l: &str, form: LeafForm) -> bool {
    l.contains("netem")
        && match form {
            LeafForm::Parent110 => l.contains("parent 1:10"),
            LeafForm::Root => l.contains(" root"),
        }
}

/// Leaf line = first line matching the form.
#[must_use]
pub fn find_leaf_line_form(qdisc_show: &str, form: LeafForm) -> Option<&str> {
    qdisc_show.lines().find(|l| leaf_line_matches(l, form))
}

/// Leaf line = first line containing netem with parent 1:10 (egress skeleton form).
#[must_use]
pub fn find_leaf_line(qdisc_show: &str) -> Option<&str> {
    find_leaf_line_form(qdisc_show, LeafForm::Parent110)
}

pub fn assert_fingerprint(qdisc_show: &str, netem_spec: &str) -> Wn<()> {
    assert_fingerprint_form(qdisc_show, netem_spec, LeafForm::Parent110)
}

/// Fingerprint assert: every distinguishing token of the spec must appear in the leaf line (normalized token set) -- missing list got/want on failure.
pub fn assert_fingerprint_form(qdisc_show: &str, netem_spec: &str, form: LeafForm) -> Wn<()> {
    let leaf = find_leaf_line_form(qdisc_show, form).ok_or_else(|| {
        Fail::env(match form {
            LeafForm::Parent110 => "read-back: 1:10 leaf has no netem (apply did not take effect = PIT-184-family green-wash predicate)".to_string(),
            LeafForm::Root => format!("read-back: {} root has no netem (ifb mirror did not take effect)", ifb::IFB_DEV),
        })
    })?;
    let words = norm_words(leaf);
    let want = fingerprint_tokens(netem_spec);
    let missing: Vec<&str> =
        want.iter().filter(|t| !words.iter().any(|w| w == *t)).map(String::as_str).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Fail::env(format!(
            "read-back fingerprint mismatch: missing {:?} (leaf line current value: {leaf})",
            missing
        )))
    }
}

/// `-s` leaf-block statistics (Sent pkt + dropped) -- data source for the tc field of the serve state frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafStats {
    pub sent_pkt: u64,
    pub dropped: u64,
}

/// `-s` output leaf block `Sent X bytes Y pkt` -> Y (port of bash leaf_pkt; scans the leaf line and the line after it).
#[must_use]
pub fn leaf_sent(qdisc_s_show: &str) -> Option<u64> {
    leaf_stats(qdisc_s_show).map(|s| s.sent_pkt)
}

/// Parses `Sent _ bytes N pkt (dropped M, ...)` in the leaf-line + next-line window; missing dropped = 0 (tolerates old tc).
#[must_use]
pub fn leaf_stats(qdisc_s_show: &str) -> Option<LeafStats> {
    leaf_stats_form(qdisc_s_show, LeafForm::Parent110)
}

/// Form-aware version (T9: ifb0 root netem leaf, same extraction mechanism).
#[must_use]
pub fn leaf_stats_form(qdisc_s_show: &str, form: LeafForm) -> Option<LeafStats> {
    let idx = qdisc_s_show
        .lines()
        .enumerate()
        .find_map(|(i, l)| if leaf_line_matches(l, form) { Some(i) } else { None })?;
    let window: Vec<&str> = qdisc_s_show.lines().skip(idx).take(2).collect();
    for line in window {
        let toks: Vec<&str> = line.split_whitespace().collect();
        for w in toks.windows(5) {
            if w[0] == "Sent"
                && w[2] == "bytes"
                && w[4] == "pkt"
                && let Ok(n) = w[3].parse::<u64>()
            {
                let dropped = toks
                    .iter()
                    .position(|t| *t == "(dropped")
                    .and_then(|i| toks.get(i + 1))
                    .and_then(|v| v.trim_end_matches(',').parse::<u64>().ok())
                    .unwrap_or(0);
                return Some(LeafStats { sent_pkt: n, dropped });
            }
        }
    }
    None
}

// ---------- verify / counter-reset (live-traffic double sampling) ----------

/// Double-sample hit-coverage assert (bash verify_measure: no leaf Sent growth in a 1s window = media missed the filter).
/// Returns (pre, post); post<pre (set path) = counter reset (this kernel does not reset in practice, header note 6 -- downgraded to an advisory note).
pub fn verify_measure(channel: &Channel, dev: &str, pre: Option<u64>) -> Wn<(u64, u64)> {
    verify_measure_form(channel, dev, pre, LeafForm::Parent110)
}

/// Form-aware version (T9: ingress proof = ifb0 leaf Sent growth, design §ifb: hit verification counts ifb0 leaf
/// Sent growth as the proof that packets truly traversed netem).
pub fn verify_measure_form(
    channel: &Channel,
    dev: &str,
    pre: Option<u64>,
    form: LeafForm,
) -> Wn<(u64, u64)> {
    let read_leaf = |show: &str| leaf_stats_form(show, form).map(|s| s.sent_pkt);
    let pre = match pre {
        Some(p) => p,
        None => read_leaf(&tc_exec(channel, &["-s", "qdisc", "show", "dev", dev])?)
            .ok_or_else(|| Fail::env("verify: cannot read leaf Sent (leaf missing?)"))?,
    };
    std::thread::sleep(Duration::from_secs(1));
    let post = read_leaf(&tc_exec(channel, &["-s", "qdisc", "show", "dev", dev])?)
        .ok_or_else(|| Fail::env("verify: second-sample leaf Sent lost"))?;
    if pre == post {
        return Err(Fail::env(format!(
            "verify: no packet growth on the leaf in 1s ({pre}->{post}) -- media missed the filter (check the stream is producing / --rtp-port)"
        )));
    }
    Ok((pre, post))
}

#[must_use]
pub fn counters_reset(pre: u64, post: u64) -> bool {
    post < pre
}

// ---------- teardown plan/execution ----------

/// Teardown plan persisted at apply (design §ifb contract, order-sensitive):
/// 1) iface ingress deleted first (break the mirror first, prevent a dangling redirect black hole) -> 2) ifb0 root ->
/// 3) **only this session's own** ifb0 gets the link del (never touch others'/system's -- state records the owner) -> 4) iface root.
///
/// **root-del is always unconditional** (bash do_clear parity): clear = firefighting channel, step failures idempotent via is_not_exist
/// (double-fire stomping acceptable); foreign qdiscs are already blocked at apply by guard_foreign_root,
/// so any root alive until the clear phase must be ours or our residue -- both should be deleted. The previous created_root gating
/// (a rescue round being "stricter than bash") is exactly the **PIT-187 clear leaves-a-tree bug root cause** (three live reproductions at T7).
/// Although pure ingress sessions never touch the iface root at apply, root-del stays unconditional (falling back to the kernel default = idempotent harmless,
/// keeping the single unconditional form prevents created_* gating from creeping back).
///
/// steps shape convention: first element `ip` = link-class command (run_step_resilient routes to the ip program),
/// otherwise a tc argv (without program name).
#[must_use]
pub fn plan_teardown(iface: &str, ifb_used: bool, ifb_created: bool) -> Vec<Vec<String>> {
    let mut steps = Vec::new();
    if ifb_used {
        steps.push(vec![
            "qdisc".into(),
            "del".into(),
            "dev".into(),
            iface.into(),
            "ingress".into(),
        ]);
        steps.push(vec![
            "qdisc".into(),
            "del".into(),
            "dev".into(),
            ifb::IFB_DEV.into(),
            "root".into(),
        ]);
    }
    if ifb_created {
        steps.push(vec!["ip".into(), "link".into(), "del".into(), ifb::IFB_DEV.into()]);
    }
    steps.push(vec!["qdisc".into(), "del".into(), "dev".into(), iface.into(), "root".into()]);
    steps
}

/// not-exist class errors = idempotent success (design rev-2.2: two watchdogs firing concurrently, the second must hit nothing; noise prevention).
#[must_use]
pub fn is_not_exist(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("no such file")
        || m.contains("not found")
        || m.contains("cannot find")
// tc 5.15 form: "Cannot delete qdisc with handle of zero" = no concrete root qdisc on the device
// (noqueue cannot be deleted) -- a pure-ingress unconditional root-del step always hits this; semantics = idempotent nothing-to-delete.
        || m.contains("handle of zero")
}

/// Execute the teardown per the plan (shared by clear/watchdog/autoheal). Returns per-step failures (C15: caller lands them in the
/// timeline -- the watchdog context writes watchdog-clear-failed). The sidecar three-layer fallback ports del_root_besteffort:
/// exec -> docker start+exec -> one-shot `run --rm`.
pub fn run_teardown(state: &State) -> Vec<String> {
    let mut failures = Vec::new();
    for step in &state.teardown.steps {
        if let Err(e) = run_step_resilient(&state.teardown, step) {
            eprintln!("weaknet(engine): teardown step failed [{:?}]: {e}", step);
            failures.push(format!("{}: {e}", step.join(" ")));
        }
    }
    failures
}

/// teardown plan -> channel rebuild (single construction source shared by run_teardown / the serve state frame;
/// a sidecar form missing its descriptor = None, callers treat as corrupt state).
#[must_use]
pub fn channel_of(teardown: &Teardown) -> Option<Channel> {
    match (teardown.channel, &teardown.sidecar) {
        (ChannelSer::LocalRoot, _) => Some(Channel::LocalRoot),
        (ChannelSer::Sidecar, Some(sc)) => {
            Some(Channel::Sidecar { container: sc.name.clone(), image: sc.image.clone() })
        }
        (ChannelSer::Sidecar, None) => None,
    }
}

/// argv[0] = program name (tc|ip) -> channel real execution. Shared single point for run_steps (apply) and teardown routing.
fn exec_argv(channel: &Channel, prog: &str, args: &[&str]) -> Wn<String> {
    if prog == Prog::Ip.str() {
        let mut full = vec![Prog::Ip.str()];
        full.extend_from_slice(args);
        exec_prog(channel, &full)
    } else {
        tc_exec(channel, args)
    }
}

/// teardown step convention (plan_teardown header): first element `ip` = link command (with program name),
/// otherwise tc argv (without program name). Split into (prog, args).
fn split_prog(step: &[String]) -> (&'static str, Vec<&str>) {
    let args: Vec<&str> = step.iter().map(String::as_str).collect();
    if matches!(args.first(), Some(p) if *p == Prog::Ip.str()) {
        (Prog::Ip.str(), args[1..].to_vec())
    } else {
        (Prog::Tc.str(), args)
    }
}

fn exec_step(channel: &Channel, step: &[String]) -> Wn<String> {
    let (prog, tail) = split_prog(step);
    exec_argv(channel, prog, &tail)
}

fn run_step_resilient(teardown: &Teardown, step: &[String]) -> Result<(), String> {
    let channel = channel_of(teardown).ok_or_else(|| {
        "teardown plan is sidecar but missing the sidecar descriptor (corrupt state)".to_string()
    })?;
    let (prog, tail) = split_prog(step);
    match exec_argv(&channel, prog, &tail) {
        Ok(_) => Ok(()),
        Err(e) if is_not_exist(&e.msg) => Ok(()),
        Err(first) => match &channel {
            // sidecar may have been docker rm'd -- qdiscs persist in the host netns; one-shot container as fallback
            Channel::Sidecar { container, image } => {
                let _ = run(&["docker", "start", container]);
                match exec_argv(&channel, prog, &tail) {
                    Ok(_) => Ok(()),
                    Err(e2) if is_not_exist(&e2.msg) => Ok(()),
                    Err(second) => match run(&[
                        "docker",
                        "run",
                        "--rm",
                        "--net",
                        "host",
                        "--cap-add",
                        "NET_ADMIN",
                        "--entrypoint",
                        prog,
                        image,
                    ]
                    .into_iter()
                    .chain(tail.iter().copied())
                    .collect::<Vec<_>>())
                    {
                        Ok(_) => Ok(()),
                        Err(third) => Err(format!(
                            "three-layer fallback exhausted: exec({}) / start+exec({}) / run --rm({})",
                            first.msg, second.msg, third.msg
                        )),
                    },
                }
            }
            Channel::LocalRoot => Err(first.msg),
        },
    }
}

/// Lazy self-heal primary insurance (bash autoheal_check: fields cannot die, covers SIGKILL/OOM too).
/// Called at set/status/scenario entry (apply/clear skip -- bash parity). Returns whether self-heal happened.
pub fn autoheal_if_expired(dirs: &Dirs, env: &Env) -> Wn<bool> {
    let Some(st) = State::read_from(&dirs.state_json()).map_err(Fail::env)? else {
        return Ok(false);
    };
    if !fuse::is_expired(Some(st.expires_at_ms), fuse::now_epoch_ms()) {
        return Ok(false);
    }
    eprintln!(
        "weaknet: expiry self-heal: apply expired (expires_at_ms={} < now), lazy sweep",
        st.expires_at_ms
    );
    match fuse::kill_watchdog(&dirs.watchdog_pid()) {
        Ok(o) => eprintln!("weaknet: watchdog reaped: {o:?}"),
        Err(e) => eprintln!("weaknet: WARN watchdog reap failed (sweeping anyway): {e}"),
    }
    let failures = run_teardown(&st);
    for f in &failures {
        eprintln!("weaknet: WARN self-heal teardown: {f}");
    }
    // bash parity: after best-effort teardown always rm state (prevents retry loops over a half-site)
    if let Err(e) = std::fs::remove_file(dirs.state_json())
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(Fail::env(format!("failed to clear state: {e}")));
    }
    let ev = if failures.is_empty() {
        serde_json::json!({"ev": "self-heal-clear"})
    } else {
        serde_json::json!({"ev": "watchdog-clear-failed", "err": failures.join("; ")})
    };
    state::timeline_append(dirs, ev).map_err(Fail::env)?;
    let _ = env; // the iface channel is already in state.teardown -- self-heal does not re-probe
    Ok(true)
}

// ---------- apply / set orchestration ----------

#[derive(Debug, Clone, Default)]
pub struct ScopeNames {
    /// Stream-targeted room set (= bash STREAM_SEL split form)
    pub rooms: Vec<String>,
    /// Device-targeted device ID set
    pub devices: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ApplyRequest {
    pub spec: ImpairSpec,
    pub scope: ScopeSel,
    /// E1/E2 (T8): targeted names (into state + the apply event stream field).
    pub names: ScopeNames,
    pub iface: String,
    /// Media: local port set; Stream/Device go through pairs (mutually exclusive non-empty with ports)
    pub ports: Vec<u16>,
    pub pairs: Vec<(u16, u16)>,
    pub sig_port: Option<u16>,
}

#[derive(Debug)]
pub enum Replay {
    Apply { duration_secs: u64 },
    Set { prior: Box<State>, before: String },
}

/// Live verify mode (design §server: verify moved off the response path): CLI=Inline (bash semantics:
/// apply verify failure -> root rollback exit2); REST=Deferred -- 200 only requires the fingerprint re-read to pass, double sampling
/// moves into a background task, results land as a timeline `verify` event (SSE state-frame ev broadcast).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verify {
    Inline,
    Deferred,
}

#[derive(Debug)]
pub struct ReplayOutcome {
    pub spec_string: String,
    pub filter_count: usize,
    pub expires_at_ms: u64,
    pub channel: String,
    /// Set-path counter-reset probe result (apply=None, set=Some(bool), same position as the bash echo)
    pub counters_reset: Option<bool>,
    pub leaf: (u64, u64),
}

/// Full replay shared by apply/set (bash: both forms are a full build_skeleton rebuild).
/// Apply: new fuse + spawn watchdog + apply event; failure rolls back root (bash L522-533).
/// Set: keep the original expiry, don't touch the watchdog, read-back failure does not roll back (prevent breaking the stream -- bash do_set L315-317 parity);
/// carries the counter-reset probe (leaf Sent before/after change).
pub fn replay(
    req: &ApplyRequest,
    mode: Replay,
    dirs: &Dirs,
    env: &Env,
    verify: Verify,
) -> Wn<ReplayOutcome> {
    req.spec.validate().map_err(Fail::bad_param)?;
    let kind = iface_kind_of(&req.iface);
    let use_ifb_want = ifb_needed(kind, req.spec.dir);
    // leg split (§dir table: physical-port ingress legs take the ifb mirror): an ifb session's egress keeps only Out legs
    // (dir=Both) or the empty set (pure In); non-ifb = all legs under root (lo unchanged, zero drift).
    let legs = if !use_ifb_want {
        build_filter_legs(req.scope, kind, req.spec.dir, &req.ports, &req.pairs)
            .map_err(Fail::env)?
    } else if req.spec.dir == Dir::Both {
        build_filter_legs(req.scope, kind, Dir::Out, &req.ports, &req.pairs).map_err(Fail::env)?
    } else {
        Vec::new()
    };
    let ingress_legs = if use_ifb_want {
        build_filter_legs(req.scope, kind, Dir::In, &req.ports, &req.pairs).map_err(Fail::env)?
    } else {
        Vec::new()
    };
    let channel = probe_channel(env, &req.iface)?;
    // T9 capability gate (fail-closed): physical-port ingress needs ifb0, probe failure = exit2 with cause, never silently degrade.
    let use_ifb = guard_ifb_required(kind, req.spec.dir, ifb::probe(&channel))?;
    // Read-before-probe (§ifb contract): ifb0 already present (someone else's/system's/previous session's) -> no re-add, teardown won't delete the link.
    let created_ifb_new = use_ifb && !ifb::link_present(&channel);
    if use_ifb && req.spec.rate_mbps.is_some() {
        eprintln!(
            "weaknet(engine): WARN the rate wall only applies on the egress htb -- {} root=netem minimal form (§ifb contract discretion), the ingress leg is not rate-limited",
            ifb::IFB_DEV
        );
    }
    if req.spec.seed.is_some() {
        version_gate_seed(&channel)?;
    }
    let show = tc_exec(&channel, &["qdisc", "show", "dev", &req.iface])?;
    let state_exists = dirs.state_json().is_file();
    // the iface root guard is kept for ifb sessions too (fail-closed: root occupied by a foreign party = that port's site is not clean,
    // reject the whole order, don't just validate ingress).
    guard_foreign_root(&show, state_exists)?;
    // Pure-In physical session = don't touch the iface root (skip the skeleton); Both / with a signaling leg builds as usual.
    let install_skeleton = !use_ifb || !legs.is_empty() || req.sig_port.is_some();
    let add_root =
        install_skeleton && matches!(root_action(&show), RootAction::Add | RootAction::Replace);
    let created_root_pre = match &mode {
        Replay::Apply { .. } => add_root,
        Replay::Set { prior, .. } => prior.created_root,
    };
    // Set direction reversal (ingress -> no ifb): tear down the old mirror chain immediately -- full-replay semantics, no old damage left behind.
    if let Replay::Set { prior, .. } = &mode
        && prior.ifb_used
        && !use_ifb
    {
        eprintln!(
            "weaknet(engine): set closing ingress (dir={:?}) -- tearing down the {} old mirror chain",
            req.spec.dir, prior.iface
        );
        rollback_ifb(&channel, &prior.iface, false);
    }
    let spec_string = render_netem_spec_leg(&req.spec);
    let rate = render_rate_arg(req.spec.rate_mbps);
    let mut steps = Vec::new();
    if install_skeleton {
        steps.extend(plan_skeleton(&req.iface, add_root, &spec_string, &rate, &legs, req.sig_port));
    }
    if use_ifb {
        steps.extend(plan_ifb(&req.iface, &spec_string, &ingress_legs, created_ifb_new));
    }
    // read-back/verify observation point: pure In = ifb0 root form (ingress proof); otherwise iface 1:10 (Both observes the outbound leg).
    let (rd_dev, rd_form) = if use_ifb && !install_skeleton {
        (ifb::IFB_DEV.to_string(), LeafForm::Root)
    } else {
        (req.iface.clone(), LeafForm::Parent110)
    };

    let pre_sent = match &mode {
        Replay::Set { .. } => {
            let s = tc_exec(&channel, &["-s", "qdisc", "show", "dev", &rd_dev])?;
            leaf_stats_form(&s, rd_form).map(|l| l.sent_pkt)
        }
        Replay::Apply { .. } => None,
    };

    if let Err(e) = run_steps(&channel, &steps) {
        if matches!(mode, Replay::Apply { .. }) {
            rollback_apply_failure(
                &channel,
                &req.iface,
                use_ifb,
                created_ifb_new,
                install_skeleton,
            );
        }
        return Err(e);
    }
    let sshow = tc_exec(&channel, &["-s", "qdisc", "show", "dev", &rd_dev])?;
    if let Err(e) = assert_fingerprint_form(&sshow, &spec_string, rd_form).and_then(|()| {
        if rd_form == LeafForm::Parent110 {
            assert_class_1_10(&channel, &req.iface)
        } else {
            Ok(())
        }
    }) {
        let e = Fail::env(match &mode {
            Replay::Apply { .. } => format!("{} (apply site rolled back)", e.msg),
            Replay::Set { .. } => format!(
                "{} (set not rolled back to protect the stream; re-run set to recover)",
                e.msg
            ),
        });
        if matches!(mode, Replay::Apply { .. }) {
            rollback_apply_failure(
                &channel,
                &req.iface,
                use_ifb,
                created_ifb_new,
                install_skeleton,
            );
        }
        return Err(e);
    }
    let sent1 = leaf_stats_form(&sshow, rd_form).map(|l| l.sent_pkt);
    let leaf = match verify {
        // Deferred: zero waiting on the response path; pre carried out as-is, the background re-samples from the same pre (incl. counter-reset probe).
        Verify::Deferred => (sent1.unwrap_or(0), sent1.unwrap_or(0)),
        Verify::Inline => match verify_measure_form(&channel, &rd_dev, sent1, rd_form) {
            Ok(pair) => pair,
            Err(e) => {
                if matches!(mode, Replay::Apply { .. }) {
                    rollback_apply_failure(
                        &channel,
                        &req.iface,
                        use_ifb,
                        created_ifb_new,
                        install_skeleton,
                    );
                    return Err(Fail::env(format!("{} (apply site rolled back)", e.msg)));
                }
                // bash do_set: verify failure dies 2 but does not roll back
                return Err(e);
            }
        },
    };
    let counters_reset = match (&mode, pre_sent) {
        (Replay::Set { .. }, Some(pre)) if matches!(verify, Verify::Inline) => {
            Some(counters_reset(pre, leaf.1))
        }
        _ => None,
    };

    let expires_at_ms = match &mode {
        Replay::Apply { duration_secs } => fuse::now_epoch_ms() + duration_secs * 1000,
        Replay::Set { prior, .. } => prior.expires_at_ms,
    };
    // teardown ownership (Set reversal only adds, never removes: an ifb chain left by a previous in/both session must enter the teardown plan).
    let (ifb_used_plan, ifb_created_plan) = match &mode {
        Replay::Apply { .. } => (use_ifb, created_ifb_new),
        Replay::Set { prior, .. } => {
            (use_ifb || prior.ifb_used, created_ifb_new || prior.created_ifb)
        }
    };
    let st = State {
        schema: state::STATE_SCHEMA.to_string(),
        spec: req.spec.clone(),
        dir: req.spec.dir,
        scope: req.scope,
        iface: req.iface.clone(),
        ports: req.ports.clone(),
        pairs: req.pairs.clone(),
        rooms: req.names.rooms.clone(),
        devices: req.names.devices.clone(),
        sig_port: req.sig_port,
        expires_at_ms,
        created_root: created_root_pre,
        ifb_used: ifb_used_plan,
        created_ifb: ifb_created_plan,
        job: match &mode {
            Replay::Set { prior, .. } => prior.job.clone(),
            Replay::Apply { .. } => None,
        },
        teardown: Teardown {
            channel: match &channel {
                Channel::LocalRoot => ChannelSer::LocalRoot,
                Channel::Sidecar { .. } => ChannelSer::Sidecar,
            },
            sidecar: match &channel {
                Channel::Sidecar { container, image } => {
                    Some(SidecarRef { name: container.clone(), image: image.clone() })
                }
                Channel::LocalRoot => None,
            },
            steps: plan_teardown(&req.iface, ifb_used_plan, ifb_created_plan),
        },
    };
    st.write_to(&dirs.state_json()).map_err(Fail::env)?;
    match &mode {
        Replay::Apply { .. } => {
            let stream_sel = if !req.names.rooms.is_empty() {
                req.names.rooms.join(",")
            } else {
                req.names.devices.join(",")
            };
            state::timeline_append(dirs, state::apply_event(&req.spec, &req.ports, &stream_sel))
                .map_err(Fail::env)?;
            spawn_watchdog(dirs);
        }
        Replay::Set { before, .. } => {
            state::timeline_append(
                dirs,
                serde_json::json!({
                    "ev": "set",
                    "before": before,
                    "after": state::param_summary(&req.spec),
                    "counters": match counters_reset { Some(true) => "reset", _ => "ok" },
                }),
            )
            .map_err(Fail::env)?;
        }
    }
    Ok(ReplayOutcome {
        spec_string,
        filter_count: steps.iter().filter(|s| s.run[0] == "filter" && s.run[1] == "add").count(),
        expires_at_ms,
        channel: channel.describe(),
        counters_reset,
        leaf,
    })
}

/// watchdog spawn (self re-exec; failure only WARNs -- the primary insurance is the state expiry field's lazy self-heal, fuse layer one).
pub fn spawn_watchdog(dirs: &Dirs) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "weaknet(engine): WARN cannot locate own binary, watchdog absent (lazy self-heal still active): {e}"
            );
            return;
        }
    };
    let state_path = dirs.state_json();
    let state_arg = match state_path.to_str() {
        Some(s) => s.to_string(),
        None => {
            eprintln!(
                "weaknet(engine): WARN state path not UTF-8, watchdog absent: {}",
                state_path.display()
            );
            return;
        }
    };
    if let Err(e) = fuse::kill_watchdog(&dirs.watchdog_pid()) {
        eprintln!("weaknet(engine): WARN old watchdog reap failed (re-mounting anyway): {e}");
    }
    if let Err(e) = fuse::spawn_watchdog(&exe, &["__watchdog", &state_arg], &dirs.watchdog_pid()) {
        eprintln!(
            "weaknet(engine): WARN watchdog spawn failed (lazy self-heal layer still active): {e}"
        );
    }
}

fn run_steps(channel: &Channel, steps: &[StepPlan]) -> Wn<()> {
    for s in steps {
        match exec_argv(
            channel,
            s.prog.str(),
            &s.run.iter().map(String::as_str).collect::<Vec<_>>(),
        ) {
            Ok(_) => {}
            Err(e) => {
                if let Some(alt) = &s.alt {
                    match exec_argv(
                        channel,
                        s.prog.str(),
                        &alt.iter().map(String::as_str).collect::<Vec<_>>(),
                    ) {
                        Ok(_) => {}
                        Err(e2) if s.best_effort => {
                            eprintln!(
                                "weaknet(engine): best-effort step [{what}] failed in both forms (tolerated): {e2}",
                                what = s.what
                            );
                        }
                        Err(e2) => {
                            return Err(Fail::env(format!(
                                "{} failed [{} || {}]: {e2}",
                                s.what,
                                s.run.join(" "),
                                alt.join(" ")
                            )));
                        }
                    }
                } else if s.best_effort {
                    eprintln!(
                        "weaknet(engine): best-effort step [{}] failed (tolerated): {}",
                        s.what, e.msg
                    );
                } else {
                    return Err(Fail::env(format!(
                        "{} failed [{} {}]: {}",
                        s.what,
                        s.prog.str(),
                        s.run.join(" "),
                        e.msg
                    )));
                }
            }
        }
    }
    Ok(())
}

fn rollback_root(channel: &Channel, dev: &str) {
    if let Err(e) = tc_exec(channel, &["qdisc", "del", "dev", dev, "root"]) {
        eprintln!("weaknet(engine): WARN root rollback failed (manual clear needed): {e}");
    }
}

/// Unified rollback on apply failure (bash L522-533 + §ifb: mirror chain and root cleared in one go).
fn rollback_apply_failure(
    channel: &Channel,
    dev: &str,
    use_ifb: bool,
    created_ifb: bool,
    install_skeleton: bool,
) {
    // pure-ifb (no root skeleton) root-del always trips "handle of zero" noise -- roll back per the actual site state
    if install_skeleton {
        rollback_root(channel, dev);
    }
    if use_ifb {
        rollback_ifb(channel, dev, created_ifb);
    }
}

/// ifb mirror chain teardown/rollback (best-effort, ENOENT idempotent; iface root belongs to rollback_root).
/// Also used by set reversal for immediate cleanup (the created ownership bit stays in the teardown plan, the link is not deleted here).
fn rollback_ifb(channel: &Channel, dev: &str, created_ifb: bool) {
    let mut steps = vec![
        vec!["qdisc".into(), "del".into(), "dev".into(), dev.into(), "ingress".into()],
        vec!["qdisc".into(), "del".into(), "dev".into(), ifb::IFB_DEV.into(), "root".into()],
    ];
    if created_ifb {
        steps.push(vec!["ip".into(), "link".into(), "del".into(), ifb::IFB_DEV.into()]);
    }
    for st in &steps {
        if let Err(e) = exec_step(channel, st)
            && !is_not_exist(&e.msg)
        {
            eprintln!(
                "weaknet(engine): WARN ifb chain teardown failed [{:?}] (residue covered by clear): {}",
                st, e.msg
            );
        }
    }
}

fn assert_class_1_10(channel: &Channel, dev: &str) -> Wn<()> {
    let out = tc_exec(channel, &["class", "show", "dev", dev])?;
    if out.contains("class htb 1:10") {
        Ok(())
    } else {
        Err(Fail::env("read-back: 1:10 class missing"))
    }
}

/// clear main body (bash do_clear: kill_watchdog -> ensure channel -> del root per plan/best-effort ->
/// rm state -> timeline clear). Lock-free reachable (firefighting channel). The watchdog context uses `quiet` (no channel ensure
/// noise, different event name).
pub fn clear(dirs: &Dirs, env: &Env, quiet: bool) -> Wn<String> {
    match fuse::kill_watchdog(&dirs.watchdog_pid()) {
        Ok(o) => eprintln!("weaknet: watchdog reaped: {o:?}"),
        Err(e) => eprintln!("weaknet: WARN watchdog reap: {e}"),
    }
    let state_path = dirs.state_json();
    let st = State::read_from(&state_path).map_err(Fail::env)?;
    if !quiet {
        // channel reachability check (non --all sites need tc to answer)
        let iface = st.as_ref().map(|s| s.iface.clone()).unwrap_or_else(|| "lo".into());
        probe_channel(env, &iface)?;
    }
    let mut failures = Vec::new();
    match &st {
        Some(s) => failures = run_teardown(s),
        None => {
            // no state: best-effort del root (manual external mounts cleared too -- clear is the firefighting channel), ENOENT idempotent
            // iface cannot be derived from state -- bash parity falls back to WEAKNET_DEV:-lo
            let dev = std::env::var("WEAKNET_DEV").unwrap_or_else(|_| "lo".into());
            let chan = probe_channel(env, &dev)?;
            let step = ["qdisc", "del", "dev", dev.as_str(), "root"];
            if let Err(e) = tc_exec(&chan, &step)
                && !is_not_exist(&e.msg)
            {
                failures.push(format!("{}: {}", step.join(" "), e.msg));
            }
        }
    }
    for f in &failures {
        eprintln!("weaknet: WARN clear teardown: {f}");
    }
    if let Err(e) = std::fs::remove_file(&state_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(Fail::env(format!("failed to delete state: {e}")));
    }
    state::timeline_append(dirs, serde_json::json!({"ev": "clear"})).map_err(Fail::env)?;
    Ok(if failures.is_empty() {
        "cleared (residue detection in status)".to_string()
    } else {
        format!("cleared-with-warnings ({} steps failed, see above)", failures.len())
    })
}

/// Write-command entry lock (apply/set/scenario -- bash flock same domain; busy=exit3); clear/status stay lock-free by caller discipline.
pub fn take_write_lock(dirs: &Dirs) -> Wn<state::WriteLock> {
    state::acquire_write_lock(dirs).map_err(Fail::conflict)
}

/// dry-run channel prefix (pure predicate, zero side effects -- no probing/no container creation: local specified or euid0 -> tc, else sidecar form).
#[must_use]
pub fn dryrun_prefix(env: &Env) -> String {
    dryrun_prog_prefix(env, Prog::Tc)
}

/// T9: display prefix per program (ifb chain steps are `ip ...`, sidecar form = `docker exec <c> ip ...`).
#[must_use]
pub fn dryrun_prog_prefix(env: &Env, prog: Prog) -> String {
    let bare = prog.str();
    let docker = || format!("docker exec {} {bare}", env.sidecar);
    match env.channel_pref.as_str() {
        "local" => bare.to_string(),
        "sidecar" => docker(),
        // auto: branch only on the root predicate (zero probe side effects).
        _ => {
            if euid_is_root().unwrap_or(false) {
                bare.to_string()
            } else {
                docker()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Dir, LossSpec};

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn root_action_pure_decision() {
        // bash: first root-line predicate -- no htb -> add; `^qdisc htb 1:` present -> change-only.
        assert_eq!(root_action("qdisc noqueue 0: root default"), RootAction::Add);
        assert_eq!(root_action("qdisc fq_codel 0: root"), RootAction::Add);
        assert_eq!(root_action(""), RootAction::Add);
        assert_eq!(root_action("qdisc htb 1: root handle 1: rmdefault"), RootAction::Change);
        assert_eq!(root_action("qdisc netem 8001: root limit 100"), RootAction::Replace);
        // an ingress line first must not misjudge as Change (first qdisc line wins)
        assert_eq!(
            root_action("qdisc ingress ffff: dev lo ingress_block 2\nqdisc htb 1: root"),
            RootAction::Add
        );
    }

    #[test]
    fn guard_foreign_root_fail_closed_matrix() {
        // allowlist pass (bash *noqueue*|*fq_codel*|*fq *|*pfifo_fast*)
        for l in [
            "qdisc noqueue 0: root",
            "qdisc fq_codel 0: root",
            "qdisc fq 8000: root",
            "qdisc pfifo_fast 0: root",
        ] {
            assert!(guard_foreign_root(l, false).is_ok(), "{l}");
        }
        // htb root: no state of ours -> conflict exit3; present -> self-prove and pass
        assert_eq!(guard_foreign_root("qdisc htb 1: root", false).unwrap_err().code, 3);
        assert!(guard_foreign_root("qdisc htb 1: root", true).is_ok());
        // netem root: M0 has no --all form -> refuse to touch (stricter than bash, fail-closed)
        assert_eq!(guard_foreign_root("qdisc netem 8001: root", true).unwrap_err().code, 3);
        // any other foreign occupant -> exit3
        assert_eq!(guard_foreign_root("qdisc sfq 12: root", false).unwrap_err().code, 3);
        // no qdisc line -> pass (empty output)
        assert!(guard_foreign_root("", false).is_ok());
    }

    #[test]
    fn fingerprint_a_row_does_not_satisfy_spec_b() {
        // design §engine negative predicate: spec A's leaf line must not satisfy spec B's fingerprint.
        let spec_a = "limit 100000 delay 40ms loss 2%";
        let leaf_a = "qdisc netem 8001: parent 1:10 handle 10: limit 100000 delay 40ms loss 2%";
        assert!(assert_fingerprint(leaf_a, spec_a).is_ok());
        let spec_b = "limit 100000 delay 80ms loss 5%";
        let e = assert_fingerprint(leaf_a, spec_b).unwrap_err();
        assert_eq!(e.code, 2);
        assert!(e.msg.contains("80ms") && e.msg.contains("5"), "{e}");
        // no netem leaf (apply did not take effect) -> report cause, never pass silently
        assert!(assert_fingerprint("qdisc noqueue 0: root", spec_a).is_err());
        // distribution/normal not in the fingerprint (old tc never echoes the distribution word -- 2015 mirror proven)
        let toks = fingerprint_tokens("limit 100000 delay 40ms 7ms distribution normal");
        assert!(!toks.iter().any(|t| t == "distribution" || t == "normal"), "{toks:?}");
        // gemodel pattern word in the fingerprint (loss-mode distinctness)
        let g = fingerprint_tokens("limit 100000 loss gemodel 8 25 0.2 0.05");
        assert!(g.iter().any(|t| t == "gemodel"), "{g:?}");
    }

    #[test]
    fn plan_teardown_order_ifb_then_own_root() {
        // §ifb teardown order (truth table): break the mirror ingress first -> ifb0 root -> del the link only if created this session -> iface root;
        // root-del always present (no gating after PIT-187).
        assert_eq!(
            plan_teardown("eth0", true, true),
            vec![
                v(&["qdisc", "del", "dev", "eth0", "ingress"]),
                v(&["qdisc", "del", "dev", "ifb0", "root"]),
                v(&["ip", "link", "del", "ifb0"]),
                v(&["qdisc", "del", "dev", "eth0", "root"]),
            ]
        );
        // foreign ifb0 (created=false): tear down the mirror and leaf, never del the link.
        assert_eq!(
            plan_teardown("eth0", true, false),
            vec![
                v(&["qdisc", "del", "dev", "eth0", "ingress"]),
                v(&["qdisc", "del", "dev", "ifb0", "root"]),
                v(&["qdisc", "del", "dev", "eth0", "root"]),
            ]
        );
        assert_eq!(
            plan_teardown("lo", false, false),
            vec![v(&["qdisc", "del", "dev", "lo", "root"])]
        );
        // regression pin: even with root not created this round (old created_root=false context) the plan must include root-del -- PIT-187
        assert_eq!(
            plan_teardown("wlan0", false, false),
            vec![v(&["qdisc", "del", "dev", "wlan0", "root"])]
        );
    }

    #[test]
    fn ifb_needed_matrix_and_guard() {
        // §dir table: physical port x {In, Both} needs the mirror; lo always false; physical x Out goes through the egress root.
        assert!(!ifb_needed(IfaceKind::Loopback, Dir::In));
        assert!(!ifb_needed(IfaceKind::Loopback, Dir::Both));
        assert!(!ifb_needed(IfaceKind::Physical, Dir::Out));
        assert!(ifb_needed(IfaceKind::Physical, Dir::In));
        assert!(ifb_needed(IfaceKind::Physical, Dir::Both));
        // error classification (unknown-device-type grouped under Physical): capability not passed = exit2 with cause, never silently degrade.
        let e = guard_ifb_required(
            IfaceKind::Physical,
            Dir::In,
            (false, "kernel has no ifb module".to_string()),
        )
        .unwrap_err();
        assert_eq!(e.code, 2);
        assert!(
            e.msg.contains("kernel has no ifb module") && e.msg.contains("modprobe ifb"),
            "{e}"
        );
        // lo doesn't need ifb -> capability false doesn't block (dir_lo always true).
        assert!(!guard_ifb_required(IfaceKind::Loopback, Dir::Both, (false, "x".into())).unwrap());
        assert!(guard_ifb_required(IfaceKind::Physical, Dir::Both, (true, String::new())).unwrap());
    }

    #[test]
    fn plan_ifb_apply_order_matches_contract() {
        // §ifb apply order: link add (need_link only) -> link up -> ifb0 root netem -> iface ingress ->
        // pre-flush -> mirred mirror leg (protocol 17 conjunction + dport AND, action redirect dev ifb0).
        let legs = vec![LegFilter {
            protocol: 17,
            matches: vec![(MatchKind::Dport, 40010)],
            flowid: spec::MEDIA_FLOWID.to_string(),
        }];
        let steps = plan_ifb("ens32", "limit 100000 delay 80ms loss 10%", &legs, true);
        let shown: Vec<String> = steps
            .iter()
            .map(|t| {
                format!(
                    "{} {}{}",
                    t.prog.str(),
                    t.run.join(" "),
                    if t.best_effort { " [be]" } else { "" }
                )
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                "ip link add ifb0 type ifb",
                "ip link set ifb0 up",
                "tc qdisc change dev ifb0 root netem limit 100000 delay 80ms loss 10%",
                "tc qdisc add dev ens32 ingress [be]",
                "tc filter del dev ens32 ingress [be]",
                "tc filter add dev ens32 ingress protocol ip u32 match ip protocol 17 0xff match ip dport 40010 0xffff action mirred egress redirect dev ifb0",
            ]
        );
        // need_link=false (read-before-probe found ifb0 present): no re-add, everything else as usual (idempotent replay).
        assert!(
            !plan_ifb("ens32", "limit 100000", &[], false)
                .iter()
                .any(|t| t.run.join(" ").contains("link add"))
        );
        assert_eq!(
            plan_ifb("ens32", "limit 100000", &[], false)[0].run.join(" "),
            "link set ifb0 up"
        );
        // missing the up step = the ENETDOWN green-wash source (contract red line): up must precede the netem leaf and the filter.
        let idx_up = shown.iter().position(|l| l.contains("link set ifb0 up")).unwrap();
        let idx_leaf = shown.iter().position(|l| l.contains("ifb0 root netem")).unwrap();
        assert!(idx_up < idx_leaf, "{shown:?}");
    }

    #[test]
    fn leaf_form_root_parsing_parent_form_untouched() {
        // T9 form-aware: ifb0 root leaf (no parent 1:10) parsing + fingerprint; the Parent110 legacy form does not regress.
        let root_show = "qdisc netem 8001: root refcnt 2 limit 100000 delay 80ms loss 10%\n Sent 1000 bytes 20 pkt (dropped 2, overlimits 0 requeues 0)";
        assert!(find_leaf_line_form(root_show, LeafForm::Root).is_some());
        assert!(find_leaf_line_form(root_show, LeafForm::Parent110).is_none());
        assert!(
            assert_fingerprint_form(root_show, "limit 100000 delay 80ms loss 10%", LeafForm::Root)
                .is_ok()
        );
        // negative: a Parent110 spec must not satisfy the Root leaf (forms don't cross-prove).
        let parent_show = "qdisc netem 8002: parent 1:10 handle 10: limit 100000 delay 80ms";
        assert!(
            assert_fingerprint_form(parent_show, "limit 100000 delay 80ms", LeafForm::Root)
                .is_err()
        );
        assert!(find_leaf_line(parent_show).is_some());
        let ls = leaf_stats_form(root_show, LeafForm::Root).unwrap();
        assert_eq!((ls.sent_pkt, ls.dropped), (20, 2));
        assert!(leaf_stats(parent_show).is_none());
    }

    #[test]
    fn is_not_exist_covers_rtnetlink_and_docker_forms() {
        assert!(is_not_exist("Error: Cannot delete qdisc with handle of zero.")); // kills pure-ifb root-del noise
        // two watchdogs firing concurrently, the second hits nothing -> ENOENT counts as idempotent success (rev-2.2 noise clause).
        assert!(is_not_exist("RTNETLINK answers: No such file or directory"));
        assert!(is_not_exist("Error: Cannot find qdisc for parent 1:10"));
        assert!(is_not_exist("Error: Exiting (failing). qdisc not found"));
        assert!(!is_not_exist("RTNETLINK answers: Operation not permitted"));
    }

    #[test]
    fn counters_reset_matches_bash_sortv_judgement() {
        // bash: sent_now < sent_pre and unequal = reset yes; equal = not reset (this kernel does not reset in practice).
        assert!(counters_reset(100, 50));
        assert!(!counters_reset(100, 100));
        assert!(!counters_reset(100, 1_862));
    }

    #[test]
    fn tc_argv_channel_shapes() {
        assert_eq!(
            Channel::LocalRoot.tc_argv(&["qdisc", "show", "dev", "lo"]),
            v(&["tc", "qdisc", "show", "dev", "lo"])
        );
        let c = Channel::Sidecar {
            container: "wnet-t4-test".into(),
            image: "gaiadocker/iproute2".into(),
        };
        assert_eq!(c.tc_argv(&["-V"]), v(&["docker", "exec", "wnet-t4-test", "tc", "-V"]));
        assert_eq!(c.describe(), "sidecar(wnet-t4-test)");
    }

    #[test]
    fn leaf_sent_parses_dash_s_block() {
        let out = "qdisc htb 1: root handle 1: prio 1: ref 4\n Sent 0 bytes 0 pkt (dropped 0, overlimits 0 requeues 0)\nqdisc netem 8002: parent 1:10 handle 10: limit 100000 delay 40ms\n Sent 12345 bytes 67 pkt (dropped 3, overlimits 0 requeues 0)";
        assert_eq!(leaf_sent(out), Some(67));
        assert_eq!(leaf_sent("qdisc noqueue 0: root"), None);
    }

    #[test]
    fn parse_iproute2_semver_both_separator_forms() {
        assert_eq!(parse_iproute2_semver("tc, from iproute2-6.11.0"), Some((6, 11)));
        assert_eq!(parse_iproute2_semver("tc, from iproute2-4.9.0"), Some((4, 9)));
        assert_eq!(parse_iproute2_semver("tc, from iproute2 ss150831"), None);
        assert_eq!(parse_iproute2_semver("tc, from iproute2-6.6"), Some((6, 6)));
    }

    #[test]
    fn dryrun_prefix_preserves_pref() {
        // explicit form judges directly with zero probing (even on a root machine the sidecar must show the docker prefix -- fix of the old || short-circuit bug).
        let local = Env { channel_pref: "local".into(), ..Default::default() };
        assert_eq!(dryrun_prefix(&local), "tc");
        let side =
            Env { channel_pref: "sidecar".into(), sidecar: "wnet-t4".into(), ..Default::default() };
        assert_eq!(dryrun_prefix(&side), "docker exec wnet-t4 tc");
    }

    #[test]
    fn write_lock_busy_maps_to_exit3() {
        // flock same-domain busy semantics = exit3 (bash flock -n failure dies 3) -- fn-level testable shape.
        let dir = std::env::temp_dir().join(format!("weaknet-enginelock-{}", std::process::id()));
        let dirs = Dirs { statedir: dir.clone() };
        let _lk = take_write_lock(&dirs).expect("first acquire must succeed");
        let e = take_write_lock(&dirs).unwrap_err();
        assert_eq!(e.code, 3, "{e}");
        drop(_lk);
        assert!(take_write_lock(&dirs).is_ok(), "re-acquirable after release");
        std::fs::remove_dir_all(&dir).ok();
    }
    #[test]
    fn seed_gate_pure_semver_judgement() {
        // version gate pure predicate (dir/spec crossing uses LossSpec to pin both shape serializations stable).
        let spec = ImpairSpec {
            rtt_ms: 80,
            jitter_ms: 0,
            loss: Some(LossSpec::GeModel {
                loss: "8%".into(),
                r: "25%".into(),
                h: "0.2".into(),
                k: "0.05".into(),
            }),
            dir: Dir::In,
            ..Default::default()
        };
        // single-leg dir=In -> full rtt (§dir table rtt calibration rule), same layer as the version gate but pinned separately.
        assert_eq!(
            render_netem_spec_leg(&spec),
            "limit 100000 delay 80ms loss gemodel 8 25 0.2 0.05"
        );
    }
}
