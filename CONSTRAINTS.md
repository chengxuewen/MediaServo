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
## Gate list (generated from `scripts/all-gates.sh`; 33 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `book structure`
5. `english gate self-test`
6. `english only`
7. `docs link self-test`
8. `docs links`
9. `docs numbering`
10. `promises self-test`
11. `doc promises`
12. `fmt --check (incremental)`
13. `changelog markers (F11-X)`
14. `wire changelog pairing (G3)`
15. `bindings version parity`
16. `crate literal versions`
17. `crate lanes reconciliation`
18. `python retirement closure`
19. `entry contract (E1-E3+golden)`
20. `shellcheck mediaservo.sh (warning+)`
21. `shellcheck parent msrtc.sh (warning+)`
22. `check --workspace`
23. `clippy -D --workspace`
24. `server stub --all-targets`
25. `test --workspace (lib)`
26. `openapi validate`
27. `build-c cdylibs`
28. `test-cxx`
29. `abi drift+client`
30. `delivery smoke`
31. `gui example`
32. `gstreamer`
33. `mediasoup`
<!-- END:ci-gates -->
