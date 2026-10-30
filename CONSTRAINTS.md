# CONSTRAINTS -- what "done" means here

Two parts: hand-written anti-gaming rules above, generated gate list below.

## Anti-gaming rules
- Never commit with `--no-verify`.
- Never silence a warning with `#[allow]` without a named invariant.
- Never delete or weaken an assertion to make a red gate green.
- Fix the cause, not the gate.
- Never hand-edit a generated layer.

<!-- BEGIN:ci-gates -->
## Gate list (generated from `scripts/all-gates.sh`; 21 sites)

1. `build mediaservo-gates`
2. `agent single source`
3. `agent toolchain`
4. `fmt --check (incremental)`
5. `changelog markers (F11-X)`
6. `wire changelog pairing (G3)`
7. `bindings version parity`
8. `crate literal versions`
9. `crate lanes reconciliation`
10. `python retirement closure`
11. `check --workspace`
12. `clippy -D --workspace`
13. `server stub --all-targets`
14. `test --workspace (lib)`
15. `openapi validate`
16. `build-c cdylibs`
17. `test-cxx`
18. `abi drift+client`
19. `delivery smoke`
20. `gui example`
21. `gstreamer`
<!-- END:ci-gates -->
