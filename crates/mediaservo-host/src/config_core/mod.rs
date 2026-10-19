//! T1″ batch2 — config-core spine (annex §2: one executor, seven steps, flock-serialized).
//!
//! Single serialized executor for host-config activation: in-lock fingerprint re-read,
//! validate + C40 token pre-check, staging render, registry-sourced reverse-order
//! teardown, C25 SHM clear, hard dual-topology verification gate, prepare/commit
//! (pending_apply before activation) and reconcile + ledger. Batch4 routes the CLI/agent
//! verbs through `execute`; this batch ships the callable core only — no bells, no
//! protocol fields.
//!
//! Topology contract: `AgentRun` = the executor IS the agent process (it never stops
//! itself; the declarative apply swaps it last), `CliRun` = a CLI process executes and
//! may stop the agent too.

use std::path::Path;
use std::time::{Duration, Instant};

pub use self::ledger::{Ledger, file_hash, short12};
use crate::conflict::{self, SurvivorInfo};
use crate::translate;

pub mod ledger;

use self::ledger::{Flags, LastPush, PendingApply, Running, now_rfc3339};

/// Blocking flock budget for step0 — a queued bell waits for the running core, it does
/// not skip it (annex §3: every trigger queues here).
const LOCK_TIMEOUT: Duration = Duration::from_secs(120);
/// step5 teardown-verification retry budget (annex §2: "Retry-with-timeout ~15s").
const GATE_TIMEOUT: Duration = Duration::from_secs(15);
/// Shared poll step for the lock wait and the verification gate.
const RETRY_STEP: Duration = Duration::from_millis(500);
/// Staging file name (activated only by the step6 rename).
const STAGING_NAME: &str = "oxfile.toml.staging";
/// conflict.rs writes this literal into `SurvivorInfo.owner` when the environ carries no
/// OXMGR_HOME — the unknown-owner branch of the instance scoping keys on it.
const OWNER_UNKNOWN: &str = "owner unknown";

/// Which process runs the core (annex §2 step3/step5 dual topology).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    /// host-agent executes: the agent entry stays in the registry and is swapped by apply.
    AgentRun,
    /// CLI executes: the agent is torn down too; the gate expects a fully empty registry.
    CliRun,
}

/// Who/why this run happened — self-report labels written into the ledger (§9.2:
/// "push" | "local" | "cli" origin, actor when known).
#[derive(Debug, Clone, Copy)]
pub struct ExecuteOpts<'a> {
    pub origin: &'a str,
    pub actor: Option<&'a str>,
}

/// End state of one core run.
#[derive(Debug)]
pub enum Outcome {
    /// Fingerprint already running (verified in-lock), nothing touched.
    NoOp { hash: String },
    /// Fresh cluster live and reconciled; ledger committed.
    Applied { hash: String },
    /// Config refused at a gate — old cluster keeps running untouched.
    Refused { reason: String },
    /// Teardown verification failed (step5): refused to start over corpses, new oxfile
    /// NOT activated; `survivors` are instance-scoped family processes still alive.
    Residue { survivors: Vec<SurvivorInfo> },
}

/// File-watch bell verdict (T1b3 k2): debounce over a 2s sampling loop so half-written
/// edits never reach the core. See [`watch_tick`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchAction {
    /// Sample equals the baseline (or is an io-error None): keep watching.
    Follow,
    /// First sighting of a change: a second equal sample must confirm it.
    Wait,
    /// Confirmed change (two consecutive equal samples != baseline); carries the new
    /// fingerprint, which the bell adopts as baseline before running the core.
    Trigger(String),
}

/// Pure state machine of the file-watch bell: Trigger only when
/// `Some(cur) != baseline && Some(cur) == prev`. The in-lock fingerprint re-read inside
/// `execute` is the correctness net -- this machine is debounce, never authority.
pub fn watch_tick(cur: Option<String>, prev: Option<String>, baseline: &str) -> WatchAction {
    match cur {
        None => WatchAction::Follow,
        Some(h) if h == baseline => WatchAction::Follow,
        Some(h) if prev.as_deref() == Some(h.as_str()) => WatchAction::Trigger(h),
        Some(_) => WatchAction::Wait,
    }
}

/// Run the seven-step core once against `dir` (an initialized instance tree).
/// `force` bypasses only the step1 fingerprint short-circuit — every gate still applies.
/// Err is reserved for operational failures (lock timeout, io, oxmgr unrunnable, ledger
/// unwritable); every config-level rejection is a typed `Outcome`.
pub fn execute(
    dir: &Path,
    topo: Topology,
    force: bool,
    opts: &ExecuteOpts<'_>,
) -> Result<Outcome, String> {
    // ── step0 — per-instance serialization (flock auto-releases on process death) ──
    let _guard = CoreLock::acquire(dir)?;

    // ── step1 — IN-LOCK re-read: the fingerprint check is the correctness boundary,
    // any bell-level pre-check outside is only an optimization and must not be trusted. ──
    let cfg_path = dir.join("etc").join("host.yaml");
    let bytes =
        std::fs::read(&cfg_path).map_err(|e| format!("read {} failed: {e}", cfg_path.display()))?;
    let hash = file_hash(&bytes);
    let mut led = Ledger::load(dir);
    let pending_mismatch = led.pending_apply.as_ref().is_some_and(|p| p.hash != hash);
    let running_match = led.running.as_ref().is_some_and(|r| r.hash == hash);
    if !should_reconcile(force, running_match, pending_mismatch, false)
        && noop_may_skip(dir, &bytes, running_match)
    {
        return Ok(Outcome::NoOp { hash });
    }

    // ── step1 validate gate — parse + D274 enum gates (inside stream_configs) +
    // V5 hmac-key guard (same source of truth as write_oxfile) + C40 token pre-check. ──
    let cfg = String::from_utf8(bytes).map_err(|e| format!("host.yaml is not valid utf-8: {e}"))?;
    if let Err(e) = translate::validate(&cfg) {
        return Ok(Outcome::Refused { reason: e });
    }
    if let Some(rel) = translate::control_hmac_key_file(&cfg)? {
        let p = dir.join(rel); // absolute rel passes through join directly
        if !p.exists() {
            return Ok(Outcome::Refused {
                reason: format!(
                    "[control].hmac_key_file points to a missing file: {}",
                    p.display()
                ),
            });
        }
    }
    // Render the roster IN MEMORY first: the token set is exactly what the roster
    // references (the T1-e tri-state drops the recorder slot from the render when
    // explicitly disabled, so its token is only demanded when the slot lives).
    let rendered = translate::to_oxfile_in_dir(&cfg, dir)?;
    let missing: Vec<String> =
        token_paths(&rendered).into_iter().filter(|p| !Path::new(p).exists()).collect();
    if !missing.is_empty() {
        return Ok(Outcome::Refused {
            reason: format!(
                "token pre-check (C40): missing {} — run `token issue --all {}`",
                missing.join(", "),
                dir.display()
            ),
        });
    }

    // ── step2 — staging render (the live oxfile is untouched until the step6 rename) ──
    let staging = dir.join("run").join(STAGING_NAME);
    write_staging(&staging, &rendered)?;

    // ── step3 — reverse-order teardown; source = LIVE registry (only the registry
    // contains F11 ghosts — never the old/new roster) minus the executor's own agent. ──
    let env = translate::oxmgr_env(dir);
    let agent = translate::app_name("agent");
    let live = match translate::live_host_apps(&env) {
        Ok(v) => v,
        Err(e) => {
            remove_quiet(&staging);
            return Ok(Outcome::Refused { reason: format!("teardown source unavailable: {e}") });
        }
    };
    let targets: Vec<String> =
        live.iter().filter(|n| !(topo == Topology::AgentRun && **n == agent)).cloned().collect();
    for name in order_teardown(&targets) {
        stop_and_delete(&env, &name);
    }

    // ── step4 — C25 SHM residue clear (safe: the instance just tore down) ──
    clear_iceoryx_residue("config-core");

    // ── step5 — teardown-verification gate (hard, dual-topology). Never start over
    // corpses: a dirty gate refuses before the new config is activated. ──
    let home = translate::oxmgr_home(dir).to_string_lossy().into_owned();
    let dir_prefix =
        format!("{}/", dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).display());
    let family = conflict::stop_sweep_names(mediaservo_common::brand::media_brand().app_prefix);
    if let Err((survivors, registry_note)) =
        wait_for_clean(dir, &env, topo, &dir_prefix, &home, &family)
    {
        remove_quiet(&staging); // content stays observable via the ledger history row
        let detail = format!(
            "refused-residue: survivors=[{}] registry={}",
            survivors
                .iter()
                .map(|s| format!("{}@{}", s.pid, s.owner))
                .collect::<Vec<_>>()
                .join(", "),
            registry_note
        );
        tracing::error!("{detail} — config-core refused to start over corpses");
        led.flags.pending = true;
        led.push_history(&hash, opts.origin, opts.actor, &detail);
        led.save(dir)?;
        return Ok(Outcome::Residue { survivors });
    }

    // ── step6 — prepare/commit: pending_apply BEFORE activation (crash in between =>
    // the boot bell re-reconciles file.hash vs pending instead of re-tearing-down). ──
    led.pending_apply = Some(PendingApply { hash: hash.clone(), at: now_rfc3339() });
    led.save(dir)?;
    let oxfile = dir.join("run").join("oxfile.toml");
    std::fs::rename(&staging, &oxfile).map_err(|e| {
        format!("activate {} -> {} failed: {e}", staging.display(), oxfile.display())
    })?;
    if let Err(e) = translate::oxmgr_apply(dir) {
        // pending_apply stays set — the boot bell owns the repair from here.
        led.flags.error = true;
        led.push_history(&hash, opts.origin, opts.actor, &format!("refused-apply: {e}"));
        led.save(dir)?;
        return Ok(Outcome::Refused { reason: e });
    }

    // ── step7 — reconcile + ledger commit (ghost-free assert, lenient on the batch1
    // falsy-recorder asymmetry). The lock releases on return, AFTER the ledger write. ──
    let roster = translate::oxfile_app_names(&oxfile)?;
    let registry = translate::live_host_apps(&env)?;
    let expected = translate::expected_process_names(&cfg)?;
    let recorder = translate::app_name("recorder");
    if let Some(detail) = reconcile_mismatch(&roster, &registry, &expected, &recorder) {
        led.flags.error = true; // pending_apply kept for boot-bell repair
        led.push_history(&hash, opts.origin, opts.actor, &format!("reconcile-mismatch: {detail}"));
        led.save(dir)?;
        return Ok(Outcome::Refused { reason: detail });
    }
    let running = Running {
        hash: hash.clone(),
        at: now_rfc3339(),
        origin: opts.origin.to_string(),
        actor: opts.actor.map(str::to_string),
    };
    led.running = Some(running.clone());
    led.pending_apply = None;
    led.flags = flags_after_commit(&running, led.last_push.as_ref());
    led.push_history(&hash, opts.origin, opts.actor, "applied");
    led.save(dir)?;
    Ok(Outcome::Applied { hash })
}

/// EX-3 boot fast path: when a previous run left `pending_apply` set (crash between step6
/// activation and step7 reconcile) AND the live registry already reconciles with the
/// activated roster, commit the ledger WITHOUT teardown/re-render/apply (zero churn).
///
/// `Ok(true)` = committed, the caller is done. `Ok(false)` = fast path not trustworthy
/// (no matching pending_apply, roster/expected unreadable, reconcile mismatch, daemon
/// unreachable) -- the caller runs the full [`execute`], which owns the repair and its
/// refusal audits. `Err` is operational only (lock timeout, host.yaml io, ledger save).
pub fn try_commit_pending(dir: &Path) -> Result<bool, String> {
    let _guard = CoreLock::acquire(dir)?;
    let cfg_path = dir.join("etc").join("host.yaml");
    let cfg = std::fs::read_to_string(&cfg_path)
        .map_err(|e| format!("read {} failed: {e}", cfg_path.display()))?;
    let hash = file_hash(cfg.as_bytes());
    if !Ledger::load(dir).pending_apply.as_ref().is_some_and(|p| p.hash == hash) {
        return Ok(false);
    }
    // An unreachable daemon cannot prove the live state: the empty registry then mismatches
    // the non-empty roster below => Ok(false) => the full execute owns the repair.
    let env = translate::oxmgr_env(dir);
    let registry = translate::live_host_apps(&env).unwrap_or_default();
    commit_pending_fast(dir, &hash, &cfg, &registry)
}

/// Fast-path verdict + commit with the registry list injected (the unit-test seam;
/// [`try_commit_pending`] is its thin live side). Same leniency as step7: reuses
/// `reconcile_mismatch` against pre-existing state only -- never re-renders or applies.
fn commit_pending_fast(
    dir: &Path,
    hash: &str,
    cfg: &str,
    registry: &[String],
) -> Result<bool, String> {
    let mut led = Ledger::load(dir);
    if !led.pending_apply.as_ref().is_some_and(|p| p.hash == hash) {
        return Ok(false);
    }
    let oxfile = dir.join("run").join("oxfile.toml");
    let Ok(roster) = translate::oxfile_app_names(&oxfile) else {
        return Ok(false); // missing/corrupt activated roster => the full execute re-renders
    };
    let Ok(expected) = translate::expected_process_names(cfg) else {
        return Ok(false); // malformed cfg => the full execute refuses with its audit
    };
    if let Some(detail) =
        reconcile_mismatch(&roster, registry, &expected, &translate::app_name("recorder"))
    {
        tracing::info!(
            "boot fast path declined: pending apply does not reconcile ({detail}) -- the full execute owns the repair"
        );
        return Ok(false);
    }
    let running = Running {
        hash: hash.to_string(),
        at: now_rfc3339(),
        origin: "boot-commit".to_string(),
        actor: None,
    };
    led.running = Some(running.clone());
    led.pending_apply = None;
    led.flags = flags_after_commit(&running, led.last_push.as_ref());
    led.push_history(hash, "boot-commit", None, "pending-committed");
    led.save(dir)?;
    tracing::info!(
        "config-core: pending apply committed at boot (fast path, zero churn) -- hash {}",
        short12(hash)
    );
    Ok(true)
}

/// T1b5 (k8) lamp truth at a commit point: a successful activation clears pending/error and
/// recomputes `conflict` from the ledger's own facts — the cloud's belief (`last_push`) vs
/// what actually runs. A vehicle that was never pushed stays conflict=false (no belief to
/// diverge from). Pure: every commit path (execute step7, boot fast commit) routes through it.
pub fn flags_after_commit(running: &Running, last_push: Option<&LastPush>) -> Flags {
    Flags {
        pending: false,
        error: false,
        conflict: last_push.is_some_and(|p| p.hash != running.hash),
    }
}

/// T1b5 (k8): record "what the cloud believes it set" — the push bell calls this ONLY after
/// its push-origin run reported Applied/NoOp (a refused push must not move the belief). The
/// hash is re-derived from the just-written etc/host.yaml inside the core lock, and the
/// conflict lamp is re-judged against the running row. Err is operational only (io/lock/save).
pub fn record_push(dir: &Path, actor: Option<&str>) -> Result<(), String> {
    let _guard = CoreLock::acquire(dir)?;
    let cfg_path = dir.join("etc").join("host.yaml");
    let bytes =
        std::fs::read(&cfg_path).map_err(|e| format!("read {} failed: {e}", cfg_path.display()))?;
    let hash = file_hash(&bytes);
    let mut led = Ledger::load(dir);
    led.last_push =
        Some(LastPush { hash: hash.clone(), actor: actor.map(str::to_string), at: now_rfc3339() });
    led.flags.conflict = led.running.as_ref().is_some_and(|r| r.hash != hash);
    led.save(dir)
}

/// Actor label (env USER) for the ledger rows written by CLI/wake origins — unset or
/// empty is None: the actor is provenance, never a gate.
pub fn env_actor() -> Option<String> {
    std::env::var("USER").ok().filter(|u| !u.is_empty())
}

/// Reconcile decision (pure, table-tested): `false` = the fingerprint short-circuit --
/// the cluster already runs this exact config. Shared by [`execute`]'s step1 (which
/// passes flags_pending as false, keeping batch2's exact boundary) and the wake/boot
/// gate ([`gate_needs_reconcile`]).
/// EX-4: `force` skips ONLY this short-circuit -- every gate downstream still runs.
fn should_reconcile(
    force: bool,
    running_match: bool,
    pending_mismatch: bool,
    flags_pending: bool,
) -> bool {
    force || !running_match || pending_mismatch || flags_pending
}

/// Wake/boot gate over a loaded ledger (pure, table-tested): NoOp only when the running
/// row equals the file and nothing flags-pending or mismatches. A pending_apply that
/// MATCHES the file is the fast path's business (try_commit_pending has already committed
/// or declined it) -- the gate stays quiet; batch3's boot-gate semantics kept verbatim.
fn gate_needs_reconcile(led: &Ledger, hash: &str) -> bool {
    let running_match = led.running.as_ref().is_some_and(|r| r.hash == hash);
    let pending_mismatch = led.pending_apply.as_ref().is_some_and(|p| p.hash != hash);
    should_reconcile(false, running_match, pending_mismatch, led.flags.pending)
}

/// k13 (F20): the fingerprint short-circuit may only skip when the cluster is PROVEN
/// live -- the pure decision core (table-tested). A matching hash over a half-dead
/// roster must take the full execute path (liveness recovery); a fingerprint mismatch
/// keeps the legacy full-execute semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoOpVerdict {
    NoOp,
    Execute,
}

pub fn noop_verdict(fingerprint_matches: bool, roster_live_ok: bool) -> NoOpVerdict {
    if fingerprint_matches && roster_live_ok { NoOpVerdict::NoOp } else { NoOpVerdict::Execute }
}

/// Expected apps NOT `running` in the registry (missing / errored / stopped all count
/// as dead). The collection step of the k13 liveness check (fixture-tested, no /proc).
fn dead_apps(expected: &[String], registry: &[crate::monitor::topology::OxProcess]) -> Vec<String> {
    expected
        .iter()
        .filter(|n| !registry.iter().any(|p| &p.name == *n && p.status == "running"))
        .cloned()
        .collect()
}

/// k13 liveness IO shell: (dead expected apps, expected total). `bytes` is the in-lock
/// host.yaml whose fingerprint already matched the ledger running row.
fn noop_liveness(dir: &Path, bytes: &[u8]) -> Result<(Vec<String>, usize), String> {
    let cfg =
        std::str::from_utf8(bytes).map_err(|e| format!("host.yaml is not valid utf-8: {e}"))?;
    let expected = translate::expected_process_names(cfg)?;
    let env = translate::oxmgr_env(dir);
    let out = translate::oxmgr_cmd_with_env(&env)
        .args(["list", "--json"])
        .output()
        .map_err(|e| format!("oxmgr exec failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "oxmgr list failed (exit {}): {}",
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let registry =
        crate::monitor::topology::parse_oxmgr_json(&String::from_utf8_lossy(&out.stdout))?;
    Ok((dead_apps(&expected, &registry), expected.len()))
}

/// Shared k13 gate: Ok(live)? => true only when every expected app is `running`; the
/// recovery reason is logged here (C15) and false returned so the caller executes.
/// `fingerprint_matches` is passed through to the pure verdict (both call sites hold
/// the ledger-current precondition; kept explicit for the table tests' sake).
fn noop_may_skip(dir: &Path, bytes: &[u8], fingerprint_matches: bool) -> bool {
    match noop_liveness(dir, bytes) {
        Ok((dead, total)) => {
            let live = dead.is_empty();
            if !live {
                tracing::warn!(
                    "config-core: ledger current but {} of {total} expected apps not running ({}) -- executing liveness recovery",
                    dead.len(),
                    dead.join(", ")
                );
            }
            noop_verdict(fingerprint_matches, live) == NoOpVerdict::NoOp
        }
        Err(e) => {
            tracing::warn!(
                "config-core: ledger current but the registry could not be read ({e}) -- executing liveness recovery"
            );
            false
        }
    }
}

/// T1b4 (k3): the shared wake/boot reconcile entry -- EX-3 fast path first (a pending
/// commit is zero-churn), then the fingerprint gate, then the full [`execute`] with
/// `force=false` when it disagrees. Callers get the honest [`Outcome`]: the NoOp branch
/// never wakes the core (bell short-circuit R10 -- the in-lock re-read stays the
/// correctness boundary inside execute itself).
///
/// Topology choice: `AgentRun` is for the agent's own boot pass (the agent is alive --
/// the gate tolerates the agent-only registry and apply swaps it last), `CliRun` for CLI
/// processes (the gate demands a fully empty registry -- anything alive there is teardown
/// work). host-agent delegates with AgentRun, the CLI startup wake uses the
/// [`startup_reconcile`] wrapper; one call site each, no duplicated boot logic.
pub fn startup_reconcile_with(dir: &Path, topo: Topology, origin: &str) -> Result<Outcome, String> {
    let committed = try_commit_pending(dir)?;
    let cfg_path = dir.join("etc").join("host.yaml");
    let bytes =
        std::fs::read(&cfg_path).map_err(|e| format!("read {} failed: {e}", cfg_path.display()))?;
    let hash = file_hash(&bytes);
    if committed {
        // the fast path committed the pending row: this config IS live (the boot-commit
        // provenance lives in the ledger history).
        return Ok(Outcome::Applied { hash });
    }
    let led = Ledger::load(dir);
    let wake_match = led.running.as_ref().is_some_and(|r| r.hash == hash);
    if !gate_needs_reconcile(&led, &hash) && noop_may_skip(dir, &bytes, wake_match) {
        return Ok(Outcome::NoOp { hash });
    }
    let actor = env_actor();
    execute(dir, topo, false, &ExecuteOpts { origin, actor: actor.as_deref() })
}

/// Thin [`startup_reconcile_with`] wrapper for CLI-side wakes: [`Topology::CliRun`] plus
/// the env-USER actor.
pub fn startup_reconcile(dir: &Path, origin: &str) -> Result<Outcome, String> {
    startup_reconcile_with(dir, Topology::CliRun, origin)
}

/// C25: iceoryx2 SHM residue (SystemInFlux) makes capturer/streamer subscription open
/// fail — must clean after a full stop (start/restart/core step4); hot apply does not
/// (processes running, SHM in use). Moved out of bin/host.rs (T1b2): the CLI verbs and
/// the core share one implementation (single source).
pub fn clear_iceoryx_residue(verb: &str) {
    for p in ["/tmp/iceoryx2", "/dev/shm/iox2_*"] {
        let _ = std::process::Command::new("rm").args(["-rf", p]).status();
    }
    println!("{verb}: cleared iceoryx2 SHM residue (C25)");
}

/// step0 — per-instance advisory lock. flock auto-releases on process death, so a
/// crashed core never leaves a dead lock (the O_EXCL-with-pid alternative would need
/// stale-liveness heuristics for exactly this case).
struct CoreLock {
    _file: std::fs::File,
}

impl CoreLock {
    fn acquire(dir: &Path) -> Result<CoreLock, String> {
        let run = dir.join("run");
        std::fs::create_dir_all(&run)
            .map_err(|e| format!("create {} failed: {e}", run.display()))?;
        let path = run.join("config-core.lock");
        let file = std::fs::File::create(&path)
            .map_err(|e| format!("create {} failed: {e}", path.display()))?;
        Self::lock_exclusive(file, &path)
    }

    #[cfg(unix)]
    fn lock_exclusive(file: std::fs::File, path: &Path) -> Result<CoreLock, String> {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let deadline = Instant::now() + LOCK_TIMEOUT;
        loop {
            // SAFETY: `fd` is a valid descriptor owned by `file` for the whole loop;
            // flock(2) touches only the kernel lock table (no memory access).
            let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(CoreLock { _file: file });
            }
            let err = std::io::Error::last_os_error();
            if rc != -1 || err.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(format!("flock({}) failed (rc {rc}): {err}", path.display()));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "config-core lock {} busy for {}s — another executor holds it",
                    path.display(),
                    LOCK_TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(RETRY_STEP);
        }
    }

    #[cfg(not(unix))]
    fn lock_exclusive(_file: std::fs::File, _path: &Path) -> Result<CoreLock, String> {
        Err("config-core: advisory locking is only implemented for unix".to_string())
    }
}

/// step2 — write the rendered oxfile to staging. Leftovers from crashed runs are swept
/// before every write (activation renames the CURRENT staging only, never a leftover).
fn write_staging(staging: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {} failed: {e}", parent.display()))?;
    }
    let _ = std::fs::remove_file(staging);
    std::fs::write(staging, content).map_err(|e| format!("write {} failed: {e}", staging.display()))
}

fn remove_quiet(p: &Path) {
    let _ = std::fs::remove_file(p);
}

/// Extract every `--token <path>` argument from the rendered roster text (C40 pre-check
/// input). Paths in rendered commands are unquoted; a quote/newline ends the value.
fn token_paths(rendered: &str) -> Vec<String> {
    const NEEDLE: &str = "--token ";
    let mut out = Vec::new();
    let mut rest = rendered;
    while let Some(rel) = rest.find(NEEDLE) {
        rest = &rest[rel + NEEDLE.len()..];
        let end = rest.find([' ', '"', '\n', '\r', '\t']).unwrap_or(rest.len());
        if end > 0 {
            out.push(rest[..end].to_string());
        }
        rest = &rest[end..];
    }
    out
}

/// step3 order (pure): consumers (streamer/audio/controller/emergency/recorder) first,
/// then producers (capturer), agent last (only CliRun ever carries it — apply swaps the
/// agent declaratively instead, never via the teardown loop).
fn order_teardown(names: &[String]) -> Vec<String> {
    const CONSUMER_BASES: [&str; 5] = ["streamer", "audio", "controller", "emergency", "recorder"];
    let rank = |n: &String| -> u8 {
        if n.ends_with("agent") {
            3
        } else if CONSUMER_BASES.iter().any(|b| n.contains(b)) {
            0
        } else if n.contains("capturer") {
            1
        } else {
            2
        }
    };
    let mut out = names.to_vec();
    out.sort_by_key(rank); // stable sort keeps caller order within a class
    out
}

/// step3 — per-name `oxmgr stop` + `oxmgr delete`. Failures only WARN here: the hard
/// verdict is step5's gate (refuse only when the state never reaches clean, not per verb).
fn stop_and_delete(env: &[(String, String)], name: &str) {
    for verb in ["stop", "delete"] {
        match translate::oxmgr_cmd_with_env(env).arg(verb).arg(name).output() {
            Ok(out) if out.status.success() => {
                tracing::info!(app = %name, verb, "config-core: teardown {verb} ok");
            }
            Ok(out) => tracing::warn!(
                app = %name, verb,
                "config-core: oxmgr {verb} failed (step5 gate decides): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => {
                tracing::warn!(app = %name, verb, "config-core: oxmgr {verb} exec failed: {e}")
            }
        }
    }
}

/// step5 registry predicate (pure, table-tested).
/// ponytail: suffix match on "agent" — if a future app base name could end in "agent",
/// thread the exact `translate::app_name("agent")` through as a parameter.
fn gate_clean(registry: &[String], topo: &Topology) -> bool {
    match topo {
        Topology::CliRun => registry.is_empty(),
        Topology::AgentRun => registry.iter().all(|n| n.ends_with("agent")),
    }
}

/// step5 survivor scoping (pure): an instance process is one whose cmdline first segment
/// lives under the instance dir, OR whose daemon home (OXMGR_HOME environ) is this
/// instance, OR whose owner is unreadable while the basename belongs to the family set.
/// Self-pid and `/target/` dev binaries are already excluded upstream (conflict.rs).
/// AgentRun corollary: the executor agent is self-pid-excluded there by construction.
fn instance_scoped(s: &SurvivorInfo, dir_prefix: &str, home: &str, family: &[String]) -> bool {
    if s.first_arg.starts_with(dir_prefix) || s.owner == home {
        return true;
    }
    s.owner == OWNER_UNKNOWN
        && family
            .iter()
            .any(|n| s.first_arg.rsplit('/').next().unwrap_or_default().starts_with(n.as_str()))
}

/// step5 retry loop (annex: never start over corpses): registry gate + instance-scoped
/// survivor scan, ~15s budget in 500ms steps. An oxmgr list error counts as NOT clean
/// (fail-closed — an unreadable registry is exactly what the gate exists to catch).
fn wait_for_clean(
    dir: &Path,
    env: &[(String, String)],
    topo: Topology,
    dir_prefix: &str,
    home: &str,
    family: &[String],
) -> Result<(), (Vec<SurvivorInfo>, String)> {
    let deadline = Instant::now() + GATE_TIMEOUT;
    loop {
        let registry = translate::live_host_apps(env);
        let survivors: Vec<SurvivorInfo> = conflict::stop_survivors(dir)
            .into_iter()
            .filter(|s| instance_scoped(s, dir_prefix, home, family))
            .collect();
        if registry.as_ref().is_ok_and(|r| gate_clean(r, &topo)) && survivors.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let note = match registry {
                Ok(r) => format!("{r:?}"),
                Err(e) => format!("list failed: {e}"),
            };
            return Err((survivors, note));
        }
        std::thread::sleep(RETRY_STEP);
    }
}

/// step7 reconcile (pure): registry == roster (set-wise), expected ⊆ roster, and
/// roster\expected may only carry the falsy-recorder slot (batch1 asymmetry kept lenient).
fn reconcile_mismatch(
    roster: &[String],
    registry: &[String],
    expected: &[String],
    recorder: &str,
) -> Option<String> {
    let missing: Vec<&String> = roster.iter().filter(|n| !registry.contains(n)).collect();
    let extra: Vec<&String> = registry.iter().filter(|n| !roster.contains(n)).collect();
    if !missing.is_empty() || !extra.is_empty() {
        return Some(format!("registry != roster: missing={missing:?} extra={extra:?}"));
    }
    let unregistered: Vec<&String> = expected.iter().filter(|n| !roster.contains(n)).collect();
    if !unregistered.is_empty() {
        return Some(format!("expected apps not in roster: {unregistered:?}"));
    }
    if let Some(bad) = roster.iter().find(|n| !expected.contains(n) && n.as_str() != recorder) {
        return Some(format!(
            "roster carries unexpected app {bad} (only the falsy recorder slot may exceed expected)"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn gate_clean_cli_requires_a_fully_empty_registry() {
        assert!(gate_clean(&[], &Topology::CliRun));
        assert!(!gate_clean(&s(&["host-agent"]), &Topology::CliRun));
    }

    #[test]
    fn gate_clean_agent_run_tolerates_only_the_agent_entry() {
        assert!(gate_clean(&s(&["host-agent"]), &Topology::AgentRun));
        assert!(gate_clean(&[], &Topology::AgentRun)); // agent dead => still passes (never stricter than needed)
        assert!(!gate_clean(&s(&["host-agent", "host-streamer-t1"]), &Topology::AgentRun));
        assert!(!gate_clean(&s(&["host-recorder"]), &Topology::AgentRun));
    }

    #[test]
    fn order_teardown_consumers_then_producers_agent_last() {
        let names = s(&[
            "host-capturer-cam0",
            "msrtc-agent",
            "msrtc-streamer-t1",
            "msrtc-audio",
            "msrtc-controller",
        ]);
        let out = order_teardown(&names);
        assert_eq!(out.last().unwrap(), "msrtc-agent");
        let idx = |n: &str| out.iter().position(|x| x == n).unwrap();
        assert!(idx("msrtc-streamer-t1") < idx("host-capturer-cam0"));
        assert!(idx("msrtc-audio") < idx("host-capturer-cam0"));
        assert!(idx("msrtc-controller") < idx("msrtc-agent"));
    }

    #[test]
    fn token_paths_extracts_every_occurrence() {
        let rendered = "[[apps]]\ncommand = \"bin/x --token /opt/h/etc/link/cam0.token\"\ncommand = \"bin/y --token /opt/h/etc/link/recorder.token\"\n";
        assert_eq!(
            token_paths(rendered),
            s(&["/opt/h/etc/link/cam0.token", "/opt/h/etc/link/recorder.token"])
        );
    }

    #[test]
    fn token_paths_is_empty_without_token_args() {
        assert!(token_paths("command = \"bin/x --camera cam0\"").is_empty());
    }

    #[test]
    fn instance_scoped_covers_dir_prefix_home_and_unknown_owner_family() {
        let mk = |first_arg: &str, owner: &str| SurvivorInfo {
            pid: 1,
            first_arg: first_arg.to_string(),
            owner: owner.to_string(),
        };
        let family = s(&["msrtc-streamer", "host-streamer"]);
        // path under the instance dir wins even with a foreign daemon
        assert!(instance_scoped(
            &mk("/opt/host/bin/msrtc-streamer-t1", "/opt/other/run/oxmgr"),
            "/opt/host/",
            "/opt/host/run/oxmgr",
            &family
        ));
        // same daemon home wins even with an odd path
        assert!(instance_scoped(
            &mk("/var/tmp/relocated-msrtc-streamer", "/opt/host/run/oxmgr"),
            "/opt/host/",
            "/opt/host/run/oxmgr",
            &family
        ));
        // unreadable owner + family basename qualifies (per-instance fallback)
        assert!(instance_scoped(
            &mk("/srv/x/bin/host-streamer-old", OWNER_UNKNOWN),
            "/opt/host/",
            "/opt/host/run/oxmgr",
            &family
        ));
        // foreign instance with a readable foreign owner and a non-family name: out
        assert!(!instance_scoped(
            &mk("/opt/other/bin/monitor", "/opt/other/run/oxmgr"),
            "/opt/host/",
            "/opt/host/run/oxmgr",
            &family
        ));
    }

    #[test]
    fn reconcile_mismatch_survives_the_recorder_falsy_asymmetry() {
        let roster = s(&["host-agent", "host-recorder", "host-streamer-t1"]);
        let expected = s(&["host-agent", "host-streamer-t1"]);
        // recorder slot present but not expected ([record] falsy): lenient pass
        assert!(reconcile_mismatch(&roster, &roster, &expected, "host-recorder").is_none());
        // any other roster surplus => mismatch
        let roster2 = s(&["host-agent", "host-mystery"]);
        assert!(reconcile_mismatch(&roster2, &roster2, &expected, "host-recorder").is_some());
        // registry drift vs roster => mismatch
        assert!(
            reconcile_mismatch(&roster, &s(&["host-agent"]), &expected, "host-recorder").is_some()
        );
        // expected not covered by the roster => mismatch
        assert!(
            reconcile_mismatch(
                &s(&["host-agent"]),
                &s(&["host-agent"]),
                &expected,
                "host-recorder"
            )
            .is_some()
        );
    }

    #[test]
    fn write_staging_sweeps_crash_leftovers_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("run").join(STAGING_NAME);
        write_staging(&staging, "fresh").unwrap();
        assert_eq!(std::fs::read_to_string(&staging).unwrap(), "fresh");
        std::fs::write(&staging, "stale leftover").unwrap();
        write_staging(&staging, "fresh2").unwrap();
        assert_eq!(std::fs::read_to_string(&staging).unwrap(), "fresh2");
    }

    #[test]
    fn watch_tick_follows_while_quiet() {
        assert_eq!(watch_tick(Some("h".into()), None, "h"), WatchAction::Follow);
        assert_eq!(watch_tick(Some("h".into()), Some("h".into()), "h"), WatchAction::Follow);
        assert_eq!(watch_tick(None, Some("x".into()), "h"), WatchAction::Follow);
        assert_eq!(watch_tick(None, None, "h"), WatchAction::Follow);
    }

    #[test]
    fn watch_tick_waits_on_first_sighting_of_a_change() {
        assert_eq!(watch_tick(Some("x".into()), None, "h"), WatchAction::Wait);
        assert_eq!(watch_tick(Some("x".into()), Some("y".into()), "h"), WatchAction::Wait);
    }

    #[test]
    fn watch_tick_triggers_on_two_equal_non_baseline_samples() {
        assert_eq!(
            watch_tick(Some("x".into()), Some("x".into()), "h"),
            WatchAction::Trigger("x".into())
        );
    }

    #[test]
    fn watch_tick_baseline_equal_never_triggers_even_with_equal_prev() {
        assert_eq!(watch_tick(Some("h".into()), Some("h".into()), "h"), WatchAction::Follow);
    }

    #[test]
    fn watch_tick_first_sample_never_triggers() {
        assert_eq!(watch_tick(Some("x".into()), None, "h"), WatchAction::Wait);
    }

    fn seed_fast_tree(dir: &Path, cfg: &str, pending: bool) -> String {
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::create_dir_all(dir.join("run")).unwrap();
        std::fs::write(dir.join("etc").join("host.yaml"), cfg).unwrap();
        let hash = file_hash(cfg.as_bytes());
        let mut led = Ledger::default();
        if pending {
            led.pending_apply = Some(PendingApply { hash: hash.clone(), at: now_rfc3339() });
            led.flags.pending = true;
        }
        led.save(dir).unwrap();
        hash
    }

    #[test]
    fn try_commit_pending_fast_path_commits_with_matching_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let cfg = "{}\n";
        let hash = seed_fast_tree(dir, cfg, true);
        let expected = translate::expected_process_names(cfg).unwrap();
        let roster_toml: String =
            expected.iter().map(|n| format!("[[apps]]\nname = \"{n}\"\n")).collect();
        std::fs::write(dir.join("run").join("oxfile.toml"), roster_toml).unwrap();
        assert!(commit_pending_fast(dir, &hash, cfg, &expected).unwrap());
        let led = Ledger::load(dir);
        assert_eq!(led.running.as_ref().unwrap().hash, hash);
        assert_eq!(led.running.as_ref().unwrap().origin, "boot-commit");
        assert!(led.pending_apply.is_none());
        assert!(!led.flags.pending && !led.flags.error);
        assert_eq!(led.history.last().unwrap().result, "pending-committed");
    }

    #[test]
    fn try_commit_pending_declines_without_matching_pending_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let cfg = "{}\n";
        let hash = seed_fast_tree(dir, cfg, false); // no pending_apply at all
        assert!(!commit_pending_fast(dir, &hash, cfg, &[]).unwrap());
        seed_fast_tree(dir, cfg, true);
        // pending_apply present but the fingerprint differs (site edited after the crash)
        assert!(!commit_pending_fast(dir, "stale-hash", cfg, &[]).unwrap());
    }

    #[test]
    fn try_commit_pending_declines_on_reconcile_mismatch_and_keeps_ledger() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let cfg = "{}\n";
        let hash = seed_fast_tree(dir, cfg, true);
        std::fs::write(dir.join("run").join("oxfile.toml"), "[[apps]]\nname = \"x\"\n").unwrap();
        // empty injected registry vs roster ["x"] => mismatch => decline, leave the ledger
        // exactly as the crashed core wrote it (the full execute owns the repair)
        assert!(!commit_pending_fast(dir, &hash, cfg, &[]).unwrap());
        let led = Ledger::load(dir);
        assert!(led.pending_apply.is_some());
        assert!(led.running.is_none());
    }

    #[test]
    fn should_reconcile_table() {
        // the single quiet cell: fingerprint equals the ledger row, nothing pending
        assert!(!should_reconcile(false, true, false, false));
        // EX-4: force skips ONLY the short-circuit -- fingerprint-equal still executes
        assert!(should_reconcile(true, true, false, false));
        assert!(should_reconcile(false, false, false, false)); // never applied (no running row)
        assert!(should_reconcile(false, true, true, false)); // pending_apply edited after a crash
        assert!(should_reconcile(false, true, false, true)); // refused-residue bell flag retries
    }

    #[test]
    fn flags_after_commit_table() {
        let running = Running {
            hash: "same".to_string(),
            at: now_rfc3339(),
            origin: "push".to_string(),
            actor: None,
        };
        // never pushed: no cloud belief to diverge from; lamps off after a commit
        let f = flags_after_commit(&running, None);
        assert!(!f.pending && !f.error && !f.conflict);
        // belief equals what runs -> consistent
        let pushed = LastPush { hash: "same".to_string(), actor: None, at: now_rfc3339() };
        assert!(!flags_after_commit(&running, Some(&pushed)).conflict);
        // site moved after the landed push (local edit applied) -> conflict lamp
        let stale = LastPush { hash: "other".to_string(), actor: None, at: now_rfc3339() };
        assert!(flags_after_commit(&running, Some(&stale)).conflict);
    }

    #[test]
    fn record_push_sets_belief_and_recomputes_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc").join("host.yaml"), "a: 1\n").unwrap();
        let push_hash = file_hash(b"a: 1\n");
        let row = |h: String| Running {
            hash: h,
            at: now_rfc3339(),
            origin: "cli".to_string(),
            actor: None,
        };
        // pushed content differs from what runs -> conflict lit, actor recorded
        let led = Ledger { running: Some(row("other".to_string())), ..Ledger::default() };
        led.save(dir).unwrap();
        record_push(dir, Some("ops")).unwrap();
        let got = Ledger::load(dir);
        assert_eq!(got.last_push.as_ref().unwrap().hash, push_hash);
        assert_eq!(got.last_push.as_ref().unwrap().actor.as_deref(), Some("ops"));
        assert!(got.flags.conflict);
        // pushed content equals the running row -> lamp clear, belief updated in place
        let mut led = Ledger::load(dir);
        led.running = Some(row(push_hash.clone()));
        led.save(dir).unwrap();
        record_push(dir, None).unwrap();
        let got = Ledger::load(dir);
        assert!(!got.flags.conflict);
        assert!(got.last_push.as_ref().unwrap().actor.is_none());
        // absent host.yaml: operational Err, never a half-written belief
        let empty = tempfile::tempdir().unwrap();
        assert!(record_push(empty.path(), None).is_err());
    }

    #[test]
    fn noop_verdict_table_k13() {
        use NoOpVerdict::*;
        // fingerprint match + live roster = the only NoOp
        assert_eq!(noop_verdict(true, true), NoOp);
        // fingerprint match but a missing app
        assert_eq!(noop_verdict(true, false), Execute);
        // fingerprint match but an errored row (same bool, same verdict)
        assert_eq!(noop_verdict(true, false), Execute);
        // fingerprint mismatch executes regardless of the roster
        assert_eq!(noop_verdict(false, true), Execute);
        assert_eq!(noop_verdict(false, false), Execute);
    }

    #[test]
    fn dead_apps_fixture_k13() {
        use crate::monitor::topology::OxProcess;
        let expected = vec!["a".into(), "b".into(), "c".into()];
        let registry = vec![
            OxProcess { name: "a".into(), status: "running".into() },
            OxProcess { name: "b".into(), status: "errored".into() },
            // "c" missing entirely
            OxProcess { name: "other".into(), status: "running".into() },
        ];
        let dead = dead_apps(&expected, &registry);
        assert_eq!(dead, vec!["b".to_string(), "c".to_string()]);
        assert!(dead_apps(&expected, &[]).len() == 3);
        assert!(dead_apps(&[], &registry).is_empty());
    }

    #[test]
    fn gate_needs_reconcile_keeps_batch3_boot_semantics() {
        let mut led = Ledger::default();
        let hash = "a".repeat(64);
        assert!(gate_needs_reconcile(&led, &hash)); // fresh tree: no ledger row at all
        let row = |h: String| Running {
            hash: h,
            at: now_rfc3339(),
            origin: "cli".to_string(),
            actor: None,
        };
        led.running = Some(row("b".to_string()));
        assert!(gate_needs_reconcile(&led, &hash)); // fingerprint drift
        led.running = Some(row(hash.clone()));
        assert!(!gate_needs_reconcile(&led, &hash)); // the quiet cell
        led.flags.pending = true;
        assert!(gate_needs_reconcile(&led, &hash)); // residue flag (batch3's boot condition)
        led.flags.pending = false;
        led.pending_apply = Some(PendingApply { hash: hash.clone(), at: now_rfc3339() });
        assert!(!gate_needs_reconcile(&led, &hash)); // matching pending = fast path's business
        led.pending_apply = Some(PendingApply { hash: "stale".to_string(), at: now_rfc3339() });
        assert!(gate_needs_reconcile(&led, &hash)); // mismatch => execute's in-lock check fires
    }
}
