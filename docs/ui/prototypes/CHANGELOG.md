# Prototypes CHANGELOG

## v0.2 — 2026-11-12 (w1-portal-shell)
Rework per web-portal-ui **P8** (workbudy transplant: borrow patterns, change boundaries).
Fixes the 4 self-violations the expert panel caught in v0.1, and lands the borrowed design rules:
- **3-layer tokens** (primitive → semantic → component) + **dual theme** (`data-theme` light/dark,
  no-flash resolve, topbar toggle ◐). Admin light + dark both first-class; video letterbox dark in both.
- **State grammar**: no state by hue alone — solid dot (held/online), 3-seg bar + rtt/loss numbers
  (degraded/queued), hollow-ring-slash (offline/signal-lost), triangle (warning/unknown). Added a
  degraded tile (CAM-03, link 2/3) and kept the offline tile (CAM-04).
- **Removed all infinite animation** (v0.1 pulsed live/floor lamps) → static indicators; transitions ≤150ms.
- **Video tiles radius 0** (was 10px); **weights 400/600 only** (was 500/700); system font stack;
  tabular-nums slashed-zero on timers/counters; space/radius from token scale.
- Role badge → neutral chip (no hue); combobox/segmented controls (no native `<select>`).
- Screens unchanged in scope: Live wall / Intercom board / Tower / Settings; WS-drop → floor UNKNOWN demo kept.

## v0.1 — 2026-11-12 (archived: archive/w1-portal-shell-v0.1.html)
First exploration draft: shell + 4 screens, control-room night palette, full interactivity.
Superseded by v0.2 (violated the borrowed rules: infinite animation, tile radius, weight 500/700, dark-only).
