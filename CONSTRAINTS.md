# CONSTRAINTS -- what "done" means here

Two parts: hand-written anti-gaming rules above, generated gate list below.

## Anti-gaming rules
- Never commit with `--no-verify`.
- Never silence a warning with `#[allow]` without a named invariant.
- Never delete or weaken an assertion to make a red gate green.
- Fix the cause, not the gate.
- Never hand-edit a generated layer.

<!-- BEGIN:ci-gates -->
## Gate list (generated from `scripts/all-gates.sh`; 19 sites)

1. `build mediaservo-gates`
2. `fmt --check (incremental)`
3. `changelog markers (F11-X)`
4. `wire changelog pairing (G3)`
5. `bindings version parity`
6. `crate literal versions`
7. `crate lanes reconciliation`
8. `python retirement closure`
9. `check --workspace`
10. `clippy -D --workspace`
11. `server stub --all-targets`
12. `test --workspace (lib)`
13. `openapi validate`
14. `build-c cdylibs`
15. `test-cxx`
16. `abi drift+client`
17. `delivery smoke`
18. `gui example`
19. `gstreamer`
<!-- END:ci-gates -->
