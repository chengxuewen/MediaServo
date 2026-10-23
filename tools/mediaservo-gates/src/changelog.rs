//! F11-X changelog scope-marker gate — port of `scripts/gate-changelog.py`.
//!
//! Verdict rules (migrated verbatim from the python face):
//! 1. Locate the FIRST `## Unreleased\n` heading (substring search, unanchored);
//!    section body = everything after it up to the first `\n## ` (lookahead, not
//!    consumed) or EOF. No heading => nothing is checked (vacuous pass — today's
//!    CHANGELOG.md uses `## [Unreleased]`, see lib.rs "inherited vacuity").
//! 2. Section body lines are numbered from L1 (blank lines count).
//! 3. Only lines starting with `- ` are entries. For each entry:
//!    head = the line up to the FIRST `": "` (colon-space), else the first 80
//!    characters. Tags = every `[a-z0-9-]+` bracket run inside head (later text
//!    is not scanned).
//! 4. No tags in head          -> `L{n}: missing [scope]`
//!    Tag outside vocabulary   -> `L{n}: [{t}] outside allowed vocabulary`
//!    Vocabulary: host server sdk-client sdk-field protocol deploy.
//! 5. Violations: joined with newlines to stderr, rc 1. Clean: stdout
//!    `changelog markers ok`, rc 0 (python: sys.exit(str) prints to stderr).

const ALLOWED: &[&str] = &["host", "server", "sdk-client", "sdk-field", "protocol", "deploy"];

/// Rules 1-4 as a pure function over the CHANGELOG.md text (fixture-testable).
pub fn violations(text: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let Some(body) = unreleased_body(text) else {
        return bad;
    };
    for (n, line) in body.lines().enumerate() {
        if !line.starts_with("- ") {
            continue;
        }
        let head = head_of(line);
        let tags = tags_in(&head);
        let num = n + 1;
        if tags.is_empty() {
            bad.push(format!("L{num}: missing [scope]"));
        }
        for t in tags {
            if !ALLOWED.contains(&t.as_str()) {
                bad.push(format!("L{num}: [{t}] outside allowed vocabulary"));
            }
        }
    }
    bad
}

/// rc table of the python face (module doc in lib.rs).
pub fn check(root: &std::path::Path) -> i32 {
    let text = match std::fs::read_to_string(root.join("CHANGELOG.md")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read CHANGELOG.md: {e}");
            return 1;
        }
    };
    let bad = violations(&text);
    if bad.is_empty() {
        println!("changelog markers ok");
        0
    } else {
        eprintln!("{}", bad.join("\n"));
        1
    }
}

/// Section body per rule 1 (`None` = heading absent = vacuous pass).
fn unreleased_body(text: &str) -> Option<&str> {
    const NEEDLE: &str = "## Unreleased\n";
    let at = text.find(NEEDLE)? + NEEDLE.len();
    let rest = &text[at..];
    Some(match rest.find("\n## ") {
        Some(end) => &rest[..end],
        None => rest,
    })
}

/// head per rule 3: char-based (python slices are codepoints — CJK entries
/// would shift a byte-slice's 80-boundary).
fn head_of(line: &str) -> String {
    let cut = match line.find(": ") {
        Some(byte_idx) => line[..byte_idx].chars().count(),
        None => 80,
    };
    line.chars().take(cut).collect()
}

/// Every `[a-z0-9-]+]` bracket run, non-overlapping, resuming after each match
/// (python findall semantics). ASCII class => byte scan is exact.
fn tags_in(head: &str) -> Vec<String> {
    let b = head.as_bytes();
    let mut tags = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'[' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && (b[j].is_ascii_lowercase() || b[j].is_ascii_digit() || b[j] == b'-') {
            j += 1;
        }
        if j > i + 1 && j < b.len() && b[j] == b']' {
            tags.push(head[i + 1..j].to_string());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(entries: &str) -> String {
        format!("# Changelog\n\n## Unreleased\n{entries}\n## v0.1.0 (x)\n\nold\n")
    }

    #[test]
    fn clean_entries_pass() {
        assert!(
            violations(&section("- [deploy] rewired: stuff\n- [host] + [server] two tags ok"))
                == Vec::<String>::new()
        );
    }

    #[test]
    fn missing_scope_is_flagged() {
        assert_eq!(violations(&section("- no tag here: stuff")), ["L1: missing [scope]"]);
    }

    #[test]
    fn vocabulary_violation_is_flagged_per_tag() {
        assert_eq!(
            violations(&section("- [deploy] [bogus] [host] [nope] mixed")),
            ["L1: [bogus] outside allowed vocabulary", "L1: [nope] outside allowed vocabulary",]
        );
    }

    #[test]
    fn head_stops_at_first_colon_space() {
        // tags AFTER the first ": " are not scanned (python head slice)
        assert!(violations(&section("- [deploy] early: [bogus] later")) == Vec::<String>::new());
    }

    #[test]
    fn line_numbers_count_blank_and_nonentry_lines() {
        assert_eq!(
            violations(&section("- [host] ok\n\nsome prose\n- bad entry")),
            ["L4: missing [scope]"]
        );
    }

    #[test]
    fn bracketed_heading_passes_vacuously_inherited_from_python() {
        // today's CHANGELOG.md shape: `## [Unreleased]` does NOT match the
        // needle `## Unreleased\n` — vacuous pass is the pinned python verdict.
        let text = "## [Unreleased]\n- no tags at all\n\n## v0.1.2\n";
        assert!(violations(text).is_empty());
    }

    #[test]
    fn no_heading_at_all_passes() {
        assert!(violations("## v0.1.0\n- whatever").is_empty());
    }

    #[test]
    fn check_cli_shape_on_tempfile_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CHANGELOG.md"), section("- bad")).unwrap();
        assert_eq!(check(dir.path()), 1);
        std::fs::write(dir.path().join("CHANGELOG.md"), section("- [deploy] fixed")).unwrap();
        assert_eq!(check(dir.path()), 0);
        // missing file = python open() traceback rc — mirrored as rc 1
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(check(empty.path()), 1);
    }
}
