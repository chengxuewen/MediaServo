# CONSTRAINTS -- what "done" means here

Two parts: hand-written anti-gaming rules above, generated gate list below.

## Anti-gaming rules
- Never commit with `--no-verify`.
- Never silence a warning with `#[allow]` without a named invariant.
- Never delete or weaken an assertion to make a red gate green.
- Fix the cause, not the gate.
- Never hand-edit a generated layer.

<!-- BEGIN:ci-gates -->
## Gate list (generated from `scripts/all-gates.sh`; 22 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `book structure`
5. `fmt --check (incremental)`
6. `changelog markers (F11-X)`
7. `wire changelog pairing (G3)`
8. `bindings version parity`
9. `crate literal versions`
10. `crate lanes reconciliation`
11. `python retirement closure`
12. `check --workspace`
13. `clippy -D --workspace`
14. `server stub --all-targets`
15. `test --workspace (lib)`
16. `openapi validate`
17. `build-c cdylibs`
18. `test-cxx`
19. `abi drift+client`
20. `delivery smoke`
21. `gui example`
22. `gstreamer`
<!-- END:ci-gates -->
