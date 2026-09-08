//! fuse.rs —— 四层熔断的进程原语层（T2 范围 = 时钟 + pidfile + spawn/kill 原语）。
//!
//! 设计契约源 = docs/plans/weaknet-agent/design.md §fuse（rev-2.2）：
//! - expiry 惰性检查的**判据纯函数**（任意命令入口 + serve 状态帧 tick 均调用同一函数）；
//! - watchdog 独立进程：spawn 用 `CommandExt::process_group(0)`，起后断言 pgid==pid；
//!   pidfile 协议 `"<pid> <starttime>"`（starttime = /proc/\<pid\>/stat 字段 22）；
//!   组杀前校验 starttime，失配=陈旧拒杀；**pgid==自身 pgid 拒杀**；
//!   not-exist / 进程已亡类 = 幂等 Ok（CLI×serve 双 watchdog 并发到点，第二发必踩空——防噪声）。
//!
//! 本模块**不执行 tc / 不碰 state.json 写路径**（engine/state = T3/T4）。
//! state 文件读取仅为 `watchdog_run` 骨架的到期时刻解析（JSON，容忍未知字段）。
//! 零依赖：/proc 全走 std::fs 解析（libc 有意未引入）；路径一律参数化（C20）。

use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// pidfile / state 缺失字段的统一语义：0 = 永不过期（bash `EXPIRES_AT=0 || 空 → return 0`）。
pub const NEVER_EXPIRES: u64 = 0;

/// 休眠分片上限。ponytail: 分片循环防单次 `sleep(huge)` 的 Duration 溢出/挂死，
/// 到 T4 换 tokio 定时器（watchdog_run 已是独立进程，线程 sleep 可接受）。
const SLEEP_CHUNK_MAX: Duration = Duration::from_secs(3600);

/// 过期判据（纯函数，时钟表测）。语义承袭 bash autoheal_check：
/// `expires_at_ms` 为 None / 0（=forever 档，--forever --i-know）→ 永不过期；
/// 边界 `now == expires` **不算过期**（bash `-gt` 严格大于）。
#[must_use]
pub fn is_expired(expires_at_ms: Option<u64>, now_ms: u64) -> bool {
    match expires_at_ms {
        Some(e) if e != NEVER_EXPIRES => now_ms > e,
        _ => false,
    }
}

/// 读 state 文件的到期毫秒。JSON `{... "expires_at_ms": N ...}`，容忍未知字段（state.rs 全量
/// schema 归 T4）。解析根形态容错：对象 或 `{"state": {...}}` 包裹。
/// 缺失字段 / null / 0 → None（永不过期）；文件不可读 / JSON 非法 / 非数字 / 负数 → Err（C15，禁静默）。
pub fn expiry_at_ms(state_path: &Path) -> Result<Option<u64>, String> {
    let raw = std::fs::read_to_string(state_path)
        .map_err(|e| format!("读取 state 失败 {}: {e}", state_path.display()))?;
    let val: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("state JSON 非法 {}: {e}", state_path.display()))?;
    let obj = match &val {
        serde_json::Value::Object(m) => Ok(m),
        other => Err(format!("state 根不是对象: {}", short_type(other))),
    }?;
    let field = obj.get("expires_at_ms").or(obj.get("expires_at"));
    Ok(match field {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => match n.as_u64() {
            Some(u) if u != NEVER_EXPIRES => Some(u),
            Some(_) => None,
            None => {
                let i = n.as_i64().ok_or_else(|| format!("expires_at 数值无法解释: {n}"))?;
                if i < 0 {
                    return Err(format!("expires_at 为负: {i}"));
                }
                None
            }
        },
        Some(other) => return Err(format!("expires_at 非数字: {other}")),
    })
}

fn short_type(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// 当前 epoch 毫秒（生产时钟源；纯函数判据不依赖它，测试注入自己的 now）。
#[must_use]
pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

// ---------- /proc 原语（字段序按 proc(5)；comm 可含空格/括号 → 以最后一个 ')' 为界） ----------

/// 越过 comm 字段后的剩余字段流（从字段 3 = state 起）。
fn stat_fields_after_comm(pid: u32) -> Result<Vec<String>, String> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|e| format!("/proc/{pid}/stat 不可读: {e}"))?;
    let tail = raw
        .rsplit_once(')')
        .ok_or_else(|| format!("/proc/{pid}/stat 缺 comm 闭括号（格式异常）"))?
        .1;
    Ok(tail.split_whitespace().map(str::to_owned).collect())
}

/// /proc/\<pid\>/stat 字段 22 = starttime（jiffies，时钟起算）。进程不存在/格式异常 → None。
#[must_use]
pub fn read_proc_starttime(pid: u32) -> Option<u64> {
    // after_comm[0] = 字段3(state)，故 字段22 = after_comm[19]
    match stat_fields_after_comm(pid) {
        Ok(f) => f.get(19).and_then(|s| s.parse().ok()),
        Err(e) => {
            eprintln!("weaknet(fuse): {e}");
            None
        }
    }
}

/// /proc/\<pid\>/stat 字段 5 = pgid。
#[must_use]
pub fn proc_pgid(pid: u32) -> Option<u64> {
    // after_comm[0] = 字段3(ppid)，故 字段5(pgid) = after_comm[2]
    match stat_fields_after_comm(pid) {
        Ok(f) => f.get(2).and_then(|s| s.parse().ok()),
        Err(e) => {
            eprintln!("weaknet(fuse): {e}");
            None
        }
    }
}

/// 本进程 pid（经 /proc/self 链接解析，零 libc）。
fn current_pid() -> Result<u32, String> {
    let target = std::fs::read_link("/proc/self")
        .map_err(|e| format!("/proc/self 链接不可读: {e}"))?;
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("/proc/self 目标异常: {}", target.display()))?;
    name.parse()
        .map_err(|e| format!("/proc/self pid 解析失败 {name}: {e}"))
}


// ---------- pidfile 协议 "<pid> <starttime>" ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PidEntry {
    pub pid: u32,
    pub starttime: u64,
}

pub fn write_pidfile(path: &Path, entry: PidEntry) -> Result<(), String> {
    std::fs::write(path, format!("{} {}\n", entry.pid, entry.starttime))
        .map_err(|e| format!("写 pidfile {} 失败: {e}", path.display()))
}

/// 读 pidfile。文件不存在 → Ok(None)（幂等）；内容畸形 → Err（协议违例必须可见，C15）。
pub fn read_pidfile(path: &Path) -> Result<Option<PidEntry>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("读 pidfile {} 失败: {e}", path.display())),
    };
    let mut it = raw.split_whitespace();
    let pid = it.next().and_then(|s| s.parse::<u32>().ok());
    let st = it.next().and_then(|s| s.parse::<u64>().ok());
    match (pid, st) {
        (Some(p), Some(s)) => Ok(Some(PidEntry {
            pid: p,
            starttime: s,
        })),
        _ => Err(format!(
            "pidfile {} 协议违例（期望 \"<pid> <starttime>\"）: {:?}",
            path.display(),
            raw.trim()
        )),
    }
}

// ---------- watchdog spawn / kill ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillOutcome {
    /// 组杀命令已下发（TERM 到 -pgid）
    Killed,
    /// 无 pidfile / 目标进程已亡 / ENOENT 类 —— 幂等成功
    Idempotent,
    /// pidfile 存在但 starttime 失配（pid 已被复用）—— 拒杀，陈旧记录已清除
    Stale,
}

/// 自我 re-exec 的 spawn 规格（program 参数化 = 测试注入 /bin/sleep 的通路；
/// 生产由 main.rs 传 `current_exe()` + `["__watchdog", state_path]`）。
#[cfg(unix)]
pub fn spawn_watchdog(
    program: &Path,
    args: &[&str],
    pidfile: &Path,
) -> Result<std::process::Child, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("watchdog spawn {} 失败: {e}", program.display()))?;
    let pid = child.id();
    match proc_pgid(pid) {
        Some(g) if g == u64::from(pid) => {}
        other => {
            let _ = child.kill();
            return Err(format!(
                "watchdog pgid 不变量破坏: pid={pid} pgid={other:?}（期望相等）"
            ));
        }
    }
    let st = match read_proc_starttime(pid) {
        Some(s) => s,
        None => {
            let _ = child.kill();
            return Err(format!("watchdog pid={pid} starttime 读取失败（进程瞬亡？）"));
        }
    };
    write_pidfile(pidfile, PidEntry { pid, starttime: st })?;
    Ok(child)
}

#[cfg(not(unix))]
pub fn spawn_watchdog(
    _program: &Path,
    _args: &[&str],
    _pidfile: &Path,
) -> Result<std::process::Child, String> {
    Err("非 unix 平台无 process_group 原语（运行期报因，禁 compile_error：design §engine 同则）".into())
}

/// 按 pidfile 组杀 watchdog。判序（design §fuse）：
/// 无文件/进程已亡 → Idempotent；starttime 失配 → Stale（清文件拒杀）；
/// 目标 pgid == 自身 pgid → Err 拒杀（自杀防御）；否则 `sh -c "kill -TERM -<pid>"`。
#[cfg(unix)]
pub fn kill_watchdog(pidfile: &Path) -> Result<KillOutcome, String> {
    let entry = match read_pidfile(pidfile)? {
        Some(e) => e,
        None => return Ok(KillOutcome::Idempotent),
    };
    let alive = Path::new(&format!("/proc/{}", entry.pid)).exists();
    if !alive {
        clear_pidfile(pidfile);
        return Ok(KillOutcome::Idempotent);
    }
    match read_proc_starttime(entry.pid) {
        Some(now_st) if now_st == entry.starttime => {}
        Some(now_st) => {
            eprintln!(
                "weaknet(fuse): pidfile 记录 pid={} starttime={}，现值 starttime={} —— pid 已复用，拒杀，清陈旧 pidfile",
                entry.pid, entry.starttime, now_st
            );
            clear_pidfile(pidfile);
            return Ok(KillOutcome::Stale);
        }
        None => {
            clear_pidfile(pidfile);
            return Ok(KillOutcome::Idempotent);
        }
    }
    let own = current_pid()?;
    let target_pgid = proc_pgid(entry.pid).ok_or_else(|| {
        format!("目标 pid={} pgid 读取失败（竞态亡？重跑 clear）", entry.pid)
    })?;
    let own_pgid = proc_pgid(own)
        .map(|g| g as u32)
        .unwrap_or_else(|| {
            eprintln!("weaknet(fuse): 自身 pgid 读取失败，按 own pid {own} 保守比对");
            own
        });
    if target_pgid == u64::from(own_pgid) {
        return Err(format!(
            "拒杀：watchdog 目标组 pgid={target_pgid} == 本进程 pgid={own_pgid}（自组防御）"
        ));
    }
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("kill -TERM -{}", entry.pid))
        .output()
        .map_err(|e| format!("spawn kill 失败: {e}"))?;
    if !out.status.success() {
        // 组在竞态中已散（kill: No such process）= 幂等成功语义；WARN 留痕（C15）
        eprintln!(
            "weaknet(fuse): kill -TERM -{} 非零退出（视为已亡）: {}",
            entry.pid,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    clear_pidfile(pidfile);
    Ok(KillOutcome::Killed)
}

#[cfg(not(unix))]
pub fn kill_watchdog(_pidfile: &Path) -> Result<KillOutcome, String> {
    Err("非 unix 平台无组杀原语（运行期报因）".into())
}

fn clear_pidfile(pidfile: &Path) {
    if let Err(e) = std::fs::remove_file(pidfile)
        && e.kind() != ErrorKind::NotFound
    {
        eprintln!("weaknet(fuse): 清 pidfile {} 失败: {e}", pidfile.display());
    }
}

/// watchdog 执行体骨架：读 state 到期时刻 → 睡到点 → 回调（引擎 teardown 接线归 T3/T4）。
/// 到期时刻缺失/为 None → Err（无期限可守 = 配置错误，禁静默常驻）。
pub fn watchdog_run(state_path: &Path, on_expire: impl Fn(&Path)) -> Result<(), String> {
    let expiry = expiry_at_ms(state_path)?
        .ok_or_else(|| format!("watchdog: state {} 无到期时刻（无期限可守）", state_path.display()))?;
    let start_wall = now_epoch_ms();
    if expiry <= start_wall {
        on_expire(state_path);
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_millis(expiry - start_wall);
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::sleep((deadline - now).min(SLEEP_CHUNK_MAX));
    }
    on_expire(state_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- 时钟表（≥4 案：missing / future / past / boundary==） ----------

    #[test]
    fn clock_table() {
        // missing（None / 0 = forever 档）→ 永不过期
        assert!(!is_expired(None, 1_000));
        assert!(!is_expired(Some(NEVER_EXPIRES), u64::MAX));
        // future → 未过期
        assert!(!is_expired(Some(2_000), 1_000));
        // past → 过期
        assert!(is_expired(Some(1_000), 1_001));
        // boundary now == expires → 不算过期（bash -gt 严格）
        assert!(!is_expired(Some(1_000), 1_000));
    }

    fn tmp_state(name: &str, body: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("weaknet-fuse-{}-{name}", std::process::id()));
        std::fs::write(&p, body).expect("write tmp state");
        p
    }

    #[test]
    fn expiry_reader_missing_field_is_never() {
        let p = tmp_state("miss", r#"{"spec": {"rtt_ms": 80}}"#);
        assert_eq!(expiry_at_ms(&p).unwrap(), None);
        let p0 = tmp_state("zero", r#"{"expires_at_ms": 0, "unknown_top": true}"#);
        assert_eq!(expiry_at_ms(&p0).unwrap(), None);
        let pn = tmp_state("null", r#"{"expires_at_ms": null}"#);
        assert_eq!(expiry_at_ms(&pn).unwrap(), None);
        let pall = tmp_state("alias", r#"{"expires_at": 123, "job": {"pid": 1}}"#);
        assert_eq!(expiry_at_ms(&pall).unwrap(), Some(123));
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(&p0).ok();
        std::fs::remove_file(&pn).ok();
        std::fs::remove_file(&pall).ok();
    }

    #[test]
    fn expiry_reader_errors_not_silent() {
        let bad = tmp_state("bad", "not json");
        assert!(expiry_at_ms(&bad).is_err());
        let wrong_type = tmp_state("wt", r#"{"expires_at_ms": "123"}"#);
        assert!(expiry_at_ms(&wrong_type).is_err());
        let neg = tmp_state("neg", r#"{"expires_at_ms": -5}"#);
        assert!(expiry_at_ms(&neg).is_err());
        let root = tmp_state("root", "[1,2]");
        assert!(expiry_at_ms(&root).is_err());
        assert!(expiry_at_ms(Path::new("/nonexistent/weaknet-test-state")).is_err());
        for f in ["bad", "wt", "neg", "root"] {
            std::fs::remove_file(tmp_state_path(f)).ok();
        }
    }

    fn tmp_state_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("weaknet-fuse-{}-{name}", std::process::id()))
    }

    #[test]
    fn watchdog_run_fires_callback_at_expiry() {
        // 30ms 后到期的小轮：线程跑 watchdog_run，channel 收回调
        let expiry = now_epoch_ms() + 30;
        let p = tmp_state("fire", &format!(r#"{{"expires_at_ms": {expiry}}}"#));
        let (tx, rx) = std::sync::mpsc::channel();
        let path = p.clone();
        std::thread::spawn(move || {
            let _ = watchdog_run(&path, |_| {
                tx.send(()).unwrap();
            });
        });
        rx.recv_timeout(Duration::from_secs(2))
            .expect("watchdog 应在到点后回调");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn watchdog_run_without_expiry_is_err() {
        let p = tmp_state("noexp", r#"{"spec": {}}"#);
        assert!(watchdog_run(&p, |_| unreachable!("不应回调")).is_err());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn pidfile_roundtrip_and_protocol_violation() {
        let mut p = std::env::temp_dir();
        p.push(format!("weaknet-fuse-pid-{}", std::process::id()));
        let missing = read_pidfile(&p).unwrap();
        assert_eq!(missing, None, "不存在 = Ok(None) 幂等");
        write_pidfile(&p, PidEntry { pid: 4242, starttime: 99 }).unwrap();
        assert_eq!(
            read_pidfile(&p).unwrap(),
            Some(PidEntry { pid: 4242, starttime: 99 })
        );
        std::fs::write(&p, "garbage-line").unwrap();
        assert!(read_pidfile(&p).is_err(), "畸形内容必须报协议违例");
        std::fs::remove_file(&p).ok();
    }

    // ---------- /proc 与 spawn/kill（Linux-only） ----------

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_readers_sane_on_self() {
        let pid = current_pid().unwrap();
        assert!(pid > 0);
        let st = read_proc_starttime(pid).expect("自身 starttime 必可读");
        assert!(st > 0, "starttime 非零（jiffies since boot）");
        let g = proc_pgid(pid).expect("自身 pgid 必可读");
        assert!(g > 0);
        assert!(read_proc_starttime(u32::MAX).is_none(), "不存在进程 → None 不 panic");
        assert!(proc_pgid(u32::MAX).is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn spawn_pgid_invariant_then_kill_and_idempotent() {
        let pf = unique_path("pgid-pidfile");
        // 注入 /bin/sleep 30（生产 = current_exe + __watchdog，同一函数路径）
        let mut child = spawn_watchdog(Path::new("/bin/sleep"), &["30"], &pf)
            .expect("spawn /bin/sleep 应成功");
        let pid = child.id();
        // 不变量：pgid == pid（process_group(0) 的效果）
        assert_eq!(proc_pgid(pid), Some(u64::from(pid)));
        let entry = read_pidfile(&pf).unwrap().expect("spawn 后 pidfile 必在场");
        assert_eq!(entry.pid, pid);

        // 组杀（kill_watchdog 的 kill -TERM -<pid> 通路）
        assert_eq!(kill_watchdog(&pf).unwrap(), KillOutcome::Killed);
        let _ = child.wait();
        // 再杀 = 幂等（文件已清 + 进程已亡双保险）
        assert_eq!(kill_watchdog(&pf).unwrap(), KillOutcome::Idempotent);
        std::fs::remove_file(&pf).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_refuses_self_group_and_stale_starttime() {
        // 自组防御：伪造 pidfile = 本进程 pid + 真 starttime，但先把 pgid 对齐——
        // 测试进程自身 pgid==自身（bash 直跑 cargo test 时通常成立）；不成立则只断言非 Killed。
        let pid = current_pid().unwrap();
        let st = read_proc_starttime(pid).unwrap();
        let pf = unique_path("self-pidfile");
        write_pidfile(&pf, PidEntry { pid, starttime: st }).unwrap();
        let r = kill_watchdog(&pf);
        if proc_pgid(pid) == Some(u64::from(pid)) {
            assert!(r.is_err(), "pgid==pid 时必须拒杀: {r:?}");
            assert!(r.unwrap_err().contains("拒杀"));
        } else {
            assert!(!matches!(r, Ok(KillOutcome::Killed)), "绝不允许杀到自身组");
        }
        std::fs::remove_file(&pf).ok();

        // 陈旧 starttime：活 pid + 错 starttime → Stale 拒杀且清文件
        let pf2 = unique_path("stale-pidfile");
        write_pidfile(&pf2, PidEntry { pid, starttime: st ^ 0xFFFF_FFFF }).unwrap();
        assert_eq!(kill_watchdog(&pf2).unwrap(), KillOutcome::Stale);
        assert!(!pf2.exists(), "Stale 路径必须回收陈旧 pidfile");
    }

    fn unique_path(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "weaknet-fuse-{}-{}-{}",
            std::process::id(),
            tag,
            now_epoch_ms()
        ));
        p
    }
}
