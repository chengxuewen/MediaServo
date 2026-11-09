# CONSTRAINTS -- what "done" means here

Two parts: hand-written anti-gaming rules above, generated gate list below.

## Anti-gaming rules
- Never commit with `--no-verify`.
- Never silence a warning with `#[allow]` without a named invariant.
- Never delete or weaken an assertion to make a red gate green.
- Fix the cause, not the gate.
- Never hand-edit a generated layer.
- A silent failure (empty catch, swallowed error, bare return) must carry the reason inline.
- Tests must not shrink silently: removing/skipping/moving an assertion requires the reason
  and the replacement coverage in the commit message.
- Every new or changed gate must demo "deliberate violation -> red, restore -> green" with
  the commands and results in the commit message; no negative demo = not done.
- A new or changed `gate "X"` must ship a paired `gate "X self-test"` (template: the three
  existing pairs in `scripts/all-gates.sh`); this checks the diff increment only -- existing
  gates are not retrofitted.

<!-- BEGIN:ci-gates -->
## Gate list (generated from `scripts/all-gates.sh`; 31 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `book structure`
5. `english gate self-test`
6. `english only`
7. `docs link self-test`
8. `docs links`
9. `promises self-test`
10. `doc promises`
11. `fmt --check (incremental)`
12. `changelog markers (F11-X)`
13. `wire changelog pairing (G3)`
14. `bindings version parity`
15. `crate literal versions`
16. `crate lanes reconciliation`
17. `python retirement closure`
18. `entry contract (E1-E3+golden)`
19. `shellcheck mediaservo.sh (warning+)`
20. `shellcheck parent msrtc.sh (warning+)`
21. `check --workspace`
22. `clippy -D --workspace`
23. `server stub --all-targets`
24. `test --workspace (lib)`
25. `openapi validate`
26. `build-c cdylibs`
27. `test-cxx`
28. `abi drift+client`
29. `delivery smoke`
30. `gui example`
31. `gstreamer`
<!-- END:ci-gates -->
