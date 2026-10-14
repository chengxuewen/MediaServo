# Code Edit Safety

> **Target audience**: AI agents editing MediaServo source code.
> **Violation of these rules causes token waste from repeated fix cycles.**

## Tool Selection

| Change Size | Tool | Reason |
|-------------|------|--------|
| Rewrite entire function/file | `write` | Guarantees brace balance, no stale lines |
| ≤20 line single-location edit | `edit` | Minimal diff, safe for small changes |
| Structural pattern replacement | `ast_grep_replace` | Syntax-aware, preserves matching |
| Complex multi-file refactor | Delegate to subagent | Isolated context, verify independently |

## Forbidden Patterns

| Anti-Pattern | Why |
|--------------|-----|
| `sed` for code modification | Quote escaping errors, regex silent failures |
| Multiple sequential `edit` calls without re-reading | Line numbers drift, stale hash IDs |
| Deleting a line by replacing with empty `lines: []` and assuming brace count is still correct | May leave unbalanced braces |
| Appending `}` to "fix" an unclosed delimiter without counting braces first | Masks root cause, may create double-close |

## Verify Immediately

After EVERY code change (edit, write, or ast_grep_replace):

```
Rust:   cargo check -p <crate>       (5-15s)
TS/TSX: npx tsc --noEmit             (3-5s)
YAML:   docker compose config --quiet (compose files)
Shell:  bash -n <script>             (scripts)
```

### Batch edits arrays must be verified operation-by-operation (PIT-41)

When multiple replace operations reference adjacent regions, boundary line numbers/content easily slip out of place (one operation may overwrite another operation's keep-region). Run the corresponding format verification immediately after each edit call; if breakage is found → re-read the file and restore, do not stack fixes on top.

If verification fails, STOP. Do NOT apply another edit on top. Instead:
1. `git diff` to see what changed
2. If the change is wrong, `git checkout -- <file>` to revert
3. Re-apply the fix correctly

## Brace Safety Checklist

Before marking any multi-line edit complete, verify:
- [ ] Every `{` has a matching `}` at the same indent level
- [ ] Every `(` has a matching `)`
- [ ] Every `[` has a matching `]`
- [ ] No duplicate function definitions or closing braces
- [ ] `cargo check` / `tsc --noEmit` passes

## When to Delegate

Delegate to a `deep` category subagent when:
- The change touches 3+ files
- The change requires understanding cross-module dependencies
- You've failed the same edit 2+ times

The subagent gets a clean context, reads the files fresh, and applies all changes atomically.

## Architectural Decision Gate (NON-NEGOTIABLE)

Before implementing ANY architectural change (protocol, data flow, transport mode, API contract):
- **ALWAYS ask the user first** using the `question` tool with explicit options
- **NEVER fall back to an alternative architecture** without user approval
- **NEVER silently switch** from the agreed architecture (e.g., SFU → P2P) even if it seems "easier"
- **NEVER implement a workaround** that changes the system's design without explicit user consent

If the agreed approach fails, report the failure and ask: "Approach X failed, cause is Y. I suggest switching to Z — do you agree?"

## Test Execution Constraint (NON-NEGOTIABLE)

After claiming tests are written or features are working:
- **ALWAYS run the tests** against the live system. Writing test files without executing them is a violation.
- **ALWAYS report actual test output** — pass/fail counts, error messages. Never claim "tests pass" without evidence.
- **E2E tests MUST run against the actual running service**, not mocked endpoints.
- If tests fail, fix them in the same turn. Do not defer to "later".

## Verification Honesty (NON-NEGOTIABLE)

- **NEVER claim a feature works based on a partial test.** A Python WS test passing does NOT mean the browser flow works.
- **ALWAYS verify at the actual user-facing layer.** If the feature is browser-based, test in the browser. If it's API-based, test with curl.
- **ALWAYS report exactly what was tested and what was NOT tested.** Example: "Python WS test passed. Browser flow NOT yet verified."
- **NEVER present a component test as end-to-end proof.** Each layer must be verified independently.
- **If you cannot verify at the user-facing layer, say so explicitly.** Do not imply success.

## Feature Flag Discipline

- **ALL required features MUST be in `default` features** in Cargo.toml — never require manual `--features` for core functionality
- **Build commands in docs MUST include all features** — never document `cargo build` without required features
- **Before running server, ALWAYS verify**: `cargo build -p mediaservo-server` (with defaults) produces working binary
- **If a feature is optional, it must be explicitly opt-out** (disable with `--no-default-features`)

## Self-Verification Requirement (NON-NEGOTIABLE)

- **ALWAYS verify browser-based features yourself using Playwright MCP tools** (`local-playwright_browser_navigate`, `local-playwright_browser_evaluate`, etc.)
- **NEVER ask the user to test what you can test yourself.** If Playwright is available, use it.
- **After fixing a browser bug, ALWAYS re-test in the browser** before reporting the fix.
- **Report the actual browser console output** as evidence of verification.

## User Confirmation Before Edit (NON-NEGOTIABLE)

- **NEVER start editing files without explicit user approval.** Describing a plan ≠ approval to execute.
- **When user asks 'what can be done' or 'is it possible to...', they are asking a question, not giving an instruction to edit.** Answer the question. Do NOT edit files.
- **Before editing, present the plan AND use the `question` tool to confirm.** Wait for affirmative response before touching files.
- **Silence / 'continue' / timeout ≠ approval.** Only explicit 'yes' / 'do it' / 'proceed' counts.

## Process Management (shell)

- **NEVER use `pgrep -f` / `pkill -f` with a pattern that matches your own shell command line** (e.g. `pgrep -f "mediaservo-host"` from a bash tool whose command string contains that literal) — it kills the shell itself, hanging the tool. Use `pgrep -x <exact-process-name>` (matches process name only, e.g. `mediaservo-host`), or exclude own PID.
- **Killing + relaunching in one shell command** can kill the just-started process (SIGTERM/SIGHUP to the process group on tool timeout). Launch with `setsid nohup ... < /dev/null & disown` and verify with `pgrep -x` in a separate call.
- **Port-in-use on relaunch** (e.g. `Failed to bind 0.0.0.0:9801`) almost always means the old process survived the kill — verify with `ss -tlnp | grep <port>` and kill by PID.
- **Container-recreated services lose apt-installed tools** (gdb etc.) — install debug tools in the Dockerfile dev target, not per-container.

**Source**: PIT-54 debugging round (2026-08-04: pgrep -f self-kill, container recreation lost gdb)

## Network Tooling (bash)

- **curl against local services must use --noproxy**: when the bash environment has `http_proxy`, `curl http://127.0.0.1:5173` goes through the proxy → timeout hang (presents as "Vite not responding"). Use `curl --noproxy "*" http://127.0.0.1:PORT/`.
- **Container tcpdump filters must account for NAT**: packets from the host to the container subnet (172.18.0.2) have their source IP rewritten to the gateway (172.18.0.1) — `not host 172.18.0.1` filters out this machine's browser/app traffic as well. Distinguish by **source port** (Host's fixed port vs browser's random ports), not by source IP.

**Source**: PIT-56/58 debugging round (2026-08-04)

## Git Recovery Operations

- **Bulk `git restore <paths>` recovering staged deletions may leave some directories not actually written back to the worktree** — `git ls-files` (index) shows files but the disk (worktree) is empty, grep in that directory returns nothing. Root cause: `restore` recovers staged-deleted paths incompletely. **Prefer `git checkout HEAD -- <paths>`** (force write-back from HEAD to the worktree).
- **Verification must be a full comparison, never a sample**: after restoring/deleting N directories, check each one with `for d in ...; do echo "[$d] index=$(git ls-files $d/ | wc -l) worktree=$(ls $d/ 2>/dev/null | wc -l)"; done` — index and worktree counts must all be equal. Only `ls`-ing some directories = missed gaps (PIT-68: 10 directories "restored", only 7 actually written back, 3 empty on disk went unnoticed).

**Source**: PIT-68 (2026-08-06 .agents trim recovery round)

### 8. After multi-line replacements with the edit tool, verify line uniqueness (PIT-78a)

**Rule**: After using edit for multi-line replacements in .py/.rs files, if the replacement content contains repeated patterns (identical lines), you must grep to verify uniqueness:

```bash
grep -c "<repeated-pattern>" <file>    # expect 1; >1 = edit inserted duplicates
```

**Precedent**: 2026-08-10 — three edit-tool anomalies in one session: ① replacement dropped preceding lines (main.rs config-path indentation corrupted but syntactically valid, compiled yet carried stale logic); ② duplicate insertion of dispatch lines (mediaservo_cli.py lines 287/288 identical → restart/run-host executed two rounds of container rebuild). **Fix**: ① switch to exact python string replacement (read file → replace → write back); ② delete duplicate lines then verify with grep -c.

**Blocking condition**: committing after a multi-line edit without verifying uniqueness/line counts.

### 9. Before consecutive edits in the same region, grep the current state first (PIT-81 round)

**Rule**: When doing consecutive edits on the same function/region of the same file, before each edit first run `grep -c "<anchor-line content>" <file>` to confirm uniqueness; for the "existing content + insertion" pattern (adding logs / changing signatures before old code), prefer exact python string replacement (read → replace → write back), not the edit tool's lines array hitting repeatedly.

**Precedent**: 2026-08-11 PIT-81 debugging round — edit tool inserted duplicates three times (stop() function signature ×2, main declaration ×2, leftover log lines), each only exposed at build time, wasting 3 rounds. Fix: route everything through python replace (assert count==1).

**Blocking condition**: a 2nd+ consecutive edit on the same function without grep verification; committing with duplicate insertions present but duplicate lines not deleted.

### 10. Python bulk-replacement scripts must write to disk block-by-block or pre-validate everything (PIT-84)

**Rule**: For python scripts doing multi-block replacement (assert → replace → write pattern), **write to disk immediately after each block's replace**, or **run all asserts as a pre-validation pass before any replacement**; forbidden: "replace everything, single write at the end" (any assert failure → total loss, PIT-84 hit twice).

**Verify**: after the script runs, `grep -c "<key replacement content>" <file>` to confirm each block landed; before a rerun after failure, check which blocks were already written.

**Blocking condition**: multi-block script with a single end-of-run write; rerunning directly after an assert failure without confirming intermediate state.
**Blocking condition**: multi-block script with a single end-of-run write; rerunning directly after an assert failure without confirming intermediate state.

### 11. For large markdown appends use heredoc, not the edit tool's JSON (PIT-85 round)

**Rule**: When **appending** new entries to large markdown files like `.agents/memorys/*.md` (containing quotes/backticks/long text), prefer `cat >> file <<'EOF'` heredoc;
**forbidden to use the edit tool for long-content appends** — the edit tool's JSON payload repeatedly fails to parse due to complex quotes/backticks/overlong content
(hit 3 times in that round: "unsupported op undefined"×2 + JSON parse error×1, each wasting a round).

**Verify**: after appending, `grep -c "<key heading>" <file>` to confirm it landed + `wc -l` shows line growth.

**Blocking condition**: using the edit tool to append long markdown/memory content and not switching to heredoc after failure.

### 12. Never run cargo fmt indiscriminately — workspace format drift (PIT-88)

**Rule**: Forbidden: `cargo fmt` / `pixi run cargo fmt` with no arguments or with `-- <path>` (nyi) — arguments after cargo fmt's `--` are **not a path filter**, they format the entire workspace; and this workspace has rustfmt version drift (historical file format vs what the current rustfmt expects, a full fmt produces a 112-file/3000+-line diff). To format a single file use `rustfmt --edition 2024 <file>` (the rustfmt CLI supports single files), or hand-maintain the style.

**Precedent**: 2026-08-12 `pixi run cargo fmt -- crates/mediaservo-host/src/sfu_media.rs` accidentally formatted 112 workspace files (3103 insertions); only after `git checkout -- .` recovery were the functional changes re-applied, wasting 3+ rounds.

**Verify**: after ANY fmt operation, `git diff --stat | wc -l` must == the expected file count (usually 1); `git status --short` shows no unexpected files.

**Blocking condition**: attempting single-file formatting via cargo fmt; not verifying the diff scope after fmt.

### 13. Bulk edit hitting hash mismatch → full re-read before retry, never stitch partial tags from the error output (2026-08-17)

**Rule**: After a bulk edit reports "hash mismatch", **fully re-read the target file first, then retry**; forbidden to directly stitch the partially-updated LINE#IDs from the error output into the second call (unchanged lines still carry stale tags → fails again, wasting 2 rounds). For global config files like `~/.config/opencode/*.jsonc` that opencode may rewrite while running, **always re-read in-place before editing** (tags read early in the session go stale).

**Precedent**: 2026-08-17 omo.jsonc bulk edit, 1st attempt: 8 lines mismatch → retried using tags updated per the error hint, still failed → succeeded only after a full re-read of lines 167-347 (3 rounds vs 2 rounds).

**Verify**: after edit, `python3 -m json.tool <file>` (json) or `grep -c '"reasoningEffort": "low"' .omo/omo.jsonc` (jsonc target field) to confirm it landed.

**Blocking condition**: stitching partial tags from the error output and retrying without a re-read; JSON replacement done without a syntax check.

### 14. Repo-wide renames/bulk edits are a subagent-concurrency forbidden zone (PIT-98)

**Rule**: While subagents are running (background tasks with no completion notification), **forbidden**: repo-wide renames, cross-file bulk replacements, `git checkout/restore` at directory scope. Subagents may: ① have a later commit overwrite/revert the worktree (git checkout "undoing pollution" also washes away the orchestrator's uncommitted changes — PIT-98 precedent: a 10-file rename was wholesale wiped); ② keep coding against stale content producing conflict merges.

**Verify**: after rename/bulk replacement, `grep -rc "<old-pattern>" <scope>` must be 0 + binary-level verification (`readelf --dyn-syms` symbol names); before redoing work, confirm `git log` is static + no background tasks.

**Blocking condition**: executing repo-wide replacement while unfinished subagents exist; committing after a bulk replacement without dual symbol/content verification.

### 15. Bash cleanup pkill/pgrep under set -e — must append || true; timeout needs -k (PIT-120/121 round)

**Rule**: ① In `set -euo pipefail` scripts, **cleanup-style `pkill`/`pgrep` calls must be suffixed with `|| true`** — pkill **exits 1 when no process matches** → set -e **instantly kills the script** (trap cleanup swallows the scene — externally it looks like "hung for 200s+", it is actually a fast failure; PIT-120 precedent: e2e-brand section 4 died before start); ② wrap long commands with **`timeout -k 5 <sec>`** (TERM goes only to the direct child and may be ignored — e.g. oxmgr ignores TERM — without -k the child-process chain never dies); ③ first diagnostic for "hung vs instant-exit" = **`bash -x scripts/x.sh`, read the last lines** (`+ pkill` followed directly by trap cleanup = instant exit; last line is the target command = genuinely hung); ④ cleanup pkills use `-x` with exact names (forbidden: `-f` matching your own command line — see Process Management rule above, hit again this session wasting a 200s timeout round).

**Verify**: `grep -nE "pkill|pgrep" scripts/*.sh` — confirm line by line that cleanup calls have `|| true`; `grep -rn "timeout [0-9]" scripts/` — confirm long-command wrappers carry -k.

**Blocking condition**: pkill/pgrep in scripts without `|| true`; timeout wrapping long commands without -k; declaring "hung" without locating via bash -x and guessing a deadlock instead.

**Source**: PIT-120/121 (2026-08-21 app-branding e2e-brand debugging round)

### 16. pkill/pgrep -f pattern containing your own cmdline literal = self-kill hanging the shell (second occurrence, 2026-09-01)

**Rule**: When cleaning up browsers/child processes, **forbidden**: `pkill -f <string>` when that string appears in the current bash command line (paths like chrome-linux64/chrome very easily land in your own cmdline). Use `ps -eo pid,comm` (exact comm column) + kill by pid, or the grep bracket trick `[c]hrome`.

**Precedent**: PIT-54 (first); 2026-09-01 play round T11 debugging: `pkill -f "ms-playwright/chromium-1234/chrome-linux64/chrome"` (pattern string in own command line) → shell silently hung for two 60s/260s tool windows.
**Blocking condition**: any `pkill -f`/`pgrep -f` pattern string that overlaps as a substring with the current command line.

### 17. Bulk-edit failures / patch no-ops — grep the current state before re-striking; gates must prevent false green (2026-09-03)

**Rule**:
① After the `edit` tool's bulk call reports hash mismatch, it **may have partially applied** — before retrying, grep/re-read the failed region to confirm each op's state, re-apply only the unlanded items; resending the whole batch double-applies (this session's proof: duplicated `await this.connect()` line + tsc syntax error).
② Python bulk patches must have `assert s.count(old)==1` before every block's replace — an anchor miss without an assert is a silent no-op (this session's proof: matrix-script definition block never inserted, only exposed at runtime as ReferenceError).
③ Gate commands must capture the real exit code: `npx tsc --noEmit && echo OK` (or `set -o pipefail`); `cmd | head; echo $?` reads the pipeline tail's exit code = false green (this session twice read syntax errors as tsc=0, burning an entire stress-matrix run).
**Verify**: after patches land, `grep -c "<new-content key string>" <file>` counted per op; after a bulk edit failure, always read the scene first before any retry.
**Blocking condition**: resending a whole failed batch; replace without assert; treating a pipeline-tail exit code as the gate.

### 11. Before starting work in a long session, reconcile with git log -1 — prevent hallucination-round handoffs (2026-09-17 incident log)

**Rule**: In context-exhausted state (after the Nth convergence warning), every round's first action = `git log --oneline -1` + `git status --short` reconciled against the HEAD the previous round claimed; function names/files/commit hashes in the last round's report count as not-happened until verified this round. Functions/features referenced by plans get a double grep before touching them (including verifying existence assertions in your own plan).
**Precedent**: 09-17 hallucination round — an entire round's tool output (build log/findings/commit echoes 0a7f29e/b6697b9) was fabricated; real HEAD stayed at 819b58c; caught by the next round's edit hash defense + grep of the actual state, zero damage (no real file was touched by the hallucination round).
**Verify**: each round's opening reconciliation output must match the previous round's message before continuing; mismatch = rewrite the books from git ground truth.
**Blocking condition**: in an exhausted session, starting any edit/build/commit without reconciling first.
