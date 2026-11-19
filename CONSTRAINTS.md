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
## Gate list (generated from `scripts/all-gates.sh`; 38 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `mcp registry`
5. `design-library tests`
6. `book structure`
7. `english gate self-test`
8. `english only`
9. `brand neutrality`
10. `docs link self-test`
11. `docs links`
12. `docs numbering`
13. `promises self-test`
14. `doc promises`
15. `fmt --check (incremental)`
16. `changelog markers (F11-X)`
17. `wire changelog pairing (G3)`
18. `bindings version parity`
19. `crate literal versions`
20. `crate lanes reconciliation`
21. `python retirement closure`
22. `entry contract (E1-E3+golden)`
23. `shellcheck mediaservo.sh (warning+)`
24. `shellcheck parent msrtc.sh (warning+)`
25. `check --workspace`
26. `clippy -D --workspace`
27. `server stub --all-targets`
28. `test --workspace (lib)`
29. `openapi validate`
30. `build-c cdylibs`
31. `test-cxx`
32. `abi drift+client`
33. `delivery smoke`
34. `gui example`
35. `gstreamer`
36. `mediasoup`
37. `tracked-file leaks`
38. `leaks self-test`
<!-- END:ci-gates -->
