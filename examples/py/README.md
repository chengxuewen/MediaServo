# examples/py — Python usage examples (batch ③, plan directory-artifacts-convergence)

Relocated from `bindings/python/mediaservo/examples/` (which lived inside the wheel
project root — never packaged: `_ms_build_pywheel` mirrors the project to staging and
rms this dir; the move makes that structural, the staging rm stays defensive).

Run (dev mode against the built .so, or against an installed wheel):
```
export MEDIASERVO_LIB_DIR=$PWD/target/debug
./mediaservo.sh run example py/vehicle_push   ws://127.0.0.1:9800/ws mediaservo-dev room-1 10
./mediaservo.sh run example py/vehicle_signal ws://127.0.0.1:9800/ws mediaservo-dev room-1
./mediaservo.sh run example py/record_playback
```
The scripts' `sys.path` dev-mode line resolves `../../bindings/python/mediaservo`
(the package project root) from this location. Batch ③ also fixed two latent
NameErrors carried over from the old location (`import os` was missing in
vehicle_push.py / vehicle_signal.py while the dev-mode line used it).
