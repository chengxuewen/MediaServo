//! watch.rs -- `status --watch` ANSI single-screen redraw (`top`-style; design §watch.rs row, T12).
//!
//! Contract: a 1s tick reads state.json + leaf counters (engine::leaf_stats_form, via the state.teardown
//! rebuilt channel -- zero ensure side effects, same method as the serve state frame) + timeline tail 5; frame content is produced by the pure
//! [`render_frame`] (the snapshot-test surface, tests/watch_frame.rs). Non-pure sources (channel description,
//! watchdog liveness, time) are fetched loop-side and injected via `sig_lines`/`now_ms`.
//!
//! Exit semantics: state cleared (down/expiry self-heal) -> final frame + a `cleared` line, exit0; SIGINT = the default
//! action exits directly (**not** triggering clear -- watch is an observation surface; the only damage-control channels are down/the auto-clear flag).

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use crate::engine::{self, Env, Fail, LeafStats, Wn};
use crate::fuse;
use crate::server;
use crate::state::{Dirs, State};

/// Seconds per tick (same cadence as the bash watch; in-frame counter differentials are left to the observer's eye).
const TICK: Duration = Duration::from_secs(1);
const TIMELINE_TAIL: usize = 5;

pub fn run(dirs: &Dirs, env: &Env) -> Wn<()> {
    let mut prev_lines = 0usize;
    loop {
        // lazy-expiry self-heal re-checked every tick (same entry discipline as do_status: watch is a resident status entry).
        engine::autoheal_if_expired(dirs, env)?;
        let st = State::read_from(&dirs.state_json()).map_err(Fail::env)?;
        let (leaf, mut sig) = match &st {
            Some(s) => {
                let mut chan = String::from("teardown channel cannot be rebuilt (corrupt state?)");
                let mut leaf = None;
                if let Some(c) = engine::channel_of(&s.teardown) {
                    chan = c.describe();
                    let (rd_dev, rd_form) = server::readback_target(Some(s), &s.iface);
                    match engine::tc_exec(&c, &["-s", "qdisc", "show", "dev", rd_dev]) {
                        Ok(show) => {
                            leaf = engine::leaf_stats_form(&show, rd_form);
                            if leaf.is_none() {
                                eprintln!(
                                    "weaknet(watch): WARN leaf counts unreadable ({rd_dev} read-back matched no leaf form -- qdisc rewritten externally?)"
                                );
                            }
                        }
                        Err(e) => eprintln!("weaknet(watch): WARN tc read-back failed: {}", e.msg),
                    }
                }
                (
                    leaf,
                    vec![
                        format!("channel={chan} watchdog={}", watchdog_value(dirs)),
                        format!("scope={:?} dir={:?} {}", s.scope, s.dir, scope_detail(s)),
                    ],
                )
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

/// Pure frame rendering (the snapshot-test surface). `sig_lines` = the non-pure sources fetched loop-side (channel/watchdog/scope/
/// timeline tail); `now_ms` injected so the countdown is purely testable -- deviation note: the plan's signature
/// `render_frame(state,leaf,timeline,w,sig_lines)` had no time param, a pure function cannot compute remaining seconds, so a 6th param was added.
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
            out.push_str("weaknet watch -- no active site (inactive)\n");
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
                "leaf sent={sent} dropped={dropped} tiers[{}|{}] {pct}%\n",
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
        return "remaining forever".into();
    }
    if now_ms >= expires_at_ms {
        return "expired (lazy self-heal will clear)".into();
    }
    format!("remaining {}s", (expires_at_ms - now_ms) / 1000)
}

/// state ports/pairs summary (same wording as do_status; the single semantic source is hauled by the caller).
fn scope_detail(s: &State) -> String {
    if s.pairs.is_empty() {
        format!("ports=[{}]", s.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(" "))
    } else {
        format!(
            "pairs=[{}]",
            s.pairs.iter().map(|(l, r)| format!("{l}:{r}")).collect::<Vec<_>>().join(" ")
        )
    }
}

fn timeline_tail(dirs: &Dirs, n: usize) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(dirs.timeline()) else {
        return vec!["timeline unreadable".to_string()];
    };
    let lines: Vec<String> = raw.lines().map(str::to_string).collect();
    lines
        .iter()
        .rev()
        .take(n)
        .rev()
        .cloned()
        .chain(std::iter::once(format!("(timeline: {} lines total)", lines.len())))
        .collect()
}

/// watchdog liveness value (lifted from main.rs -- do_status and --watch share one predicate, no copies).
#[must_use]
pub fn watchdog_value(dirs: &Dirs) -> &'static str {
    match fuse::read_pidfile(&dirs.watchdog_pid()) {
        Ok(None) => "none",
        Ok(Some(e)) => {
            let alive = Path::new(&format!("/proc/{}", e.pid)).exists()
                && fuse::read_proc_starttime(e.pid) == Some(e.starttime);
            if alive { "ok" } else { "dead (only the lazy self-heal layer remains)" }
        }
        Err(e) => {
            eprintln!("weaknet: WARN {e}");
            "protocol violation"
        }
    }
}

fn term_width() -> usize {
    std::env::var("COLUMNS").ok().and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(100)
}

fn truncate_chars(s: &str, w: usize) -> String {
    s.chars().take(w).collect()
}

/// Single-block redraw: non-first frames move the cursor back to the block head, overwrite line by line + erase to EOL (no alt-screen, no fullscreen control).
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
