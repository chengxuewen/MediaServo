//! T8 场景引擎集成测试：NoopExec（零 tc/docker 触达）串起 run_full 全链——
//! 事件序列逐字面钉（bash do_scenario timeline_note 口径）、drift 列、job 进度、
//! abort/stop/陈旧恢复三面，并把 clean/aborted 两份 timeline 工件落 CARGO_TARGET_TMPDIR
//! 供 tests/scenario_judge_contract.py（judge.py 消费合同）二次消费。

use std::sync::{Arc, Mutex};

use mediaservo_weaknet::engine::{self, ApplyRequest, Fail, Wn};
use mediaservo_weaknet::fuse;
use mediaservo_weaknet::scenario::{self, PlanLine, RunSpec, StepExec};
use mediaservo_weaknet::scope::Targeting;
use mediaservo_weaknet::spec::{ImpairSpec, ScopeSel};
use mediaservo_weaknet::state::{self, ChannelSer, Dirs, JobRef, State, STATE_SCHEMA, Teardown};

// ---------- Noop StepExec（镜像 EngineExec 的落盘/事件形，零特权触达） ----------

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
        teardown: Teardown {
            channel: ChannelSer::LocalRoot,
            sidecar: None,
            steps: vec![],
        },
    };
    st.write_to(&dirs.state_json()).unwrap();
}

impl StepExec for NoopExec {
    fn apply(&self, req: &ApplyRequest, duration_secs: u64) -> Wn<()> {
        write_state(
            &self.dirs,
            req,
            None,
            fuse::now_epoch_ms() + duration_secs * 1000,
        );
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
            r.set_jobs_done
                .push(prior.job.as_ref().map_or(u32::MAX, |j| j.done));
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
        ).map_err(Fail::env)
    }
    fn clear(&self) -> Wn<()> {
        if let Err(e) = std::fs::remove_file(self.dirs.state_json())
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(Fail::env(format!("rm state 失败: {e}")));
        }
        state::timeline_append(&self.dirs, serde_json::json!({"ev": "clear"})).map_err(Fail::env)
    }
}

// ---------- 脚手架 ----------

fn dirs_for(tag: &str) -> Dirs {
    let d = std::env::temp_dir().join(format!("wnet-sceneng-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    Dirs { statedir: d }
}

/// (exec 门面, 记录器)——exec 以 Arc<dyn> 形直交，免调用方散落 cast。
fn noop(dirs: &Dirs) -> (Arc<dyn StepExec>, Arc<Mutex<Recorder>>) {
    let rec = Arc::new(Mutex::new(Recorder::default()));
    (
        Arc::new(NoopExec {
            dirs: dirs.clone(),
            rec: rec.clone(),
        }),
        rec,
    )
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
        recovery_dwell_secs: Some(0), // 测试缩形（生产=None→25s bash 真值）
    }
}

fn timeline_events(dirs: &Dirs) -> Vec<serde_json::Value> {
    let raw = std::fs::read_to_string(dirs.timeline()).unwrap();
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn ev_seq(dirs: &Dirs) -> Vec<String> {
    timeline_events(dirs)
        .iter()
        .map(|e| e["ev"].as_str().unwrap().to_string())
        .collect()
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

// ---------- A：完整干净跑——事件序列逐字面 + drift + 恢复步 after + job 进度 ----------

#[tokio::test]
async fn clean_run_exact_event_sequence_and_job_progression() {
    let dirs = dirs_for("clean");
    let (exec, rec) = noop(&dirs);
    let out = scenario::run_full(rs("clean", CLEAN_YAML, false), exec, dirs.clone()).await;
    assert_eq!(out.aborted, None, "干净跑不该 abort: {:?}", out.aborted);
    assert_eq!(out.exit_code, 0);

    // 事件精确序（bash do_scenario 口径：apply→scenario-start→baseline-done→(set,step)×2→clear）
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

    // scenario-start note = bash L571 形：`<file> baseline=Ns steps_dur=Ns`（total=max(at+15)=17）
    assert_eq!(
        by("scenario-start")["note"].as_str().unwrap(),
        "clean.yaml baseline=1s steps_dur=17s"
    );
    assert_eq!(by("baseline-done")["note"], "window_s=1");
    // drift 列必在且=1（轮询粒度下容差内）
    let steps: Vec<_> = tl.iter().filter(|e| e["ev"] == "scenario-step").collect();
    assert_eq!(steps.len(), 2);
    assert!(steps[0]["note"].as_str().unwrap().starts_with("at_s=1 actual_s="));
    assert!(steps[0]["note"].as_str().unwrap().ends_with("drift_le_1s=1"));
    assert!(steps[1]["note"].as_str().unwrap().starts_with("at_s=2 actual_s="));
    assert!(steps[1]["note"].as_str().unwrap().ends_with("drift_le_1s=1"));
    // 恢复（清零）步：set.after 前缀 rtt=0ms ∧ 含 loss=0%（judge.py:71 判据的消费面）
    let sets: Vec<_> = tl.iter().filter(|e| e["ev"] == "set").collect();
    let after0 = sets[0]["after"].as_str().unwrap();
    let after1 = sets[1]["after"].as_str().unwrap();
    assert!(after0.starts_with("rtt=80ms") && after0.contains("loss=5%"), "{after0}");
    assert!(
        after1.starts_with("rtt=0ms") && after1.contains("loss=0%"),
        "恢复步 after 需满足 judge 白名单判据: {after1}"
    );
    // scenario-end elapsed_s 列在位
    let end = by("scenario-end");
    assert!(end["note"].as_str().unwrap().starts_with("elapsed_s="));
    assert!(end.get("aborted").is_none(), "干净跑 scenario-end 不得带 aborted");

    // job 进度经 prior 流入落盘：done 依次 1,2（step_sync 先置 done 再 exec.set）
    assert_eq!(rec.lock().unwrap().set_jobs_done, vec![1, 2]);
    assert_eq!(rec.lock().unwrap().applies, 1);
    // 收尾：state 随现场清除；timeline 保留；aborted 集空
    assert!(!dirs.state_json().exists());
    assert_eq!(scenario::aborted_in_timeline(&dirs).unwrap(), Vec::<String>::new());

    // judge 合同工件（供 tests/scenario_judge_contract.py 消费）
    std::fs::copy(dirs.timeline(), artifact_path("scenario_clean_timeline.jsonl")).unwrap();
    std::fs::write(artifact_path("abort_clean.txt"), "none\n").unwrap();
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- B：keep 形——不清场 + 收尾摘 job（serve 长驻进程防自锁）+ names 回声 ----------

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
    // keep → 无 clear 事件、现场保留
    let seq = ev_seq(&dirs);
    assert!(!seq.contains(&"clear".to_string()), "keep 形不得清场: {seq:?}");
    let st = State::read_from(&dirs.state_json()).unwrap().unwrap();
    // 收尾兜底摘 job（否则 serve 属主=存活进程 → 自锁死后续 apply/set）
    assert!(st.job.is_none(), "scenario 完成后 job 旗必摘: {:?}", st.job);
    // E2 ledger：names 入 state（面板回显真值源）
    assert_eq!(st.rooms, vec!["room-x".to_string()]);
    // E1 ledger：apply 事件 stream 键 = 名字并集（bash STREAM_SEL 同键名同口径）
    let tl = timeline_events(&dirs);
    assert_eq!(tl[0]["ev"], "apply");
    assert_eq!(tl[0]["stream"], "room-x");
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- C：lock-busy abort 形——scenario-end.aborted + 判据作废面 ----------

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
    // job 初形落盘：done=0 total=行数
    let j = State::read_from(&dirs.state_json()).unwrap().unwrap().job.unwrap();
    assert_eq!((j.name.as_str(), j.done, j.total), ("busy", 0, 1));
    let cur = State::read_from(&dirs.state_json()).unwrap().unwrap().spec;
    // 外部抢锁 → 首步瞬持锁失败 → aborted:"lock-busy"
    let _held = engine::take_write_lock(&dirs).unwrap();
    let out = scenario::drive(spec_rs, exec, Arc::new(dirs.clone()), cur).await;
    drop(_held);
    assert_eq!(out.aborted.as_deref(), Some("lock-busy"));
    assert_eq!(out.exit_code, 3, "lock-busy → exit3（guard 冲突族）");
    let tl = timeline_events(&dirs);
    let end = tl.iter().rev().find(|e| e["ev"] == "scenario-end").unwrap();
    assert_eq!(end["ev"], "scenario-end");
    assert_eq!(end["aborted"], "lock-busy", "rev-2.2 判据作废信号字段");
    assert!(end["note"].as_str().unwrap().starts_with("elapsed_s="));
    assert_eq!(
        scenario::aborted_in_timeline(&dirs).unwrap(),
        vec!["lock-busy".to_string()]
    );
    // abort 收尾（非 keep）：现场清除
    assert!(!dirs.state_json().exists());
    // 合同工件（aborted 形）
    std::fs::copy(dirs.timeline(), artifact_path("scenario_aborted_timeline.jsonl")).unwrap();
    std::fs::write(artifact_path("abort_aborted.txt"), "lock-busy\n").unwrap();
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- D：job 独占门——属主存活拦二次 run，零副作用 ----------

#[tokio::test]
async fn alive_job_gates_scenario_start() {
    let dirs = dirs_for("gate");
    let req = media_req(ImpairSpec::default());
    write_state(
        &dirs,
        &req,
        Some(own_job("ghost-run", true)),
        fuse::now_epoch_ms() + 300_000,
    );
    let e = scenario::job_gate(&dirs).unwrap_err();
    assert_eq!(e.code, 3, "冲突族 exit3（bash guard 同码）");
    assert!(e.msg.contains("scenario 进行中"), "{}", e.msg);
    // 二次 run 同样被拦（互斥 = job 旗层，不依赖锁）
    let (exec, _rec) = noop(&dirs);
    let err = scenario::start_blocking(&Arc::new(rs("second", CLEAN_YAML, false)), &exec, &dirs)
        .unwrap_err();
    assert_eq!(err.code, 3);
    // 拦在门外 = 零事件写入（write_state 只动 state.json）
    assert!(
        !dirs.timeline().exists(),
        "job_gate 拦截先于任何 timeline 写"
    );
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- E：陈旧属主恢复——判亡放行 + stop 自清旗标 ----------

#[tokio::test]
async fn stale_job_recovery_gate_passes_and_stop_clears() {
    let dirs = dirs_for("stale");
    let req = media_req(ImpairSpec {
        rtt_ms: 40,
        ..Default::default()
    });
    write_state(
        &dirs,
        &req,
        Some(own_job("dead-run", false)),
        fuse::now_epoch_ms() + 300_000,
    );
    scenario::job_gate(&dirs).expect("死主旗标不得拦");
    let msg = scenario::stop(&dirs).unwrap();
    assert!(msg.contains("陈旧") && msg.contains("dead-run"), "{msg}");
    let st = State::read_from(&dirs.state_json()).unwrap().unwrap();
    assert!(st.job.is_none(), "stop 落盘自清死主旗");
    assert_eq!(st.spec.rtt_ms, 40, "清旗不动现场（spec 保留）");
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- F：跨进程 stop——cancel 旗标 → aborted:"user" 正常收尾 exit0 ----------

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
    assert!(msg.contains("已请求停止"), "{msg}");
    let out = h.await.unwrap();
    assert_eq!(out.aborted.as_deref(), Some("user"));
    assert_eq!(out.exit_code, 0, "主动 stop = 正常收尾形（非错误族）");
    let tl = timeline_events(&dirs);
    let end = tl.iter().rev().find(|e| e["ev"] == "scenario-end").unwrap();
    assert_eq!(
        (end["ev"].as_str(), end["aborted"].as_str()),
        (Some("scenario-end"), Some("user"))
    );
    // cancel 旗标消费 + 收尾清除后现场归零
    assert!(!scenario::cancel_path(&dirs).exists());
    assert!(!dirs.state_json().exists());
    std::fs::remove_dir_all(&dirs.statedir).ok();
}

// ---------- G：文法 fail-closed 与 emit.py 平价面 ----------

#[test]
fn parse_grammar_parity_with_emit() {
    // 未知键即拒且报错含合法集全清单（fail-closed 合同）
    let err =
        scenario::parse_plan_yaml("scenario:\n  steps:\n    - at_s: 1\n      set: {bogus_key: 1}\n")
            .unwrap_err();
    assert!(
        err.contains("未知键") && err.contains("rate_mbps") && err.contains("gemodel"),
        "{err}"
    );
    // at_s 严格递增
    let err = scenario::parse_plan_yaml(
        "scenario:\n  steps:\n    - at_s: 2\n      set: {rtt_ms: 1}\n    - at_s: 2\n      set: {rtt_ms: 2}\n",
    )
    .unwrap_err();
    assert!(err.contains("严格递增"), "{err}");
    // 空计划即拒
    assert!(scenario::parse_plan_yaml("scenario: {baseline_s: 5}").is_err());
    // baseline 缺省 = total（末步 +15 观察尾）；at_s 浮点截断形（bash ${f2%.*}）
    let p = scenario::parse_plan_yaml("scenario:\n  steps:\n    - at_s: 4.7\n      set: {rtt_ms: 30}\n")
        .unwrap();
    assert_eq!((p.baseline_s, p.total_s), (19, 19));
    let PlanLine::Step { at_disp, at_secs, .. } = &p.lines[0] else {
        panic!("单步应为 Step");
    };
    assert_eq!((at_disp.as_str(), *at_secs), ("4.7", 4));
    // repeat：off 固定零参形（emit.py:107 → judge 恢复判据生成源）
    let p = scenario::parse_plan_yaml(
        "scenario:\n  repeat: {rounds: 2, on_s: 6, off_s: 5, set: {rtt_ms: 80}}\n",
    )
    .unwrap();
    let PlanLine::Repeat {
        rounds,
        on_s,
        off_s,
        on_flags,
        off_flags,
        ..
    } = &p.lines[0]
    else {
        panic!("应为 Repeat");
    };
    assert_eq!((*rounds, *on_s, *off_s), (2, 6, 5));
    assert_eq!(on_flags, "--rtt 80");
    assert_eq!(off_flags, "--rtt 0 --jitter 0 --loss 0%");
    // clear 假值语义（python truthiness）
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
    // 值域校验（emit.py:50-56 同规）
    assert!(
        scenario::parse_plan_yaml("scenario:\n  steps:\n    - at_s: 1\n      set: {loss: \"abc\"}\n")
            .is_err()
    );
    assert!(scenario::parse_plan_yaml(
        "scenario:\n  steps:\n    - at_s: 1\n      set: {rate_mbps: 0}\n"
    )
    .is_err());
}

// ---------- H：merge_step 继承语义（bash do_set：显式覆盖、缺省继承） ----------

#[test]
fn merge_step_inherits_base_like_bash_do_set() {
    let base = ImpairSpec {
        rtt_ms: 80,
        jitter_ms: 15,
        rate_mbps: Some(4.0),
        ..Default::default()
    };
    // 只动 rtt：jitter/rate 继承；loss 显式覆盖
    let m = scenario::merge_step(
        &base,
        &scenario::StepOverride {
            rtt_ms: Some(0),
            loss: Some("0%".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!((m.rtt_ms, m.jitter_ms, m.rate_mbps), (0, 15, Some(4.0)));
    // gemodel 步缺省 loss 且基底无 loss → 报因（bash --gemodel 必伴 --loss 组合语义，C15 禁静默）
    let g = scenario::merge_step(
        &base,
        &scenario::StepOverride {
            gemodel: Some(("0.01".into(), "3".into(), "1".into())),
            ..Default::default()
        },
    );
    assert!(g.is_err(), "基底无 loss 时 gemodel 需报因");
}
