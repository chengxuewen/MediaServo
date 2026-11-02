# examples/rs — Rust usage example package (batch ④, plan directory-artifacts-convergence)

One workspace-member cargo package holding all eight user-facing Rust examples
(C-D6=B: every example asset converges under the top-level examples/, VE-parity;
the per-crate `crates/*/examples/` development slots are emptied in this move).

Consume three ways:
```
cargo run -p examples --example <name>                 # pure cargo (never needs CMake)
./mediaservo.sh run example rs/<name>                  # CLI band (runs the built artifact)
(IDE) mediaservo/examples/rs group — per-example build & run buttons + ALL aggregate
```

The CMake face (band registry in examples/CMakeLists.txt) mirrors the Cargo.toml
`[[example]]` table with a double-FATAL disk<->table audit; ctest roster lists ONLY
self-terminating server-free console examples (today: square-gen) — the field/client
trio needs a running signaling server per their usage headers.

| example | needs server | display | note |
|---|---|---|---|
| basic, pull_observe, push_observe, vehicle_push | yes | console | |
| square-gen | no | console | ctest-registered |
| square-gen-egui, viewer, webrtc_loopback_egui | no | egui/display | |

## Feature table — deliberately one line

`mediaservo-media` accepts exactly one backend (`backend-yuv-sys` or `backend-native`,
`compile_error` on both-or-none), and the client/field/webrtc deps of this package
unify `backend-yuv-sys` in whatever you ask for. So the package declares the yuv line
as its default and offers **no** `backend-native` toggle: a per-lane `required-features`
pointing at it could never be satisfied inside this graph.

Both transform demos therefore go through `mediaservo_media::backend::ActiveTransform`
— media's own compile-time alias for whichever backend is active. That closes a lane
that had been dead since it was written: `viewer` used to hard-code `NativeTransform`
while its file-level `#![cfg(feature = "backend-yuv-sys")]` demanded the other backend,
so no feature combination could compile it (`required-features = ["backend-native"]` was
never satisfiable here). Fixing it is three symbols, zero SDK surface: the whole change
lives inside `examples/`.
