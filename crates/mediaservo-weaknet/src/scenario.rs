//! scenario.rs -- T8: scenario engine (emit.py grammar parsing + tokio scheduling runner + job flag layer).
//!
//! Semantic truth = `scripts/weaknet.sh do_scenario` + `scripts/weaknet.d/emit.py scenario` (plan grammar)
//! and the `judge.py` event whitelist (`scenario*` union {baseline-done, set}; recovery-step predicate = a set event whose after
//! prefix is `rtt=0ms` AND contains `loss=0%`). PIT-187 discipline: bash semantics ported verbatim, deviations logged in the
//! T8 delivery report (rev-2.2 authorized divergence: per-step transient locks + a job exclusivity layer replacing bash's full-run flock).
//!
//! rev-2.2 lock model (design §server: scenario job exclusivity semantics):
//! - the physical write lock is held transiently per step (keeps stop reachable);
//! - exclusivity = the `state.job` flag layer: while a job is active (owner alive), external CLI/REST apply/set -> 409/exit3;
//! - stale owner (declared dead via pid+starttime, same method as the fuse) -> clear the flag and pass;
//! - step lock failure -> `{ev:"scenario-end", note:"elapsed_s=...", aborted:"lock-busy"}` terminates
//!   (any scenario-end with aborted invalidates that round's judge verdict; W6 re-verification asserts aborted==0);
//! - stop channel = the `scenario.cancel` flag file (cross-process reachable; when the owner is dead [`stop`] clears the job flag directly).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::engine::{self, ApplyRequest, Env, Fail, Replay, Verify, Wn};
use crate::fuse;
use crate::scope::Targeting;
use crate::spec::{ImpairSpec, LossSpec};
use crate::state::{self, Dirs, JobRef, State};

/// bash do_scenario recovery dwell window (L603: log text says 12s, actually sleeps 25s -- ported per the true sleep value).
pub const RECOVERY_DWELL_SECS: u64 = 25;
/// bash do_scenario fuse margin (L570: duration = baseline + steps_dur + 90).
pub const COVERING_MARGIN_SECS: u64 = 90;
/// Cross-process stop flag (consumed and deleted by the runner at each checkpoint).
pub const CANCEL_FILE: &str = "scenario.cancel";

// ---------- Parsing (branch-by-branch port of emit.py scenario mode, fail-closed whitelist) ----------

/// step/repeat param dict -> overrides (typed equivalent of the bash flags rendering; range validation in merge).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepOverride {
    pub rtt_ms: Option<u64>,
    pub jitter_ms: Option<u64>,
    pub loss: Option<String>,
    pub gemodel: Option<(String, String, String)>,
    pub rate_mbps: Option<f64>,
    pub reorder_pct: Option<String>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlanLine {
    Step {
        /// at_s original string (the `at_s=` slot of the scenario-step note -- copied straight from the bash plan line)
        at_disp: String,
        /// target second (bash `target=${f2%.*}` truncation form)
        at_secs: u64,
        /// rendered flags string (source of the recovery-dwell awk `--rtt 0` substring predicate)
        flags: String,
        ov: StepOverride,
    },
    Repeat {
        rounds: u64,
        on_s: u64,
        off_s: u64,
        on_flags: String,
        off_flags: String,
        on_ov: StepOverride,
        off_ov: StepOverride,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioPlan {
    pub baseline_s: u64,
    pub lines: Vec<PlanLine>,
    /// TOTAL line (bash stepsdur; covered duration = baseline + total_s + COVERING_MARGIN)
    pub total_s: u64,
    /// CLEAR line (sc.clear truth, default true -> auto clear at the end)
    pub clear: bool,
}

type Y = serde_yaml::Value;
type YM = serde_yaml::Mapping;

/// Same shape as python str(scalar) (the convention for rendering yaml values into flags strings).
fn py_str(v: &Y) -> String {
    match v {
        Y::Null => "None".into(),
        Y::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else if let Some(f) = n.as_f64() {
                // python str(4.0)="4.0"; Rust 4.0f64.to_string()="4" -> append ".0"
                if f.fract() == 0.0 && f.is_finite() { format!("{f:.1}") } else { format!("{f}") }
            } else {
                n.to_string()
            }
        }
        Y::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

fn as_num(v: &Y) -> Option<f64> {
    match v {
        Y::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn as_int(v: &Y) -> Option<i64> {
    match v {
        Y::Number(n) => {
            n.as_i64().or_else(|| n.as_u64().map(|u| i64::try_from(u).unwrap_or(i64::MAX)))
        }
        _ => None,
    }
}

fn yk(k: &str) -> Y {
    Y::String(k.to_string())
}

/// emit.py check_params: keys outside the whitelist are rejected (the legal set matches emit.py:21,47 verbatim -- the FLAG five keys + loss/gemodel/
/// scope/clear + the lenient exemptions at_s/set/steps/repeat/scenario). Value validation matches emit.py:50-56.
fn check_params(p: &YM, ctx: &str) -> Result<(), String> {
    const LEGAL: &[&str] = &[
        "at_s",
        "clear",
        "gemodel",
        "jitter_ms",
        "loss",
        "rate_mbps",
        "reorder_pct",
        "repeat",
        "rtt_ms",
        "scenario",
        "scope",
        "seed",
        "set",
        "steps",
    ];
    for k in p.keys() {
        let ks = k.as_str().unwrap_or_default();
        if !LEGAL.contains(&ks) {
            return Err(format!("{ctx}: unknown keys [{ks}] (whitelist {LEGAL:?})"));
        }
    }
    if let Some(l) = p.get(yk("loss")) {
        let s = py_str(l);
        let core = s.replace('.', "");
        let core = core.trim_end_matches('%');
        if core.is_empty() || !core.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("{ctx}: loss must be a number/percentage, got {s:?}"));
        }
    }
    for k in ["rtt_ms", "jitter_ms", "reorder_pct", "seed"] {
        if let Some(v) = p.get(yk(k))
            && !as_num(v).is_some_and(|n| n >= 0.0)
        {
            return Err(format!("{ctx}: {k} must be a non-negative number"));
        }
    }
    if let Some(v) = p.get(yk("rate_mbps"))
        && !as_num(v).is_some_and(|n| n > 0.0)
    {
        return Err(format!("{ctx}: rate_mbps must be positive"));
    }
    Ok(())
}

/// emit.py flags_from: dict insertion order -> CLI flag string (gemodel composition validation; scope/clear silently stripped like the ancestor).
fn flags_from(p: &YM, ctx: &str) -> Result<String, String> {
    const FLAG: &[(&str, &str)] = &[
        ("rtt_ms", "--rtt"),
        ("jitter_ms", "--jitter"),
        ("rate_mbps", "--rate"),
        ("reorder_pct", "--reorder"),
        ("seed", "--seed"),
    ];
    let mut out: Vec<String> = Vec::new();
    for (k, v) in p {
        let Some(ks) = k.as_str() else { continue };
        if let Some((_, fl)) = FLAG.iter().find(|(n, _)| *n == ks) {
            out.push((*fl).to_string());
            out.push(py_str(v));
        } else if ks == "loss" {
            out.push("--loss".to_string());
            out.push(py_str(v));
        } else if ks == "gemodel" {
            let Some(m) = v.as_mapping() else {
                return Err(format!("{ctx}: gemodel needs dict{{r,h,k}}: {}", py_str(v)));
            };
            let mut trio = Vec::with_capacity(3);
            for key in ["r", "h", "k"] {
                match m.get(yk(key)) {
                    Some(x) => trio.push(py_str(x)),
                    None => {
                        return Err(format!("{ctx}: gemodel needs dict{{r,h,k}}: {}", py_str(v)));
                    }
                }
            }
            out.push("--gemodel".to_string());
            out.extend(trio);
        }
    }
    Ok(out.join(" "))
}

/// dict -> StepOverride. bash convention: --rtt/--jitter/--seed/--reorder need integer literals
/// (str(4.0)="4.0" is rejected by the parse_opts regex with rc=4) -- floats report cause explicitly here.
fn override_from(p: &YM) -> Result<StepOverride, String> {
    let mut ov = StepOverride::default();
    let int_slot = |k: &str| -> Result<Option<u64>, String> {
        match p.get(yk(k)) {
            None => Ok(None),
            Some(v) => match as_int(v) {
                Some(i) if i >= 0 => Ok(Some(i as u64)),
                _ => Err(format!("{k} must be an integer (bash flag domain), got {}", py_str(v))),
            },
        }
    };
    ov.rtt_ms = int_slot("rtt_ms")?;
    ov.jitter_ms = int_slot("jitter_ms")?;
    ov.seed = int_slot("seed")?;
    if let Some(r) = int_slot("reorder_pct")? {
        ov.reorder_pct = Some(r.to_string());
    }
    if let Some(v) = p.get(yk("loss")) {
        ov.loss = Some(py_str(v));
    }
    if let Some(v) = p.get(yk("rate_mbps")) {
        ov.rate_mbps = as_num(v);
    }
    if let Some(g) = p.get(yk("gemodel")) {
        let Some(m) = g.as_mapping() else {
            return Err(format!("gemodel needs dict{{r,h,k}}: {}", py_str(g)));
        };
        let mut trio = Vec::with_capacity(3);
        for key in ["r", "h", "k"] {
            match m.get(yk(key)) {
                Some(x) => trio.push(py_str(x)),
                None => {
                    return Err(format!("gemodel needs dict{{r,h,k}}: {}", py_str(g)));
                }
            }
        }
        ov.gemodel = Some((trio[0].clone(), trio[1].clone(), trio[2].clone()));
    }
    Ok(ov)
}

/// emit.py scenario main parsing (doc already yaml-typed). Plan line order = STEP(s) one by one -> REPEAT -> (TOTAL/CLEAR into the struct).
pub fn parse_plan_doc(doc: &Y) -> Result<ScenarioPlan, String> {
    let empty_map = YM::new();
    let root = doc.as_mapping().unwrap_or(&empty_map);
    let sc = match root.get(yk("scenario")) {
        Some(v) => v,
        None => doc,
    }
    .as_mapping()
    .ok_or_else(|| "scenario must be a mapping".to_string())?;
    let steps: Vec<Y> =
        sc.get(yk("steps")).and_then(|s| s.as_sequence().cloned()).unwrap_or_default();
    let repeat_map: YM =
        sc.get(yk("repeat")).cloned().unwrap_or(Y::Null).as_mapping().cloned().unwrap_or_default();
    if steps.is_empty() && repeat_map.is_empty() {
        return Err("scenario needs at least steps or repeat".to_string());
    }
    let mut lines = Vec::new();
    let mut prev = -1.0f64;
    let mut total = 0.0f64;
    for (i, st) in steps.iter().enumerate() {
        let ctx = format!("step{i}");
        let Some(stm) = st.as_mapping() else {
            return Err(format!("{ctx}: must be a mapping"));
        };
        let at = stm.get(yk("at_s")).cloned().unwrap_or(Y::Null);
        let Some(atf) = as_num(&at) else {
            return Err(format!("{ctx}: at_s must be a non-negative number"));
        };
        if atf < 0.0 {
            return Err(format!("{ctx}: at_s must be a non-negative number"));
        }
        if atf <= prev {
            return Err(format!("{ctx}: at_s must be strictly increasing ({atf} <= {prev})"));
        }
        let set_map = match stm.get(yk("set")).cloned().unwrap_or(Y::Null) {
            Y::Null => YM::new(),
            Y::Mapping(m) => m,
            _ => return Err(format!("{ctx}: set must be a mapping")),
        };
        check_params(&set_map, &ctx)?;
        let flags = flags_from(&set_map, &ctx)?;
        let ov = override_from(&set_map)?;
        lines.push(PlanLine::Step { at_disp: py_str(&at), at_secs: atf as u64, flags, ov });
        prev = atf;
        total = total.max(atf + 15.0); // bash: leave a 15s observation tail after the last step
    }
    if !repeat_map.is_empty() {
        let num = |k: &str| repeat_map.get(yk(k)).cloned().unwrap_or_else(|| Y::Number(0.into()));
        let (Some(r), Some(on_s), Some(off_s)) =
            (as_int(&num("rounds")), as_int(&num("on_s")), as_int(&num("off_s")))
        else {
            return Err("repeat needs rounds>0 and on_s/off_s>=5".to_string());
        };
        if r <= 0 || on_s < 5 || off_s < 5 {
            return Err("repeat needs rounds>0 and on_s/off_s>=5".to_string());
        }
        let set_map = match repeat_map.get(yk("set")).cloned().unwrap_or(Y::Null) {
            Y::Null => YM::new(),
            Y::Mapping(m) => m,
            _ => return Err("repeat.set must be a mapping".to_string()),
        };
        check_params(&set_map, "repeat.set")?;
        let on_flags = flags_from(&set_map, "repeat.set")?;
        let on_ov = override_from(&set_map)?;
        // emit.py:107 off = the fixed zero-param set {rtt_ms:0, jitter_ms:0, loss:"0%"} -- the recovery step's after string
        // contains rtt=0ms + loss=0% (the generation source of the judge recovery predicate).
        let off_ov = StepOverride {
            rtt_ms: Some(0),
            jitter_ms: Some(0),
            loss: Some("0%".into()),
            ..Default::default()
        };
        let off_flags = "--rtt 0 --jitter 0 --loss 0%".to_string();
        lines.push(PlanLine::Repeat {
            rounds: r as u64,
            on_s: on_s as u64,
            off_s: off_s as u64,
            on_flags,
            off_flags,
            on_ov,
            off_ov,
        });
        total = total.max(prev).max((r * (on_s + off_s)) as f64);
    }
    let baseline_s = match sc.get(yk("baseline_s")).and_then(as_num) {
        Some(n) => n.max(0.0) as u64, // python int() truncation form
        None => total.max(0.0) as u64,
    };
    // python truthiness（emit.py:115 `1 if sc.get('clear', True) else 0`）
    let clear = match sc.get(yk("clear")) {
        None => true,
        Some(Y::Null) => false,
        Some(Y::Bool(b)) => *b,
        Some(Y::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Y::String(s)) => !s.is_empty(),
        Some(Y::Sequence(_) | Y::Mapping(_)) => true,
        Some(Y::Tagged(_)) => true, // py yaml tag forms exceed the emit surface: non-empty objects default truthy
    };
    Ok(ScenarioPlan { baseline_s, lines, total_s: total.max(0.0) as u64, clear })
}

/// emit.py scenario grammar port (source = scripts/weaknet.d/emit.py:81-115; keys outside the whitelist rejected,
/// the error lists the full legal set -- fail-closed with the same convention as emit.py:47-49). Grammar:
///
/// ```yaml
/// scenario:            # top-level key optional (bare form read directly, structurally same as emit.py:81 `doc.get('scenario') or doc`)
///   baseline_s: 25     # no-fault twin window (default = auto from steps/repeat durations)
///   clear: true        # teardown at the end (false/0/"" = keep; python truthiness semantics)
///   steps:             # at_s strictly increasing (relative to t0 after baseline); set values = whitelist
///     - at_s: 30
///       set: {rtt_ms: 100, jitter_ms: 20, loss: "10%"}  # rtt_ms/jitter_ms/rate_mbps/reorder_pct/seed/loss/gemodel
///     - at_s: 75
///       set: {rtt_ms: 0, loss: "0%"}   # recovery step: judge predicate = set.after prefix rtt=0ms AND contains loss=0%
///   repeat:            # recovery half-predicate (rounds>0 and on_s/off_s>=5)
///     rounds: 4
///     on_s: 30
///     off_s: 30
///     set: {rtt_ms: 80, loss: "5%"}    # off state fixed zero params --rtt 0 --jitter 0 --loss 0%
/// ```
pub fn parse_plan_yaml(src: &str) -> Result<ScenarioPlan, String> {
    let doc: Y = serde_yaml::from_str(src).map_err(|e| format!("YAML parse failed: {e}"))?;
    parse_plan_doc(&doc)
}

/// Base + step override -> new spec (bash do_set: state base + explicit flags, default fields inherited).
pub fn merge_step(base: &ImpairSpec, ov: &StepOverride) -> Result<ImpairSpec, String> {
    let mut s = base.clone();
    if let Some(v) = ov.rtt_ms {
        s.rtt_ms = v;
    }
    if let Some(v) = ov.jitter_ms {
        s.jitter_ms = v;
    }
    if let Some(v) = ov.rate_mbps {
        s.rate_mbps = Some(v);
    }
    if let Some(v) = &ov.reorder_pct {
        s.reorder_pct = Some(v.clone());
    }
    if let Some(v) = ov.seed {
        s.seed = Some(v);
    }
    if let Some((r, h, k)) = &ov.gemodel {
        let loss = ov.loss.clone().or_else(|| match &s.loss {
            Some(LossSpec::GeModel { loss, .. }) => Some(loss.clone()),
            Some(LossSpec::Simple(p)) if !p.is_empty() => Some(p.clone()),
            _ => None,
        });
        let Some(loss) = loss else {
            return Err("gemodel needs loss (p13; the step inherits the previous value -- the base has no loss either)".to_string());
        };
        s.loss = Some(LossSpec::GeModel { loss, r: r.clone(), h: h.clone(), k: k.clone() });
    } else if let Some(p) = &ov.loss {
        s.loss = Some(LossSpec::Simple(p.clone()));
    }
    s.validate()?;
    Ok(s)
}

/// First-step params (bash L566-568: the first STEP flags, else the first REPEAT on_flags -- steps come first).
fn first_override(plan: &ScenarioPlan) -> Option<&StepOverride> {
    plan.lines.first().map(|l| match l {
        PlanLine::Step { ov, .. } => ov,
        PlanLine::Repeat { on_ov, .. } => on_ov,
    })
}

/// Dry-run the whole plan (rejects illegal values at load time -- bash only explodes step-by-step at set; failing early closes the door on half-applied state,
/// deviation logged in the T8 report). Returns the first-step spec (the base apply shape).
fn validate_full_plan(plan: &ScenarioPlan) -> Result<ImpairSpec, String> {
    let ov = first_override(plan).ok_or_else(|| "scenario has no first-step params".to_string())?;
    let mut cur = merge_step(&ImpairSpec::default(), ov)?;
    for line in &plan.lines {
        let ovs: Vec<&StepOverride> = match line {
            PlanLine::Step { ov, .. } => vec![ov],
            PlanLine::Repeat { on_ov, off_ov, .. } => vec![on_ov, off_ov],
        };
        for o in ovs {
            cur = merge_step(&cur, o)?;
        }
    }
    Ok(cur)
}

// ---------- StepExec abstraction (production = the engine facade; tests = the recording Noop, zero tc touching) ----------

/// Scenario physical-action facade (the runner owns locks/flags/timeline; the impl only applies + emits events, called while the caller holds the lock).
pub trait StepExec: Send + Sync {
    /// apply-shaped first step (impl writes state + the apply event)
    fn apply(&self, req: &ApplyRequest, duration_secs: u64) -> Wn<()>;
    /// set-shaped step (full replay; state keeps prior.job -- done progress flows in through it and lands on disk)
    fn set(&self, req: &ApplyRequest, prior: State) -> Wn<()>;
    /// Site cleanup (lock-free reachability guaranteed by the runner; impl is idempotent)
    fn clear(&self) -> Wn<()>;
}

/// Production facade: engine::replay(Verify::Inline) + engine::clear (structurally identical to bash do_apply/do_set).
#[derive(Debug, Clone)]
pub struct EngineExec {
    pub dirs: Dirs,
    pub env: Env,
}

impl EngineExec {
    #[must_use]
    pub fn new(dirs: Dirs, env: Env) -> Self {
        Self { dirs, env }
    }
}

impl StepExec for EngineExec {
    fn apply(&self, req: &ApplyRequest, duration_secs: u64) -> Wn<()> {
        engine::replay(req, Replay::Apply { duration_secs }, &self.dirs, &self.env, Verify::Inline)
            .map(|_| ())
    }
    fn set(&self, req: &ApplyRequest, prior: State) -> Wn<()> {
        let before = state::param_summary(&prior.spec);
        engine::replay(
            req,
            Replay::Set { prior: Box::new(prior), before },
            &self.dirs,
            &self.env,
            Verify::Inline,
        )
        .map(|_| ())
    }
    fn clear(&self) -> Wn<()> {
        engine::clear(&self.dirs, &self.env, false).map(|_| ())
    }
}

// ---------- job flag layer ----------

pub fn cancel_path(dirs: &Dirs) -> PathBuf {
    dirs.statedir.join(CANCEL_FILE)
}

fn job_alive(j: &JobRef) -> bool {
    fuse::read_proc_starttime(j.pid) == Some(j.starttime)
}

/// External writes (CLI/REST apply/set; incl. a second scenario run) exclusivity gate: owner alive = 409/exit3.
pub fn job_gate(dirs: &Dirs) -> Wn<()> {
    let st = State::read_from(&dirs.state_json()).map_err(Fail::env)?;
    if let Some(j) = st.and_then(|s| s.job).filter(job_alive) {
        return Err(Fail::conflict(format!(
            "scenario in progress (stop first): job {} pid {}",
            j.name, j.pid
        )));
    }
    Ok(())
}

/// runner checkpoint: cancel flag / own job removed or ownership changed (the cross-process stop form).
fn cancel_reason(dirs: &Dirs, name: &str) -> Option<&'static str> {
    if cancel_path(dirs).exists() {
        return Some("user");
    }
    match State::read_from(&dirs.state_json()) {
        Ok(Some(st)) => match &st.job {
            Some(j) if j.name == name && job_alive(j) => None,
            _ => Some("user"),
        },
        Ok(None) => Some("user"),
        Err(_) => None, // state invalid/concurrent write window -- judged again next step (C15: no silent abort, but left to the re-read)
    }
}

fn remove_cancel(dirs: &Dirs) {
    if let Err(e) = std::fs::remove_file(cancel_path(dirs))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("weaknet(scenario): WARN failed to clear the stop flag: {e}");
    }
}

/// Scenario finish fallback: if state still exists and the job owner is us, remove the flag (prevents a long-lived serve process self-locking -- the keep form /
/// the clear-failure path keeps the site but exclusivity must be lifted; a job cleared with the site naturally needs no handling = no-op).
fn clear_own_job(dirs: &Dirs, name: &str) {
    let Ok(Some(st)) = State::read_from(&dirs.state_json()) else {
        return;
    };
    let owned = st.job.as_ref().is_some_and(|j| {
        j.name == name
            && j.pid == std::process::id()
            && fuse::read_proc_starttime(j.pid) == Some(j.starttime)
    });
    if !owned {
        return;
    }
    let Ok(_lk) = engine::take_write_lock(dirs) else {
        eprintln!(
            "weaknet(scenario): WARN failed to take the lock while removing the finish job flag (the dead-owner liveness fallback can still pass)"
        );
        return;
    };
    if let Ok(Some(mut fresh)) = State::read_from(&dirs.state_json())
        && fresh.job.as_ref().is_some_and(|j| j.name == name && j.pid == std::process::id())
    {
        fresh.job = None;
        if let Err(e) = fresh.write_to(&dirs.state_json()) {
            eprintln!(
                "weaknet(scenario): WARN failed to write back after removing the finish job flag: {e}"
            );
        }
    }
}

/// timeline abort marker surface: the reason string of every scenario-end carrying aborted. Consumption contract: non-empty => that round's
/// judge verdict is invalidated (rev-2.2; W6/T10 gates assert the empty set before any verdict may be trusted).
pub fn aborted_in_timeline(dirs: &Dirs) -> Wn<Vec<String>> {
    let raw = match std::fs::read_to_string(dirs.timeline()) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Fail::env(format!("failed to read the timeline: {e}"))),
    };
    Ok(raw
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e.get("ev").and_then(serde_json::Value::as_str) == Some("scenario-end"))
        .filter_map(|e| e.get("aborted").and_then(serde_json::Value::as_str).map(str::to_string))
        .collect())
}

/// `scenario stop` / POST /v1/scenario/stop (lock-free reachable, always exit0/200 semantics).
///
/// Owner alive -> write the cancel flag (the runner self-terminates at the next checkpoint within 1s; the runner emits scenario-end);
/// owner dead/no job -> idempotent zero events (stale flags self-clear; deviation: a crash site does not fabricate a scenario-end moment).
pub fn stop(dirs: &Dirs) -> Wn<String> {
    let path = dirs.state_json();
    let st = State::read_from(&path).map_err(Fail::env)?;
    if st.as_ref().and_then(|s| s.job.clone()).is_some_and(|j| job_alive(&j)) {
        std::fs::write(cancel_path(dirs), "user\n")
            .map_err(|e| Fail::env(format!("failed to write the stop flag: {e}")))?;
        return Ok(
            "stop requested (owner alive, takes effect at the next checkpoint at the latest)"
                .into(),
        );
    }
    if let Some(j) = st.and_then(|s| s.job) {
        // stale owner: clear the flag and pass (rev-2.2 m1). Transient lock guards against concurrent-write overwrite.
        let _lk = engine::take_write_lock(dirs)?;
        if let Some(mut fresh) = State::read_from(&path).map_err(Fail::env)? {
            fresh.job = None;
            fresh.write_to(&path).map_err(Fail::env)?;
        }
        drop(_lk);
        return Ok(format!("stale job '{}' owner dead -- flag cleared", j.name));
    }
    remove_cancel(dirs);
    Ok("no active scenario job (idempotent)".into())
}

// ---------- Scheduler ----------

#[derive(Debug, Clone)]
pub struct RunSpec {
    pub name: String,
    /// the `$file` slot of the scenario-start note (bash = expanded path; serve inline = "inline")
    pub file_disp: String,
    pub plan: ScenarioPlan,
    pub no_baseline: bool,
    pub keep: bool,
    pub targeting: Targeting,
    pub iface: String,
    pub sig_port: Option<u16>,
    /// test injection (production None -> constant 25s; not a bash invention, just so unit tests can run)
    pub recovery_dwell_secs: Option<u64>,
}

pub struct RunOutcome {
    pub aborted: Option<String>,
    pub elapsed_s: u64,
    pub summary: String,
    /// CLI exit-code mapping: lock-busy -> 3 / user -> 0 / step-failed -> original code / not started -> original code
    pub exit_code: u32,
}

fn req_for(spec: ImpairSpec, rs: &RunSpec) -> ApplyRequest {
    ApplyRequest {
        spec,
        scope: rs.targeting.scope,
        names: rs.targeting.scope_names(),
        iface: rs.iface.clone(),
        ports: rs.targeting.ports.clone(),
        pairs: rs.targeting.pairs.clone(),
        sig_port: rs.sig_port,
    }
}

fn log(msg: &str) {
    // bash log() same shape: stderr + "weaknet: " prefix
    eprintln!("weaknet: {msg}");
}

/// Blocking phase (CLI spawn_blocking / serve before the synchronous response): exclusivity gate -> dry-run validation -> first-step apply ->
/// job flag persistence -> scenario-start event (bash order; scenario-start comes after apply, L571).
pub fn start_blocking(rs: &RunSpec, exec: &Arc<dyn StepExec>, dirs: &Dirs) -> Wn<()> {
    job_gate(dirs)?;
    remove_cancel(dirs);
    validate_full_plan(&rs.plan).map_err(Fail::bad_param)?;
    let ov = first_override(&rs.plan)
        .ok_or_else(|| Fail::bad_param("scenario has no first-step params"))?;
    let spec = merge_step(&ImpairSpec::default(), ov).map_err(Fail::bad_param)?;
    let covering = rs.plan.baseline_s + rs.plan.total_s + COVERING_MARGIN_SECS;
    let req = req_for(spec, rs);
    {
        let _lk = engine::take_write_lock(dirs)?;
        exec.apply(&req, covering)?;
        let mut st = State::read_from(&dirs.state_json())
            .map_err(Fail::env)?
            .ok_or_else(|| Fail::env("scenario: state missing after apply succeeded"))?;
        st.job = Some(JobRef {
            name: rs.name.clone(),
            pid: std::process::id(),
            starttime: fuse::read_proc_starttime(std::process::id()).unwrap_or(0),
            done: 0,
            total: rs.plan.lines.len() as u32,
        });
        st.write_to(&dirs.state_json()).map_err(Fail::env)?;
    }
    state::timeline_append(
        dirs,
        json!({
            "ev": "scenario-start",
            "note": format!(
                "{} baseline={}s steps_dur={}s",
                rs.file_disp, rs.plan.baseline_s, rs.plan.total_s
            ),
        }),
    )
    .map_err(Fail::env)?;
    Ok(())
}

async fn sleep_checked(dirs: &Dirs, name: &str, secs: u64) -> Result<(), &'static str> {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        let now = Instant::now();
        if now >= until {
            return Ok(());
        }
        if let Some(r) = cancel_reason(dirs, name) {
            return Err(r);
        }
        tokio::time::sleep((until - now).min(Duration::from_secs(1))).await;
    }
}

/// Wait until t0+target (bash while+sleep 1s polling); returns actual_s (integer seconds, same convention as bash SECONDS).
async fn wait_until(
    dirs: &Dirs,
    name: &str,
    base: Instant,
    target: u64,
) -> Result<u64, &'static str> {
    loop {
        let got = base.elapsed().as_secs();
        if got >= target {
            return Ok(got);
        }
        if let Some(r) = cancel_reason(dirs, name) {
            return Err(r);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

enum StepErr {
    LockBusy,
    Cancelled(&'static str),
    Other(Fail),
}

/// Single step (inside the transient lock: read state and verify the job -> done progress into prior -> merge -> exec.set (full replay,
/// job persisted with prior) -> timeline note). Returns the new cur spec (Ok) for the caller to chain.
// all 8 params are required context inside the transient lock (dirs/exec/rs/cur/ov/done/note_ev/note) -- splitting into a struct only adds noise.
#[allow(clippy::too_many_arguments)]
fn step_sync(
    dirs: &Dirs,
    exec: &Arc<dyn StepExec>,
    rs: &RunSpec,
    cur: ImpairSpec,
    ov: &StepOverride,
    done: u32,
    note_ev: &'static str,
    note: String,
) -> Result<ImpairSpec, StepErr> {
    let _lk = engine::take_write_lock(dirs).map_err(|_| StepErr::LockBusy)?;
    let mut prior = State::read_from(&dirs.state_json())
        .map_err(|e| StepErr::Other(Fail::env(e)))?
        .ok_or(StepErr::Cancelled("user"))?; // state removed externally = the cross-process stop/clear form
    match &prior.job {
        Some(j) if j.name == rs.name && fuse::read_proc_starttime(j.pid) == Some(j.starttime) => {}
        _ => return Err(StepErr::Cancelled("user")),
    }
    if let Some(j) = &mut prior.job {
        j.done = done;
    }
    let merged = merge_step(&cur, ov).map_err(|e| StepErr::Other(Fail::bad_param(e)))?;
    let req = req_for(merged.clone(), rs);
    exec.set(&req, prior).map_err(StepErr::Other)?;
    state::timeline_append(dirs, json!({"ev": note_ev, "note": note}))
        .map_err(|e| StepErr::Other(Fail::env(e)))?;
    Ok(merged)
}

/// abort wrap-up: scenario-end + aborted field (the judge-verdict-invalidation signal, rev-2.2) -> site cleanup (unless keep) ->
/// flag cleanup. bash has no abort event shape (the process just dies) -- emitting one is this repo's authorized contract (the tasks T8 contract row).
async fn finish_abort(
    dirs: &Arc<Dirs>,
    exec: &Arc<dyn StepExec>,
    rs: &RunSpec,
    elapsed_s: u64,
    reason: String,
    code: u32,
) -> RunOutcome {
    let _ = state::timeline_append(
        dirs,
        json!({
            "ev": "scenario-end",
            "note": format!("elapsed_s={elapsed_s}"),
            "aborted": reason,
        }),
    );
    if !rs.keep
        && let Err(e) = exec.clear()
    {
        log(&format!(
            "clear after abort failed (site residue, left to the watchdog/manual clear): {}",
            e.msg
        ));
    }
    remove_cancel(dirs);
    clear_own_job(dirs, &rs.name);
    RunOutcome {
        aborted: Some(reason),
        elapsed_s,
        summary: format!("scenario aborted: elapsed_s={elapsed_s}"),
        exit_code: code,
    }
}

/// Scheduling main body (non-blocking phase: baseline window -> steps/repeat -> recovery dwell -> scenario-end -> finish clear).
/// Precondition: [`start_blocking`] passed. t0 convention = after the baseline (bash L577, elapsed excludes the baseline).
pub async fn drive(
    rs: Arc<RunSpec>,
    exec: Arc<dyn StepExec>,
    dirs: Arc<Dirs>,
    mut cur: ImpairSpec,
) -> RunOutcome {
    let plan = rs.plan.clone();
    if !rs.no_baseline && plan.baseline_s > 0 {
        log(&format!(
            "baseline window {}s (no-fault twin; keeps the retention ratio = fault/baseline J2>0.95 convention)",
            plan.baseline_s
        ));
        if let Err(r) = sleep_checked(&dirs, &rs.name, plan.baseline_s).await {
            return finish_abort(&dirs, &exec, &rs, 0, r.into(), if r == "user" { 0 } else { 2 })
                .await;
        }
        if let Err(e) = state::timeline_append(
            &dirs,
            json!({"ev": "baseline-done", "note": format!("window_s={}", plan.baseline_s)}),
        ) {
            return finish_abort(&dirs, &exec, &rs, 0, format!("timeline-failed: {e}"), 2).await;
        }
    }
    let t0 = Instant::now();
    for (i, line) in plan.lines.iter().enumerate() {
        let done = (i + 1) as u32;
        match line {
            PlanLine::Step { at_disp, at_secs, ov, .. } => {
                let got = match wait_until(&dirs, &rs.name, t0, *at_secs).await {
                    Ok(g) => g,
                    Err(r) => {
                        let code = if r == "user" { 0 } else { 2 };
                        return finish_abort(
                            &dirs,
                            &exec,
                            &rs,
                            t0.elapsed().as_secs(),
                            r.into(),
                            code,
                        )
                        .await;
                    }
                };
                let note = format!(
                    "at_s={at_disp} actual_s={got} drift_le_1s={}",
                    if got.saturating_sub(*at_secs) <= 1 { 1 } else { 0 }
                );
                let (dirs2, exec2, rs2, cur2) =
                    (dirs.clone(), exec.clone(), rs.clone(), cur.clone());
                let ov_owned = ov.clone();
                let done_l = done;
                let note_l = note.clone();
                let res = tokio::task::spawn_blocking(move || {
                    step_sync(
                        &dirs2,
                        &exec2,
                        &rs2,
                        cur2,
                        &ov_owned,
                        done_l,
                        "scenario-step",
                        note_l,
                    )
                })
                .await;
                match res {
                    Ok(Ok(new_cur)) => cur = new_cur,
                    Ok(Err(StepErr::LockBusy)) => {
                        return finish_abort(
                            &dirs,
                            &exec,
                            &rs,
                            t0.elapsed().as_secs(),
                            "lock-busy".into(),
                            3,
                        )
                        .await;
                    }
                    Ok(Err(StepErr::Cancelled(r))) => {
                        return finish_abort(
                            &dirs,
                            &exec,
                            &rs,
                            t0.elapsed().as_secs(),
                            r.into(),
                            if r == "user" { 0 } else { 2 },
                        )
                        .await;
                    }
                    Ok(Err(StepErr::Other(f))) => {
                        return finish_abort(
                            &dirs,
                            &exec,
                            &rs,
                            t0.elapsed().as_secs(),
                            format!("step-failed: {}", f.msg),
                            f.code,
                        )
                        .await;
                    }
                    Err(j) => {
                        return finish_abort(
                            &dirs,
                            &exec,
                            &rs,
                            t0.elapsed().as_secs(),
                            format!("step-failed: task crashed {j}"),
                            2,
                        )
                        .await;
                    }
                }
            }
            PlanLine::Repeat { rounds, on_s, off_s, on_ov, off_ov, .. } => {
                log(&format!(
                    "repeat: rounds={rounds} on={on_s}s off={off_s}s (recovery half-predicate, shrunk from the CH 4x30s philosophy)"
                ));
                for r in 1..=*rounds {
                    for (phase, secs, ov) in [("on", *on_s, on_ov), ("off", *off_s, off_ov)] {
                        let (dirs2, exec2, rs2, cur2) =
                            (dirs.clone(), exec.clone(), rs.clone(), cur.clone());
                        let ov_owned = ov.clone();
                        let note = format!("r={r} phase={phase}");
                        let res = tokio::task::spawn_blocking(move || {
                            step_sync(
                                &dirs2,
                                &exec2,
                                &rs2,
                                cur2,
                                &ov_owned,
                                done,
                                "repeat-round",
                                note,
                            )
                        })
                        .await;
                        cur = match res {
                            Ok(Ok(c)) => c,
                            Ok(Err(StepErr::LockBusy)) => {
                                return finish_abort(
                                    &dirs,
                                    &exec,
                                    &rs,
                                    t0.elapsed().as_secs(),
                                    "lock-busy".into(),
                                    3,
                                )
                                .await;
                            }
                            Ok(Err(StepErr::Cancelled(r))) => {
                                let code = if r == "user" { 0 } else { 2 };
                                return finish_abort(
                                    &dirs,
                                    &exec,
                                    &rs,
                                    t0.elapsed().as_secs(),
                                    r.into(),
                                    code,
                                )
                                .await;
                            }
                            Ok(Err(StepErr::Other(f))) => {
                                return finish_abort(
                                    &dirs,
                                    &exec,
                                    &rs,
                                    t0.elapsed().as_secs(),
                                    format!("step-failed: {}", f.msg),
                                    f.code,
                                )
                                .await;
                            }
                            Err(j) => {
                                return finish_abort(
                                    &dirs,
                                    &exec,
                                    &rs,
                                    t0.elapsed().as_secs(),
                                    format!("step-failed: task crashed {j}"),
                                    2,
                                )
                                .await;
                            }
                        };
                        if let Err(c) = sleep_checked(&dirs, &rs.name, secs).await {
                            let code = if c == "user" { 0 } else { 2 };
                            return finish_abort(
                                &dirs,
                                &exec,
                                &rs,
                                t0.elapsed().as_secs(),
                                c.into(),
                                code,
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }
    // bash L602 awk predicate: any STEP flags containing "--rtt 0" -> recovery dwell window
    if plan
        .lines
        .iter()
        .any(|l| matches!(l, PlanLine::Step { flags, .. } if flags.contains("--rtt 0")))
    {
        let dw = rs.recovery_dwell_secs.unwrap_or(RECOVERY_DWELL_SECS);
        log(&format!("recovery dwell window {dw}s (recovery ratio sampling)"));
        if let Err(r) = sleep_checked(&dirs, &rs.name, dw).await {
            let code = if r == "user" { 0 } else { 2 };
            return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), r.into(), code).await;
        }
    }
    let elapsed_s = t0.elapsed().as_secs();
    if let Err(e) = state::timeline_append(
        &dirs,
        json!({"ev": "scenario-end", "note": format!("elapsed_s={elapsed_s}")}),
    ) {
        return finish_abort(&dirs, &exec, &rs, elapsed_s, format!("timeline-failed: {e}"), 2)
            .await;
    }
    let keep = rs.keep || !plan.clear;
    let mut code = 0u32;
    let mut extra = String::new();
    if keep {
        log("scenario finished: site kept");
    } else if let Err(e) = exec.clear() {
        log(&format!("finish clear failed: {}", e.msg));
        code = e.code;
        extra = " clear-failed".into();
    }
    remove_cancel(&dirs);
    clear_own_job(&dirs, &rs.name);
    RunOutcome {
        aborted: None,
        elapsed_s,
        summary: format!(
            "scenario done: elapsed_s={elapsed_s} steps={} keep={keep}{extra}",
            plan.lines.len(),
        ),
        exit_code: code,
    }
}

/// CLI full chain (start blocking phase + drive); serve splits phases and calls [`start_blocking`]+tokio::spawn([`drive`]) directly.
pub async fn run_full(rs: RunSpec, exec: Arc<dyn StepExec>, dirs: Dirs) -> RunOutcome {
    let rs = Arc::new(rs);
    let dirs = Arc::new(dirs);
    let (rs2, exec2, dirs2) = (rs.clone(), exec.clone(), dirs.clone());
    let start = tokio::task::spawn_blocking(move || {
        let cur_probe = start_blocking(&rs2, &exec2, &dirs2);
        (cur_probe, read_start_spec(&dirs2))
    })
    .await;
    let (sr, cur) = match start {
        Ok(v) => v,
        Err(j) => {
            return RunOutcome {
                aborted: None,
                elapsed_s: 0,
                summary: format!("startup task crashed: {j}"),
                exit_code: 2,
            };
        }
    };
    if let Err(e) = sr {
        return RunOutcome {
            aborted: None,
            elapsed_s: 0,
            summary: format!("startup failed: {}", e.msg),
            exit_code: e.code,
        };
    }
    drive(rs, exec, dirs, cur).await
}

/// Signal-layer wrapper around [`run_full`] (the CLI side of design §fuse SIGINT/SIGTERM --auto-clear --
/// no second damage-control channel invented): any signal writes the same cancel flag file as [`stop`]'s owner-alive form,
/// then **keeps waiting** for run_full to self-terminate at its next checkpoint (sleep_checked/wait_until at 1s granularity, not interruptible
/// inside the step_sync lock window). Output timeline shape = `scenario-end{aborted:"user"}` + the existing abort wrap-up
/// (exec.clear site cleanup when not keep), exit_code=0 -- byte-identical to cross-process `scenario stop`
/// (test F pins this path; this function only swaps the flag writer: signal -> flag).
///
/// Boundary (minimal semantics, logged here): if the signal lands during the start_blocking blocking phase (before apply persists),
/// the entry `remove_cancel` consumes that flag and run proceeds as usual -- pressing the signal again then takes the abort path.
///
/// The serve-side /v1/scenario/run does not go through this path (in a long-lived process SIGINT = the whole service going down; its stop surface already has
/// the /v1/scenario/stop endpoint) -- behavior unchanged, this wrapper only serves the CLI foreground `scenario run`.
pub async fn run_full_auto_clear(rs: RunSpec, exec: Arc<dyn StepExec>, dirs: Dirs) -> RunOutcome {
    let cancel = dirs.clone(); // Dirs: Clone (used by stop to write the flag)
    let outcome = run_full(rs, exec, dirs);
    tokio::pin!(outcome);
    // listens on both ctrl_c + SIGTERM (the signal feature is already in Cargo.toml, no new dependency).
    let signals = async {
        match unix_signals().await {
            Ok(()) => request_stop(&cancel),
            Err(e) => eprintln!(
                "weaknet(scenario): WARN failed to set up signal listening (--auto-clear inert, falls back to the default Ctrl-C kill): {e}"
            ),
        }
    };
    tokio::pin!(signals);
    // single select: if the signal arrives first, write the flag then keep waiting for the outcome (the runner self-terminates at the next checkpoint);
    // if the outcome completes first, drop the listener and return. No loop -- avoids re-polling a completed future.
    tokio::select! {
            biased; // run completion exits with priority, no orphan listener left
            o = &mut outcome => o,
            () = &mut signals => {
    // flag written: keep waiting for run_full to self-terminate at the next checkpoint
                outcome.await
            }
        }
}

/// Returns when either termination signal (SIGINT/SIGTERM) arrives. Registration failure propagates up (the caller downgrades to WARN,
/// does not kill run -- the signal layer is a convenience surface, not a correctness dependency).
async fn unix_signals() -> Result<(), std::io::Error> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }
    Ok(())
}

/// Flag write (same file and content as [`stop`]'s owner-alive branch -- during run_full this process is the owner,
/// no need to re-read state for liveness; stop's dead-owner self-clear branch is unreachable here).
fn request_stop(dirs: &Dirs) {
    if let Err(e) = std::fs::write(cancel_path(dirs), "user\n") {
        eprintln!("weaknet(scenario): WARN failed to write the stop flag: {e}");
    }
}

/// Re-read the currently installed spec after start (the base the drive chains onto; disk is authoritative, guarding against missed external changes between start and drive).
fn read_start_spec(dirs: &Dirs) -> ImpairSpec {
    State::read_from(&dirs.state_json()).ok().flatten().map(|s| s.spec).unwrap_or_default()
}

// ---------- File resolution (CLI/serve both entries; structurally identical to the bash L551 expansion rule) ----------

/// serve: basename-only -> scenarios/<name>.yaml + canonicalize containment assert (rev-2.2 m-batch;
/// the panel passes the stem, .yaml auto-appended).
pub fn resolve_serve_file(basename: &str) -> Wn<(std::path::PathBuf, String)> {
    let root = crate::scope::weaknet_d_root().ok_or_else(|| {
        Fail::env("no scenario asset dir (WEAKNET_ASSETS_DIR / weaknet.d next to the binary / scripts/weaknet.d)")
    })?;
    let sdir = root.join("scenarios");
    let fname = if basename.ends_with(".yaml") || basename.ends_with(".yml") {
        basename.to_string()
    } else {
        format!("{basename}.yaml")
    };
    let rp = sdir
        .join(&fname)
        .canonicalize()
        .map_err(|e| Fail::bad_param(format!("no scenario: {fname} ({}: {e})", sdir.display())))?;
    let rd = sdir
        .canonicalize()
        .map_err(|e| Fail::env(format!("scenario dir unreachable {}: {e}", sdir.display())))?;
    if !rp.starts_with(&rd) {
        return Err(Fail::bad_param(
            "scenario path escapes the scenarios/ dir (after canonicalize)",
        ));
    }
    let stem = rp.file_stem().and_then(|s| s.to_str()).unwrap_or("scenario").to_string();
    Ok((rp, stem))
}

/// CLI: bash expansion form (*.yaml used directly; else scenarios root/<name>.yaml); --dir overrides the root.
pub fn resolve_run_file(
    file: &str,
    dir_override: Option<&str>,
) -> Wn<(std::path::PathBuf, String)> {
    let path = if file.ends_with(".yaml") || file.ends_with(".yml") {
        std::path::PathBuf::from(file)
    } else {
        let root = match dir_override {
            Some(d) => std::path::PathBuf::from(d),
            None => crate::scope::weaknet_d_root()
                .ok_or_else(|| {
                    Fail::env("no scenario asset dir (--dir given, or WEAKNET_ASSETS_DIR/weaknet.d in place)")
                })?
                .join("scenarios"),
        };
        root.join(format!("{file}.yaml"))
    };
    if !path.is_file() {
        return Err(Fail::bad_param(format!("no scenario: {}", path.display())));
    }
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("scenario").to_string();
    Ok((path, stem))
}

/// Embedded asset-name guard: basename-only (no path separators/wildcards/empty/`..`) -- the §serve security
/// "scenario basename" contract applies to the embedded side too (rust-embed is a key lookup, not fs, but an explicit gate anyway).
#[must_use]
pub fn is_safe_asset_name(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('/')
        && !s.contains('\\')
        && !s.contains('\0')
        && !s.contains('*')
        && s != ".."
        && s != "."
}

/// Asset name -> relative key (`<name>.yaml` auto-appended, same rule as resolve_serve_file fname).
#[must_use]
pub fn asset_rel(sub: &str, name: &str) -> String {
    if name.ends_with(".yaml") || name.ends_with(".yml") {
        format!("{sub}/{name}")
    } else {
        format!("{sub}/{name}.yaml")
    }
}

/// CLI scenario read (T13 vehicle-side resolution, single function): fs resolution first, bare names fall back to embedded.
/// Returns (raw, stem, disp); fs cause is passed through for "non-embeddable forms" (with paths/wildcards/--dir override).
pub fn read_run_yaml(file: &str, dir_override: Option<&str>) -> Wn<(String, String, String)> {
    match resolve_run_file(file, dir_override) {
        Ok((path, stem)) => {
            let raw = std::fs::read_to_string(&path).map_err(|e| {
                Fail::env(format!("failed to read scenario {}: {e}", path.display()))
            })?;
            Ok((raw, stem, path.display().to_string()))
        }
        Err(fs_err) => {
            if dir_override.is_some() || !is_safe_asset_name(file) {
                return Err(fs_err);
            }
            let rel = asset_rel("scenarios", file);
            match crate::scope::read_asset(&rel)? {
                Some(raw) => Ok((
                    raw,
                    file.trim_end_matches(".yaml").trim_end_matches(".yml").to_string(),
                    format!("embedded {rel}"),
                )),
                None => Err(fs_err),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dirs(tag: &str) -> Dirs {
        let d = std::env::temp_dir().join(format!("wnet-sig-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        Dirs { statedir: d }
    }

    // real-signal E2E (self-killing kill -TERM this process) deliberately absent: a signal handler is process-global state
    // (after signal_hook registration all listeners in the same process wake together); under libtest parallelism any sibling test registering
    // unix_signals (e.g. the wrap test below going through run_full_auto_clear) would be spuriously triggered by this SIGTERM --
    // nondeterministic cross-contamination. unix_signals itself is an 8-line direct wrapper of tokio::signal; its effect surface
    // (-> request_stop lands the same flag -> the runner self-terminates at a checkpoint) is pinned by the idempotence test below + the wrap test +
    // tests/scenario_engine.rs::I.

    /// request_stop idempotence: writing again while the flag exists must not error (the window where the runner hasn't consumed it after a signal).
    #[test]
    fn request_stop_idempotent() {
        let dirs = tmp_dirs("idem");
        request_stop(&dirs);
        request_stop(&dirs);
        assert!(cancel_path(&dirs).exists());
        remove_cancel(&dirs);
        std::fs::remove_dir_all(&dirs.statedir).ok();
    }

    /// The signal-layer equivalent of path F: a concurrent task writes the cancel flag (= request_stop's effect; the handler itself
    /// is just a thin unix_signals -> request_stop wrapper; the reason real-signal E2E can't be parallel-tested is in the comment block above). Asserts
    /// run_full_auto_clear keeps waiting for run_full to self-terminate at a checkpoint and returns the aborted:user outcome.
    /// The full teardown surface (clear event / site zeroed) is pinned in tests/scenario_engine.rs::I (needs the NoopExec scaffold).
    #[tokio::test]
    async fn auto_clear_waits_checkpoint_after_flag_write() {
        struct Nop(Dirs);
        impl StepExec for Nop {
            fn apply(&self, req: &ApplyRequest, _dur: u64) -> Wn<()> {
                let st = State {
                    schema: crate::state::STATE_SCHEMA.to_string(),
                    spec: req.spec.clone(),
                    dir: req.spec.dir,
                    scope: req.scope,
                    iface: req.iface.clone(),
                    ports: req.ports.clone(),
                    pairs: req.pairs.clone(),
                    rooms: req.names.rooms.clone(),
                    devices: req.names.devices.clone(),
                    sig_port: req.sig_port,
                    expires_at_ms: fuse::now_epoch_ms() + 300_000,
                    created_root: true,
                    ifb_used: false,
                    created_ifb: false,
                    job: None,
                    teardown: crate::state::Teardown {
                        channel: crate::state::ChannelSer::LocalRoot,
                        sidecar: None,
                        steps: vec![],
                    },
                };
                st.write_to(&self.0.state_json()).map_err(Fail::env)
            }
            fn set(&self, _req: &ApplyRequest, prior: State) -> Wn<()> {
                prior.write_to(&self.0.state_json()).map_err(Fail::env)
            }
            fn clear(&self) -> Wn<()> {
                Ok(())
            }
        }
        let dirs = tmp_dirs("wrap");
        let exec = Arc::new(Nop(dirs.clone()));
        // single-line short plan + baseline 0: run is inside the wait_until(60s) window, the flag must be hit at a <=1s checkpoint.
        let plan = parse_plan_yaml(
            "scenario:\n  baseline_s: 0\n  steps:\n    - at_s: 60\n      set: {rtt_ms: 80}\n",
        )
        .unwrap();
        let rs = RunSpec {
            name: "wrap".into(),
            file_disp: "wrap.yaml".into(),
            plan,
            no_baseline: true,
            keep: true, // this case only validates the wrapper's waiting shape; site cleanup belongs to test I
            targeting: crate::scope::Targeting {
                scope: crate::spec::ScopeSel::Media,
                ports: vec![40010],
                pairs: vec![],
                names: vec![],
            },
            iface: "lo".into(),
            sig_port: None,
            recovery_dwell_secs: Some(0),
        };
        let exec: Arc<dyn StepExec> = exec;
        let dirs_cl = dirs.clone();
        let flagger = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            request_stop(&dirs_cl);
        });
        let out = tokio::time::timeout(
            Duration::from_secs(10),
            run_full_auto_clear(rs, exec, dirs.clone()),
        )
        .await
        .expect(
            "the wrapper should self-terminate at a checkpoint, not hang until the 60s step window",
        );
        let _ = flagger.await;
        assert_eq!(out.aborted.as_deref(), Some("user"));
        assert_eq!(out.exit_code, 0);
        assert!(!cancel_path(&dirs).exists(), "abort wrap-up should consume the flag");
        std::fs::remove_dir_all(&dirs.statedir).ok();
    }
}
