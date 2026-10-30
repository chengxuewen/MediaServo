# CONSTRAINTS -- what "done" means here

Two parts: hand-written anti-gaming rules above, generated gate list below.

## Anti-gaming rules
- Never commit with `--no-verify`.
- Never silence a warning with `#[allow]` without a named invariant.
- Never delete or weaken an assertion to make a red gate green.
- Fix the cause, not the gate.
- Never hand-edit a generated layer.

<!-- BEGIN:ci-gates -->
## Gate list (generated from `scripts/all-gates.sh`; 28 sites)

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
18. `check --workspace`
19. `clippy -D --workspace`
20. `server stub --all-targets`
21. `test --workspace (lib)`
22. `openapi validate`
23. `build-c cdylibs`
24. `test-cxx`
25. `abi drift+client`
26. `delivery smoke`
27. `gui example`
28. `gstreamer`
<!-- END:ci-gates -->
