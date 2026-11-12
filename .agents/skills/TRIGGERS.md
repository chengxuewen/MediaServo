# Skill Trigger Matrix (B2a)

> This is a boundary map, not a skill. The skill loader only reads directories
> containing `SKILL.md`; this file exists so future agents do not add overlapping
> triggers.

| Intent | Owning skill | Hand-off / non-goal |
|---|---|---|
| Reproduce and fix a failure | `debugging-and-error-recovery` | Design/scope -> `think-before-act`; performance -> `performance-optimization` |
| Write a failing test first | `test-driven-development` | Coverage skeletons from specs -> `test-harness` |
| Find the layer/files for a broad ask | `repo-triage` | Skill choice -> `skill-router`; requirements -> `openspec-explore` |
| Retire a public surface safely | `deprecation-and-migration` | New contract -> `api-interface-design`; rollout slices -> `incremental-implementation` |
| Record a decision / fix docs | `documentation-and-adrs` | Whole-set audit -> `doc-audit`; draft proposal -> `openspec-propose` |
| Add logs/metrics/traces/alerts | `observability-and-instrumentation` | Tuning -> `performance-optimization`; one failure -> `debugging-and-error-recovery` |
| Review a diff / merge verdict | `code-review-and-quality` | Deep security -> `security-hardening`; complexity -> `code-simplification` |
| Release / tag / rollback | `shipping-and-launch` | Pipeline mechanics -> `ci-cd-automation`; security sign-off -> `security-hardening` |
| Semantic impact before editing a symbol | `lsp-code-analysis` | Layer map -> `repo-triage`; contract -> `api-interface-design` |
| Build vs borrow before writing a new mechanism | `reuse-first` | Approval to act -> `think-before-act`; upstream/external docs -> `source-driven-development` |
| Multi-role review of one plan/verdict/decision | `expert-panel-review` | Whole doc corpus -> `doc-audit`; act/convene gate -> `think-before-act`; item-by-item verdicts -> `adjudication-walkthrough` |
| Convert a book/document into a skill | `book-to-skill` | Skill-crafting quality gate -> `create-skill`; registration -> a TRIGGERS row here |
| Verify a browser-facing feature | `browser-testing` | Network-impairment angle -> `weaknet-testing`; UI code changes -> `incremental-implementation` |
| Pick the right context for a polyglot task | `context-engineering` | Which layer/files -> `repo-triage`; symbol impact -> `lsp-code-analysis` |
| Audit .agents/ or scan the community for adoptables | `repo-audit` | Internal doc consistency -> `doc-audit`; whether to adopt -> `think-before-act` |
| Legacy ecosystem-scan triggers | `ecosystem-scan` | **INOPERATIVE** -- superseded by `repo-audit` (B1b-1); tombstone kept, use `repo-audit` |
| Batch-review a session and bank the lessons | `lesson-review` | One concrete failure -> `debugging-and-error-recovery`; durable decision -> `documentation-and-adr` |
| Implement tasks from an OpenSpec change | `openspec-apply-change` | **INOPERATIVE** -- requires the missing `openspec` CLI (status: inoperative); until it lands use `openspec-explore` + manual planning |
| Archive a completed OpenSpec change | `openspec-archive-change` | **INOPERATIVE** -- requires the missing `openspec` CLI (status: inoperative) |
| Sync delta specs to main specs | `openspec-sync-specs` | **INOPERATIVE** -- requires the missing `openspec` CLI (status: inoperative); `openspec-propose`/`openspec-explore` stay usable (no CLI dependency) |
| Quick hardcoded secrets/ports/URLs scan | `review-hardcode` | Folded into `security-hardening` (its Phase 1); the full audit -> `security-hardening` |
| Reproduce/measure under network impairment | `weaknet-testing` | Latency/bitrate tracing -> `performance-optimization`; reproduce-first discipline -> `debugging-and-error-recovery` |

## Trigger Boundaries

- `debugging-and-error-recovery` owns "reproduce -> root cause"; `think-before-act` owns "should we act".
- `test-driven-development` owns red-green-refactor; `test-harness` owns skeleton/coverage generation.
- `repo-triage` owns "which layer/files"; `skill-router` owns "which skill".
- `deprecation-and-migration` owns compatibility windows; `api-interface-design` owns the replacement contract.
- `documentation-and-adrs` owns the durable decision; `doc-audit` owns finding inconsistencies.
- `observability-and-instrumentation` owns signal design; `performance-optimization` owns measurement/tuning.
- `code-review-and-quality` owns the five-axis verdict; `security-hardening` owns the deep security pass.
- `shipping-and-launch` owns the release decision; `ci-cd-automation` owns the pipeline that executes it.
- `lsp-code-analysis` owns the symbol impact map; `repo-triage` owns the layer/file map.
- `reuse-first` owns "build vs borrow" for in-repo faces (C60 ladder); `source-driven-development` owns upstream/external docs; `think-before-act` owns the act gate that follows both.
- `expert-panel-review` owns running one multi-role review (execution engine); `doc-audit` owns whole-corpus audits; `think-before-act` owns the convene-or-not gate; `adjudication-walkthrough` owns presenting disagreements for user verdicts.
- `book-to-skill` owns document-to-skill conversion; `create-skill` owns the skill-crafting gate itself.
- `browser-testing` owns browser-facing verification; `weaknet-testing` owns network-impairment measurement (tc netem + the weaknet agent).
- `context-engineering` owns task-context routing; `repo-triage` owns the layer/file map.
- `repo-audit` owns the external community scan; `doc-audit` owns internal consistency.
- `lesson-review` owns batch lesson extraction; `documentation-and-adr` owns the durable decision record.
- `openspec-apply-change` / `openspec-archive-change` / `openspec-sync-specs` are inoperative until the `openspec` CLI exists (frontmatter `status: inoperative`, B4-4); `openspec-propose` / `openspec-explore` remain usable (file-surface only, no CLI).
- `review-hardcode` is folded into `security-hardening` Phase 1; the deep security pass is `security-hardening`'s.

## Naming Rule

A new skill must state its own boundary and hand-off here before it is added.
