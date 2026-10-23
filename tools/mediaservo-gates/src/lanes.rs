//! Crate-lane reconciliation gate — port of `scripts/gate-lanes.py` (S4).
//!
//! THE problem this prevents (verbatim from the python docstring): cmake/
//! MediaServoCrates.cmake carries the lane table (single source of the IDE
//! surface); the workspace member list lives in Cargo.toml. A crate added or
//! removed without a table move silently loses (or invents) a lane — the same
//! drift class as a stale bridge copy.
//!
//! Verdict rules (hard failures, rc 1 — migrated verbatim):
//! 1. Every `crates/*` workspace member appears in the table.
//! 2. Every table row points at an existing `crates/*` member (no ghosts).
//! 3. Each row's lib name is derived from the crate name (dash -> underscore;
//!    a deliberate mismatch must be justified, so it is treated as drift).
//! 4. The table is not empty.
//!
//! Parsing details pinned to the python: the table block is
//! `set(MEDIASERVO_CRATE_LANES ... )` cut at the FIRST `)`; rows are `"..."`
//! quoted tokens split on `@` (exactly 3 parts = crate@lib@folder); members
//! come from the FIRST `[workspace]` line-anchored section up to the next
//! line-anchored `[`, its first `members = [ ... ]` array (non-greedy first
//! `]`), filtered to `crates/`-prefixed paths, crate name = text after the
//! first `/` trimmed, sorted.
//!
//! Faces: plain (rc 0 `OK: crate lanes reconciled (N lanes == M crates/*
//! members)`; rc 1 with one `FAIL: {e}` line per error on stdout);
//! `--self-test` (detector proof: the row of the lexicographically last member
//! is planted-missing in memory — rc 0 iff the reconciliation check catches
//! it; the python `__main__` fixture contract, kept here and unit-tested in
//! the house tempfile style).
//!
//! Path anchor: the python used `__file__`'s parent-parent; this face uses the
//! cwd (`scripts/all-gates.sh` cd's to the repo root first) with `--root`
//! override for fixture tests.

use std::path::Path;

type Row = (String, String);

/// Rules 1-4 as a pure function (fixture-testable; message order = python's).
pub fn reconcile(rows: &[Row], want: &[String]) -> Vec<String> {
    let mut errs = Vec::new();
    if rows.is_empty() {
        return vec!["lane table is EMPTY".to_string()];
    }
    let have: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    for c in want {
        if !have.contains(&c.as_str()) {
            errs.push(format!("member {c} has no lane row"));
        }
    }
    for c in &have {
        if !want.iter().any(|w| w == c) {
            errs.push(format!("lane row {c} is not a crates/* workspace member"));
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for (c, lib) in rows {
        if seen.contains(&c.as_str()) {
            errs.push(format!("duplicate lane row {c}"));
        }
        seen.push(c.as_str());
        let expect = c.replace('-', "_");
        if *lib != expect {
            errs.push(format!(
                "lane row {c}: lib '{lib}' != crate-derived '{expect}' (justify or fix)"
            ));
        }
    }
    errs
}

/// `[(crate, lib)]` from the MEDIASERVO_CRATE_LANES block; Err = the python
/// SystemExit fatal messages.
pub fn table_from_text(text: &str) -> Result<Vec<Row>, String> {
    const MARKER: &str = "set(MEDIASERVO_CRATE_LANES";
    let at = text
        .find(MARKER)
        .ok_or("FAIL: MEDIASERVO_CRATE_LANES block not found in cmake/MediaServoCrates.cmake")?
        + MARKER.len();
    let rest = &text[at..];
    let body = match rest.find(')') {
        Some(end) => &rest[..end],
        None => rest,
    };
    let mut rows = Vec::new();
    for row in quoted_tokens(body) {
        let parts: Vec<&str> = row.split('@').collect();
        if parts.len() != 3 {
            return Err(format!("FAIL: malformed lane row '{row}' (want crate@lib@folder)"));
        }
        rows.push((parts[0].to_string(), parts[1].to_string()));
    }
    Ok(rows)
}

/// Sorted `crates/*` member names from root Cargo.toml text; Err = fatal rules.
pub fn members_from_text(text: &str) -> Result<Vec<String>, String> {
    let block = workspace_section(text).ok_or("FAIL: [workspace] section missing in Cargo.toml")?;
    let body = members_array(block).ok_or("FAIL: workspace.members missing in Cargo.toml")?;
    let mut rels: Vec<String> = Vec::new();
    for rel in quoted_tokens(body) {
        if rel.starts_with("crates/")
            && let Some(c) = rel.split_once('/').map(|x| x.1)
        {
            rels.push(c.trim().to_string());
        }
    }
    rels.sort();
    Ok(rels)
}

/// Face dispatcher: rc table in lib.rs doc.
pub fn check(root: &Path, self_test: bool) -> i32 {
    let cmake = match std::fs::read_to_string(root.join("cmake/MediaServoCrates.cmake")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read cmake/MediaServoCrates.cmake: {e}");
            return 1;
        }
    };
    let cargo = match std::fs::read_to_string(root.join("Cargo.toml")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read Cargo.toml: {e}");
            return 1;
        }
    };
    let rows = match table_from_text(&cmake) {
        Ok(r) => r,
        Err(fatal) => {
            eprintln!("{fatal}");
            return 1;
        }
    };
    let want = match members_from_text(&cargo) {
        Ok(w) => w,
        Err(fatal) => {
            eprintln!("{fatal}");
            return 1;
        }
    };
    if self_test {
        // planted-missing-row detector proof (python --self-test face)
        let Some(last) = want.last() else {
            eprintln!("FAIL(self-test): no crates/* members to plant against");
            return 1;
        };
        let planted: Vec<Row> = rows.into_iter().filter(|r| &r.0 != last).collect();
        let errs = reconcile(&planted, &want);
        return match errs.first() {
            Some(e) => {
                println!("OK(self-test): planted missing row detected ({e})");
                0
            }
            None => {
                println!("FAIL(self-test): planted missing row was NOT detected");
                1
            }
        };
    }
    let errs = reconcile(&rows, &want);
    if errs.is_empty() {
        println!(
            "OK: crate lanes reconciled ({} lanes == {} crates/* members)",
            rows.len(),
            want.len()
        );
        0
    } else {
        for e in errs {
            println!("FAIL: {e}");
        }
        1
    }
}

/// text from the FIRST line-anchored `[workspace]` header to the next
/// line-anchored `[` (python `^\[workspace\](.*?)(^\[|\Z)` re.M|re.S).
fn workspace_section(text: &str) -> Option<&str> {
    for (idx, _) in text.match_indices("[workspace]") {
        if idx == 0 || text.as_bytes()[idx - 1] == b'\n' {
            let rest = &text[idx + "[workspace]".len()..];
            return Some(&rest[..line_start_bracket(rest)]);
        }
    }
    None
}

/// Offset of the first line-anchored `[` in `rest` (else rest end = \Z).
/// The first chunk is the remainder of the header line — python re.M `^`
/// requires a real newline boundary, so that chunk is exempt.
fn line_start_bracket(rest: &str) -> usize {
    let mut pos = 0usize;
    let mut first = true;
    for chunk in rest.split_inclusive('\n') {
        if !first && chunk.starts_with('[') {
            return pos;
        }
        first = false;
        pos += chunk.len();
    }
    rest.len()
}

/// first `members = [ ... ]` array body (python non-greedy first `]`).
fn members_array(block: &str) -> Option<&str> {
    let mut from = 0usize;
    while let Some(rel) = block[from..].find("members") {
        let mut i = from + rel + "members".len();
        let b = block.as_bytes();
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if b.get(i) == Some(&b'[') {
                let rest = &block[i + 1..];
                let end = rest.find(']')?;
                return Some(&rest[..end]);
            }
        }
        from = from + rel + "members".len();
    }
    None
}

/// non-empty `"..."` quoted tokens, resume after each close quote (python
/// findall `"([^"]+)"` semantics).
fn quoted_tokens(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && b[j] != b'"' {
            j += 1;
        }
        if j > i + 1 && j < b.len() {
            out.push(text[i + 1..j].to_string());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO_OK: &str = "[workspace]\nmembers = [\"crates/a-one\", \"crates/b-two\", \"bindings/c/x\"]\nresolver = \"3\"\n[workspace.package]\nversion = \"1\"\n";
    const CMAKE_OK: &str = "# comment with ) paren trap is BEFORE the marker\nset(MEDIASERVO_CRATE_LANES\n \"a-one@a_one@crates/a-one\"\n \"b-two@b_two@crates/b-two\")\n";

    fn fixture_tree(rows_cmake: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cmake")).unwrap();
        std::fs::write(dir.path().join("cmake/MediaServoCrates.cmake"), rows_cmake).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), CARGO_OK).unwrap();
        dir
    }

    fn want() -> Vec<String> {
        members_from_text(CARGO_OK).unwrap()
    }

    #[test]
    fn green_table_reconciles() {
        let rows = table_from_text(CMAKE_OK).unwrap();
        assert!(reconcile(&rows, &want()).is_empty());
        let dir = fixture_tree(CMAKE_OK);
        assert_eq!(check(dir.path(), false), 0);
    }

    #[test]
    fn planted_missing_row_is_detected_by_cli_shape() {
        // house fixture: a table missing a member MUST fail rc=1 (S4 Done-when)
        let cut = CMAKE_OK.replace(" \"b-two@b_two@crates/b-two\"", "");
        let dir = fixture_tree(&cut);
        assert_eq!(check(dir.path(), false), 1);
        assert_eq!(
            reconcile(&table_from_text(&cut).unwrap(), &want()),
            ["member b-two has no lane row"]
        );
    }

    #[test]
    fn ghost_duplicate_and_lib_drift_all_flag() {
        let cmake = "set(MEDIASERVO_CRATE_LANES\n \"a-one@a_one@crates/a-one\"\n \"a-one@a_one@crates/a-one\"\n \"b-two@bTwo@crates/b-two\"\n \"ghost@ghost@crates/ghost\")\n";
        let rows = table_from_text(cmake).unwrap();
        let errs = reconcile(&rows, &want());
        assert!(errs.iter().any(|e| e.contains("duplicate lane row a-one")));
        assert!(
            errs.iter().any(|e| e.starts_with("lane row ghost is not a crates/* workspace member"))
        );
        assert!(errs.iter().any(|e| e.contains("lib 'bTwo' != crate-derived 'b_two'")));
    }

    #[test]
    fn empty_table_and_malformed_and_missing_block_fatal() {
        let empty = fixture_tree("set(MEDIASERVO_CRATE_LANES)\n");
        assert_eq!(check(empty.path(), false), 1);
        assert_eq!(reconcile(&[], &want()), ["lane table is EMPTY"]);
        let mal = table_from_text("set(MEDIASERVO_CRATE_LANES \"a@b\")");
        assert!(mal.unwrap_err().starts_with("FAIL: malformed lane row"));
        let none = table_from_text("nothing here");
        assert_eq!(
            none.unwrap_err(),
            "FAIL: MEDIASERVO_CRATE_LANES block not found in cmake/MediaServoCrates.cmake"
        );
    }

    #[test]
    fn cargo_section_rules() {
        assert_eq!(
            members_from_text("[features]\n[workspace]\nnada\n").unwrap_err(),
            "FAIL: workspace.members missing in Cargo.toml"
        );
        assert_eq!(
            members_from_text("[nawork]\nx\n").unwrap_err(),
            "FAIL: [workspace] section missing in Cargo.toml"
        );
        // members inside [workspace] only; the section stops at the next [
        assert_eq!(want(), ["a-one", "b-two"]);
    }

    #[test]
    fn self_test_face_proves_the_detector() {
        let dir = fixture_tree(CMAKE_OK);
        // healthy tree: planting the last member's row removal MUST be caught
        assert_eq!(check(dir.path(), true), 0);
        // an EMPTY member list (key present, no crates/* rows) leaves nothing
        // to plant = rc 1 (python want[-1] IndexError shape)
        let dir2 = fixture_tree(CMAKE_OK);
        std::fs::write(dir2.path().join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        assert_eq!(check(dir2.path(), true), 1);
    }
}
