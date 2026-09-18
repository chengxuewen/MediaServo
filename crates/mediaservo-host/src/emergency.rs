//! 紧急停车平面（Task F2）— EmergencyActuator 闩锁 + 强审计。
//!
//! 语义（D-H3/D-H11 落地）：
//! - 急停通道 = 独立进程（host-emergency）+ 独立 PC（controller PC 崩不影响
//!   急停）+ 单 DC label "emergency"（reliable-ordered，D-H3 急停必须可靠有序）
//! - 命令唯一（无参数化控制）: `{"seq": N, "cmd": "stop"}`（[`crate::control`]
//!   信封复用，F1 wire）；`payload` 忽略
//! - `EmergencyActuator` 为概念上的一次性闩锁（latch）: 首次触发 armed，
//!   后续触发不重复 armed 但仍记录（审计）并回执 `latched=false`
//! - 强审计（D-H11 本地侧）: JSONL 追加文件，每触发一行
//!   `{"ts","source","seq","cmd","latched","trigger_count"}`
//!   `source` = `"dc"`（经 DC 的远端急停）| `"local"`（本地兜底，F2 = SIGUSR1）；
//!   "谁"（舱端身份）由 Server 侧会话/授权层承担（Phase G，D-H11 授权矩阵）
//! - 本地兜底接口: 同一 trigger() trait 面；CAN/GPIO 实现 Phase I 后替换
//!   （Stub = 闩锁 + 审计文件，行为即本模块实证）

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// emergency 通道唯一命令（EMERGENCY STOP）。
pub const EMERGENCY_STOP: &str = "stop";

/// 急停触发来源（审计 source 字段 + 回执 source 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmergencySource {
    /// 经 emergency DC 的远端急停（舱端下发）。
    Dc,
    /// 本地兜底触发（F2 = SIGUSR1；网络无关路径，D-H3）。
    Local,
}

impl EmergencySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmergencySource::Dc => "dc",
            EmergencySource::Local => "local",
        }
    }
}

/// 一次触发的结果（回执 result 数据源）。
#[derive(Debug, Clone)]
pub struct EmergencyTrigger {
    /// 本次是否 armed 闩锁（首次 = true，后续 = false）。
    pub latched: bool,
    /// 累计触发次数（审计 + 回执；重复急停也可观测）。
    pub trigger_count: u64,
    /// 审计写入结果（Err 已打日志 — C15：不静默，但急停本身已发生）。
    pub audit: Result<(), String>,
}

/// 急停执行器接口 — 一次性闩锁语义。
/// 实现方必须打日志（C15）；错误信息返回给对端（ACK 语义，非静默）。
pub trait EmergencyActuator: Send + Sync {
    /// 触发急停闩锁。`seq` 为 DC 信封序号（本地触发为 None，审计区分）。
    fn trigger(
        &self,
        source: EmergencySource,
        seq: Option<u64>,
    ) -> Result<EmergencyTrigger, String>;
}

/// Stub 急停执行器（F2 阶段）：闩锁 + JSONL 强审计文件。
/// CAN/GPIO 真实实现在 Phase I 后替换（接口不变；审计留驻）。
pub struct StubEmergencyActuator {
    latch: AtomicBool,
    count: AtomicU64,
    audit: Mutex<File>,
}

impl StubEmergencyActuator {
    /// 打开（创建 + 追加）审计文件；失败 = 启动即退出（强审计不可用不可运行）。
    pub fn new(audit_path: &Path) -> Result<Self, String> {
        let file = File::options()
            .create(true)
            .append(true)
            .open(audit_path)
            .map_err(|e| format!("审计文件打开失败 {}: {e}", audit_path.display()))?;
        Ok(Self {
            latch: AtomicBool::new(false),
            count: AtomicU64::new(0),
            audit: Mutex::new(file),
        })
    }

    fn append_audit(
        &self,
        source: EmergencySource,
        seq: Option<u64>,
        latched: bool,
        trigger_count: u64,
    ) -> Result<(), String> {
        let line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "source": source.as_str(),
            "seq": seq,
            "cmd": EMERGENCY_STOP,
            "latched": latched,
            "trigger_count": trigger_count,
        })
        .to_string()
            + "\n";
        let mut f = self.audit.lock().map_err(|e| format!("审计锁: {e}"))?;
        f.write_all(line.as_bytes()).and_then(|_| f.flush()).map_err(|e| format!("审计追加: {e}"))
    }
}

impl EmergencyActuator for StubEmergencyActuator {
    fn trigger(
        &self,
        source: EmergencySource,
        seq: Option<u64>,
    ) -> Result<EmergencyTrigger, String> {
        let trigger_count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        let latched = !self.latch.swap(true, Ordering::SeqCst);
        tracing::info!(
            source = source.as_str(),
            seq,
            latched,
            trigger_count,
            "EMERGENCY STOP 触发"
        );
        // C15: 审计写入失败必须大声打日志（急停已发生，错误不吞）
        let audit = self.append_audit(source, seq, latched, trigger_count);
        if let Err(e) = &audit {
            tracing::error!(source = source.as_str(), seq, error = %e, "审计写入失败");
        }
        Ok(EmergencyTrigger { latched, trigger_count, audit })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latch_first_trigger_arms_subsequent_reported() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = StubEmergencyActuator::new(f.path()).unwrap();
        let t1 = a.trigger(EmergencySource::Dc, Some(1)).unwrap();
        assert!(t1.latched, "首次触发必须 armed");
        assert_eq!(t1.trigger_count, 1);
        assert!(t1.audit.is_ok());
        let t2 = a.trigger(EmergencySource::Dc, Some(2)).unwrap();
        assert!(!t2.latched, "重复触发不得重复 armed");
        assert_eq!(t2.trigger_count, 2, "重复触发仍计数（审计可观测）");
        let t3 = a.trigger(EmergencySource::Local, None).unwrap();
        assert!(!t3.latched);
        assert_eq!(t3.trigger_count, 3);
    }

    #[test]
    fn audit_line_records_source_seq_latched() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let a = StubEmergencyActuator::new(f.path()).unwrap();
        a.trigger(EmergencySource::Dc, Some(7)).unwrap();
        a.trigger(EmergencySource::Local, None).unwrap();

        let lines: Vec<serde_json::Value> = std::fs::read_to_string(f.path())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        // dc 行: 全字段 + RFC3339 时间戳
        let l0 = &lines[0];
        assert_eq!(l0["source"], "dc");
        assert_eq!(l0["seq"], 7);
        assert_eq!(l0["cmd"], EMERGENCY_STOP);
        assert_eq!(l0["latched"], true);
        assert_eq!(l0["trigger_count"], 1);
        let ts = l0["ts"].as_str().expect("ts 必须是字符串");
        chrono::DateTime::parse_from_rfc3339(ts).expect("ts 必须可解析（RFC3339）");
        // local 行: seq = null（无信封）
        let l1 = &lines[1];
        assert_eq!(l1["source"], "local");
        assert!(l1["seq"].is_null());
        assert_eq!(l1["latched"], false);
        assert_eq!(l1["trigger_count"], 2);
    }

    #[test]
    fn new_rejects_unwritable_audit_path() {
        let dir = tempfile::TempDir::new().unwrap();
        // 目录路径不可追加 → 启动即失败（强审计不可用不可运行）
        assert!(StubEmergencyActuator::new(dir.path()).is_err());
        // 不存在的父目录 → 失败
        assert!(StubEmergencyActuator::new(&dir.path().join("no/such/dir/a.jsonl")).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn audit_write_failure_is_surfaced_not_silent() {
        // /dev/full: 写必 ENOSPC（经典错误注入面）
        let a = StubEmergencyActuator::new(Path::new("/dev/full")).unwrap();
        let t = a.trigger(EmergencySource::Dc, Some(1)).unwrap();
        assert!(t.latched);
        assert!(t.audit.is_err(), "审计失败必须体现在结果中（C15）");
    }
}
