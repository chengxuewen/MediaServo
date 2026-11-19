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
## Gate list (generated from `scripts/all-gates.sh`; 37 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `mcp registry`
5. `design-library tests`
6. `book structure`
7. `english gate self-test`
8. `english only`
9. `docs link self-test`
10. `docs links`
11. `docs numbering`
12. `promises self-test`
13. `doc promises`
14. `fmt --check (incremental)`
15. `changelog markers (F11-X)`
16. `wire changelog pairing (G3)`
17. `bindings version parity`
18. `crate literal versions`
19. `crate lanes reconciliation`
20. `python retirement closure`
21. `entry contract (E1-E3+golden)`
22. `shellcheck mediaservo.sh (warning+)`
23. `shellcheck parent msrtc.sh (warning+)`
24. `check --workspace`
25. `clippy -D --workspace`
26. `server stub --all-targets`
27. `test --workspace (lib)`
28. `openapi validate`
29. `build-c cdylibs`
30. `test-cxx`
31. `abi drift+client`
32. `delivery smoke`
33. `gui example`
34. `gstreamer`
35. `mediasoup`
36. `tracked-file leaks`
37. `leaks self-test`
<!-- END:ci-gates -->
