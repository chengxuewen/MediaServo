---
name: design-system
description: Govern the MediaServo design-token layer (@mediaservo/ui). Use when adding or changing UI styling, creating a component, choosing colors/spacing/radii, or asked about the design system or design tokens. Ensures styles consume shared tokens instead of hardcoding values.
---

# design-system — shared design tokens, not scattered hex

> The design language lives in ONE place: `www/packages/ui/tokens.css`
> (`@mediaservo/ui`). This skill is the method for consuming and extending it.
> Aesthetic judgment (is it good?) belongs to `ui-ux-review` / the `ui-ux` role;
> this skill owns the token mechanics (is it consistent and centralized?).

## Trigger Conditions

- Adding or changing CSS / styling in `www/**`.
- Creating a component that needs color, spacing, radius, or type choices.
- A review flags "hardcoded hex / magic spacing" or "off-palette".
- Asked "what are our design tokens / design system?"

## The token layer

- Source: `www/packages/ui/tokens.css`, exported as `@mediaservo/ui/tokens.css`.
- Naming: `--ms-<category>-<role>[-<variant>]`; categories `color / font / space / radius / shadow / size`.
- Values were extracted from the existing admin styles (B8, closes O-7); they name
  what is already there. The scattered admin CSS converges onto them in B8 part 2.

## Method

1. **Consume, don't hardcode.** Import once at the app entry
   (`import "@mediaservo/ui/tokens.css";`), then use `var(--ms-...)`. A new raw hex
   or magic px in a component is a finding, not a style choice.
2. **Find the existing token first.** Before inventing a value, grep `tokens.css`
   for a token that already fits (reuse-first). Most needs map to an existing token.
3. **Extend only with a real consumer.** A new token requires at least one concrete
   component that uses it and a value that cannot be expressed by existing tokens.
   Add it to the right category with the naming convention; never add a token
   "just in case" (YAGNI).
4. **Keep it a single source.** Do not fork a second palette in an app or component.
   If a surface needs a variant, add a token variant here, not a local override.
5. **Check the quality bar.** Token consistency is necessary, not sufficient — the
   anti-template / hierarchy constraints live in `.agents/rules/web/design-quality.md`.
   Hand the "does it look intentional" question to `ui-ux-review`.

## Verification

- `grep -rnE '#[0-9a-fA-F]{3,6}' <changed css>` shows no new raw hex (existing ones
  are the B8-part-2 migration backlog, not new violations).
- Every new `--ms-*` token has a consumer and follows the naming convention.
- The app entry imports `@mediaservo/ui/tokens.css` exactly once.

## Boundaries

- This skill governs token mechanics; it does not approve visual quality
  (`ui-ux-review` / `ui-ux` role) and does not implement features (`web-frontend`).
- Do not migrate the whole admin CSS in one pass without a plan — that is B8 part 2,
  scheduled separately to avoid overlapping active frontend work.