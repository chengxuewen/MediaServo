//! state.rs -- state.json (single JSON writer, carries the teardown plan) + timeline (JSONL = the judge contract, now `measure` in this crate) +
//! path resolution + write lock + param_summary (verbatim bash port; the judge line-71 predicate and the W7 muscle memory depend on it).
//!
//! Design contract source = design.md §state.rs: `{spec, scope, dir, iface, expires_at, teardown:{channel,
//! sidecar?:{name,image}, steps[]}, job?, schema}` -- the teardown plan **is persisted at apply and carries its channel info**
//! (a watchdog waking cross-process knows whether to use local tc or docker exec, executing per the plan instead of hardcoding).
//!
//! Path (C20): `WEAKNET_STATEDIR` env override > exe-dir ancestor probing of `<repo>/out/.weaknet` (same path as bash
//! `ROOT/out/.weaknet` = the compat predicate letting both tools share the timeline during the T10 window) > cwd fallback.
//! Filenames: state.json (this tool's sole fact source; the bash-era `state` KEY=VAL file coexists but neither reads the other),
//! timeline.jsonl (shared append), watchdog.pid (fuse protocol `<pid> <starttime>`), lock (write lock, see [`acquire_write_lock`]).

use serde::{Deserialize, Serialize};

use crate::spec::{Dir, ImpairSpec, ScopeSel};

pub const STATE_SCHEMA: &str = "weaknet-state/1";
pub const STATE_FILE: &str = "state.json";
pub const TIMELINE_FILE: &str = "timeline.jsonl";
pub const WATCHDOG_PID_FILE: &str = "watchdog.pid";
pub const LOCK_FILE: &str = "lock";
/// bash-side default dir name (ROOT/out/.weaknet) -- the probe chain's end aligns with it.
const DEFAULT_OUT_SUBDIR: &str = "out/.weaknet";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelSer {
    /// serde shape = design §state.rs: channel "local"|"sidecar" + sidecar{name,image}
    #[serde(rename = "local")]
    LocalRoot,
    #[serde(rename = "sidecar")]
    Sidecar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarRef {
    pub name: String,
    pub image: String,
}

/// teardown plan (steps = a tc argv sequence, without the `tc` prefix; execution semantics in engine::run_teardown --
/// ENOENT-class idempotent, everything else WARNed per step without aborting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Teardown {
    pub channel: ChannelSer,
    #[serde(default)]
    pub sidecar: Option<SidecarRef>,
    pub steps: Vec<Vec<String>>,
}

/// scenario job owner (written at T8; this struct led the schema -- stale liveness via /proc starttime,
/// reusing the fuse's same validation method, design §server "stale job recovery").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRef {
    pub name: String,
    pub pid: u32,
    pub starttime: u64,
    /// T8: progress echo source (the writer updates done each step; total = the plan line count).
    /// `#[serde(default)]` for compat with the first rev-2.2 shape (persisted without done/total).
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub schema: String,
    pub spec: ImpairSpec,
    /// Top-level mirror of spec.dir (design field-list shape; kept in sync by the single writer, read directly by status/serve).
    pub dir: Dir,
    pub scope: ScopeSel,
    pub iface: String,
    /// Port/pair snapshot at apply time (for status display and debugging; Media = local ports, Stream/Device = flattened peers).
    #[serde(default)]
    pub ports: Vec<u16>,
    /// T5: ordered pairs (local, remote) resolved at Stream/Device apply time -- the resolution lands on disk,
    /// replay/status skip re-pulling stats; `#[serde(default)]` for backward compat with old state (no such key).
    #[serde(default)]
    pub pairs: Vec<(u16, u16)>,
    /// E2 (T8 minor): targeted names (rooms = the --stream room set; devices = the --device device set).
    /// Single truth source for panel checkbox echo and the apply event stream field (E1).
    #[serde(default)]
    pub rooms: Vec<String>,
    #[serde(default)]
    pub devices: Vec<String>,
    #[serde(default)]
    pub sig_port: Option<u16>,
    pub expires_at_ms: u64,
    /// Whether this site's root was created by us (teardown only deletes what we created -- the guard discipline persisted).
    pub created_root: bool,
    /// T9: whether this session installed the ifb0 mirror chain (dir=in/both x physical port). The single truth source shared by
    /// read-back/verify observation-point routing (ifb0 root form vs iface 1:10) and the teardown plan.
    #[serde(default)]
    pub ifb_used: bool,
    /// T9: whether the ifb0 link was created by us this session (§ifb owner contract: never touch links built by others/the system).
    #[serde(default)]
    pub created_ifb: bool,
    #[serde(default)]
    pub job: Option<JobRef>,
    pub teardown: Teardown,
}

impl State {
    /// `expires_at_ms == 0` = the forever tier (--forever semantics, not exposed by the M0 CLI; fuse.is_expired already handles it).
    #[must_use]
    pub fn is_forever(&self) -> bool {
        self.expires_at_ms == 0
    }

    /// Atomic write (tmp+rename -- single JSON writer + defending the watchdog's cross-process read of a half file).
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("state path has no parent dir: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create the state dir {}: {e}", parent.display()))?;
        let tmp = path.with_file_name(format!("{}.tmp-{}", STATE_FILE, std::process::id()));
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| format!("state serialization failed: {e}"))?;
        std::fs::write(&tmp, body)
            .map_err(|e| format!("failed to write the state tmp file: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            format!("state rename {} -> {} failed: {e}", tmp.display(), path.display())
        })
    }

    /// Read state. Missing -> Ok(None); schema mismatch / invalid JSON -> Err (never silent divergence, C15).
    pub fn read_from(path: &std::path::Path) -> Result<Option<State>, String> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("failed to read state {}: {e}", path.display())),
        };
        let st: State = serde_json::from_str(&raw)
            .map_err(|e| format!("invalid state {}: {e}", path.display()))?;
        if st.schema != STATE_SCHEMA {
            return Err(format!(
                "state schema mismatch: {} != {STATE_SCHEMA} (version drift? reset with clear first)",
                st.schema
            ));
        }
        Ok(Some(st))
    }
}

// ---------- Path resolution ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    pub statedir: std::path::PathBuf,
}

impl Dirs {
    #[must_use]
    pub fn state_json(&self) -> std::path::PathBuf {
        self.statedir.join(STATE_FILE)
    }
    #[must_use]
    pub fn timeline(&self) -> std::path::PathBuf {
        self.statedir.join(TIMELINE_FILE)
    }
    #[must_use]
    pub fn watchdog_pid(&self) -> std::path::PathBuf {
        self.statedir.join(WATCHDOG_PID_FILE)
    }
    #[must_use]
    pub fn lock(&self) -> std::path::PathBuf {
        self.statedir.join(LOCK_FILE)
    }

    /// Pure resolution (env / exe_dir / cwd all injected -- tests touch zero environment):
    /// `WEAKNET_STATEDIR` > the nearest repo root (with Cargo.toml+crates/) among exe_dir ancestors, under which
    /// `out/.weaknet` (= bash `ROOT/out/.weaknet` same path) > an existing `out/.weaknet` ancestor >
    /// cwd/`out/.weaknet` fallback.
    #[must_use]
    pub fn resolve(
        env_val: Option<&str>,
        exe_dir: &std::path::Path,
        cwd: &std::path::Path,
    ) -> Self {
        if let Some(v) = env_val.filter(|s| !s.is_empty()) {
            return Self { statedir: std::path::PathBuf::from(v) };
        }
        let mut probe = Some(exe_dir);
        let mut repo_hit: Option<std::path::PathBuf> = None;
        let mut exists_hit: Option<std::path::PathBuf> = None;
        for _ in 0..6 {
            let Some(dir) = probe else { break };
            let cand = dir.join(DEFAULT_OUT_SUBDIR);
            if exists_hit.is_none() && cand.is_dir() {
                exists_hit = Some(cand.clone());
            }
            if repo_hit.is_none() && dir.join("Cargo.toml").is_file() && dir.join("crates").is_dir()
            {
                repo_hit = Some(cand);
            }
            probe = dir.parent();
        }
        Self { statedir: repo_hit.or(exists_hit).unwrap_or_else(|| cwd.join(DEFAULT_OUT_SUBDIR)) }
    }

    #[must_use]
    pub fn from_env() -> Self {
        let env_val = std::env::var("WEAKNET_STATEDIR").ok();
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_default();
        let cwd = std::env::current_dir().unwrap_or_default();
        Self::resolve(env_val.as_deref(), &exe_dir, &cwd)
    }
}

// ---------- Write lock (cross-tool window note) ----------

/// Transient write lock (apply/set/scenario full run; clear/status lock-free reachable -- bash parity).
/// Mechanism = the `<statedir>/lock` pid+starttime lock file (create_new atomicity + fuse::read_proc_starttime
/// liveness reaping). **Same path as bash flock(1), different mechanism**: Rust->bash mutual exclusion is guaranteed by
/// "window discipline = bash writes 0 times" (tasks.md header); while bash holds the lock we conservatively report busy on the empty lock file.
pub fn acquire_write_lock(dirs: &Dirs) -> Result<WriteLock, String> {
    std::fs::create_dir_all(&dirs.statedir)
        .map_err(|e| format!("failed to create the state dir: {e}"))?;
    let path = dirs.lock();
    for attempt in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => {
                use std::io::Write;
                let pid = std::process::id();
                let st = crate::fuse::read_proc_starttime(pid).unwrap_or(0);
                let mut f = f;
                if let Err(e) = writeln!(f, "{pid} {st}") {
                    eprintln!("weaknet(state): failed to write lock content: {e}");
                }
                return Ok(WriteLock { path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if holder_alive(&path)? {
                    return Err(format!(
                        "another weaknet instance is running (holds the lock; to take over wait for it to finish or clear first): {}",
                        path.display()
                    ));
                }
                // dead owner -> reap the stale lock and retry once
                std::fs::remove_file(&path)
                    .map_err(|e| format!("failed to reap the stale lock: {e}"))?;
                if attempt == 1 {
                    return Err(
                        "second create after reaping still failed (race?) -- retry".to_string()
                    );
                }
            }
            Err(e) => return Err(format!("failed to open the lock file: {e}")),
        }
    }
    Err("unreachable".to_string())
}

fn holder_alive(path: &std::path::Path) -> Result<bool, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read the lock file: {e}"))?;
    let mut it = raw.split_whitespace();
    let (Some(pid), Some(starttime)) = (it.next(), it.next()) else {
        // empty/malformed = a bash flock-era leftover (flock writes no content) -- conservatively treat as held, leave the verdict to a manual clear
        eprintln!(
            "weaknet(state): lock file {} has no owner info (bash flock leftover? malformed) -- conservatively reporting busy; delete {} once sure nothing is concurrent",
            path.display(),
            path.display()
        );
        return Ok(true);
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return Ok(true);
    };
    let Ok(starttime) = starttime.parse::<u64>() else {
        return Ok(true);
    };
    Ok(match crate::fuse::read_proc_starttime(pid) {
        Some(now_st) => now_st == starttime,
        None => false,
    })
}

/// RAII: Drop releases (the write op transient semantics; a crash-leaked lock file is reaped via starttime liveness).
#[derive(Debug)]
pub struct WriteLock {
    path: std::path::PathBuf,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("weaknet(state): failed to release write lock {}: {e}", self.path.display());
        }
    }
}

// ---------- timeline (JSONL, UTC -- PIT-181: the lesson of local-timezone forensics misalignment) ----------

/// Append one event (auto-injects t_utc; callers pass `json!({...})` without t_utc).
/// Event-name whitelist (the judge contract pinned at T8): apply/set/clear/self-heal-clear/watchdog-clear/
/// watchdog-clear-failed/scenario-start/baseline-done/scenario-step/repeat-round/scenario-end。
pub fn timeline_append(dirs: &Dirs, ev: serde_json::Value) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(&dirs.statedir)
        .map_err(|e| format!("failed to create the timeline dir: {e}"))?;
    let mut obj = match ev {
        serde_json::Value::Object(m) => m,
        _ => return Err("a timeline event must be an object".to_string()),
    };
    if !obj.contains_key("t_utc") {
        obj.insert("t_utc".to_string(), serde_json::Value::String(utc_now_string()));
    }
    let line = serde_json::Value::Object(obj).to_string();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dirs.timeline())
        .map_err(|e| format!("failed to open the timeline: {e}"))?;
    writeln!(f, "{line}").map_err(|e| format!("failed to write the timeline: {e}"))
}

/// bash `utc()` same shape: `%Y-%m-%dT%H:%M:%SZ` (epoch -> calendar day = Hinnant civil_from_days, zero deps).
#[must_use]
pub fn utc_now_string() -> String {
    let secs = crate::fuse::now_epoch_ms() / 1000;
    format_utc_secs(secs)
}

#[must_use]
fn format_utc_secs(epoch: u64) -> String {
    let (y, m, d) = civil_from_days((epoch / 86400) as i64);
    let tod = epoch % 86400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", tod / 3600, (tod % 3600) / 60, tod % 60)
}

/// Howard Hinnant civil_from_days (z = days since 1970-01-01).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------- param_summary (verbatim bash port) ----------

/// bash `param_summary()` exact shape:
/// `rtt=80ms jitter=15ms loss=2%/uniform rate=sentinel reorder=0 seed=off`
/// (LOSS_MODE never empty -> `/<mode>` always present; rate unset = sentinel; reorder unset = 0; seed unset = off.
/// the judge (measure) recognizes the recovery step by the `rtt=0ms` prefix + `loss=0%` substring -- the wording is the contract, do not change.)
#[must_use]
pub fn param_summary(s: &ImpairSpec) -> String {
    let (loss, mode) = match &s.loss {
        Some(crate::spec::LossSpec::Simple(p)) if !p.is_empty() => (p.clone(), "uniform"),
        Some(crate::spec::LossSpec::GeModel { loss, .. }) => (loss.clone(), "gemodel"),
        _ => ("0%".to_string(), "uniform"),
    };
    let rate = match s.rate_mbps {
        Some(n) if n.fract() == 0.0 => format!("{}", n as u64),
        Some(n) => format!("{n}"),
        None => "sentinel".to_string(),
    };
    let reorder = s.reorder_pct.clone().unwrap_or_else(|| "0".into());
    let seed = match s.seed {
        Some(n) => n.to_string(),
        None => "off".into(),
    };
    format!(
        "rtt={}ms jitter={}ms loss={}/{} rate={} reorder={} seed={}",
        s.rtt_ms, s.jitter_ms, loss, mode, rate, reorder, seed
    )
}

/// apply event body (bash state_write printf shape: rtt_ms/jitter_ms numbers, loss/rate_mbps/seed/stream strings).
#[must_use]
pub fn apply_event(s: &ImpairSpec, ports: &[u16], stream_sel: &str) -> serde_json::Value {
    let loss = match &s.loss {
        Some(crate::spec::LossSpec::Simple(p)) => p.clone(),
        Some(crate::spec::LossSpec::GeModel { loss, .. }) => loss.clone(),
        None => "0%".to_string(),
    };
    serde_json::json!({
            "ev": "apply",
            "rtt_ms": s.rtt_ms,
            "jitter_ms": s.jitter_ms,
            "loss": loss,
            "rate_mbps": s.rate_mbps.map(|n| if n.fract() == 0.0 { format!("{}", n as u64) } else { format!("{n}") }).unwrap_or_default(),
            "ports": ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" "),
            "seed": s.seed.map(|n| n.to_string()).unwrap_or_default(),
    // E1: same key as bash STREAM_SEL -- Stream/Device targeting lands the name string (room-level = empty string, same as the old form).
            "stream": stream_sel,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::LossSpec;

    fn sample_state() -> State {
        State {
            schema: STATE_SCHEMA.to_string(),
            spec: ImpairSpec {
                rtt_ms: 80,
                jitter_ms: 15,
                loss: Some(LossSpec::Simple("2%".into())),
                reorder_pct: Some("5".into()),
                rate_mbps: Some(4.0),
                seed: Some(7),
                ..Default::default()
            },
            dir: Dir::Both,
            scope: ScopeSel::Media,
            iface: "lo".into(),
            ports: vec![40010, 40011],
            pairs: vec![(20000, 40001)],
            rooms: vec!["cam0".into()],
            devices: vec![],
            sig_port: None,
            expires_at_ms: 1_757_000_000_000,
            created_root: true,
            ifb_used: true,
            created_ifb: true,
            job: Some(JobRef {
                name: "cell-edge".into(),
                pid: 4242,
                starttime: 99,
                done: 2,
                total: 4,
            }),
            teardown: Teardown {
                channel: ChannelSer::Sidecar,
                sidecar: Some(SidecarRef {
                    name: "weaknet-ctrl".into(),
                    image: "gaiadocker/iproute2".into(),
                }),
                steps: vec![vec![
                    "qdisc".into(),
                    "del".into(),
                    "dev".into(),
                    "lo".into(),
                    "root".into(),
                ]],
            },
        }
    }

    #[test]
    fn state_json_roundtrip_and_schema_guard() {
        let st = sample_state();
        let json = serde_json::to_string(&st).unwrap();
        let back: State = serde_json::from_str(&json).unwrap();
        assert_eq!(st, back, "roundtrip equal field by field");
        let dir = std::env::temp_dir().join(format!("weaknet-state-{}", std::process::id()));
        let p = dir.join(STATE_FILE);
        st.write_to(&p).unwrap();
        assert_eq!(State::read_from(&p).unwrap().as_ref(), Some(&st));
        // schema mismatch -> Err (never silent)
        let mut bad = st.clone();
        bad.schema = "weaknet-state/999".into();
        bad.write_to(&p).unwrap();
        assert!(State::read_from(&p).unwrap_err().contains("schema"));
        std::fs::remove_file(&p).ok();
        std::fs::remove_dir_all(&dir).ok();
        // missing = Ok(None)
        assert_eq!(State::read_from(&p).unwrap(), None);
    }

    // knife-C companion (T5): old state JSON without the pairs key -> serde(default) reads as empty,
    // a Media-form site stays readable across the upgrade (no schema cliff).
    #[test]
    fn state_json_without_pairs_key_still_reads() {
        let st = sample_state();
        let mut json = serde_json::to_value(&st).unwrap();
        json.as_object_mut().unwrap().remove("pairs");
        let back: State = serde_json::from_value(json).unwrap();
        assert_eq!(back.pairs, vec![]);
        assert_eq!(back.ports, st.ports, "other fields unaffected");
    }

    #[test]
    fn teardown_roundtrip_channel_ser_sidecar_shape() {
        // design §state.rs contract: teardown.channel persisted as the "sidecar" string + sidecar{name,image} object.
        let st = sample_state();
        let json = serde_json::to_string(&st).unwrap();
        assert!(
            json.contains(r#""channel":"sidecar""#),
            "channel word must be sidecar, got {json}"
        );
        assert!(json.contains(r#""name":"weaknet-ctrl""#));
        assert!(json.contains(r#""image":"gaiadocker/iproute2""#));
        let local = Teardown {
            channel: ChannelSer::LocalRoot,
            sidecar: None,
            steps: vec![vec!["qdisc".into(), "del".into()]],
        };
        let s = serde_json::to_string(&local).unwrap();
        assert!(s.contains(r#""channel":"local""#), "local form {s}");
        let back: Teardown = serde_json::from_str(&s).unwrap();
        assert_eq!(back, local);
        // a sidecar form with default-None fields round-trips (#[serde(default)] compat with old persisted state).
        let no_sc = r#"{"channel":"local","steps":[]}"#;
        let t: Teardown = serde_json::from_str(no_sc).unwrap();
        assert_eq!(t.channel, ChannelSer::LocalRoot);
        assert_eq!(t.sidecar, None);
    }

    #[test]
    fn utc_form_matches_bash_date_u_shape() {
        // bash `date -u +%Y-%m-%dT%H:%M:%SZ` -- the timeline forensics convention (PIT-181).
        assert_eq!(utc_now_string().len(), 20, "2026-09-08T01:26:49Z");
        assert!(utc_now_string().ends_with('Z'));
    }

    #[test]
    fn param_summary_bash_parity_strings() {
        // expected string = actual bash param_summary output (character by character; consumed by the judge/measure zeroing predicate)
        let s = ImpairSpec {
            rtt_ms: 80,
            jitter_ms: 15,
            loss: Some(LossSpec::Simple("2%".into())),
            ..Default::default()
        };
        assert_eq!(
            param_summary(&s),
            "rtt=80ms jitter=15ms loss=2%/uniform rate=sentinel reorder=0 seed=off"
        );
        let g = ImpairSpec {
            rtt_ms: 120,
            jitter_ms: 0,
            loss: Some(LossSpec::GeModel {
                loss: "8%".into(),
                r: "25%".into(),
                h: "0.2".into(),
                k: "0.05".into(),
            }),
            reorder_pct: Some("5".into()),
            rate_mbps: Some(4.5),
            seed: Some(42),
            ..Default::default()
        };
        assert_eq!(
            param_summary(&g),
            "rtt=120ms jitter=0ms loss=8%/gemodel rate=4.5 reorder=5 seed=42"
        );
        // recovery-step predicate compat (measure.rs: after.starts_with("rtt=0ms") && after.contains("loss=0%"))
        let z = param_summary(&ImpairSpec::default());
        assert!(z.starts_with("rtt=0ms") && z.contains("loss=0%"), "{z}");
    }

    #[test]
    fn apply_event_shape_bash_parity() {
        let s = ImpairSpec {
            rtt_ms: 160,
            jitter_ms: 30,
            loss: Some(LossSpec::Simple("8%".into())),
            rate_mbps: Some(4.0),
            ..Default::default()
        };
        let v = apply_event(&s, &[20000], "cam0,cam1");
        assert_eq!(v["ev"], "apply");
        assert_eq!(v["rtt_ms"], 160);
        assert_eq!(v["loss"], "8%");
        assert_eq!(v["rate_mbps"], "4");
        assert_eq!(v["ports"], "20000");
        assert_eq!(v["seed"], "");
        assert_eq!(
            v["stream"], "cam0,cam1",
            "E1: targeted name string in the stream key (same slot as bash STREAM_SEL)"
        );
    }

    #[test]
    fn utc_format_known_epochs() {
        assert_eq!(format_utc_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc_secs(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_utc_secs(1_757_292_409), "2025-09-08T00:46:49Z");
        // leap-year boundary 2024-02-29
        assert_eq!(format_utc_secs(1_709_208_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn statedir_resolution_chain_no_absolute_literal() {
        let root = std::env::temp_dir().join(format!("weaknet-dirs-{}", std::process::id()));
        let repo = root.join("proj");
        std::fs::create_dir_all(repo.join("crates/mediaservo-weaknet/src")).unwrap();
        std::fs::create_dir_all(repo.join("out")).unwrap();
        std::fs::write(repo.join("Cargo.toml"), "[workspace]").unwrap();
        let exe = repo.join("target/debug");
        std::fs::create_dir_all(&exe).unwrap();
        // env override wins
        let d = Dirs::resolve(Some("/tmp/injected-wnet"), &exe, &repo);
        assert_eq!(d.statedir, std::path::PathBuf::from("/tmp/injected-wnet"));
        // repo-root probing (exe -> ancestor repo)
        let d = Dirs::resolve(None, &exe, std::path::Path::new("/nowhere"));
        assert_eq!(d.statedir, repo.join("out/.weaknet"), "= same shape as bash ROOT/out/.weaknet");
        // cwd fallback
        let d = Dirs::resolve(None, std::path::Path::new("/nonexistent/deep/path"), &repo);
        assert_eq!(d.statedir, repo.join("out/.weaknet"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_lock_lifecycle_stale_reclaim_and_busy() {
        let dir = std::env::temp_dir().join(format!("weaknet-lock-{}", std::process::id()));
        let dirs = Dirs { statedir: dir.clone() };
        {
            let _lk = acquire_write_lock(&dirs).expect("first acquire must succeed");
            assert!(dirs.lock().exists());
            // second acquire while held = busy (same-process second create_new -> content pid=self starttime=self -> alive)
            let e = acquire_write_lock(&dirs).unwrap_err();
            assert!(e.contains("another weaknet instance"), "{e}");
        }
        assert!(!dirs.lock().exists(), "Drop releases");
        // stale owner (pid missing) -> reap and retry succeeds
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dirs.lock(), "4294967 12345\n").unwrap();
        let _lk = acquire_write_lock(&dirs).expect("stale lock should be reaped");
        // empty file (bash flock leftover form) -> conservatively report busy
        drop(_lk);
        std::fs::write(dirs.lock(), "").unwrap();
        assert!(acquire_write_lock(&dirs).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn timeline_appends_utc_and_rejects_non_object() {
        let dir = std::env::temp_dir().join(format!("weaknet-tl-{}", std::process::id()));
        let dirs = Dirs { statedir: dir.clone() };
        timeline_append(&dirs, serde_json::json!({"ev":"apply","rtt_ms":80})).unwrap();
        timeline_append(&dirs, serde_json::json!({"ev":"clear","t_utc":"2026-01-01T00:00:00Z"}))
            .unwrap();
        let lines: Vec<String> =
            std::fs::read_to_string(dirs.timeline()).unwrap().lines().map(str::to_string).collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert!(first["t_utc"].as_str().unwrap().ends_with("Z"), "first line auto-injects UTC");
        assert_eq!(
            lines[1], r#"{"ev":"clear","t_utc":"2026-01-01T00:00:00Z"}"#,
            "existing t_utc not overwritten, key order kept"
        );
        assert!(timeline_append(&dirs, serde_json::json!("nope")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
