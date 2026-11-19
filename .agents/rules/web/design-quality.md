> This file extends [common/patterns.md](../common/patterns.md) with web-specific design-quality guidance.

# Web Design Quality Standards

<!-- De-slop tells, direction lines, the nine-step pass, and the template test
     are adapted from heydamir/unslop (CC BY 4.0,
     https://github.com/heydamir/unslop), rewritten to this repo's token
     vocabulary. -->


## Anti-Template Policy

Do not ship generic template-looking UI. Frontend output should look intentional, opinionated, and specific to the product.

### Banned Patterns

- Default card grids with uniform spacing and no hierarchy
- Stock hero section with centered headline, gradient blob, and generic CTA
- Unmodified library defaults passed off as finished design
- Flat layouts with no layering, depth, or motion
- Uniform radius, spacing, and shadows across every component
- Safe gray-on-white styling with one decorative accent color
- Dashboard-by-numbers layouts with sidebar + cards + charts and no point of view
- Default font stacks used without a deliberate reason

### AI-look tells (signs; audit before styling)

Grade severity first: heavy / light / none. "None" earns at most 2-3 small
suggestions -- not a redesign. Tells, beyond the banned patterns above:

- Purple gradients, glow effects, colored shadows, cross-color blends
- Emoji used as icons; "New ✨"-style badges
- Decorative fills: floating orbs, grid overlays, holographic brains
- Greeting copy, exclamation marks, hype words, fake-statistic cards
- Uppercase letter-spaced labels, italic emphasis, 3+ font weights
- Every element centered; every section the same width

Industry-specific anti-patterns (beyond these generic tells): consult the
design-library skill (`data/ui-reasoning.csv`, Anti_Patterns column).

### Required Qualities

Every meaningful frontend surface should demonstrate at least four of these:

1. Clear hierarchy through scale contrast
2. Intentional rhythm in spacing, not uniform padding everywhere
3. Depth or layering through overlap, shadows, surfaces, or motion
4. Typography with character and a real pairing strategy
5. Color used semantically, not just decoratively
6. Hover, focus, and active states that feel designed
7. Grid-breaking editorial or bento composition where appropriate
8. Texture, grain, or atmosphere when it fits the visual direction
9. Motion that clarifies flow instead of distracting from it
10. Data visualization treated as part of the design system, not an afterthought

### Template Test (acceptance)

Swap in another product's name and content: if the design still works
unchanged, the direction is not pushed enough -- it is a template. Name WHICH
elements stayed generic. Never fix slop by adding NEW decoration:
glassmorphism, noise, and bento grids are trends too -- swapping decoration
for decoration is the same disease in a new outfit. (The style directions
below stay valid when they serve the product's pushed direction, not as
decoration swaps.)

### Style-system precedence (multi-style era)

Faithfully implementing a CHOSEN design system (Fluent / Material / Ant /
Carbon via `www/packages/ui/themes/`) is not "unmodified library defaults
passed off as finished design": when a style system is the ruled direction,
conformance is judged against THAT system's patterns (see the theme file's
projection notes). The template test still applies to what the system
leaves open -- layout composition, content hierarchy, signature moments.

## Accessibility floor (universal, all styles)

- Text/background contrast meets 4.5:1 for body text (3:1 for large text).
- Keyboard focus is visible; nothing is keyboard-trapped.
- Motion respects `prefers-reduced-motion`; no information is conveyed by
  motion or by color alone.
- Interactive targets are reachable and operable without a pointer.

## Base style parameters (style-relative values)

The values below are the BASE style's parameters -- an active style system
(`themes/`) overrides them per its own parameter set; see the
design-library skill and each theme file's projection notes. Quality
floors (hierarchy, alignment discipline, the a11y floor, no-AI-tells) are
style-independent and stay binding under every theme.

- Grid: 4px base (Carbon's 2x grid = 8px under its theme)
- Icons: lucide at 16/18px; the icon-size dimension is owned by the token layer
- Type: two weights (400/500), sentence case
- Accent budget: one restrained accent (= --ms-color-primary; semantic
  roles exempt) in the BASE style; Material/Ant bring their own scales
- Motion: one easing curve in the BASE style; each system ships its family
- Radius and nested-radius math: base values; per-system scales in themes

## Nine-Step Visual Pass (ordered)

Visual changes run in this order -- each step builds on the previous frame.
Micro-fixes touch only the relevant step. Values adapt to the token vocabulary
(`tokens.css` stays the single source for color and type roles), and the
numeric values below are BASE parameters -- an active theme overrides them
per its projection notes:

1. Spacing -- 4px grid; same-class elements share sizes; navigation densest.
2. Color -- neutral scale + three text roles, all via tokens; ONE restrained
   accent budget (= `--ms-color-primary`; semantic roles danger/success/warning
   are exempt); kill cross-color gradients, glows, colored shadows.
3. Icons -- emoji out; one line-icon set (lucide), uniform 16/18px.
4. Copy -- no exclamations, greetings, or hype words; mock data stays realistic.
5. Type -- two weights (400/500), sentence case, no uppercase letter-spaced labels.
6. Layout -- block order preserved; hero prefers scene imagery + asymmetric split.
7. Details -- nested-radius math; one hero chart; stat cards carry numbers only.
8. Visuals -- delete decorative fills; dashboards rarely need generated imagery.
9. Motion -- no loops or springs; explanatory motion only; one easing curve.

## Before Writing Frontend Code

### Design direction (five lines; write first, confirm, then build)

Adapted from heydamir/unslop (CC BY 4.0). Any visual pass beyond a micro-fix
starts with five written lines, confirmed by the user before edits begin
(bottom line #4):

1. Positioning -- the product in one sentence.
2. Personality -- 2-3 concrete words; "clean", "modern", "sleek" are banned as empty.
3. World -- the material environment the product lives in.
4. Moments -- the 1-2 spots allowed to be visually rich; the frame stays quiet.
5. Signature -- the small detail only this product would have.

Micro-fixes skip the direction step: one element, no new tokens/colors/layout
semantics, roughly under ten changed lines -- when in doubt, it is NOT micro
(write the lines). Then gather a
small set of real references and load the `design-system` / `ui-ux-review`
skills.

## Worthwhile Style Directions

- Editorial / magazine
- Neo-brutalism
- Glassmorphism with real depth
- Dark luxury or light luxury with disciplined contrast
- Bento layouts
- Scrollytelling
- 3D integration
- Swiss / International
- Retro-futurism

Do not default to dark mode automatically. Choose the visual direction the product actually wants.

## Component Checklist

- [ ] Does it avoid looking like a default Tailwind or shadcn template?
- [ ] Does it have intentional hover/focus/active states?
- [ ] Does it use hierarchy rather than uniform emphasis?
- [ ] Would this look believable in a real product screenshot?
- [ ] If it supports both themes, do both light and dark feel intentional?
