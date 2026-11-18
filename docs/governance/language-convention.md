# Language Convention (R22)

> container-build-rework R22=C (user-adjudicated). **Zero code change** — this
> writes down the existing language split as a norm. The boundary is **repo
> ownership**. (This doc lives in the submodule = product face, hence English.)

## Three faces

| Face | Owner | Language | Scope |
|---|---|---|---|
| **Product** | submodule `3rdparty/MediaServo` | **English** | CLI output/help, code comments, commit messages, product docs (docs/modules), error messages |
| **Shell** | parent `ms_rtc` (release shell) | **Chinese** | `msrtc.sh` internal comments/prompts, parent commits, shell docs, plan ledgers (docs/plans) |
| **Runbook** | site delivery | **Chinese** | site deployment runbook (`34-site-deployment.md`), ops runbooks |

## Rules

1. **Submodule english-only culture intact**: the product face (including
   `mediaservo.sh` output/comments, Rust/cmake code, product docs) is English.
   Submodule commit messages are English.
2. **Parent shell internal Chinese**: `msrtc.sh` is the internal release shell;
   its comments/prompts and parent commits are Chinese. When the parent shell
   **forwards** a submodule command, the submodule's own output stays English
   (product face).
3. **Site runbooks Chinese**: delivery docs for domestic site ops (deployment
   runbook / ops runbook) are Chinese (R22 explicitly rules the runbook face ZH).
4. **Boundary criterion = repo ownership**: a file in the submodule → product
   face EN; in the parent shell → shell face ZH; a site delivery runbook →
   runbook face ZH. Cross-face references each follow their own face's language.

## Existing-fact anchors

- Submodule `mediaservo.sh` comments/output English (english-only);
  `docker/tree-entrypoint.sh` comments English.
- Parent `msrtc.sh` comments Chinese (e.g. "③b 发布根注入", "④ 品牌注入").
- `docs/modules/34-site-deployment.md` Chinese (B3.7 site runbook).
- Plan ledgers `docs/plans/**/PLAN.md` Chinese (shell face).

## Severity vocabulary note (R21④)

New CLI code (B4 `build image` / `dev` verb family) uses the unified
**WARN / ERROR / FATAL** uppercase severity prefixes. The legacy `Error:` /
`Warning:` spellings are **kept for now** — several gates
(`e2e-deploy-host.sh` / `e2e-trap-lib.sh` / `gate-wire-changelog.sh`) grep those
old spellings for failure detection, so a global rename would break the gates
(blast radius checked). A unified rename must migrate the gate consumers in
lockstep — scheduled as a separate micro-round; new code introduces no legacy
spelling.