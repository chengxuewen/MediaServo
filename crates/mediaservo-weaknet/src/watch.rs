//! watch.rs —— `status --watch` ANSI 一屏重绘（`top` 式；design §watch.rs 行，T12）。
//!
//! 契约：1s tick 读 state.json + leaf 计数（engine::leaf_stats_form，经 state.teardown
//! 重建通道——零 ensure 副作用，与 serve 状态帧同法）+ timeline 尾 5；帧内容由纯函数
//! [`render_frame`] 产出（快照测面对象，tests/watch_frame.rs）。非纯源（通道描述、
//! watchdog 存活、时刻）由循环侧取好后以 `sig_lines`/`now_ms` 注入。
//!
//! 退出语义：state 被清除（down/过期自愈）→ 末帧 + `cleared` 行 exit0；SIGINT = 默认
//! 动作直接退出（**不**触发 clear——watch 是观测面，撤损通道只有 down/auto-clear 旗标）。

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::engine::{self, Env, Fail, LeafStats, Wn};
use crate::fuse;
use crate::server;
use crate::state::{Dirs, State};

/// 每 tick 秒数（bash watch 同节奏；帧内计数差分留给观测者肉眼）。
const TICK: Duration = Duration::from_secs(1);
const TIMELINE_TAIL: usize = 5;

pub fn run(dirs: &Dirs, env: &Env) -> Wn<()> {
    let mut prev_lines = 0usize;
    loop {
        // 惰性过期自愈每 tick 复检（与 do_status 同入口纪律：watch 是常驻 status 入口）。
        engine::autoheal_if_expired(dirs, env)?;
        let st = State::read_from(&dirs.state_json()).map_err(Fail::env)?;
        let (leaf, mut sig) = match &st {
            Some(s) => {
                let mut chan = String::from("teardown 通道不可重建（state 损坏？）");
                let mut leaf = None;
                if let Some(c) = engine::channel_of(&s.teardown) {
                    chan = c.describe();
                    let (rd_dev, rd_form) = server::readback_target(Some(s), &s.iface);
                    match engine::tc_exec(&c, &["-s", "qdisc", "show", "dev", rd_dev]) {
                        Ok(show) => {
                            leaf = engine::leaf_stats_form(&show, rd_form);
                            if leaf.is_none() {
                                eprintln!(
                                    "weaknet(watch): WARN 叶计数不可读（{rd_dev} 回读无匹配叶形——qdisc 被外部改写？）"
                                );
                            }
                        }
                        Err(e) => eprintln!("weaknet(watch): WARN tc 回读失败: {}", e.msg),
                    }
                }
                (leaf, vec![
                    format!("channel={chan} watchdog={}", watchdog_value(dirs)),
                    format!(
                        "scope={:?} dir={:?} {}",
                        s.scope,
                        s.dir,
                        scope_detail(s)
                    ),
                ])
            }
            None => (None, vec![format!("watchdog={}", watchdog_value(dirs))]),
        };
        sig.extend(timeline_tail(dirs, TIMELINE_TAIL));
        let frame = render_frame(st.as_ref(), leaf, &sig, term_width(), fuse::now_epoch_ms());
        emit(&frame, prev_lines);
        prev_lines = frame.lines().count();
        if st.is_none() {
            println!("cleared");
            return Ok(());
        }
        std::thread::sleep(TICK);
    }
}

/// 纯帧渲染（快照测面对象）。`sig_lines` = 循环侧取好的非纯源（通道/watchdog/scope/
/// timeline 尾行），`now_ms` 注入使倒计时可纯测——deviation 注记：票面签名
/// `render_frame(state,leaf,timeline,w,sig_lines)` 无时刻参，纯函数无法算剩余秒，补第 6 参。
#[must_use]
pub fn render_frame(
    st: Option<&State>,
    leaf: Option<LeafStats>,
    sig_lines: &[String],
    w: usize,
    now_ms: u64,
) -> String {
    let w = w.clamp(40, 200);
    let mut out = String::new();
    match st {
        None => {
            out.push_str("weaknet watch — 无活跃现场（inactive）\n");
        }
        Some(s) => {
            out.push_str(&format!(
                "weaknet watch — {} {}\n",
                s.iface,
                countdown_text(s.expires_at_ms, now_ms)
            ));
            out.push_str(&crate::state::param_summary(&s.spec));
            out.push('\n');
            let (sent, dropped) = match leaf {
                Some(l) => (l.sent_pkt, l.dropped),
                None => (0, 0),
            };
            let pct = (dropped * 100).checked_div(sent).unwrap_or(0);
            let bar_w = (w.saturating_sub(34)).clamp(10, 50);
            let filled = (bar_w as u128 * pct as u128 / 100) as usize;
            out.push_str(&format!(
                "leaf sent={sent} dropped={dropped} 阶梯[{}|{}] {pct}%\n",
                "█".repeat(filled),
                "░".repeat(bar_w - filled),
            ));
        }
    }
    for l in sig_lines {
        out.push_str(&truncate_chars(l, w));
        out.push('\n');
    }
    out
}

fn countdown_text(expires_at_ms: u64, now_ms: u64) -> String {
    if expires_at_ms == 0 {
        return "剩余 forever".into();
    }
    if now_ms >= expires_at_ms {
        return "已过期（惰性自愈将清）".into();
    }
    format!("剩余 {}s", (expires_at_ms - now_ms) / 1000)
}

/// state 的 ports/pairs 摘要（do_status 同形措辞，单一语义源在调用方搬运）。
fn scope_detail(s: &State) -> String {
    if s.pairs.is_empty() {
        format!(
            "ports=[{}]",
            s.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" ")
        )
    } else {
        format!(
            "pairs=[{}]",
            s.pairs
                .iter()
                .map(|(l, r)| format!("{l}:{r}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

fn timeline_tail(dirs: &Dirs, n: usize) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(dirs.timeline()) else {
        return vec!["timeline 不可读".to_string()];
    };
    let lines: Vec<String> = raw.lines().map(str::to_string).collect();
    lines
        .iter()
        .rev()
        .take(n)
        .rev()
        .cloned()
        .chain(std::iter::once(format!("(timeline 共 {} 行)", lines.len())))
        .collect()
}

/// watchdog 存活值（自 main.rs 上提——do_status 与 --watch 共用同一判据，禁复制）。
#[must_use]
pub fn watchdog_value(dirs: &Dirs) -> &'static str {
    match fuse::read_pidfile(&dirs.watchdog_pid()) {
        Ok(None) => "none",
        Ok(Some(e)) => {
            let alive = Path::new(&format!("/proc/{}", e.pid)).exists()
                && fuse::read_proc_starttime(e.pid) == Some(e.starttime);
            if alive {
                "ok"
            } else {
                "dead（仅剩惰性自愈层）"
            }
        }
        Err(e) => {
            eprintln!("weaknet: WARN {e}");
            "协议违例"
        }
    }
}

fn term_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.trim().parse::<usize>().ok())
        .unwrap_or(100)
}

fn truncate_chars(s: &str, w: usize) -> String {
    s.chars().take(w).collect()
}

/// 单块重绘：非首帧先游标上移回块首，逐行覆写 + 行尾擦除（无 alt-screen、无全屏控制）。
fn emit(frame: &str, prev_lines: usize) {
    let mut so = std::io::stdout();
    if prev_lines > 0 {
        let _ = write!(so, "\x1b[{prev_lines}A");
    }
    for line in frame.lines() {
        let _ = writeln!(so, "{line}\x1b[K");
    }
    let _ = write!(so, "\x1b[J");
    let _ = so.flush();
}
