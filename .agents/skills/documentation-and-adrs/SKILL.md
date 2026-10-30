---
name: documentation-and-adrs
description: "Record MediaServo decisions and keep docs consistent: D-series entries, architecture and protocol updates, book chapters, changelog, and link checks. Use when a decision must be durable or docs drift from code."
---

# Documentation and ADRs

> A decision that is not recorded will be re-litigated.

## Trigger Conditions

- A design choice changes a public contract, a gate, or a cross-crate boundary.
- Code and `docs/architecture.md` disagree.
- A new feature needs a book chapter or a migration note.
- 用户说“记录决策 / ADR / 更新架构文档 / 写进文档” <!-- c49:trigger -->

## Do Not Use

- Auditing the whole documentation set: use `doc-audit`.
- A typo-only fix: edit it and run the doc gate.
- Requirements exploration: use `openspec-explore`.

## Decision Record

Use the D-series in `.agents/memorys/decisions.md`:

```markdown
## D<n>: <short decision title> (<date>)

- **Context**: <the problem and the constraints>
- **Decision**: <what was chosen>
- **Alternatives**: <what was rejected and why>
- **Consequences**: <trade-offs, compatibility, follow-ups>
- **Evidence**: <paths, gates, measurements>
```

Rules:

- One decision per entry; do not bundle unrelated choices.
- State the rejected alternatives; a decision without alternatives is an assertion.
- Include the verification path; "done" is not evidence.
- If the decision supersedes an older D-entry, link both.
- Update the memory index/high-water if the project requires it.

## Docs Consistency Workflow

1. **Find the source of truth.** Code, protocol schema, or config schema; docs follow it.
2. **Update the nearest doc, not every copy.**
   - Architecture: `docs/architecture.md`.
   - Plans: `docs/plans/<slug>/PLAN.md` (C45 lifecycle).
   - Book: `book/toc.toml` + the chapter.
   - Protocol: the API/WS contract doc.
3. **Check links and paths.** Every referenced path must exist.
4. **Check language.** English unless a line is an explicit trigger/translation.
5. **Run the docs gates.**
   ```bash
   pixi run book-check
   pixi run doc-links
   pixi run ci
   ```
6. **Record the decision** if the change alters a contract or a gate.

## What Goes Where

| Content | Home |
|---|---|
| Why a choice was made | `.agents/memorys/decisions.md` |
| Current architecture | `docs/architecture.md` |
| Forward-looking plan | `docs/plans/<slug>/PLAN.md` |
| User-facing narrative | `book/` |
| Operational steps | module/deploy docs |
| Code contract | code + protocol schema |

## Anti-Patterns

| Smell | Fix |
|---|---|
| Decision only in a chat or commit message | Move it to a D-entry. |
| Docs copy code that already has a schema | Link to the schema; explain the why. |
| "Will update docs later" | The change is not done until the docs match. |
| Multiple conflicting pages | Pick one source; link the rest. |
| Dead relative links | Resolve or remove them before the gate. |

## Exit Criteria

- The decision is durable and dated.
- The nearest doc is updated; no duplicate source of truth.
- Links resolve; language gate green.
- The change is traceable from code to decision.

## Boundaries

- `doc-audit`: finds gaps and inconsistencies across the set; this skill writes the fix.
- `api-interface-design`: defines the contract; this skill records and explains it.
- `openspec-propose`: drafts a change proposal; this skill records the accepted decision.
