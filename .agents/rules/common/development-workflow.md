# Development Workflow

The Feature Implementation Workflow describes the development pipeline: research, planning, TDD, code review, and then committing to git.

## Feature Implementation Workflow

0. **Research & Reuse** _(mandatory before any new implementation)_
   - **GitHub code search first:** Run `gh search repos` and `gh search code` to find existing implementations, templates, and patterns before writing anything new.
   - **Library docs second:** Use Context7 or primary vendor docs to confirm API behavior, package usage, and version-specific details before implementing.
   - **Exa only when the first two are insufficient:** Use Exa for broader web research or discovery after GitHub search and primary docs.
   - **Check package registries:** Search npm, PyPI, crates.io, and other registries before writing utility code. Prefer battle-tested libraries over hand-rolled solutions.
   - **Search for adaptable implementations:** Look for open-source projects that solve 80%+ of the problem and can be forked, ported, or wrapped.
   - Prefer adopting or porting a proven approach over writing net-new code when it meets the requirement.

1. **Plan First**
   - Use **planner** agent to create implementation plan
   - Generate planning docs before coding: PRD, architecture, system_design, tech_doc, task_list
   - Identify dependencies and risks
   - Break down into phases
   - Present the plan to the user

1.5 **Execution Confirmation Gate** _(mandatory — NEVER skip)_
   - **MUST get explicit user confirmation** before executing ANY plan or todo list:
     - Use the `question` tool with a "Proceed with execution?" prompt (interactive confirm)
     - Wait for user's affirmative response ("\u786e\u8ba4", "\u6267\u884c", "yes", "proceed")
   - **What requires confirmation**:
     - Creating or deleting crates/workspace members
     - Moving modules between crates
     - Changing directory structures
     - Altering Cargo.toml features (adding/removing features, deps)
     - Modifying CMake build infrastructure
     - Any multi-step todo list execution
   - **What does NOT count as confirmation**:
     - System directives (TODO CONTINUATION, SYSTEM REMINDER)
     - The word "\u7ee7\u7eed" (continue) \u2014 means "continue discussing" or "continue with trivial/safe work"
     - Silence or timeout
   - **Process**: Present plan → display confirmation prompt → wait for user → execute only after explicit affirmative

2. **TDD Approach**
   - Use **tdd-guide** agent
   - Write tests first (RED)
   - Implement to pass tests (GREEN)
   - Refactor (IMPROVE)
   - Verify 80%+ coverage

3. **Code Review**
   - Use **code-reviewer** agent immediately after writing code
   - Address CRITICAL and HIGH issues
   - Fix MEDIUM issues when possible

4. **Commit & Push**
   - Detailed commit messages following conventional commits format (see below)
   - Follow the Pull Request Workflow (see below)

5. **Pre-Review Checks**
   - Verify all automated checks (CI/CD) are passing
   - Resolve any merge conflicts
   - Ensure branch is up to date with target branch
   - Only request review after these checks pass

## Commit Message Format
```
<type>: <description>

<optional body>
```

Types: feat, fix, refactor, docs, test, chore, perf, ci

Note: Attribution disabled globally via ~/.claude/settings.json.

## Pull Request Workflow

When creating PRs:
1. Analyze full commit history (not just latest commit)
2. Use `git diff [base-branch]...HEAD` to see all changes
3. Draft comprehensive PR summary
4. Include test plan with TODOs
5. Push with `-u` flag if new branch
