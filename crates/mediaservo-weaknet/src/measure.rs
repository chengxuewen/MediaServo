//! measure.rs -- SR6 P3-half-B: the weaknet judge ported to Rust (`mediaservo-weaknet measure`).
//!
//! 1:1 semantic port of the parent-repo `scripts/weaknet.d/judge.py` (deleted in the same round);
//! truth-after-deletion = the frozen goldens in `tests/goldens/` (N3 golden-fixture discipline:
//! frozen (input, expected-stdout, expected-rc) triples replace a second language). Ancestry:
//! judge.py @ parent `git log --follow -- scripts/weaknet.d/judge.py` (last content commit before
//! deletion carries the full python source).
//!
//! Contract (byte-faithful to judge.py):
//! - windows derived from the timeline: baseline = scenario-start..baseline-done (default end =
//!   start+10s); fault = baseline-done..first-zeroing-step (only when > baseline-done); recovery =
//!   first-zeroing-step..scenario-end+1s (only when end > recovery_from)
//! - event whitelist = `scenario*` union {baseline-done, set}; probes filtered to src=="browser"
//!   and [a, b) windows, kbps/fps means over truthy values only
//! - zeroing predicate (judge.py:71): trailing set.after startswith "rtt=0ms" AND contains "loss=0%"
//! - retention/recover ratios; verdict PASS >= tol, WARN >= 0.8, FAIL < 0.8 (FAIL flips rc 0->1)
//! - nan printed as bare `nan`, missing cells printed as nan, verdict `—` when no retention
//! - footer echoes tol with `%.2f`

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::engine::Fail;
use serde_json::Value;

type Wn<T> = Result<T, Fail>;

/// python `datetime.fromisoformat(s.replace('Z', '+00:00'))` -- we accept `Z` or an explicit
/// offset, second granularity is what every producer writes (samplers/scenario runner).
fn ts(s: &str) -> Wn<chrono::DateTime<chrono::Utc>> {
    let owned;
    let normalized = if let Some(stripped) = s.strip_suffix('Z') {
        owned = format!("{stripped}+00:00");
        owned.as_str()
    } else {
        s
    };
    chrono::DateTime::parse_from_rfc3339(normalized)
        .map(|d| d.with_timezone(&chrono::Utc))
        .map_err(|e| Fail::env(format!("bad timestamp `{s}`: {e}")))
}

fn load_jsonl(path: &Path) -> Wn<Vec<Value>> {
    let mut raw = String::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_string(&mut raw))
        .map_err(|e| Fail::env(format!("failed to read {}: {e}", path.display())))?;
    let mut rows = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        rows.push(serde_json::from_str(line)
            .map_err(|e| Fail::env(format!("{}: bad json line: {e}", path.display())))?);
    }
    Ok(rows)
}

/// mean over the truthy (present, non-null, non-zero... python truthiness = non-null AND non-zero
/// for numbers; python `0` is falsy) values of `field` within the [a, b) window.
fn window_avg(
    probe: &[Value],
    a: chrono::DateTime<chrono::Utc>,
    b: chrono::DateTime<chrono::Utc>,
) -> Wn<BTreeMap<String, BTreeMap<&'static str, f64>>> {
    let mut acc: BTreeMap<String, BTreeMap<&'static str, Vec<f64>>> = BTreeMap::new();
    for r in probe {
        if r.get("src").and_then(Value::as_str) != Some("browser") {
            continue;
        }
        let t = match r.get("t_utc").and_then(Value::as_str).map(ts) {
            Some(Ok(t)) => t,
            _ => continue,
        };
        if !(a <= t && t < b) {
            continue;
        }
        let Some(room) = r.get("room").and_then(Value::as_str) else {
            continue;
        };
        for f in ["kbps", "fps"] {
            if let Some(v) = r.get(f).and_then(Value::as_f64) {
                // python truthiness: 0.0 is falsy -> skipped, exactly as judge.py `if r.get(f)`
                if v != 0.0 {
                    acc.entry(room.to_string()).or_default().entry(f).or_default().push(v);
                }
            }
        }
    }
    Ok(acc
        .into_iter()
        .map(|(k, d)| {
            (
                k,
                d.into_iter()
                    .map(|(f, v)| (f, v.iter().sum::<f64>() / v.len() as f64))
                    .collect(),
            )
        })
        .collect())
}

/// First step whose trailing set.after satisfies the zeroing predicate (the recovery moment).
fn recovery_from(tl: &[Value]) -> Wn<Option<chrono::DateTime<chrono::Utc>>> {
    let mut sets: Vec<(chrono::DateTime<chrono::Utc>, Option<&str>)> = Vec::new();
    for e in tl {
        if e.get("ev").and_then(Value::as_str) == Some("set") {
            let t = ts(e.get("t_utc").and_then(Value::as_str).unwrap_or_default())?;
            let after = e.get("after").and_then(Value::as_str);
            sets.push((t, after));
        }
    }
    for e in tl {
        if e.get("ev").and_then(Value::as_str) != Some("scenario-step") {
            continue;
        }
        let t = ts(e.get("t_utc").and_then(Value::as_str).unwrap_or_default())?;
        // judge.py: `next((s.get('after','') for s in reversed(sets<=t) if 'after' in s), '')`
        let mut after = "";
        for (st, a) in sets.iter().rev() {
            if *st <= t
                && let Some(a) = a {
                    after = a;
                    break;
                }
        }
        if after.starts_with("rtt=0ms") && after.contains("loss=0%") {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

fn judge_core(probe: &[Value], tl: &[Value], tol: f64) -> Wn<(String, bool)> {
    let mut start: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut base_done: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut end_ev: Option<Value> = None;
    let mut steps: Vec<Value> = Vec::new();
    for e in tl {
        let ev = e.get("ev").and_then(Value::as_str).unwrap_or("");
        if !(ev.starts_with("scenario") || ev == "baseline-done" || ev == "set") {
            continue;
        }
        match ev {
            "scenario-start" => {
                if start.is_none() {
                    start = Some(ts(e["t_utc"].as_str().unwrap_or_default())?);
                }
            }
            "baseline-done" => {
                if base_done.is_none() {
                    base_done = Some(ts(e["t_utc"].as_str().unwrap_or_default())?);
                }
            }
            "scenario-step" => steps.push(e.clone()),
            "scenario-end"
                if end_ev.is_none() => {
                    end_ev = Some(e.clone());
                }
            _ => {}
        }
    }
    let start = start.ok_or_else(|| Fail::env("timeline has no scenario-start"))?;
    let base_done =
        base_done.unwrap_or(start + chrono::Duration::seconds(10));
    let ends: chrono::DateTime<chrono::Utc> = match &end_ev {
        Some(e) => ts(e["t_utc"].as_str().unwrap_or_default())?,
        None => {
            if let Some(last) = steps.last() {
                ts(last["t_utc"].as_str().unwrap_or_default())?
            } else {
                base_done
            }
        }
    };
    // impairment window: baseline-done -> last step; the first zeroing step ends the fault window.
    let mut fault_end = base_done;
    let rec = recovery_from(tl)?;
    for e in &steps {
        let t = ts(e["t_utc"].as_str().unwrap_or_default())?;
        if let Some(rt) = rec
            && rt == t {
                fault_end = fault_end.max(base_done).min(t);
                continue;
            }
        fault_end = fault_end.max(t);
    }
    let recovery_from = rec.unwrap_or(ends);

    let bw = window_avg(probe, start, base_done)?;
    let fw = if fault_end > base_done {
        window_avg(
            probe,
            fault_end.max(base_done),
            if recovery_from > fault_end { recovery_from } else { ends },
        )?
    } else {
        BTreeMap::new()
    };
    let rw = if ends > recovery_from {
        window_avg(probe, recovery_from, ends + chrono::Duration::seconds(1))?
    } else {
        BTreeMap::new()
    };

    let mut out = String::new();
    out.push_str(&format!(
        "{:<16} {:>9} {:>10} {:>9} {:>10} {:>13}  verdict\n",
        "room", "base_kbps", "fault_kbps", "retention", "recov_kbps", "recover_ratio"
    ));
    let mut ok = true;
    for (room, b) in &bw {
        let b0 = b.get("kbps").copied().unwrap_or(0.0);
        let f0 = fw.get(room.as_str()).and_then(|d| d.get("kbps"));
        let r0 = rw.get(room.as_str()).and_then(|d| d.get("kbps"));
        let ret = match f0 {
            Some(f) if *f != 0.0 && b0 != 0.0 => Some(f / b0),
            _ => None,
        };
        let rec_ratio = match r0 {
            Some(r) if *r != 0.0 && b0 != 0.0 => Some(r / b0),
            _ => None,
        };
        let v = match ret {
            // fault window covering all streams: non-targeted rooms should be >= tol
            Some(r) => {
                let v = if r >= tol {
                    "PASS"
                } else if r >= 0.8 {
                    "WARN"
                } else {
                    "FAIL"
                };
                if v == "FAIL" {
                    ok = false;
                }
                v
            }
            None => "\u{2014}",
        };
        let fmt = |x: Option<f64>, w: usize, dec: usize| match x {
            Some(x) => {
                let body = format!("{:.*}", dec, x);
                format!("{body:>w$}")
            }
            None => format!("{:>w$}", "nan"),
        };
        out.push_str(&format!(
            "{:<16} {:>9.0} {} {} {} {}  {}\n",
            room,
            b0,
            fmt(f0.copied(), 10, 0),
            fmt(ret, 9, 2),
            fmt(r0.copied(), 10, 0),
            fmt(rec_ratio, 13, 2),
            v
        ));
    }
    out.push_str(&format!(
        "# retention \u{5224}\u{636E}\u{53E3}\u{5F84}\u{FF1A}\u{5BF9}\u{7167}\u{6D41}\u{FF08}\u{975E} --stream \u{5B9A}\u{5411}\u{5BF9}\u{8C61}\u{FF09}\u{5E94} \u{2265}{:.2}\u{FF1B}\u{5B9A}\u{5411}\u{6D41}\u{584C}\u{9677}\u{4E3A}\u{9884}\u{671F}\n",
        tol
    ));
    Ok((out, ok))
}

/// CLI face: `measure <probe.jsonl> <timeline.jsonl> [--tol 0.95]`. Exit 0 iff every room OK
/// (python sys.exit(0 if ok else 1); usage/parse failures exit non-zero via the Fail surface).
pub fn measure_cmd(args: &[String]) -> Wn<()> {
    let mut pos: Vec<String> = Vec::new();
    let mut tol = 0.95f64;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(rest) = a.strip_prefix("--tol") {
            let v = if let Some(eq) = rest.strip_prefix('=') {
                eq.to_string()
            } else {
                i += 1;
                args.get(i)
                    .ok_or_else(|| Fail::bad_param("--tol requires a value".to_string()))?
                    .clone()
            };
            tol = v.parse().map_err(|_| {
                Fail::bad_param(format!("--tol: not a number: {v}"))
            })?;
        } else if !a.starts_with("--") {
            pos.push(a.clone());
        }
        i += 1;
    }
    if pos.len() < 2 {
        return Err(Fail::bad_param(
            "usage: measure <probe.jsonl> <timeline.jsonl> [--tol 0.95]".to_string(),
        ));
    }
    let probe = load_jsonl(PathBuf::from(&pos[0]).as_path())?;
    let tl = load_jsonl(PathBuf::from(&pos[1]).as_path())?;
    let (table, ok) = judge_core(&probe, &tl, tol)?;
    print!("{table}");
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

