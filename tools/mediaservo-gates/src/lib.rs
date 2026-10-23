//! mediaservo-gates — build-gate checker logics (SR6 P3, N4 ruling A′: thin bash
//! drivers + in-workspace Rust face for checkers; xtask-style dev tool).
//!
//! Ported verbatim from the retired python faces (rules migrate, none invented).
//! Each subcommand mirrors its python file's verdict semantics — the exit code
//! and the LIST of flagged items are what callers judge (text may differ
//! cosmetically):
//!
//! | sub        | python prey            | rc 0 (pass)                      | rc 1 (fail)                                        |
//! |------------|------------------------|----------------------------------|----------------------------------------------------|
//! | `changelog`| gate-changelog.py      | stdout `changelog markers ok`    | stderr: one `L{n}: …` line per violation (joined)  |
//! | `config-validate` | cli.py `config validate` | stdout `OK: <rel>` per file | stderr `missing:`/`YAML error:` line per bad file, rc 1 |
//! | `lanes`    | gate-lanes.py          | stdout `OK: crate lanes reconciled (N lanes == M crates/* members)` | stderr `FAIL: <block/member rule>`; stdout `FAIL: {e}` per reconciliation error |
//! | `openapi`  | gate-openapi.py        | stdout `openapi ok: {N} paths`   | stderr: parse/assert message (python: traceback)   |
//! | `parity`   | gate-parity.py         | stdout `bindings parity ok ({v})`| stderr: one `{path} != {v}` line per stale copy    |
//!
//! Path anchors: the python faces read repo-root-relative paths (gate-lanes.py
//! anchored on `__file__`'s parent-parent; the others on the process cwd).
//! The sanctioned caller (`scripts/all-gates.sh`) cd's to the repo root first,
//! so this binary anchors on the cwd too; `--root <dir>` overrides for fixture
//! tests. Special faces: `lanes --self-test` keeps the planted-missing-row
//! detector proof (rc 0 = the planted drift WAS detected).
//!
//! Known inherited vacuity (NOT fixed here — verdict parity with the python
//! face): the changelog section pattern is `## Unreleased\n` (verbatim from
//! gate-changelog.py and ci.yml), while the current CHANGELOG.md heading is
//! `## [Unreleased]` — the gate passes vacuously until either side is ruled.

pub mod changelog;
pub mod config_validate;
pub mod lanes;
pub mod openapi;
pub mod parity;

/// Dispatch `mediaservo-gates <sub> [--root <dir>] [--self-test(lanes)]`.
/// Returns the process exit code (0 pass / 1 fail — table in the module doc).
pub fn run(args: &[String]) -> i32 {
    let Some(sub) = args.first() else {
        eprintln!(
            "usage: mediaservo-gates <changelog|config-validate|lanes|openapi|parity> [--root <dir>]"
        );
        return 1;
    };
    let root = flag_value(&args[1..], "--root").unwrap_or_else(|| ".".to_string());
    let self_test = args[1..].iter().any(|a| a == "--self-test");
    match sub.as_str() {
        "changelog" => changelog::check(std::path::Path::new(&root)),
        "config-validate" => config_validate::check(std::path::Path::new(&root)),
        "lanes" => lanes::check(std::path::Path::new(&root), self_test),
        "openapi" => openapi::check(std::path::Path::new(&root)),
        "parity" => parity::check(std::path::Path::new(&root)),
        other => {
            eprintln!("unknown subcommand: {other}");
            1
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}
