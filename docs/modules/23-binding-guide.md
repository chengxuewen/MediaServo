# Binding Matrix Usage Guide (link / deck / field × C / C++ / Python)

> **Positioning**: the multi-language consumption layer of MediaServo's three SDKs (link device-side IPC, deck media data surface, field composed push surface). **The C ABI is the contract foundation**; C++/Python are thin wrappers on top (D227).
>
> **Related**: [20-sdk-api-contract.md](../modules/20-sdk-api-contract.md) §7, [22-field-guide.md](../modules/22-field-guide.md),
> D227 (binding family c/cxx/py), D240 (single dynamic library), D241 (soname + ABI stability), D247 (mediaservo_ prefix), D248 (header strategy)

## Layout

```
bindings/
├── c/
│   ├── include/mediaservo/common.h        # Shared C types (err_t/frame_meta_t 36B/frame_t)
│   ├── mediaservo-field-c/                # cdylib → libmediaservo_field.so
│   │   └── include/mediaservo/field.h     #   headers (hand-maintained, D248)
│   ├── mediaservo-link-c/                 # → libmediaservo_link.so (signal + bus)
│   │   └── include/mediaservo/link.h
│   └── mediaservo-deck-c/                 # → libmediaservo_deck.so (camera/recorder/player)
│       └── include/mediaservo/deck.h
├── cxx/
│   ├── include/mediaservo/detail/result.hpp     # Shared Result (alias tl::expected, C++11 baseline)
│   ├── include/mediaservo/3rdparty/tl/expected.hpp + NOTICE  # vendored (CC0, do not modify)
│   └── mediaservo-{field,link,deck}-cxx/  # header-only RAII (namespace mediaservo::{field,link,deck})
└── python/mediaservo/                     # ctypes package (not a cargo member, D228)
    └── mediaservo/{_ffi,field,link,deck}.py
```

## Build & Test (pixi tasks)

```bash
pixi run build-c          # Three cdylibs + dev .so.<MAJOR> symlinks (D241 DT_NEEDED)
pixi run test-cxx         # C++ three-SDK tests (g++ compile + assertions)
pixi run test-py          # Python unittest (needs MEDIASERVO_LIB_DIR or env inheritance)
pixi run parity-bindings  # Cross-language consistency (version/error-path assertions across all three)
pixi run abi-drift        # ABI drift gate (header declarations ↔ .so exports cross-check, D248)
```

## Per-Language Integration

### C (contract foundation)

```c
#include <mediaservo/field.h>          /* or link.h / deck.h */

mediaservo_push_config_t cfg = MEDIASERVO_PUSH_CONFIG_DEFAULT;  /* struct_size automatic */
cfg.url = "ws://host:9800/ws"; cfg.psk = "..."; cfg.room = "room";
mediaservo_field_push_t* s = NULL;
if (mediaservo_field_push_connect(&cfg, &s) != MEDIASERVO_OK) {
    char err[256]; mediaservo_field_last_error(err, sizeof(err));
}
```

Compile: `gcc app.c -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
       -L target/debug -lmediaservo_field` (run with `LD_LIBRARY_PATH=target/debug`)

### C++ (header-only RAII)

```cpp
#include <mediaservo/field.hpp>        /* or link.hpp / deck.hpp */
using mediaservo::field::PushConfig;
using mediaservo::field::PushSession;

auto s = PushSession::connect(PushConfig{"ws://host:9800/ws", "psk", "room"});
if (!s) { /* s.error().code / s.error().message */ }
s.value().start_video_frames();        /* RAII: destructor closes automatically */
```

Compile: `g++ -std=c++11 app.cpp -I bindings/cxx/mediaservo-field-cxx/include -I bindings/cxx/include \
       -I bindings/c/mediaservo-field-c/include -I bindings/c/include \
       -L target/debug -lmediaservo_field`

### Python (ctypes)

```python
from mediaservo.field import PushConfig, PushSession
s = PushSession.connect(PushConfig(url="ws://host:9800/ws", psk="...", room="room"))
s.publish_video()
```

Run: `export MEDIASERVO_LIB_DIR=$PWD/target/debug && python3 app.py`
(or `LD_LIBRARY_PATH=target/debug`; the three SDK submodules are `mediaservo.{field,link,deck}`)

## Version & soname Chain (the panorama rules after V1a's four independent sources)

| Aspect | Rule | Single source of truth |
|--------|------|------------------------|
| Crate version | The four delivery targets (host/server/field/client) each carry an independent `version`; internal crates inherit `[workspace.package]` (F12/C43) | Each `Cargo.toml` |
| Delivery-domain mapping | package/assembly reads versions per the `_TARGET_CRATE` table: host package←mediaservo-host · server package←server · sdk(bindings) package←**field** | cli, same table (V1b keeps the four package names) |
| soname trio | `libmediaservo_<sdk>.so.<M.m.p>` real file + `.so.<MAJOR>` (DT_SONAME/loader) + `.so` (link time); MAJOR comes from the **field domain version** (the bindings domain = the version source of the delivered sdk package) | D241 + `_crate_version` |
| py/node hand-copied surface | `pyproject.toml`/`__init__.py`/`package.json` align with the **field** version; CI fmt job parity gate (cargo metadata truth comparison, drift = red) | C43③ + CI step |
| FrameMeta wire version | `WIRE_VERSION=0` (decode rejects unknown versions = N4); `frame_meta_version` inside version.txt is **generated from the Rust source** (hand-copied floating numbers forbidden) | `link/src/frame.rs` |
| Release package split | Device half sdk-field (field/link/deck+py+node) / cockpit half sdk-client (client-only), two packages with independent version sources; `bindings` = one-cycle alias (rename WARN) (F12/V1b) | cli `_stage_sdk_package` |
| In-package version declaration | `<target>-version.txt` = domain version + protocol pairing (D-H13); `token_schema_version` still hand-copied (N2 generation pending) | cli `_write_version_file` |

## Contract Essentials (cross-language consistency)

| Item | Rule |
|------|------|
| Prefix | Symbols/types/macros `mediaservo_*` (D247); C++ namespace `mediaservo::<sdk>`; Python module of the same name |
| soname | `libmediaservo_<sdk>.so.<MAJOR>` (D241; MAJOR = C ABI version) |
| struct_size | Every cross-FFI config struct starts with `size_t struct_size` (R3; the caller fills sizeof) |
| Lifecycle | Handles are single-thread-owned; any API call after close is UB; close is idempotent |
| Callbacks | Fired only on the pump thread; calling close inside a callback is forbidden; pointers/event strings are valid only inside the callback (copy if you need to keep them) |
| Errors | `mediaservo_<sdk>_last_error(buf, len)` is thread-safe; 0 = ok, <0 = error code |
| Version | `mediaservo_<sdk>_version(buf, len)` → MAJOR.MINOR.PATCH |
| Compatibility | Within a MAJOR, additive-only (D241); header changes = ABI changes, must pass `abi-drift` |

## Verified Capabilities (2026-08-18)

| Language | Verification |
|----------|--------------|
| C | field/link/deck live e2e on all three (real server frame receipt / event pump / 91-frame closed-loop recording) |
| C++ | Three-SDK tests (version/error paths/move semantics/Result misuse) + parity |
| Python | 22 tests + field push live e2e (connected/published/frames) |
| Full matrix | `parity-bindings` consistent across all three + `abi-drift` declarations=exports (7/12/15) |

## Known Limitations

- **PullSession frame receive is pending** (see 22-field-guide known limitations): the consumer is client; not exposed in the binding layer
- **deck Player**: the C face has only open/frames/close (seek/set_rate deferred as YAGNI, noted N2)
- **Python**: first ctypes version (D227 two-step plan); pyo3 acceleration backend awaits its trigger conditions (frame path >10% of budget etc.)
- **Windows/macOS**: soname is Linux-only (R8 gated); macOS uses default dylib naming; Python `_lib_filename` already dispatches per platform

## Appendix A — SDD Traceability Matrix (functions × test coverage, 2026-08-18)

> Unit tests = cargo test (inside the c crate); e2e = scripts/e2e-bindings.sh (real server/FFmpeg); parity = parity-bindings.
> ⚠️ = coverage gap (see below). Error paths/state machines not listed per function (uniform pattern inside the c crate).

### field (7 functions)
| Function | Unit test | Positive e2e |
|----------|-----------|--------------|
| push_connect | ✓ (null/small-size/missing-required) | ✓ vehicle_push |
| push_publish_video | ✓ (null handle) | ✓ vehicle_push (track=video) |
| push_start_video_frames | ✓ (null failure) | ✓ vehicle_push (frames running) |
| push_stop_video_frames | ✓ (null noop) | ✓ vehicle_push (called before close) |
| push_close | ✓ (null/idempotent) | ✓ vehicle_push |
| last_error (+deprecated alias) | ✓ roundtrip ×2 | ✓ parity |
| version | ✓ roundtrip | ✓ parity (consistent across all three) |

### link (12 functions)
| Function | Unit test | Positive e2e |
|----------|-----------|--------------|
| signal_connect | ✓ (null/small-size/missing-required/bad-role) | ✓ vehicle_signal |
| signal_send | ✓ (null/empty/disconnected/closed) | ✓ vehicle_signal (encoder_status echo) |
| signal_on_event | ✓ (null noop) | ✓ vehicle_signal (event pump) |
| signal_close | ✓ (null) | ✓ vehicle_signal |
| bus_attach | ✓ (null) + positive (two-node signature verify) | ⚠️ no e2e (unit tests cover the SHM roundtrip) |
| bus_publish | ✓ (null/disconnected/closed) + positive | ⚠️ no e2e (unit-test covered) |
| bus_subscribe | ✓ (null) + positive | ⚠️ no e2e (unit-test covered) |
| bus_recv | ✓ (null) + positive (meta/payload assertions) | ⚠️ no e2e (unit-test covered) |
| stream_close | ✓ (null) | — |
| bus_close | ✓ (null) | — |
| last_error | ✓ roundtrip | ✓ parity |
| version | ✓ roundtrip | ✓ parity |

### deck (15 functions)
| Function | Unit test | Positive e2e |
|----------|-----------|--------------|
| devices_enumerate | ✓ (double call/empty/invalid kind) | ✓ record_playback |
| camera_open | ✓ (null/small-size/unknown device/roundtrip) | ✓ record_playback |
| camera_start | ✓ (double-start) | ✓ record_playback (frame pump) |
| camera_frames_cb | ✓ (null) | ✓ record_playback (90+ frame callbacks) |
| camera_stop | ✓ (inside the close roundtrip) | ✓ record_playback |
| camera_close | ✓ | ✓ record_playback |
| recorder_new | ✓ (null/missing parent dir) | ✓ record_playback |
| recorder_record | ✓ (null/camera not started) | ✓ record_playback (mp4 artifact) |
| recorder_stop | ✓ (null failure) | ✓ record_playback |
| recorder_close | ✓ (null) | ✓ record_playback |
| player_open | ✓ (null/missing file) | ✓ record_playback |
| player_frames_cb | ✓ (null failure) | ✓ record_playback (91 frames decoded) |
| player_close | ✓ (null) | ✓ record_playback |
| last_error | ✓ roundtrip | ✓ parity |
| version | ✓ roundtrip | ✓ parity |

### Cross-Language
- parity-bindings: version + empty-config connect error paths (C/C++/Python assert identically) ✓
- abi-drift: 34 declarations == 34 exports ✓

### Completed (2026-08-18, second sweep)
1. ✅ link bus positive-chain unit tests (two-node token/ACL verification → subscribe → publish → recv asserting meta/payload, real SHM roundtrip)
2. ✅ field start/stop, link stream_close/bus_close, deck recorder_stop/player_frames_cb null unit tests
3. ✅ C++ live e2e (step 6 of e2e-bindings, vehicle_field.cpp against a real server)

### Remaining Gaps (acceptable)
- link bus's e2e-level coverage rests on unit tests (no real cross-process server scenario); cxx/py's remaining SDKs rely on the C layer's already-verified chains for positive coverage

## CLI Integration (mediaservo.sh, unified entry)

```bash
./mediaservo.sh build bindings                # Builds the three cdylibs + dev .so.<MAJOR> symlinks
./mediaservo.sh deploy bindings --prefix /usr/local
  # lib/    libmediaservo_{field,link,deck}.so.0.1.0 + .so.0 + .so (the D241 trio)
  # include/mediaservo/  {common,field,link,deck}.h + {field,link,deck}.hpp (D248)
# Consumption (pick one):
#   pkg-config:  PKG_CONFIG_PATH=<prefix>/lib/pkgconfig cc app.c $(pkg-config --cflags --libs mediaservo-field)
#   CMake:       find_package(mediaservo COMPONENTS field REQUIRED) → target_link_libraries(app mediaservo::field)
#                (one package, multiple components, the OpenCV/Boost pattern; link only: COMPONENTS link → mediaservo::link; -DCMAKE_PREFIX_PATH=<prefix>)
#   gcc direct:  gcc app.c -I <prefix>/include -L <prefix>/lib -lmediaservo_field
# Python: pip install bindings/python/mediaservo (thin package; .so location at runtime: LD_LIBRARY_PATH or ldconfig)
```

## Node.js Bindings (napi-rs, isomorphic to livekit rtc-ffi-bindings, 2026-08-18)

- **Architecture**: `bindings/node/rust/mediaservo-node` (napi 3 cdylib → .node) binds the Rust SDK async API directly; thin TS wrapper `lib/index.mjs` (PushSession/SignalSession/CameraSource/Recorder/Player); events/frames cross over ThreadsafeFunction → the JS main thread (isomorphic to livekit's async_queue)
- **Build/install**: `./mediaservo.sh build bindings` (includes mediaservo.node) + `deploy bindings` → `<prefix>/node/mediaservo/`
- **Usage**: `import { PushSession } from '<prefix>/node/mediaservo/lib/index.mjs'` (or NODE_PATH)
- **Runtime prerequisites**: LD_LIBRARY_PATH includes the pixi lib (FFmpeg dynamic libs libavformat.so.63 etc. — the conda toolchain); tests via `npm test` (bindings/node/)
- **Verified**: real-server push (connected/published/frames) + signaling event bridge + record-playback closed loop (node:test 5/5)
- **Contract**: napi methods camelCase (publishVideo); tuple parameters → JS arrays ([meta, data]); `Function<T,()>` + `build_threadsafe_function::<T>().build()`

## C++11 Promise & the Result Contract (2026-08-18)

- **C++11 baseline works**: the three SDK header-only bindings + the shared Result all compile under `-std=c++11` with zero warnings (test-cxx full gate)
- **Result definition**: `mediaservo::Result<T> = tl::expected<T, mediaservo::Error>` (alias, `detail/result.hpp`)
  - Native API: `has_value()/value()/error()/value_or()`; **no operator bool**; `Result<void>` is just `tl::expected<void, Error>`
  - **Contract change (source-breaking, 2026-08-18)**: misusing `value()` throws `tl::bad_expected_access<Error>` (a std::exception subclass; what() is generic text; details via the exception's `.error().code/.message`); old `std::logic_error` assertions must change the catch type (`catch(std::exception)` covers old and new); `error()` on success is standard UB (the old version threw)
  - The bindings shipped with zero real consumers, so the change has no external breakage on record
- **CC0 attribution**: `tl/expected.hpp` vendored from TartanLlama/expected v1.2.0 (CC0-1.0), see `3rdparty/NOTICE` (do not modify; upgrades re-vendor)
- **Future std::expected swap path**: after the compiler reaches C++23, two alias lines in `detail/result.hpp` become `std::expected<T, Error>` (API-isomorphic); the contract test `bindings/cxx/tests/test_result_common.cpp` is the regression anchor
