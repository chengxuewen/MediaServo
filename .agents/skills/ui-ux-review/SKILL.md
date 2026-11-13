---
name: ui-ux-review
description: Structured interface-quality review method. Use when evaluating a UI surface, reviewing a frontend change for usability/accessibility, or checking empty/error/loading states, contrast, focus order, and design-token consistency. Produces a screenshot-backed verdict.
---

# ui-ux-review — a repeatable interface-quality bar

> The method behind the `ui-ux` role's verdict. The aesthetic constraints live in
> `.agents/rules/web/design-quality.md` (reference, never copy); token mechanics
> live in `design-system`. This skill is the checklist that turns "looks off" into
> evidence-backed findings.

## Trigger Conditions

- Reviewing a UI surface or a frontend diff for quality (not just correctness).
- Asked "is this UI good / accessible / consistent?"
- Before signing off a visual change (the `web-frontend` role hands off here).

## Method (run in order, capture evidence as you go)

0. **Self-serve capture.** Drive the browser yourself (Playwright is in the
   toolchain) -- do not wait for someone to paste screenshots. When a prototype
   exists under `www/prototype/`, capture it too: the prototype screenshot is
   the expected output the implementation is compared against.
1. **Three states.** Inspect the surface in **empty**, **error**, and **loading**
   states — not just the happy path. A surface that only works when full of data is
   not done. Screenshot each state that is reachable.
2. **Contrast.** Check text/background contrast for primary, secondary, and muted
   text (the `--ms-color-text*` tokens on their surfaces). Flag any pair below the
   readable threshold; muted-on-white is the usual offender.
3. **Focus order & keyboard.** Tab through interactive elements: order matches the
   visual layout, focus is visible, nothing is keyboard-trapped. Note the focus ring.
4. **Token consistency.** Verify styles consume `@mediaservo/ui` tokens (see
   `design-system`); flag raw hex / magic spacing / off-palette values.
5. **Anti-template.** Apply `.agents/rules/web/design-quality.md`: hierarchy,
   layering, intentional spacing — not a generic uniform card grid. This is a
   judgment call; state the reason, not just "looks template-y".
6. **Verdict.** Emit a verdict (pass / needs-work) plus a findings table:
   `check | evidence (screenshot / file:line) | rule or token violated | severity`.

## Evidence discipline

- Every finding cites a screenshot or a `file:line`; no evidence -> mark it
  "speculation", not a finding (evidence-critic standard).
- Read-only: do not edit the UI under review. Implementation belongs to
  `web-frontend`; this review only reports.

## Boundaries

- Functional correctness / tests -> `browser-testing` and `web-frontend`, not here.
- Token mechanics (adding/renaming tokens) -> `design-system`.
- This skill reviews; it does not self-approve its own implementation.