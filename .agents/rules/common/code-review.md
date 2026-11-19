# Code Review Standards

> Projection notice: the review-verdict authority is the `code-review-and-quality`
> skill (Blocking/Optional/Nit/FYI; Approve / Request Changes / Needs Discussion).
> This file is a checklist projection -- its ladder and three-value approval are
> mapped views, not a second entry point (review-convergence T7).

## Purpose

Code review ensures quality, security, and maintainability before code is merged. This rule defines when and how to conduct code reviews.

## When to Review

**MANDATORY review triggers:**

- After writing or modifying code
- Before any commit to shared branches
- When security-sensitive code is changed (auth, payments, user data)
- When architectural changes are made
- Before merging pull requests

**Pre-Review Requirements:**

Before requesting review, ensure:

- All automated checks (CI/CD) are passing
- Merge conflicts are resolved
- Branch is up to date with target branch

## Review Checklist

Before marking code complete:

- [ ] Code is readable and well-named
- [ ] Functions &lt;50 lines: verify with `wc -L` on changed files
- [ ] Files &lt;800 lines: verify with `wc -l` on changed files
- [ ] No deep nesting (&gt;4 levels): check indent depth in diff
- [ ] Errors handled explicitly: `grep -c 'unwrap()'` must be 0 in changed .rs
- [ ] No hardcoded secrets: run `scripts/scan-hardcode.sh` or `/security-hardening`
- [ ] No dbg!/console.log: `grep -r 'dbg!(' crates/` must return 0
- [ ] Tests exist: `cargo test --workspace`
- [ ] Coverage &gt;=80%: `cargo tarpaulin --workspace`

## Security Review Triggers

**STOP and invoke the `security-hardening` skill when:**

- Authentication or authorization code
- User input handling
- Database queries
- File system operations
- External API calls
- Cryptographic operations
- Payment or financial code

## Review Severity Levels

| Level | Meaning | Action |
|-------|---------|--------|
| CRITICAL | Security vulnerability or data loss risk | **BLOCK** - Must fix before merge |
| HIGH | Bug or significant quality issue | **WARN** - Should fix before merge |
| MEDIUM | Maintainability concern | **INFO** - Consider fixing |
| LOW | Style or minor suggestion | **NOTE** - Optional |

## Agent Usage

Use these agents for code review:

| Agent | Purpose |
|-------|---------|
| **code-reviewer** | General code quality, patterns, best practices |
| **security-hardening** skill | Security vulnerabilities, OWASP Top 10 |

Language-specific findings route through the five axes above, executed by the real `code-reviewer` role -- the four former `*-reviewer` rows referenced roles that do not exist (fixed, review-convergence T7).

## Review Workflow

```
1. Run git diff to understand changes
2. Check security checklist first
3. Review code quality checklist
4. Run relevant tests
5. Verify coverage >= 80%
6. Use appropriate agent for detailed review
```

## Common Issues to Catch

### Security

- Hardcoded credentials (API keys, passwords, tokens)
- SQL injection (string concatenation in queries)
- XSS vulnerabilities (unescaped user input)
- Path traversal (unsanitized file paths)
- CSRF protection missing
- Authentication bypasses

### Code Quality

- Large functions (>50 lines) - split into smaller
- Large files (>800 lines) - extract modules
- Deep nesting (>4 levels) - use early returns
- Missing error handling - handle explicitly
- Mutation patterns - prefer immutable operations
- Missing tests - add test coverage

### Performance

- N+1 queries - use JOINs or batching
- Missing pagination - add LIMIT to queries
- Unbounded queries - add constraints
- Missing caching - cache expensive operations

## Approval Criteria

- **Approve**: No CRITICAL or HIGH issues
- **Warning**: Only HIGH issues (merge with caution)
- **Block**: CRITICAL issues found

## Integration with Other Rules

This rule works with:

- [testing.md](testing.md) - Test coverage requirements
- [security.md](security.md) - Security checklist
- [git-workflow.md](git-workflow.md) - Commit standards
- [agents.md](agents.md) - Agent delegation\n