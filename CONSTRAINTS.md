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
## Gate list (generated from `scripts/all-gates.sh`; 39 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `mcp registry`
5. `design-library tests`
6. `book structure`
7. `english gate self-test`
8. `english only`
9. `brand neutrality`
10. `prod compose safety`
11. `docs link self-test`
12. `docs links`
13. `docs numbering`
14. `promises self-test`
15. `doc promises`
16. `fmt --check (incremental)`
17. `changelog markers (F11-X)`
18. `wire changelog pairing (G3)`
19. `bindings version parity`
20. `crate literal versions`
21. `crate lanes reconciliation`
22. `python retirement closure`
23. `entry contract (E1-E3+golden)`
24. `shellcheck mediaservo.sh (warning+)`
25. `shellcheck parent shell (warning+)`
26. `check --workspace`
27. `clippy -D --workspace`
28. `server stub --all-targets`
29. `test --workspace (lib)`
30. `openapi validate`
31. `build-c cdylibs`
32. `test-cxx`
33. `abi drift+client`
34. `delivery smoke`
35. `gui example`
36. `gstreamer`
37. `mediasoup`
38. `tracked-file leaks`
39. `leaks self-test`
<!-- END:ci-gates -->
