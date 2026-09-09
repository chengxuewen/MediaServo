//! state.rs —— state.json（JSON 单写者，含 teardown 计划）+ timeline（JSONL=judge.py 契约）+
//! 路径解析 + 写锁 + param_summary（bash 措辞逐字平移，judge.py:71 与 W7 肌肉记忆依赖）。
//!
//! 设计契约源 = design.md §state.rs：`{spec, scope, dir, iface, expires_at, teardown:{channel,
//! sidecar?:{name,image}, steps[]}, job?, schema}`——teardown 计划 **apply 时落盘且自带通道信息**
//! （watchdog 跨进程苏醒后知道走本地 tc 还是 docker exec，照单执行而非硬编码）。
//!
//! 路径（C20）：`WEAKNET_STATEDIR` env 覆写 > exe 目录祖先探测 `<repo>/out/.weaknet`（与 bash
//! `ROOT/out/.weaknet` 同路径 = T10 窗口期双工具共享 timeline 的兼容判据）> cwd 兜底。
//! 文件名：state.json（本工具唯一事实源；bash 时代的 `state` KEY=VAL 文件与其并存互不读取）、
//! timeline.jsonl（共享追加）、watchdog.pid（fuse 协议 `<pid> <starttime>`）、lock（写锁见 [`acquire_write_lock`]）。

use serde::{Deserialize, Serialize};

use crate::spec::{Dir, ImpairSpec, ScopeSel};

pub const STATE_SCHEMA: &str = "weaknet-state/1";
pub const STATE_FILE: &str = "state.json";
pub const TIMELINE_FILE: &str = "timeline.jsonl";
pub const WATCHDOG_PID_FILE: &str = "watchdog.pid";
pub const LOCK_FILE: &str = "lock";
/// bash 侧默认目录名（ROOT/out/.weaknet）——探测链末端与此对齐。
const DEFAULT_OUT_SUBDIR: &str = "out/.weaknet";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelSer {
    /// serde 形 = design §state.rs：channel "local"|"sidecar" + sidecar{name,image}
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

/// teardown 计划（steps = tc argv 序列，不含 `tc` 前缀；执行语义见 engine::run_teardown——
/// ENOENT 类幂等、其余逐条 WARN 不中断）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Teardown {
    pub channel: ChannelSer,
    #[serde(default)]
    pub sidecar: Option<SidecarRef>,
    pub steps: Vec<Vec<String>>,
}

/// scenario job 属主（T8 写入；本结构先行入 schema——陈旧判活走 /proc starttime，
/// 复用 fuse 同一校验法，design §server「陈旧 job 恢复」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRef {
    pub name: String,
    pub pid: u32,
    pub starttime: u64,
    /// T8：进度回显源（写处理器每 step 更新 done；total = plan 行数）。
    /// `#[serde(default)]` 兼容 rev-2.2 首形（无 done/total 的落盘）。
    #[serde(default)]
    pub done: u32,
    #[serde(default)]
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub schema: String,
    pub spec: ImpairSpec,
    /// spec.dir 的顶层镜像（design 字段清单形；单写者恒同步，status/serve 直读）。
    pub dir: Dir,
    pub scope: ScopeSel,
    pub iface: String,
    /// 施加时的端口/配对快照（status 展示与排障用；Media=本地口集，Stream/Device=拍平对端）。
    #[serde(default)]
    pub ports: Vec<u16>,
    /// T5：Stream/Device 施加时解析的有序对 (local, remote)——解析结果入盘，
    /// replay/status 免重复拉 stats；`#[serde(default)]` 向后兼容旧 state（无此键）。
    #[serde(default)]
    pub pairs: Vec<(u16, u16)>,
    /// E2（T8 小账）：定向名字（rooms=--stream 房间集；devices=--device 设备集）。
    /// 面板勾选回显与 apply 事件 stream 字段（E1）的单一真值源。
    #[serde(default)]
    pub rooms: Vec<String>,
    #[serde(default)]
    pub devices: Vec<String>,
    #[serde(default)]
    pub sig_port: Option<u16>,
    pub expires_at_ms: u64,
    /// 本次现场是否由我方建立 root（teardown 只删我方建立的——guard 纪律的持久化）。
    pub created_root: bool,
    /// T9：本会话是否装了 ifb0 镜像链（dir=in/both × 物理口）。回读/verify 观测点路由
    /// （ifb0 root 形 vs iface 1:10）与 teardown 计划共用的单一真值源。
    #[serde(default)]
    pub ifb_used: bool,
    /// T9：ifb0 链路本次是否由我方所建（§ifb owner 合同：别人/系统建的撤除不碰）。
    #[serde(default)]
    pub created_ifb: bool,
    #[serde(default)]
    pub job: Option<JobRef>,
    pub teardown: Teardown,
}

impl State {
    /// `expires_at_ms == 0` = forever 档（--forever 语义位，M0 CLI 未暴露；fuse.is_expired 已兼容）。
    #[must_use]
    pub fn is_forever(&self) -> bool {
        self.expires_at_ms == 0
    }

    /// 原子写（tmp+rename——JSON 单写者 + watchdog 跨进程读半文件防御）。
    pub fn write_to(&self, path: &std::path::Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("state 路径无父目录: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("建 state 目录 {} 失败: {e}", parent.display()))?;
        let tmp = path.with_file_name(format!(
            "{}.tmp-{}",
            STATE_FILE,
            std::process::id()
        ));
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| format!("state 序列化失败: {e}"))?;
        std::fs::write(&tmp, body).map_err(|e| format!("写 state 临时文件失败: {e}"))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("state rename {} → {} 失败: {e}", tmp.display(), path.display()))
    }

    /// 读 state。不存在 → Ok(None)；schema 失配 / JSON 非法 → Err（禁静默背离，C15）。
    pub fn read_from(path: &std::path::Path) -> Result<Option<State>, String> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("读 state {} 失败: {e}", path.display())),
        };
        let st: State = serde_json::from_str(&raw)
            .map_err(|e| format!("state {} 非法: {e}", path.display()))?;
        if st.schema != STATE_SCHEMA {
            return Err(format!(
                "state schema 失配: {} != {STATE_SCHEMA}（版本漂移？先 clear 复位）",
                st.schema
            ));
        }
        Ok(Some(st))
    }
}

// ---------- 路径解析 ----------

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

    /// 纯解析（env / exe_dir / cwd 全部注入——测试零环境触碰）：
    /// `WEAKNET_STATEDIR` > exe_dir 祖先中最近的仓库根（含 Cargo.toml+crates/）下
    /// `out/.weaknet`（= bash `ROOT/out/.weaknet` 同路径）> 已存在的 `out/.weaknet` 祖先 >
    /// cwd/`out/.weaknet` 兜底。
    #[must_use]
    pub fn resolve(
        env_val: Option<&str>,
        exe_dir: &std::path::Path,
        cwd: &std::path::Path,
    ) -> Self {
        if let Some(v) = env_val.filter(|s| !s.is_empty()) {
            return Self {
                statedir: std::path::PathBuf::from(v),
            };
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
            if repo_hit.is_none()
                && dir.join("Cargo.toml").is_file()
                && dir.join("crates").is_dir()
            {
                repo_hit = Some(cand);
            }
            probe = dir.parent();
        }
        Self {
            statedir: repo_hit.or(exists_hit).unwrap_or_else(|| cwd.join(DEFAULT_OUT_SUBDIR)),
        }
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

// ---------- 写锁（跨工具窗口注记） ----------

/// 瞬持写锁（apply/set/scenario 全程；clear/status 免锁可达——bash 同规）。
/// 机制 = `<statedir>/lock` 内容 pid+starttime 锁文件（create_new 原子性 + fuse::read_proc_starttime
/// 判活回收）。**与 bash flock(1) 同路径不同机制**：Rust→bash 方向的互斥由「窗口纪律 = bash 写入
/// 次数 0」保证（tasks.md 头注）；bash 持锁期间我方对空 lock 文件保守报忙。
pub fn acquire_write_lock(dirs: &Dirs) -> Result<WriteLock, String> {
    std::fs::create_dir_all(&dirs.statedir)
        .map_err(|e| format!("建 state 目录失败: {e}"))?;
    let path = dirs.lock();
    for attempt in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => {
                use std::io::Write;
                let pid = std::process::id();
                let st = crate::fuse::read_proc_starttime(pid).unwrap_or(0);
                let mut f = f;
                if let Err(e) = writeln!(f, "{pid} {st}") {
                    eprintln!("weaknet(state): 写锁内容失败: {e}");
                }
                return Ok(WriteLock { path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if holder_alive(&path)? {
                    return Err(format!(
                        "另一 weaknet 实例运行中（锁持有；确需接管先等其结束或 clear）: {}",
                        path.display()
                    ));
                }
                // 属主已亡 → 回收陈旧锁重试一次
                std::fs::remove_file(&path)
                    .map_err(|e| format!("回收陈旧锁失败: {e}"))?;
                if attempt == 1 {
                    return Err("锁回收后二次创建仍失败（竞态？）——重试".to_string());
                }
            }
            Err(e) => return Err(format!("打开锁文件失败: {e}")),
        }
    }
    Err("unreachable".to_string())
}

fn holder_alive(path: &std::path::Path) -> Result<bool, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("读锁文件失败: {e}"))?;
    let mut it = raw.split_whitespace();
    let (Some(pid), Some(starttime)) = (it.next(), it.next()) else {
        // 空/畸形 = bash flock 时代遗留（flock 不落内容）——保守视为持有，交人工 clear 裁决
        eprintln!(
            "weaknet(state): 锁文件 {} 无属主信息（bash flock 遗留？畸形）——保守报忙，确认无并发后删除 {}",
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

/// RAII：Drop 即释放（写操作瞬持语义；进程崩溃时锁文件残留由 starttime 判活回收）。
#[derive(Debug)]
pub struct WriteLock {
    path: std::path::PathBuf,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!(
                "weaknet(state): 释放写锁 {} 失败: {e}",
                self.path.display()
            );
        }
    }
}

// ---------- timeline（JSONL，UTC——PIT-181：本机时区取证错位教训） ----------

/// 追加一行事件（自动注入 t_utc；调用方给 `json!({...})` 不带 t_utc 即可）。
/// 事件名白名单（judge 契约 T8 钉）：apply/set/clear/self-heal-clear/watchdog-clear/
/// watchdog-clear-failed/scenario-start/baseline-done/scenario-step/repeat-round/scenario-end。
pub fn timeline_append(
    dirs: &Dirs,
    ev: serde_json::Value,
) -> Result<(), String> {
    use std::io::Write;
    std::fs::create_dir_all(&dirs.statedir)
        .map_err(|e| format!("建 timeline 目录失败: {e}"))?;
    let mut obj = match ev {
        serde_json::Value::Object(m) => m,
        _ => return Err("timeline 事件必须是对象".to_string()),
    };
    if !obj.contains_key("t_utc") {
        obj.insert(
            "t_utc".to_string(),
            serde_json::Value::String(utc_now_string()),
        );
    }
    let line = serde_json::Value::Object(obj).to_string();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dirs.timeline())
        .map_err(|e| format!("开 timeline 失败: {e}"))?;
    writeln!(f, "{line}").map_err(|e| format!("写 timeline 失败: {e}"))
}

/// bash `utc()` 同形：`%Y-%m-%dT%H:%M:%SZ`（epoch → 日历日 = Hinnant civil_from_days，零依赖）。
#[must_use]
pub fn utc_now_string() -> String {
    let secs = crate::fuse::now_epoch_ms() / 1000;
    format_utc_secs(secs)
}

#[must_use]
fn format_utc_secs(epoch: u64) -> String {
    let (y, m, d) = civil_from_days((epoch / 86400) as i64);
    let tod = epoch % 86400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Howard Hinnant civil_from_days（z = 自 1970-01-01 的天数）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe =
        (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------- param_summary（bash 逐字平移） ----------

/// bash `param_summary()` 精确形：
/// `rtt=80ms jitter=15ms loss=2%/uniform rate=sentinel reorder=0 seed=off`
/// （LOSS_MODE 恒非空 → `/<mode>` 恒在；rate 未设=sentinel；reorder 未设=0；seed 未设=off。
/// judge.py 以 `rtt=0ms` 前缀 + `loss=0%` 子串识别恢复步——措辞即契约，禁改。）
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

/// apply 事件体（bash state_write printf 形：rtt_ms/jitter_ms 数字，loss/rate_mbps/seed/stream 字符串）。
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
        // E1：bash STREAM_SEL 同键名——Stream/Device 定向时落名字串（段级=空串同旧形）。
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
                steps: vec![vec!["qdisc".into(), "del".into(), "dev".into(), "lo".into(), "root".into()]],
            },
        }
    }

    #[test]
    fn state_json_roundtrip_and_schema_guard() {
        let st = sample_state();
        let json = serde_json::to_string(&st).unwrap();
        let back: State = serde_json::from_str(&json).unwrap();
        assert_eq!(st, back, "roundtrip 逐字段等");
        let dir = std::env::temp_dir().join(format!("weaknet-state-{}", std::process::id()));
        let p = dir.join(STATE_FILE);
        st.write_to(&p).unwrap();
        assert_eq!(State::read_from(&p).unwrap().as_ref(), Some(&st));
        // schema 失配 → Err（禁静默）
        let mut bad = st.clone();
        bad.schema = "weaknet-state/999".into();
        bad.write_to(&p).unwrap();
        assert!(State::read_from(&p).unwrap_err().contains("schema"));
        std::fs::remove_file(&p).ok();
        std::fs::remove_dir_all(&dir).ok();
        // 不存在 = Ok(None)
        assert_eq!(State::read_from(&p).unwrap(), None);
    }

    /// 小刀 C 配套（T5）：旧 state JSON 无 pairs 键 → serde(default) 读为空集，
    /// Media 形现场跨升级可读（禁 schema 断崖）。
    #[test]
    fn state_json_without_pairs_key_still_reads() {
        let st = sample_state();
        let mut json = serde_json::to_value(&st).unwrap();
        json.as_object_mut().unwrap().remove("pairs");
        let back: State = serde_json::from_value(json).unwrap();
        assert_eq!(back.pairs, vec![]);
        assert_eq!(back.ports, st.ports, "其余字段不受影响");
    }

    #[test]
    fn teardown_roundtrip_channel_ser_sidecar_shape() {
        // design §state.rs 合同：teardown.channel 落盘为 "sidecar" 串 + sidecar{name,image} 对象。
        let st = sample_state();
        let json = serde_json::to_string(&st).unwrap();
        assert!(
            json.contains(r#""channel":"sidecar""#),
            "channel 词形必须 sidecar，得 {json}"
        );
        assert!(json.contains(r#""name":"weaknet-ctrl""#));
        assert!(json.contains(r#""image":"gaiadocker/iproute2""#));
        let local = Teardown {
            channel: ChannelSer::LocalRoot,
            sidecar: None,
            steps: vec![vec!["qdisc".into(), "del".into()]],
        };
        let s = serde_json::to_string(&local).unwrap();
        assert!(s.contains(r#""channel":"local""#), "local 形 {s}");
        let back: Teardown = serde_json::from_str(&s).unwrap();
        assert_eq!(back, local);
        // sidecar 形缺省 None 字段可反序（#[serde(default)] 兼容旧落盘）。
        let no_sc = r#"{"channel":"local","steps":[]}"#;
        let t: Teardown = serde_json::from_str(no_sc).unwrap();
        assert_eq!(t.channel, ChannelSer::LocalRoot);
        assert_eq!(t.sidecar, None);
    }

    #[test]
    fn utc_form_matches_bash_date_u_shape() {
        // bash `date -u +%Y-%m-%dT%H:%M:%SZ`——timeline 取证口径（PIT-181）。
        assert_eq!(utc_now_string().len(), 20, "2026-09-08T01:26:49Z");
        assert!(utc_now_string().ends_with('Z'));
    }

    #[test]
    fn param_summary_bash_parity_strings() {
        // 期望串 = bash param_summary 实测输出（逐字符，judge.py:71 消费面）
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
        // 恢复步判据兼容（judge.py: after.startswith('rtt=0ms') && 'loss=0%' in after）
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
        assert_eq!(v["stream"], "cam0,cam1", "E1: 定向名串入 stream 键（bash STREAM_SEL 同位）");
    }

    #[test]
    fn utc_format_known_epochs() {
        assert_eq!(format_utc_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc_secs(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_utc_secs(1_757_292_409), "2025-09-08T00:46:49Z");
        // 闰年边界 2024-02-29
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
        // env 覆写优先
        let d = Dirs::resolve(Some("/tmp/injected-wnet"), &exe, &repo);
        assert_eq!(d.statedir, std::path::PathBuf::from("/tmp/injected-wnet"));
        // 仓库根探测（exe → 祖先 repo）
        let d = Dirs::resolve(None, &exe, std::path::Path::new("/nowhere"));
        assert_eq!(d.statedir, repo.join("out/.weaknet"), "= bash ROOT/out/.weaknet 同形");
        // 兜底 cwd
        let d = Dirs::resolve(None, std::path::Path::new("/nonexistent/deep/path"), &repo);
        assert_eq!(d.statedir, repo.join("out/.weaknet"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_lock_lifecycle_stale_reclaim_and_busy() {
        let dir = std::env::temp_dir().join(format!("weaknet-lock-{}", std::process::id()));
        let dirs = Dirs { statedir: dir.clone() };
        {
            let _lk = acquire_write_lock(&dirs).expect("首次获取必成");
            assert!(dirs.lock().exists());
            // 持锁中二次获取 = 报忙（同进程二次 create_new → 内容 pid=自己 starttime=自己 → alive）
            let e = acquire_write_lock(&dirs).unwrap_err();
            assert!(e.contains("另一 weaknet 实例"), "{e}");
        }
        assert!(!dirs.lock().exists(), "Drop 即释放");
        // 陈旧属主（pid 不存在）→ 回收重试成功
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dirs.lock(), "4294967 12345\n").unwrap();
        let _lk = acquire_write_lock(&dirs).expect("陈旧锁应被回收");
        // 空文件（bash flock 遗留形）→ 保守报忙
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
        timeline_append(&dirs, serde_json::json!({"ev":"clear","t_utc":"2026-01-01T00:00:00Z"})).unwrap();
        let lines: Vec<String> = std::fs::read_to_string(dirs.timeline())
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert!(first["t_utc"].as_str().unwrap().ends_with("Z"), "首行自动 UTC 注入");
        assert_eq!(lines[1], r#"{"ev":"clear","t_utc":"2026-01-01T00:00:00Z"}"#, "已有 t_utc 不覆写，键序保持");
        assert!(timeline_append(&dirs, serde_json::json!("nope")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

}
