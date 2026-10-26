//! T8 scenario engine integration tests: NoopExec (zero tc/docker touching) chains the run_full full path --
//! event sequence pinned verbatim (bash do_scenario timeline_note conventions), drift columns, job progress,
//! the abort/stop/stale-recovery three surfaces, and lands clean/aborted timeline artifacts in CARGO_TARGET_TMPDIR
//! The judge consumption contract is now covered in-Rust: tests/measure_goldens.rs (SR6 P3-half-B; the python contract test was retired with judge.py).

use std::sync::{Arc, Mutex};

use mediaservo_weaknet::engine::{self, ApplyRequest, Fail, Wn};
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scenario::{self, PlanLine, RunSpec, StepExec};
use mediaservo_weaknet::scope::Targeting;
use mediaservo_weaknet::spec::{ImpairSpec, ScopeSel};
use mediaservo_weaknet::state::{self, ChannelSer, Dirs, JobRef, STATE_SCHEMA, State, Teardown};

// ---------- Noop StepExec (mirrors EngineExec's persistence/event shapes, zero privileged touching) ----------

#[derive(Default)]
struct Recorder {
    applies: u32,
    set_jobs_done: Vec<u32>,
}

struct NoopExec {
    dirs: Dirs,
    rec: Arc<Mutex<Recorder>>,
}

fn stream_sel(req: &ApplyRequest) -> String {
    if !req.names.rooms.is_empty() {
        req.names.rooms.join(",")
    } else {
        req.names.devices.join(",")
    }
}

fn write_state(dirs: &Dirs, req: &ApplyRequest, job: Option<JobRef>, expires_at_ms: u64) {
    let st = State {
        schema: STATE_SCHEMA.to_string(),
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
        created_root: true,
        ifb_used: false,
        created_ifb: false,
        job,
        teardown: Teardown { channel: ChannelSer::LocalRoot, sidecar: None, steps: vec![] },
    };
    st.write_to(&dirs.state_json()).unwrap();
}

impl StepExec for NoopExec {
    fn apply(&self, req: &ApplyRequest, duration_secs: u64) -> Wn<()> {
        write_state(&self.dirs, req, None, fuse::now_epoch_ms() + duration_secs * 1000);
        state::timeline_append(
            &self.dirs,
            state::apply_event(&req.spec, &req.ports, &stream_sel(req)),
        )
        .map_err(Fail::env)?;
        self.rec.lock().unwrap().applies += 1;
        Ok(())
    }
    fn set(&self, req: &ApplyRequest, prior: State) -> Wn<()> {
        let after = state::param_summary(&req.spec);
        {
            let mut r = self.rec.lock().unwrap();
            r.set_jobs_done.push(prior.job.as_ref().map_or(u32::MAX, |j| j.done));
        }
        write_state(&self.dirs, req, prior.job.clone(), prior.expires_at_ms);
        state::timeline_append(
            &self.dirs,
            serde_json::json!({
                "ev": "set",
                "before": state::param_summary(&prior.spec),
                "after": after,
                "counters": "ok",
            }),
        )
        .map_err(Fail::env)
    }
    fn clear(&self) -> Wn<()> {
        if let Err(e) = std::fs::remove_file(self.dirs.state_json())
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Fail::env(format!("failed to rm state: {e}")));
        }
        state::timeline_append(&self.dirs, serde_json::json!({"ev": "clear"})).map_err(Fail::env)
    }
}

// ---------- scaffold ----------

fn dirs_for(tag: &str) -> Dirs {
    let d = std::env::temp_dir().join(format!("wnet-sceneng-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    Dirs { statedir: d }
}

/// (exec facade, recorder) -- exec handed over as Arc<dyn>, sparing callers scattered casts.
fn noop(dirs: &Dirs) -> (Arc<dyn StepExec>, Arc<Mutex<Recorder>>) {
    let rec = Arc::new(Mutex::new(Recorder::default()));
    (Arc::new(NoopExec { dirs: dirs.clone(), rec: rec.clone() }), rec)
}

fn rs(name: &str, yaml: &str, keep: bool) -> RunSpec {
    RunSpec {
        name: name.to_string(),
        file_disp: format!("{name}.yaml"),
        plan: scenario::parse_plan_yaml(yaml).unwrap(),
        no_baseline: false,
        keep,
        targeting: Targeting {
            scope: ScopeSel::Media,
            ports: vec![40010],
            pairs: vec![],
            names: vec![],
        },
        iface: "lo".into(),
        sig_port: None,
        recovery_dwell_secs: Some(0), // test-shrunk form (production None -> the 25s bash truth)
    }
}

fn timeline_events(dirs: &Dirs) -> Vec<serde_json::Value> {
    let raw = std::fs::read_to_string(dirs.timeline()).unwrap();
    raw.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn ev_seq(dirs: &Dirs) -> Vec<String> {
    timeline_events(dirs).iter().map(|e| e["ev"].as_str().unwrap().to_string()).collect()
}

fn own_job(name: &str, alive: bool) -> JobRef {
    JobRef {
        name: name.to_string(),
        pid: if alive { std::process::id() } else { u32::MAX },
        starttime: fuse::read_proc_starttime(std::process::id()).unwrap_or(0),
        done: 0,
        total: 1,
    }
}

fn media_req(spec: ImpairSpec) -> ApplyRequest {
    ApplyRequest {
        spec,
        scope: ScopeSel::Media,
        names: Default::default(),
        iface: "lo".into(),
        ports: vec![40010],
        pairs: vec![],
        sig_port: None,
    }
}

fn artifact_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name)
}

const CLEAN_YAML: &str = "scenario:\n  baseline_s: 1\n  steps:\n    - at_s: 1\n      set: {rtt_ms: 80, loss: \"5%\"}\n    - at_s: 2\n      set: {rtt_ms: 0, jitter_ms: 0, loss: \"0%\"}\n";

// ---------- A: full clean run -- event sequence verbatim + drift + the recovery step's after + job progress ----------

#[tokio::test]
async fn clean_run_exact_event_sequence_and_job_progression() {
    let dirs = dirs_for("clean");
    let (exec, rec) = noop(&dirs);
    let out = scenario::run_full(rs("clean", CLEAN_YAML, false), exec, dirs.clone()).await;
    assert_eq!(out.aborted, None, "a clean run must not abort: {:?}", out.aborted);
    assert_eq!(out.exit_code, 0);

    // exact event order (bash do_scenario convention: apply -> scenario-start -> baseline-done -> (set,step)x2 -> clear)
    assert_eq!(
        ev_seq(&dirs),
        vec![
            "apply",
            "scenario-start",
            "baseline-done",
            "set",
            "scenario-step",
            "set",
            "scenario-step",
            "scenario-end",
            "clear"
        ]
    );
    let tl = timeline_events(&dirs);
    let by = |ev: &str| tl.iter().find(|e| e["ev"] == ev).unwrap().clone();

    // the scenario-start note = the bash L571 form: `<file> baseline=Ns steps_dur=Ns` (total=max(at+15)=17)
    assert_eq!(
        by("scenario-start")["note"].as_str().unwrap(),
        "clean.yaml baseline=1s steps_dur=17s"
    );
    assert_eq!(by("baseline-done")["note"], "window_s=1");
    // the drift column must be present and =1 (within tolerance at this polling granularity)
    let steps: Vec<_> = tl.iter().filter(|e| e["ev"] == "scenario-step").collect();
    assert_eq!(steps.len(), 2);
    assert!(steps[0]["note"].as_str().unwrap().starts_with("at_s=1 actual_s="));
    assert!(steps[0]["note"].as_str().unwrap().ends_with("drift_le_1s=1"));
    assert!(steps[1]["note"].as_str().unwrap().starts_with("at_s=2 actual_s="));
    assert!(steps[1]["note"].as_str().unwrap().ends_with("drift_le_1s=1"));
    // the recovery (zeroing) step: set.after prefix rtt=0ms AND contains loss=0% (the consumption surface of the measure zeroing predicate)
    let sets: Vec<_> = tl.iter().filter(|e| e["ev"] == "set").collect();
    let after0 = sets[0]["after"].as_str().unwrap();
    let after1 = sets[1]["after"].as_str().unwrap();
    assert!(after0.starts_with("rtt=80ms") && after0.contains("loss=5%"), "{after0}");
    assert!(
        after1.starts_with("rtt=0ms") && after1.contains("loss=0%"),
        "the recovery step's after must satisfy the judge whitelist predicate: {after1}"
    );
    // the scenario-end elapsed_s column in place
    let end = by("scenario-end");
    assert!(end["note"].as_str().unwrap().starts_with("elapsed_s="));
    assert!(end.get("aborted").is_none(), "a clean run's scenario-end must not carry aborted");

    // job progress flows in through prior and lands on disk: done 1,2 in order (step_sync sets done before exec.set)
    assert_eq!(rec.lock().unwrap().set_jobs_done, vec![1, 2]);
    assert_eq!(rec.lock().unwrap().applies, 1);
    // wrap-up: state cleared with the site; timeline kept; the aborted set empty
    assert!(!dirs.state_json().exists());
    assert_eq!(scenario::aborted_in_timeline(&dirs).unwrap(), Vec::<String>::new());

    // judge contract artifacts (the timeline contract now pinned by tests/measure_goldens.rs; these copies kept as run evidence)
    std::fs::copy(dirs.timeline(), artifact_path("scenario_clean_timeline.jsonl")).unwrap();
    std::fs::write(artifact_path("abort_clean.txt"), "none\n").unwrap();
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- B: the keep form -- no cleanup + finish-time job removal (long-lived serve anti-self-lock) + names echo ----------

#[tokio::test]
async fn keep_run_clears_job_but_preserves_site() {
    let dirs = dirs_for("keep");
    let (exec, _rec) = noop(&dirs);
    let mut spec_rs = rs("keep", CLEAN_YAML, true);
    spec_rs.targeting = Targeting {
        scope: ScopeSel::Stream,
        ports: vec![],
        pairs: vec![(40010, 50001)],
        names: vec!["room-x".into()],
    };
    let out = scenario::run_full(spec_rs, exec, dirs.clone()).await;
    assert_eq!(out.aborted, None);
    // keep -> no clear event, site kept
    let seq = ev_seq(&dirs);
    assert!(!seq.contains(&"clear".to_string()), "the keep form must not clean the site: {seq:?}");
    let st = State::read_from(&dirs.state_json()).unwrap().unwrap();
    // finish-time fallback removes the job (otherwise the serve owner = a live process -> self-locks all later apply/set)
    assert!(
        st.job.is_none(),
        "the job flag must be removed after the scenario completes: {:?}",
        st.job
    );
    // E2 ledger: names into state (the panel echo truth source)
    assert_eq!(st.rooms, vec!["room-x".to_string()]);
    // E1 ledger: the apply event stream key = the name union (same key and convention as bash STREAM_SEL)
    let tl = timeline_events(&dirs);
    assert_eq!(tl[0]["ev"], "apply");
    assert_eq!(tl[0]["stream"], "room-x");
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- C: the lock-busy abort form -- scenario-end.aborted + the verdict-invalidation surface ----------

#[tokio::test]
async fn lock_busy_step_aborts_with_flagged_scenario_end() {
    let dirs = dirs_for("busy");
    let (exec, _rec) = noop(&dirs);
    let spec_rs = Arc::new(rs(
        "busy",
        "scenario:\n  baseline_s: 0\n  steps:\n    - at_s: 0\n      set: {rtt_ms: 80}\n",
        false,
    ));
    scenario::start_blocking(&spec_rs, &exec, &dirs).unwrap();
    // the job's initial shape on disk: done=0 total=line count
    let j = State::read_from(&dirs.state_json()).unwrap().unwrap().job.unwrap();
    assert_eq!((j.name.as_str(), j.done, j.total), ("busy", 0, 1));
    let cur = State::read_from(&dirs.state_json()).unwrap().unwrap().spec;
    // external lock grab -> the first step's transient lock fails -> aborted:"lock-busy"
    let _held = engine::take_write_lock(&dirs).unwrap();
    let out = scenario::drive(spec_rs, exec, Arc::new(dirs.clone()), cur).await;
    drop(_held);
    assert_eq!(out.aborted.as_deref(), Some("lock-busy"));
    assert_eq!(out.exit_code, 3, "lock-busy -> exit3 (the guard conflict family)");
    let tl = timeline_events(&dirs);
    let end = tl.iter().rev().find(|e| e["ev"] == "scenario-end").unwrap();
    assert_eq!(end["ev"], "scenario-end");
    assert_eq!(end["aborted"], "lock-busy", "the rev-2.2 verdict-invalidation signal field");
    assert!(end["note"].as_str().unwrap().starts_with("elapsed_s="));
    assert_eq!(scenario::aborted_in_timeline(&dirs).unwrap(), vec!["lock-busy".to_string()]);
    // abort wrap-up (not keep): the site is cleared
    assert!(!dirs.state_json().exists());
    // contract artifacts (the aborted form)
    std::fs::copy(dirs.timeline(), artifact_path("scenario_aborted_timeline.jsonl")).unwrap();
    std::fs::write(artifact_path("abort_aborted.txt"), "lock-busy\n").unwrap();
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- D: the job exclusivity gate -- a live owner blocks a second run, zero side effects ----------

#[tokio::test]
async fn alive_job_gates_scenario_start() {
    let dirs = dirs_for("gate");
    let req = media_req(ImpairSpec::default());
    write_state(&dirs, &req, Some(own_job("ghost-run", true)), fuse::now_epoch_ms() + 300_000);
    let e = scenario::job_gate(&dirs).unwrap_err();
    assert_eq!(e.code, 3, "the conflict family exits 3 (bash guard same code)");
    assert!(e.msg.contains("scenario in progress"), "{}", e.msg);
    // a second run is blocked the same way (exclusivity = the job flag layer, independent of the lock)
    let (exec, _rec) = noop(&dirs);
    let err = scenario::start_blocking(&Arc::new(rs("second", CLEAN_YAML, false)), &exec, &dirs)
        .unwrap_err();
    assert_eq!(err.code, 3);
    // blocked at the door = zero event writes (write_state only touches state.json)
    assert!(!dirs.timeline().exists(), "job_gate intercepts before any timeline write");
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- E: stale owner recovery -- declared dead passes + stop self-clears the flag ----------

#[tokio::test]
async fn stale_job_recovery_gate_passes_and_stop_clears() {
    let dirs = dirs_for("stale");
    let req = media_req(ImpairSpec { rtt_ms: 40, ..Default::default() });
    write_state(&dirs, &req, Some(own_job("dead-run", false)), fuse::now_epoch_ms() + 300_000);
    scenario::job_gate(&dirs).expect("a dead owner's flag must not block");
    let msg = scenario::stop(&dirs).unwrap();
    assert!(msg.contains("stale") && msg.contains("dead-run"), "{msg}");
    let st = State::read_from(&dirs.state_json()).unwrap().unwrap();
    assert!(st.job.is_none(), "stop persists and self-clears the dead owner's flag");
    assert_eq!(st.spec.rtt_ms, 40, "clearing the flag doesn't touch the site (spec kept)");
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- F: cross-process stop -- the cancel flag -> aborted:"user" normal wrap-up exit0 ----------

#[tokio::test]
async fn stop_mid_run_emits_aborted_user_and_exits_zero() {
    let dirs = dirs_for("stopmid");
    let (exec, _rec) = noop(&dirs);
    let spec_rs = Arc::new(rs(
        "stopmid",
        "scenario:\n  baseline_s: 0\n  steps:\n    - at_s: 60\n      set: {rtt_ms: 80}\n",
        false,
    ));
    scenario::start_blocking(&spec_rs, &exec, &dirs).unwrap();
    let cur = State::read_from(&dirs.state_json()).unwrap().unwrap().spec;
    let dirs_arc = Arc::new(dirs.clone());
    let h = tokio::spawn(scenario::drive(spec_rs, exec, dirs_arc, cur));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let msg = scenario::stop(&dirs).unwrap();
    assert!(msg.contains("stop requested"), "{msg}");
    let out = h.await.unwrap();
    assert_eq!(out.aborted.as_deref(), Some("user"));
    assert_eq!(out.exit_code, 0, "an active stop = a normal wrap-up shape (not the error family)");
    let tl = timeline_events(&dirs);
    let end = tl.iter().rev().find(|e| e["ev"] == "scenario-end").unwrap();
    assert_eq!((end["ev"].as_str(), end["aborted"].as_str()), (Some("scenario-end"), Some("user")));
    // the cancel flag consumed + post-wrap-up cleanup returns the site to zero
    assert!(!scenario::cancel_path(&dirs).exists());
    assert!(!dirs.state_json().exists());
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- G: grammar fail-closed and emit.py parity surface (@git ancestry: parent scripts/weaknet.d/emit.py, deleted SR6 P3-half-B) ----------

#[test]
fn parse_grammar_parity_with_emit() {
    // unknown keys rejected and the error lists the full legal set (the fail-closed contract)
    let err = scenario::parse_plan_yaml(
        "scenario:\n  steps:\n    - at_s: 1\n      set: {bogus_key: 1}\n",
    )
    .unwrap_err();
    assert!(
        err.contains("unknown keys") && err.contains("rate_mbps") && err.contains("gemodel"),
        "{err}"
    );
    // at_s strictly increasing
    let err = scenario::parse_plan_yaml(
        "scenario:\n  steps:\n    - at_s: 2\n      set: {rtt_ms: 1}\n    - at_s: 2\n      set: {rtt_ms: 2}\n",
    )
    .unwrap_err();
    assert!(err.contains("strictly increasing"), "{err}");
    // an empty plan rejected
    assert!(scenario::parse_plan_yaml("scenario: {baseline_s: 5}").is_err());
    // baseline default = total (the last step +15 observation tail); the at_s float truncation form (bash ${f2%.*})
    let p = scenario::parse_plan_yaml(
        "scenario:\n  steps:\n    - at_s: 4.7\n      set: {rtt_ms: 30}\n",
    )
    .unwrap();
    assert_eq!((p.baseline_s, p.total_s), (19, 19));
    let PlanLine::Step { at_disp, at_secs, .. } = &p.lines[0] else {
        panic!("a single step should be Step");
    };
    assert_eq!((at_disp.as_str(), *at_secs), ("4.7", 4));
    // repeat: the off fixed zero-param form (emit.py @git ancestry line 107 -> the recovery-predicate generation source)
    let p = scenario::parse_plan_yaml(
        "scenario:\n  repeat: {rounds: 2, on_s: 6, off_s: 5, set: {rtt_ms: 80}}\n",
    )
    .unwrap();
    let PlanLine::Repeat { rounds, on_s, off_s, on_flags, off_flags, .. } = &p.lines[0] else {
        panic!("should be Repeat");
    };
    assert_eq!((*rounds, *on_s, *off_s), (2, 6, 5));
    assert_eq!(on_flags, "--rtt 80");
    assert_eq!(off_flags, "--rtt 0 --jitter 0 --loss 0%");
    // clear falsy semantics (python truthiness)
    assert!(
        !scenario::parse_plan_yaml(
            "scenario:\n  clear: false\n  steps:\n    - at_s: 1\n      set: {rtt_ms: 10}\n"
        )
        .unwrap()
        .clear
    );
    assert!(
        scenario::parse_plan_yaml("scenario:\n  steps:\n    - at_s: 1\n      set: {rtt_ms: 10}\n")
            .unwrap()
            .clear
    );
    // range validation (same rules as the emit.py ancestor's lines 50-56)
    assert!(
        scenario::parse_plan_yaml(
            "scenario:\n  steps:\n    - at_s: 1\n      set: {loss: \"abc\"}\n"
        )
        .is_err()
    );
    assert!(
        scenario::parse_plan_yaml(
            "scenario:\n  steps:\n    - at_s: 1\n      set: {rate_mbps: 0}\n"
        )
        .is_err()
    );
}

// ---------- H: merge_step inheritance semantics (bash do_set: explicit overrides, defaults inherited) ----------

#[test]
fn merge_step_inherits_base_like_bash_do_set() {
    let base = ImpairSpec { rtt_ms: 80, jitter_ms: 15, rate_mbps: Some(4.0), ..Default::default() };
    // only rtt touched: jitter/rate inherited; loss explicitly overridden
    let m = scenario::merge_step(
        &base,
        &scenario::StepOverride { rtt_ms: Some(0), loss: Some("0%".into()), ..Default::default() },
    )
    .unwrap();
    assert_eq!((m.rtt_ms, m.jitter_ms, m.rate_mbps), (0, 15, Some(4.0)));
    // a gemodel step with no loss and a loss-less base -> reports cause (bash --gemodel must pair with --loss, C15 never silent)
    let g = scenario::merge_step(
        &base,
        &scenario::StepOverride {
            gemodel: Some(("0.01".into(), "3".into(), "1".into())),
            ..Default::default()
        },
    );
    assert!(g.is_err(), "gemodel must report cause when the base has no loss");
}

// ---------- I: the --auto-clear signal-layer wrapper -- the flag writer = the signal handler's equivalent form ----------
// run_full_auto_clear internally = unix_signals(SIGINT/SIGTERM) -> request_stop writing the same cancel flag file as scenario::stop's
// owner-alive branch; the handler itself is a thin wrapper (unix_signals/request_stop both surfaces
// already pinned by src/scenario.rs unit tests: a real SIGTERM delivered + idempotent flag writes); this case validates the wrapper's overall contract:
// an external flag write (simulating the signal effect) -> keep waiting for run_full to self-terminate at a checkpoint -> the aborted:"user" shape
// scenario-end + teardown (the exec.clear event, site zeroed, flag consumed) + exit0.
// the difference vs test F: goes through the auto-clear entry with keep=false, exercising the abort wrap-up's clear chain end to end.

#[tokio::test]
async fn auto_clear_wrapper_aborts_with_teardown_like_stop() {
    let dirs = dirs_for("autoclear");
    let (exec, _rec) = noop(&dirs);
    let spec_rs = rs(
        "autoclear",
        "scenario:\n  baseline_s: 0\n  steps:\n    - at_s: 60\n      set: {rtt_ms: 80}\n",
        false,
    );
    let dirs_cl = dirs.clone();
    // the signal effect's equivalent form: a concurrent task writes the stop flag (handler = unix_signals -> request_stop, same file and content)
    let flagger = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        scenario::stop(&dirs_cl).unwrap();
    });
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        scenario::run_full_auto_clear(spec_rs, exec, dirs.clone()),
    )
    .await
    .expect(
        "the wrapper should self-terminate at a checkpoint, not hang until the 60s step window",
    );
    let _ = flagger.await;
    assert_eq!(out.aborted.as_deref(), Some("user"), "the signal-layer form = the stop-flag form");
    assert_eq!(out.exit_code, 0, "an active abort = the normal wrap-up family");
    let tl = timeline_events(&dirs);
    let end = tl.iter().rev().find(|e| e["ev"] == "scenario-end").unwrap();
    assert_eq!(end["aborted"], "user");
    assert!(end["note"].as_str().unwrap().starts_with("elapsed_s="));
    // teardown goes through the existing abort wrap-up: the exec.clear event lands in the timeline, site state zeroed, flag consumed
    assert_eq!(tl.last().unwrap()["ev"], "clear", "the abort wrap-up chain must run clear");
    assert!(!scenario::cancel_path(&dirs).exists());
    assert!(!dirs.state_json().exists());
    std::fs::remove_dir_all(&dirs.statedir).ok();
}
