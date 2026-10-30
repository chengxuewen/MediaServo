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

## Naming Rule

A new skill must state its own boundary and hand-off here before it is added.
