//! scenario.rs —— T8：剧本引擎（emit.py 文法解析 + tokio 调度 runner + job 旗标层）。
//!
//! 语义真值 = `scripts/weaknet.sh do_scenario` + `scripts/weaknet.d/emit.py scenario`（计划文法）
//! 及 `judge.py` 事件白名单（`scenario*` ∪ {baseline-done, set}；恢复步判据 = set 事件 after
//! 前缀 `rtt=0ms` ∧ 含 `loss=0%`）。PIT-187 纪律：bash 语义原样平移，偏离清单登记在
//! T8 交付报告（rev-2.2 授权差异：每步瞬持锁 + job 独占层取代 bash 全程 flock）。
//!
//! rev-2.2 锁模型（design §server「scenario job 独占语义」）：
//! - 物理写锁**每步瞬持**（保 stop 可达）；
//! - 独占 = `state.job` 旗标层：job 活跃期（属主存活）外部 CLI/REST apply/set → 409/exit3；
//! - 陈旧属主（pid+starttime 判亡，fuse 同法）→ 清旗标放行；
//! - step 取锁失败 → `{ev:"scenario-end", note:"elapsed_s=…", aborted:"lock-busy"}` 终止
//!   （**timeline 任一 scenario-end 带 aborted → 本轮 judge 判据作废**，W6 复验断言 aborted==0）；
//! - stop 通路 = `scenario.cancel` 旗标文件（跨进程可达；属主亡时 [`stop`] 直接清 job 旗标）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::engine::{self, ApplyRequest, Env, Fail, Replay, Verify, Wn};
use crate::fuse;
use crate::scope::Targeting;
use crate::spec::{ImpairSpec, LossSpec};
use crate::state::{self, Dirs, JobRef, State};

/// bash do_scenario 恢复驻留窗（L603：log 文案 12s、实睡 25s——以 sleep 真值平移）。
pub const RECOVERY_DWELL_SECS: u64 = 25;
/// bash do_scenario 保险丝余量（L570：duration = baseline + steps_dur + 90）。
pub const COVERING_MARGIN_SECS: u64 = 90;
/// 跨进程 stop 旗标（runner 每检查点消费后删除）。
pub const CANCEL_FILE: &str = "scenario.cancel";

// ---------- 解析（emit.py scenario 模式逐分支平移，fail-closed 白名单） ----------

/// step/repeat 参数字典 → 覆盖项（bash flags 渲染的 typed 等价形；值域校验在 merge）。
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
        /// at_s 原值串（scenario-step note 的 `at_s=` 位——bash 直抄 plan 行）
        at_disp: String,
        /// 目标秒（bash `target=${f2%.*}` 截断形）
        at_secs: u64,
        /// 渲染 flags 串（恢复驻留 awk `--rtt 0` 子串判据源）
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
    /// TOTAL 行（bash stepsdur；覆盖时长 = baseline + total_s + COVERING_MARGIN）
    pub total_s: u64,
    /// CLEAR 行（sc.clear 真值，缺省 true → 收尾自动 clear）
    pub clear: bool,
}

type Y = serde_yaml::Value;
type YM = serde_yaml::Mapping;

/// python str(scalar) 同形（yaml 值渲染进 flags 串的口径）。
fn py_str(v: &Y) -> String {
    match v {
        Y::Null => "None".into(),
        Y::Bool(b) => if *b { "True".into() } else { "False".into() },
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else if let Some(f) = n.as_f64() {
                // python str(4.0)="4.0"；Rust 4.0f64.to_string()="4" → 补 ".0"
                if f.fract() == 0.0 && f.is_finite() {
                    format!("{f:.1}")
                } else {
                    format!("{f}")
                }
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
        Y::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| i64::try_from(u).unwrap_or(i64::MAX))),
        _ => None,
    }
}

fn yk(k: &str) -> Y {
    Y::String(k.to_string())
}

/// emit.py check_params：白名单外键即拒（合法集逐字同 emit.py:21,47——FLAG 五键 + loss/gemodel/
/// scope/clear + 宽容豁免 at_s/set/steps/repeat/scenario）。值校验同 emit.py:50-56。
fn check_params(p: &YM, ctx: &str) -> Result<(), String> {
    const LEGAL: &[&str] = &[
        "at_s", "clear", "gemodel", "jitter_ms", "loss", "rate_mbps", "reorder_pct", "repeat",
        "rtt_ms", "scenario", "scope", "seed", "set", "steps",
    ];
    for k in p.keys() {
        let ks = k.as_str().unwrap_or_default();
        if !LEGAL.contains(&ks) {
            return Err(format!("{ctx}: 未知键 [{ks}]（白名单 {LEGAL:?}）"));
        }
    }
    if let Some(l) = p.get(yk("loss")) {
        let s = py_str(l);
        let core = s.replace('.', "");
        let core = core.trim_end_matches('%');
        if core.is_empty() || !core.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("{ctx}: loss 需数值/百分数，得 {s:?}"));
        }
    }
    for k in ["rtt_ms", "jitter_ms", "reorder_pct", "seed"] {
        if let Some(v) = p.get(yk(k))
            && !as_num(v).is_some_and(|n| n >= 0.0)
        {
            return Err(format!("{ctx}: {k} 需非负数值"));
        }
    }
    if let Some(v) = p.get(yk("rate_mbps"))
        && !as_num(v).is_some_and(|n| n > 0.0)
    {
        return Err(format!("{ctx}: rate_mbps 需正数"));
    }
    Ok(())
}

/// emit.py flags_from：dict 插入序 → CLI flag 串（gemodel 组合校验；scope/clear 静默剥离同祖先）。
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
                return Err(format!("{ctx}: gemodel 需 dict{{r,h,k}}：{}", py_str(v)));
            };
            let mut trio = Vec::with_capacity(3);
            for key in ["r", "h", "k"] {
                match m.get(yk(key)) {
                    Some(x) => trio.push(py_str(x)),
                    None => {
                        return Err(format!("{ctx}: gemodel 需 dict{{r,h,k}}：{}", py_str(v)));
                    }
                }
            }
            out.push("--gemodel".to_string());
            out.extend(trio);
        }
    }
    Ok(out.join(" "))
}

/// dict → StepOverride。bash 口径：--rtt/--jitter/--seed/--reorder 需整数字面量
/// （str(4.0)="4.0" 会被 parse_opts regex 拒 rc=4）——浮点在此显式报因。
fn override_from(p: &YM) -> Result<StepOverride, String> {
    let mut ov = StepOverride::default();
    let int_slot = |k: &str| -> Result<Option<u64>, String> {
        match p.get(yk(k)) {
            None => Ok(None),
            Some(v) => match as_int(v) {
                Some(i) if i >= 0 => Ok(Some(i as u64)),
                _ => Err(format!("{k} 需整数值（bash flag 域），得 {}", py_str(v))),
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
            return Err(format!("gemodel 需 dict{{r,h,k}}：{}", py_str(g)));
        };
        let mut trio = Vec::with_capacity(3);
        for key in ["r", "h", "k"] {
            match m.get(yk(key)) {
                Some(x) => trio.push(py_str(x)),
                None => {
                    return Err(format!("gemodel 需 dict{{r,h,k}}：{}", py_str(g)));
                }
            }
        }
        ov.gemodel = Some((trio[0].clone(), trio[1].clone(), trio[2].clone()));
    }
    Ok(ov)
}

/// emit.py scenario 主解析（doc 已 yaml 化）。计划行序 = STEP(s) 逐条 → REPEAT →（TOTAL/CLEAR 入结构体）。
pub fn parse_plan_doc(doc: &Y) -> Result<ScenarioPlan, String> {
    let empty_map = YM::new();
    let root = doc.as_mapping().unwrap_or(&empty_map);
    let sc = match root.get(yk("scenario")) {
        Some(v) => v,
        None => doc,
    }
    .as_mapping()
    .ok_or_else(|| "scenario 需 mapping".to_string())?;
    let steps: Vec<Y> = sc
        .get(yk("steps"))
        .and_then(|s| s.as_sequence().cloned())
        .unwrap_or_default();
    let repeat_map: YM = sc
        .get(yk("repeat"))
        .cloned()
        .unwrap_or(Y::Null)
        .as_mapping()
        .cloned()
        .unwrap_or_default();
    if steps.is_empty() && repeat_map.is_empty() {
        return Err("scenario 需至少 steps 或 repeat".to_string());
    }
    let mut lines = Vec::new();
    let mut prev = -1.0f64;
    let mut total = 0.0f64;
    for (i, st) in steps.iter().enumerate() {
        let ctx = format!("step{i}");
        let Some(stm) = st.as_mapping() else {
            return Err(format!("{ctx}: 需 mapping"));
        };
        let at = stm.get(yk("at_s")).cloned().unwrap_or(Y::Null);
        let Some(atf) = as_num(&at) else {
            return Err(format!("{ctx}: at_s 需非负数值"));
        };
        if atf < 0.0 {
            return Err(format!("{ctx}: at_s 需非负数值"));
        }
        if atf <= prev {
            return Err(format!("{ctx}: at_s 必须严格递增（{atf} <= {prev}）"));
        }
        let set_map = match stm.get(yk("set")).cloned().unwrap_or(Y::Null) {
            Y::Null => YM::new(),
            Y::Mapping(m) => m,
            _ => return Err(format!("{ctx}: set 需 mapping")),
        };
        check_params(&set_map, &ctx)?;
        let flags = flags_from(&set_map, &ctx)?;
        let ov = override_from(&set_map)?;
        lines.push(PlanLine::Step {
            at_disp: py_str(&at),
            at_secs: atf as u64,
            flags,
            ov,
        });
        prev = atf;
        total = total.max(atf + 15.0); // bash：末步留 15s 观察尾
    }
    if !repeat_map.is_empty() {
        let num = |k: &str| repeat_map.get(yk(k)).cloned().unwrap_or_else(|| Y::Number(0.into()));
        let (Some(r), Some(on_s), Some(off_s)) =
            (as_int(&num("rounds")), as_int(&num("on_s")), as_int(&num("off_s")))
        else {
            return Err("repeat 需 rounds>0 且 on_s/off_s≥5".to_string());
        };
        if r <= 0 || on_s < 5 || off_s < 5 {
            return Err("repeat 需 rounds>0 且 on_s/off_s≥5".to_string());
        }
        let set_map = match repeat_map.get(yk("set")).cloned().unwrap_or(Y::Null) {
            Y::Null => YM::new(),
            Y::Mapping(m) => m,
            _ => return Err("repeat.set 需 mapping".to_string()),
        };
        check_params(&set_map, "repeat.set")?;
        let on_flags = flags_from(&set_map, "repeat.set")?;
        let on_ov = override_from(&set_map)?;
        // emit.py:107 off = 固定零参集 {rtt_ms:0, jitter_ms:0, loss:"0%"}——恢复步 after 串
        // 含 rtt=0ms + loss=0%（judge 恢复判据的生成源）。
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
        Some(n) => n.max(0.0) as u64, // python int() 截断形
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
        Some(Y::Tagged(_)) => true, // py yaml tag 形超 emit 面：非空对象 truthy 缺省
    };
    Ok(ScenarioPlan {
        baseline_s,
        lines,
        total_s: total.max(0.0) as u64,
        clear,
    })
}

/// emit.py scenario 文法平移版（源 = scripts/weaknet.d/emit.py:81-115；白名单外键即拒，
/// 报错含合法集全清单——fail-closed 与 emit.py:47-49 同口径）。文法：
///
/// ```yaml
/// scenario:            # 顶层键可省（裸形直读，emit.py:81 `doc.get('scenario') or doc` 同构）
///   baseline_s: 25     # no-fault twin 窗（缺省=按 steps/repeat 时长自动）
///   clear: true        # 结束清场（false/0/"" = 保留；python truthiness 语义）
///   steps:             # at_s 严格递增（相对 baseline 后 t0）；set 值域 = 白名单
///     - at_s: 30
///       set: {rtt_ms: 100, jitter_ms: 20, loss: "10%"}  # rtt_ms/jitter_ms/rate_mbps/reorder_pct/seed/loss/gemodel
///     - at_s: 75
///       set: {rtt_ms: 0, loss: "0%"}   # 恢复步：judge 判据 = set.after 前缀 rtt=0ms ∧ 含 loss=0%
///   repeat:            # 恢复性半判据（rounds>0 且 on_s/off_s>=5）
///     rounds: 4
///     on_s: 30
///     off_s: 30
///     set: {rtt_ms: 80, loss: "5%"}    # off 态固定零参 --rtt 0 --jitter 0 --loss 0%
/// ```
pub fn parse_plan_yaml(src: &str) -> Result<ScenarioPlan, String> {
    let doc: Y = serde_yaml::from_str(src).map_err(|e| format!("YAML 解析失败: {e}"))?;
    parse_plan_doc(&doc)
}

/// 基底 + 步覆盖 → 新 spec（bash do_set：state 基底 + 显式 flag，缺省字段继承）。
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
            return Err("gemodel 需配 loss（p13，step 继承前值——基底也无 loss）".to_string());
        };
        s.loss = Some(LossSpec::GeModel {
            loss,
            r: r.clone(),
            h: h.clone(),
            k: k.clone(),
        });
    } else if let Some(p) = &ov.loss {
        s.loss = Some(LossSpec::Simple(p.clone()));
    }
    s.validate()?;
    Ok(s)
}

/// 首步参数（bash L566-568：首个 STEP flags，无则首个 REPEAT on_flags——steps 在前）。
fn first_override(plan: &ScenarioPlan) -> Option<&StepOverride> {
    plan.lines.first().map(|l| match l {
        PlanLine::Step { ov, .. } => ov,
        PlanLine::Repeat { on_ov, .. } => on_ov,
    })
}

/// 预演全计划（加载期拦非法值——bash 逐步到 set 才炸；提前 fail-closed 防半程施加，
/// 偏离登记 T8 报告）。返回首步 spec（apply 打底形）。
fn validate_full_plan(plan: &ScenarioPlan) -> Result<ImpairSpec, String> {
    let ov = first_override(plan).ok_or_else(|| "scenario 无首步参数".to_string())?;
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

// ---------- StepExec 抽象（生产=engine 门面；测试=记录形 Noop，零 tc 触达） ----------

/// 剧本物理动作门面（runner 负责锁/旗标/时间线；impl 只做施加+事件，调用方持锁时同持）。
pub trait StepExec: Send + Sync {
    /// apply 形首步（impl 写 state + apply 事件）
    fn apply(&self, req: &ApplyRequest, duration_secs: u64) -> Wn<()>;
    /// set 形步（全量重放；state 保留 prior.job——done 进度经此流入落盘）
    fn set(&self, req: &ApplyRequest, prior: State) -> Wn<()>;
    /// 现场清除（免锁可达语义由 runner 保证；impl 幂等）
    fn clear(&self) -> Wn<()>;
}

/// 生产门面：engine::replay(Verify::Inline) + engine::clear（bash do_apply/do_set 同构）。
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
            Replay::Set {
                prior: Box::new(prior),
                before,
            },
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

// ---------- job 旗标层 ----------

pub fn cancel_path(dirs: &Dirs) -> PathBuf {
    dirs.statedir.join(CANCEL_FILE)
}

fn job_alive(j: &JobRef) -> bool {
    fuse::read_proc_starttime(j.pid) == Some(j.starttime)
}

/// 外部写（CLI/REST apply/set；含第二个 scenario run 互斥）独占门：属主存活 = 409/exit3。
pub fn job_gate(dirs: &Dirs) -> Wn<()> {
    let st = State::read_from(&dirs.state_json()).map_err(Fail::env)?;
    if let Some(j) = st.and_then(|s| s.job).filter(job_alive) {
        return Err(Fail::conflict(format!(
            "scenario 进行中（先 stop）：job {} pid {}",
            j.name, j.pid
        )));
    }
    Ok(())
}

/// runner 检查点：cancel 旗标 / 自有 job 被摘、易主（跨进程 stop 形）。
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
        Err(_) => None, // state 非法/并发写窗——下一步再判（C15：不静默中止，但留给重读）
    }
}

fn remove_cancel(dirs: &Dirs) {
    if let Err(e) = std::fs::remove_file(cancel_path(dirs))
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("weaknet(scenario): WARN 清 stop 旗标失败: {e}");
    }
}

/// 场景收尾兜底：state 若仍存且 job 属主=我方，则摘旗（serve 长驻进程防自锁——keep 形 /
/// clear 失败路径下现场保留但独占必须解除；随现场 clear 掉的 job 自然无须处理=无 op）。
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
        eprintln!("weaknet(scenario): WARN 收尾摘 job 旗取锁失败（属主亡后判活兜底仍可放行）");
        return;
    };
    if let Ok(Some(mut fresh)) = State::read_from(&dirs.state_json())
        && fresh.job.as_ref().is_some_and(|j| j.name == name && j.pid == std::process::id())
    {
        fresh.job = None;
        if let Err(e) = fresh.write_to(&dirs.state_json()) {
            eprintln!("weaknet(scenario): WARN 收尾摘 job 旗写回失败: {e}");
        }
    }
}

/// timeline 中止标记面：全部带 aborted 的 scenario-end 原因串。消费合同：非空 ⇒ 该轮
/// judge 判据作废（rev-2.2；W6/T10 门禁断言空集后才许采信 verdict）。
pub fn aborted_in_timeline(dirs: &Dirs) -> Wn<Vec<String>> {
    let raw = match std::fs::read_to_string(dirs.timeline()) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Fail::env(format!("读 timeline 失败: {e}"))),
    };
    Ok(raw
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e.get("ev").and_then(serde_json::Value::as_str) == Some("scenario-end"))
        .filter_map(|e| {
            e.get("aborted")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

/// `scenario stop` / POST /v1/scenario/stop（无锁可达，恒 exit0/200 语义）。
///
/// 属主存活 → 写 cancel 旗标（runner ≤1s 内见点自终，scenario-end 由 runner 补）；
/// 属主亡/无 job → 幂等零事件（陈旧旗标自清；deviation：崩溃现场不伪造 scenario-end 时刻）。
pub fn stop(dirs: &Dirs) -> Wn<String> {
    let path = dirs.state_json();
    let st = State::read_from(&path).map_err(Fail::env)?;
    if st.as_ref().and_then(|s| s.job.clone()).is_some_and(|j| job_alive(&j)) {
        std::fs::write(cancel_path(dirs), "user\n")
            .map_err(|e| Fail::env(format!("写 stop 旗标失败: {e}")))?;
        return Ok("已请求停止（属主存活，最迟下一检查点生效）".into());
    }
    if let Some(j) = st.and_then(|s| s.job) {
        // 陈旧属主：清旗标放行（rev-2.2 m1）。锁瞬持防并发写覆盖。
        let _lk = engine::take_write_lock(dirs)?;
        if let Some(mut fresh) = State::read_from(&path).map_err(Fail::env)? {
            fresh.job = None;
            fresh.write_to(&path).map_err(Fail::env)?;
        }
        drop(_lk);
        return Ok(format!("陈旧 job「{}」属主已亡——旗标已清", j.name));
    }
    remove_cancel(dirs);
    Ok("无活跃 scenario job（幂等）".into())
}

// ---------- 调度器 ----------

#[derive(Debug, Clone)]
pub struct RunSpec {
    pub name: String,
    /// scenario-start note 的 `$file` 位（bash=展开后路径；serve inline="inline"）
    pub file_disp: String,
    pub plan: ScenarioPlan,
    pub no_baseline: bool,
    pub keep: bool,
    pub targeting: Targeting,
    pub iface: String,
    pub sig_port: Option<u16>,
    /// 测试注入（生产=None→常量 25s；非 bash 发明，仅为单测可跑）
    pub recovery_dwell_secs: Option<u64>,
}

pub struct RunOutcome {
    pub aborted: Option<String>,
    pub elapsed_s: u64,
    pub summary: String,
    /// CLI 退出码映射：lock-busy→3 / user→0 / step-failed→原码 / 未启动→原码
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
    // bash log() 同形：stderr + "weaknet: " 前缀
    eprintln!("weaknet: {msg}");
}

/// 阻塞相（CLI spawn_blocking / serve 同步响应前）：互斥门 → 预演校验 → 首步 apply →
/// job 旗标落盘 → scenario-start 事件（bash 同序；scenario-start 在 apply 之后 L571）。
pub fn start_blocking(rs: &RunSpec, exec: &Arc<dyn StepExec>, dirs: &Dirs) -> Wn<()> {
    job_gate(dirs)?;
    remove_cancel(dirs);
    validate_full_plan(&rs.plan).map_err(Fail::bad_param)?;
    let ov = first_override(&rs.plan).ok_or_else(|| Fail::bad_param("scenario 无首步参数"))?;
    let spec = merge_step(&ImpairSpec::default(), ov).map_err(Fail::bad_param)?;
    let covering = rs.plan.baseline_s + rs.plan.total_s + COVERING_MARGIN_SECS;
    let req = req_for(spec, rs);
    {
        let _lk = engine::take_write_lock(dirs)?;
        exec.apply(&req, covering)?;
        let mut st = State::read_from(&dirs.state_json())
            .map_err(Fail::env)?
            .ok_or_else(|| Fail::env("scenario: apply 成功但 state 缺失"))?;
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

/// 等到 t0+target（bash while+sleep 1s 轮询）；返回 actual_s（整数秒，bash SECONDS 同口径）。
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

/// 单步（瞬持锁内：读 state 验 job → done 进度入 prior → merge → exec.set（全量重放，
/// job 随 prior 落盘）→ 时间线 note）。返新 cur spec（Ok）供调用方串接。
// 8 参均为瞬持锁内必需上下文（dirs/exec/rs/cur/ov/done/note_ev/note）——拆 struct 反增噪声。
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
        .ok_or(StepErr::Cancelled("user"))?; // state 被外摘 = 跨进程 stop/clear 形
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


/// abort 收口：scenario-end + aborted 字段（judge 判据作废信号，rev-2.2）→ 现场清除（非 keep）→
/// 旗标清理。bash 无 abort 事件形（进程直接 die）——补发是本仓授权合同（tasks T8 契约行）。
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
        log(&format!("abort 后 clear 失败（现场残留，交 watchdog/手工 clear）: {}", e.msg));
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

/// 调度主体（非阻塞相：baseline 窗 → steps/repeat → 恢复驻留 → scenario-end → 收尾 clear）。
/// 前置：[`start_blocking`] 已过。t0 口径 = baseline 之后（bash L577，elapsed 不含 baseline）。
pub async fn drive(
    rs: Arc<RunSpec>,
    exec: Arc<dyn StepExec>,
    dirs: Arc<Dirs>,
    mut cur: ImpairSpec,
) -> RunOutcome {
    let plan = rs.plan.clone();
    if !rs.no_baseline && plan.baseline_s > 0 {
        log(&format!(
            "基线窗 {}s（no-fault twin，保持比=fault/baseline J2>0.95 口径）",
            plan.baseline_s
        ));
        if let Err(r) = sleep_checked(&dirs, &rs.name, plan.baseline_s).await {
            return finish_abort(&dirs, &exec, &rs, 0, r.into(), if r == "user" { 0 } else { 2 }).await;
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
                        return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), r.into(), code).await;
                    }
                };
                let note = format!(
                    "at_s={at_disp} actual_s={got} drift_le_1s={}",
                    if got.saturating_sub(*at_secs) <= 1 { 1 } else { 0 }
                );
                let (dirs2, exec2, rs2, cur2) = (dirs.clone(), exec.clone(), rs.clone(), cur.clone());
                let ov_owned = ov.clone();
                let done_l = done;
                let note_l = note.clone();
                let res = tokio::task::spawn_blocking(move || {
                    step_sync(&dirs2, &exec2, &rs2, cur2, &ov_owned, done_l, "scenario-step", note_l)
                })
                .await;
                match res {
                    Ok(Ok(new_cur)) => cur = new_cur,
                    Ok(Err(StepErr::LockBusy)) => {
                        return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), "lock-busy".into(), 3).await;
                    }
                    Ok(Err(StepErr::Cancelled(r))) => {
                        return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), r.into(), if r == "user" { 0 } else { 2 }).await;
                    }
                    Ok(Err(StepErr::Other(f))) => {
                        return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), format!("step-failed: {}", f.msg), f.code).await;
                    }
                    Err(j) => {
                        return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), format!("step-failed: 任务崩溃 {j}"), 2).await;
                    }
                }
            }
            PlanLine::Repeat { rounds, on_s, off_s, on_ov, off_ov, .. } => {
                log(&format!(
                    "repeat：rounds={rounds} on={on_s}s off={off_s}s（恢复性半判据，CH 4×30s 哲学缩形）"
                ));
                for r in 1..=*rounds {
                    for (phase, secs, ov) in [
                        ("on", *on_s, on_ov),
                        ("off", *off_s, off_ov),
                    ] {
                        let (dirs2, exec2, rs2, cur2) =
                            (dirs.clone(), exec.clone(), rs.clone(), cur.clone());
                        let ov_owned = ov.clone();
                        let note = format!("r={r} phase={phase}");
                        let res = tokio::task::spawn_blocking(move || {
                            step_sync(
                                &dirs2, &exec2, &rs2, cur2, &ov_owned, done, "repeat-round", note,
                            )
                        })
                        .await;
                        cur = match res {
                            Ok(Ok(c)) => c,
                            Ok(Err(StepErr::LockBusy)) => {
                                return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), "lock-busy".into(), 3).await;
                            }
                            Ok(Err(StepErr::Cancelled(r))) => {
                                let code = if r == "user" { 0 } else { 2 };
                                return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), r.into(), code).await;
                            }
                            Ok(Err(StepErr::Other(f))) => {
                                return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), format!("step-failed: {}", f.msg), f.code).await;
                            }
                            Err(j) => {
                                return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), format!("step-failed: 任务崩溃 {j}"), 2).await;
                            }
                        };
                        if let Err(c) = sleep_checked(&dirs, &rs.name, secs).await {
                            let code = if c == "user" { 0 } else { 2 };
                            return finish_abort(&dirs, &exec, &rs, t0.elapsed().as_secs(), c.into(), code).await;
                        }
                    }
                }
            }
        }
    }
    // bash L602 awk 判据：任一 STEP flags 含 "--rtt 0" → 恢复驻留窗
    if plan.lines.iter().any(|l| {
        matches!(l, PlanLine::Step { flags, .. } if flags.contains("--rtt 0"))
    }) {
        let dw = rs.recovery_dwell_secs.unwrap_or(RECOVERY_DWELL_SECS);
        log(&format!("恢复驻留窗 {dw}s（recovery ratio 采样）"));
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
        return finish_abort(&dirs, &exec, &rs, elapsed_s, format!("timeline-failed: {e}"), 2).await;
    }
    let keep = rs.keep || !plan.clear;
    let mut code = 0u32;
    let mut extra = String::new();
    if keep {
        log("scenario 结束：现场保留");
    } else if let Err(e) = exec.clear() {
        log(&format!("收尾 clear 失败: {}", e.msg));
        code = e.code;
        extra = " clear-failed".into();
    }
    remove_cancel(&dirs);
    clear_own_job(&dirs, &rs.name);
    RunOutcome {
        aborted: None,
        elapsed_s,
        summary: format!(
            "scenario 完成: elapsed_s={elapsed_s} steps={} keep={keep}{extra}",
            plan.lines.len(),
        ),
        exit_code: code,
    }
}

/// CLI 全链（start 阻塞相 + drive）；serve 分相直调 [`start_blocking`]+tokio::spawn([`drive`])。
pub async fn run_full(
    rs: RunSpec,
    exec: Arc<dyn StepExec>,
    dirs: Dirs,
) -> RunOutcome {
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
                summary: format!("启动任务崩溃: {j}"),
                exit_code: 2,
            }
        }
    };
    if let Err(e) = sr {
        return RunOutcome {
            aborted: None,
            elapsed_s: 0,
            summary: format!("启动失败: {}", e.msg),
            exit_code: e.code,
        };
    }
    drive(rs, exec, dirs, cur).await
}

/// start 后重读当前已安装 spec（drive 串接基座；以磁盘现场为准，防 start/驱动间外部变更漏判）。
fn read_start_spec(dirs: &Dirs) -> ImpairSpec {
    State::read_from(&dirs.state_json())
        .ok()
        .flatten()
        .map(|s| s.spec)
        .unwrap_or_default()
}


// ---------- 文件寻径（CLI/serve 两入口；bash L551 展开规则同构） ----------

/// serve：basename-only → scenarios/<名>.yaml + canonicalize 断言（rev-2.2 m-batch；
/// 面板传 stem，.yaml 自动补全）。
pub fn resolve_serve_file(basename: &str) -> Wn<(std::path::PathBuf, String)> {
    let root = crate::scope::weaknet_d_root().ok_or_else(|| {
        Fail::env("无 scenario 资产目录（WEAKNET_ASSETS_DIR / 二进制同级 weaknet.d / scripts/weaknet.d）")
    })?;
    let sdir = root.join("scenarios");
    let fname = if basename.ends_with(".yaml") || basename.ends_with(".yml") {
        basename.to_string()
    } else {
        format!("{basename}.yaml")
    };
    let rp = sdir.join(&fname).canonicalize().map_err(|e| {
        Fail::bad_param(format!("无 scenario: {fname}（{}：{e}）", sdir.display()))
    })?;
    let rd = sdir
        .canonicalize()
        .map_err(|e| Fail::env(format!("scenario 目录不可达 {}: {e}", sdir.display())))?;
    if !rp.starts_with(&rd) {
        return Err(Fail::bad_param("scenario 路径越界（canonicalize 后不在 scenarios/ 内）"));
    }
    let stem = rp
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scenario")
        .to_string();
    Ok((rp, stem))
}

/// CLI：bash 展开形（*.yaml 直用；否则 scenarios 根/<名>.yaml）；--dir 覆写根。
pub fn resolve_run_file(file: &str, dir_override: Option<&str>) -> Wn<(std::path::PathBuf, String)> {
    let path = if file.ends_with(".yaml") || file.ends_with(".yml") {
        std::path::PathBuf::from(file)
    } else {
        let root = match dir_override {
            Some(d) => std::path::PathBuf::from(d),
            None => crate::scope::weaknet_d_root()
                .ok_or_else(|| {
                    Fail::env("无 scenario 资产目录（--dir 指定，或 WEAKNET_ASSETS_DIR/weaknet.d 就位）")
                })?
                .join("scenarios"),
        };
        root.join(format!("{file}.yaml"))
    };
    if !path.is_file() {
        return Err(Fail::bad_param(format!("无 scenario: {}", path.display())));
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scenario")
        .to_string();
    Ok((path, stem))
}
