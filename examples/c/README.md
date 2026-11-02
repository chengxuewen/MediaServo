# examples/c — C usage material (batch ②, plan directory-artifacts-convergence)

Pure C examples per domain, compiled by `scripts/e2e-bindings.sh` (gcc + the crate
include dirs + target/debug .so) — no CMake targets by design (ruled: material, not
lane). Relocated from `bindings/c/mediaservo-<domain>-c/examples/` (deck-c kept its
`record_playback.c` naming; the crate header contract `<domain>.h` is unchanged).

| file | domain | e2e consumer |
|---|---|---|
| `deck/record_playback.c` | deck | e2e-bindings.sh §5 (closed loop) |
| `field/vehicle_push.c`   | field | e2e-bindings.sh §3 |
| `link/vehicle_signal.c`  | link  | e2e-bindings.sh §4 |
