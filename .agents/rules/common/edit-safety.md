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
YAML:   docker compose config --quiet (compose 文件)
Shell:  bash -n <script>             (脚本)
```

### 批量 edits 数组必须逐操作验证 (PIT-41)

多个 replace 操作引用相邻区域时，边界行号/内容易错位（一个操作可能覆盖另一个操作的保留区域）。每次 edit 调用后立即跑对应格式验证；发现破坏 → 重读文件恢复，不叠加修复。

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

If the agreed approach fails, report the failure and ask: "方案 X 失败，原因是 Y。建议改用 Z，是否同意？"

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
- **Silence / '继续' / timeout ≠ approval.** Only explicit 'yes' / 'do it' / '执行' counts.

## Process Management (shell)

- **NEVER use `pgrep -f` / `pkill -f` with a pattern that matches your own shell command line** (e.g. `pgrep -f "mediaservo-host"` from a bash tool whose command string contains that literal) — it kills the shell itself, hanging the tool. Use `pgrep -x <exact-process-name>` (matches process name only, e.g. `mediaservo-host`), or exclude own PID.
- **Killing + relaunching in one shell command** can kill the just-started process (SIGTERM/SIGHUP to the process group on tool timeout). Launch with `setsid nohup ... < /dev/null & disown` and verify with `pgrep -x` in a separate call.
- **Port-in-use on relaunch** (e.g. `Failed to bind 0.0.0.0:9801`) almost always means the old process survived the kill — verify with `ss -tlnp | grep <port>` and kill by PID.
- **Container-recreated services lose apt-installed tools** (gdb etc.) — install debug tools in the Dockerfile dev target, not per-container.

**来源**：PIT-54 调试轮 (2026-08-04: pgrep -f 自杀、容器重建丢 gdb)

## Network Tooling (bash)

- **curl 本机服务必须 --noproxy**：bash 环境有 `http_proxy` 时，`curl http://127.0.0.1:5173` 会走代理 → 超时假死（表现为"Vite 无响应"）。用 `curl --noproxy "*" http://127.0.0.1:PORT/`。
- **容器 tcpdump 过滤注意 NAT**：宿主发往容器网段（172.18.0.2）的包源 IP 被改写为网关（172.18.0.1）——`not host 172.18.0.1` 会把本机浏览器/应用的流量一并过滤掉。按**源端口**区分（Host 的固定端口 vs 浏览器随机端口），不要按源 IP。

**来源**：PIT-56/58 调试轮 (2026-08-04)

## Git 恢复操作

- **批量 `git restore <paths>` 恢复已 staged 删除时，可能部分目录工作区未实际写回**——`git ls-files`（index）有文件但磁盘（worktree）为空，grep 该目录无结果。根因：`restore` 对 staged 删除的路径恢复不完整。**优先用 `git checkout HEAD -- <paths>`**（强制从 HEAD 写回工作区）。
- **验证必须是全量对比，不能抽样**：恢复/删除 N 个目录后，逐个 `for d in ...; do echo "[$d] index=$(git ls-files $d/ | wc -l) worktree=$(ls $d/ 2>/dev/null | wc -l)"; done` 核对，index 与 worktree 计数必须全部相等。只 `ls` 部分目录 = 遗漏（PIT-68：恢复 10 个目录仅 7 个实际写回，3 个磁盘为空未被发现）。

**来源**：PIT-68 (2026-08-06 .agents 精简恢复轮)

### 8. edit 工具多行替换后必须验证行唯一性 (PIT-78a)

**规则**: 对 .py/.rs 文件用 edit 做多行替换后，若替换内容含重复模式（相同行），必须 grep 验证唯一性：

```bash
grep -c "重复模式" <file>    # 期望 1；>1 = edit 重复插入
```

**先例**: 2026-08-10 会话内 edit 工具三次异常——① 替换丢失前几行（main.rs 配置路径缩进损坏但语法合法，编译通过但逻辑旧）；② 重复插入分派行（mediaservo_cli.py 287/288 相同行 → restart/run-host 执行两轮容器重建）。**修复**: ① 改用 python 精确字符串替换（读文件→replace→写回）；② 删除重复行后 grep -c 验证。

**阻塞条件**: 多行 edit 后未验证唯一性/行数即提交。

### 9. 同区域连续 edit 前必须 grep 现状 (PIT-81 轮)

**规则**: 对同一文件同一函数/区域做连续 edit 时，每次 edit 前先 `grep -c "<锚点行内容>" <file>` 确认唯一性；对"已有内容 + 插入"模式（在旧代码前加日志/改签名），优先用 python 精确字符串替换（读→replace→写回），不用 edit 的 lines 数组重复命中。

**先例**: 2026-08-11 PIT-81 调试轮 — edit 工具三次重复插入（stop() 函数签名 ×2、main 声明 ×2、日志行残留），每次 build 才暴露，浪费 3 轮。修复统一走 python replace（assert count==1）。

**阻塞条件**: 同一函数连续第 2 次 edit 前未 grep 验证；已出现重复插入但未删除重复行即提交。

### 10. python 批量替换脚本必须逐块写盘或前置验证 (PIT-84)

**规则**: 多块替换的 python 脚本（assert → replace → write 模式），**每块 replace 后立即写盘**，或**所有 assert 前置验证后再统一替换**；禁止"全部替换后末尾一次写盘"（任一 assert 失败 → 全盘丢失，PIT-84 踩 2 次）。

**验证**: 脚本执行后 `grep -c "<关键替换内容>" <file>` 确认每块生效；失败重跑前检查哪些块已写。

**阻塞条件**: 多块脚本末尾一次性写盘；assert 失败后未确认中间状态直接重跑。
**阻塞条件**: 多块脚本末尾一次性写盘；assert 失败后未确认中间状态直接重跑。

### 11. 大块 markdown 追加用 heredoc，不用 edit 工具 JSON (PIT-85 轮)

**规则**: 对 `.agents/memorys/*.md` 等大块 markdown（含引号/反引号/长中文）**追加**新条目时，优先用 `cat >> file <<'EOF'` heredoc；
**禁止用 edit 工具做长内容 append**——edit 的 JSON 载荷会因复杂引号/反引号/超长内容反复解析失败
（本次踩 3 次："unsupported op undefined"×2 + JSON parse error×1，each 浪费一轮）。

**验证**: 追加后 `grep -c "<关键标题>" <file>` 确认生效 + `wc -l` 行数增长。

**阻塞条件**: 长 markdown/memory 内容用 edit 工具 append 且失败后未改用 heredoc。

### 12. 禁止 cargo fmt 无差别运行 — workspace 格式漂移 (PIT-88)

**规则**: 禁止 `cargo fmt` / `pixi run cargo fmt` 无参数或带 `-- <path>` 运行nyi——cargo fmt 的 `--` 后参数**不是路径过滤**，会格式化整个 workspace；且本工作区存在 rustfmt 版本漂移（历史文件格式与当前 rustfmt 期望不一致，全量 fmt 产生 112 文件/3000+ 行 diff）。需要单文件格式化时用 `rustfmt --edition 2024 <file>`（rustfmt CLI 支持单文件），或手动保持风格。

**先例**: 2026-08-12 `pixi run cargo fmt -- crates/mediaservo-host/src/sfu_media.rs` 误格式化 workspace 112 文件（3103 insertions），`git checkout -- .` 恢复后才重新应用功能改动，浪费 3+ 轮。

**验证**: 任何 fmt 操作后 `git diff --stat | wc -l` 必须 == 预期文件数（通常 1）；`git status --short` 无意外文件。

**阻塞条件**: 试图用 cargo fmt 做单文件格式化；fmt 后未验证 diff 范围。

### 13. 批量 edit 遇 hash mismatch → 完整 re-read 再重试，禁止用错误输出的部分 tags 拼接 (2026-08-17)

**规则**: 批量 edit 报 "hash mismatch" 后，**先完整 re-read 目标文件再重试**；禁止直接用错误输出中更新的部分 LINE#ID 拼接第二次调用（未变化行仍用旧 tag → 再次失败，浪费 2 轮）。对全局配置（`~/.config/opencode/*.jsonc`）等 opencode 运行中可能被改写的文件，**编辑前必须现场 re-read**（会话早段读取的 tags 会失效）。

**先例**: 2026-08-17 omo.jsonc 批量 edit 第 1 次 8 行 mismatch → 用错误提示更新 tags 重试仍失败 → 完整 re-read 167-347 行后才成功（3 轮 vs 2 轮）。

**验证**: edit 后 `python3 -m json.tool <file>`（json）或 `grep -c '"reasoningEffort": "low"' .omo/omo.jsonc`（jsonc 目标字段）确认生效。

**阻塞条件**: 未 re-read 直接拼接错误输出的部分 tags 重试；JSON 替换后未做语法校验。

### 14. 仓级重命名/批量编辑是代理并发禁区 (PIT-98)

**规则**: 有子代理在运行（background task 未收到完成通知）时，**禁止**执行仓级重命名、跨文件批量替换、`git checkout/restore` 目录级操作。子代理可能：① 后续提交覆盖/还原工作区（git checkout 恢复"污染"会连带冲掉编排者的未提交改动——PIT-98 实证 10 文件重命名被整体冲掉）；② 基于旧内容继续编码产生冲突合并。

**验证**: 重命名/批量替换后 `grep -rc "<旧模式>" <范围>` 必须为 0 + 二进制级验证（`readelf --dyn-syms` 符号名）；重做前确认 `git log` 静止 + 无 background 任务。

**阻塞条件**: 有未完成子代理时执行仓级替换；批量替换后未做符号/内容双重验证即提交。

### 15. bash 脚本清理 pkill/pgrep + set -e — 必须 || true；timeout 加 -k (PIT-120/121 轮)

**规则**: ① `set -euo pipefail` 脚本中**清理型 `pkill`/`pgrep` 必须后缀 `|| true`**——pkill **无匹配进程时 exit 1** → set -e **秒退脚本**（trap 清理吞掉现场——外界观察像"卡死 200s+"，实为快速失败，PIT-120 实证 e2e-brand 段 4 在 start 前死）；② 包长命令的 `timeout` 用 **`timeout -k 5 <sec>`**（TERM 只发直接子进程且可能被忽略——如 oxmgr 忽略 TERM——不带 -k 打不死子进程链）；③ "卡住 vs 秒退"第一诊断 = **`bash -x scripts/x.sh` 看尾行**（`+ pkill` 后直接 trap 清理 = 秒退；尾行是目标命令 = 真卡）；④ 清理型 pkill 用 `-x` 精确名（禁止 `-f` 匹配自己命令行——见上 Process Management 规则，本会话再次踩中浪费 200s 超时轮）。

**验证**: `grep -nE "pkill|pgrep" scripts/*.sh`——清理型调用逐行确认 `|| true`；`grep -rn "timeout [0-9]" scripts/` 确认带 -k 的长命令。

**阻塞条件**: 脚本内 pkill/pgrep 无 `|| true`；timeout 包裹长命令不带 -k；"卡住"未用 bash -x 定位直接猜死锁。

**来源**: PIT-120/121 (2026-08-21 app-branding e2e-brand 调试轮)

### 16. pkill/pgrep -f 模式含自身 cmdline 字面量 = 自杀挂死 shell（第二次犯，2026-09-01）

**规则**: 清理浏览器/子进程时**禁止** `pkill -f <字符串>`，当该字符串出现在当前 bash 命令行里（路径如 chrome-linux64/chrome 极易入 own cmdline）。用 `ps -eo pid,comm`（comm 精确列）+ 按 pid kill，或 grep 方括号法 `[c]hrome`。
**先例**: PIT-54（首次）；2026-09-01 play 轮 T11 调试中 `pkill -f "ms-playwright/chromium-1234/chrome-linux64/chrome"`（模式串在自身命令行）→ shell 无声挂死两个 60s/260s 工具窗口。
**阻塞条件**: 任何 `pkill -f`/`pgrep -f` 模式串与当前命令行有子串重叠。

### 17. 批量编辑失败/补丁 no-op——先 grep 现状再补刀，门禁防假绿 (2026-09-03)

**规则**:
① `edit` 工具批量调用报 hash mismatch 后**可能已部分应用**——重试前必须 grep/重读失败区确认每个 op 现状，只补未落地项；整批重发会双重应用（本会话实证：`await this.connect()` 重复行 + tsc 语法错）。
② python 批量补丁每块 replace 前必须 `assert s.count(old)==1`——无 assert 的锚点不匹配是静默 no-op（本会话实证：矩阵脚本定义块未插入，运行至 ReferenceError 才暴露）。
③ 门禁命令取真实退出码：`npx tsc --noEmit && echo OK`（或 `set -o pipefail`）；`cmd | head; echo $?` 读到的是管道尾命令退出码 = 假绿（本会话两次把语法错误读成 tsc=0，白跑一整个压测矩阵）。
**验证**: 补丁落盘后 `grep -c "<新内容关键串>" <文件>` 逐 op 计数；批量 edit 失败后任何重试前必须先读现场。
**阻塞条件**: 整批重发失败批次；无 assert replace；管道尾退出码当门禁。
