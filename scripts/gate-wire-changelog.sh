#!/usr/bin/env bash
# G3 gate (C50 rule 5 / wire-compat-governance): a wire-shape change in the signaling
# protocol must pair with a CHANGELOG [protocol] entry inside the Unreleased section.
#
# Judge = COMMENT-STRIPPED CODE-PROJECTION diff (team-review F3: the i18n round touched
#   #[serde...] lines only to drop trailing comments — raw-line diff false-reds; strings
#   are preserved because serde tags inside them ARE wire).
# Baseline = upstream branch derived via @{u} — NEVER hardcoded: this repo tracks
#   origin/main while the parent repo uses master (team-review M4/F2: a literal
#   origin/master is a bad revision = permanent silent degrade).
#   No upstream -> degrade to dirty-worktree-only scan (baseline HEAD) + WARN.
# Pairing predicate: [protocol] line ADDED (baseline -> worktree) between
#   "## Unreleased" and the next "^## " heading. Entries outside that section do not count
#   (published package CHANGES.md is frozen — announcements must land in Unreleased).
set -u
cd "$(git rev-parse --show-toplevel)"

UP=$(git rev-parse --abbrev-ref '@{u}' 2>/dev/null || true)
[ -z "$UP" ] && UP=$(git rev-parse --abbrev-ref origin/HEAD 2>/dev/null || true)
if [ -n "$UP" ]; then
    BASE="$UP"
else
    BASE="HEAD"
    echo "WARN: no upstream ref — G3 degrades to dirty-worktree-only scan (baseline HEAD)" >&2
fi

python3 - "$BASE" <<'PYG'
import re, subprocess, sys

base_ref = sys.argv[1]
PROTO = "crates/mediaservo-common/src/protocol.rs"
CHLOG = "CHANGELOG.md"

def git_show(ref, path):
    r = subprocess.run(["git", "show", f"{ref}:{path}"], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None

def read_worktree(path):
    try:
        return open(path, encoding="utf-8").read()
    except OSError:
        return ""

def project(text):
    """Strip // and /* */ comments (string/char literals kept verbatim — serde tags are
    wire), cut the #[cfg(test)] tail, collapse whitespace, drop empty lines."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            while i < n and text[i] != "\n":
                i += 1
        elif c == "/" and i + 1 < n and text[i + 1] == "*":
            i += 2
            while i + 1 < n and not (text[i] == "*" and text[i + 1] == "/"):
                i += 1
            i += 2
        elif c in ('"', "'"):
            q, j = c, i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == q:
                    j += 1
                    break
                j += 1
            out.append(text[i:j])
            i = j
        else:
            out.append(c)
            i += 1
    s = "".join(out)
    m = re.search(r"#\[cfg\(test\)\]", s)
    if m:
        s = s[: m.start()]
    return [re.sub(r"\s+", " ", ln).strip() for ln in s.splitlines() if ln.strip()]

# wire-shape hit vocabulary (pinned regex, C50 rule 1 / PLAN B2)
WIRE = re.compile(
    r"^(pub (struct|enum|const|fn|type|mod)\b"
    r"|#\[serde"
    r"|pub(\([a-z,]+\))?\s"
    r"|[A-Z][A-Za-z0-9]*\s*[,{(]"
    r"|[a-z_0-9]+\s*:\s*\S)"
)

def wire_lines(lines):
    return {l for l in lines if WIRE.match(l)}

def unreleased(text):
    m = re.search(r"^## Unreleased\s*$", text, re.M)
    if not m:
        return ""
    rest = text[m.end():]
    nxt = re.search(r"^## ", rest, re.M)
    return rest[: nxt.start()] if nxt else rest

base_proto = git_show(base_ref, PROTO)
if base_proto is None:
    print("WARN: protocol.rs absent at baseline — comparing against empty", file=sys.stderr)
    base_proto = ""
work_proto = read_worktree(PROTO)

# both directions: additions AND deletions of wire lines are wire changes
delta = (wire_lines(project(work_proto)) - wire_lines(project(base_proto))) | \
        (wire_lines(project(base_proto)) - wire_lines(project(work_proto)))
wire_delta = sorted(delta)

base_ch = git_show(base_ref, CHLOG) or ""
work_ch = read_worktree(CHLOG)
added_tagged = [
    l for l in set(unreleased(work_ch).splitlines()) - set(unreleased(base_ch).splitlines())
    if "[protocol]" in l
]

if wire_delta and not added_tagged:
    print(f"G3 FAIL: {len(wire_delta)} wire-shape line(s) changed in {PROTO} "
          f"(baseline {base_ref} -> worktree) but no CHANGELOG [protocol] entry in Unreleased.")
    for l in wire_delta[:8]:
        print(f"  ~ {l[:120]}")
    print("  C50 rule 1: classify the change — additive needs no bump but still needs the "
          "entry if wire-visible; semantic/removal needs the bump quadruple.")
    sys.exit(1)

detail = f"{len(wire_delta)} wire delta" if wire_delta else "no wire delta"
pair = f"{len(added_tagged)} paired [protocol] entries"
print(f"G3 ok: {PROTO} {detail}; CHANGELOG Unreleased {pair}.")
PYG
