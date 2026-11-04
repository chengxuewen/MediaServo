# cmake/gui-deps -- consumer-side dependency recipes (sr8)

Two lanes live here. Keeping them apart in this file is the point: they answer
different questions, and the CLI refuses to mix them.

| | source lane (P3-P5, shipped) | ecosystem lane (P7c, dev-only proof) |
|--|---|---|
| library | SDL3, Dear ImGui from a vendored archive | fmt through vcpkg or conan |
| recipe | `recipes/sdl3.json`, `recipes/imgui.json` | `recipes/<pkg>-<ecosystem>.json` |
| pin | archive sha256 + `patches/` | `pkg-deps/<pkg>/` island: vcpkg.json + conanfile.txt + conan.lock |
| verb | `build gui-deps [--profile] [--provider] [--pixi-env]` | `build gui-deps --ecosystem <vcpkg or conan> --pkg <name> [--generate-ecosystem]` |
| default path | yes -- unchanged since P3 | no -- nothing is fetched unless `--generate-ecosystem` is given |
| where the result goes | the build anchor, keyed by full-field fingerprint | same, plus a pack in `.cache/3rdparty-ecosystem` |

## Why one island serves both providers

`pkg-deps/fmt/` holds vcpkg's manifest and conan's manifest+lock together, and
the two recipes differ by exactly one line (`"provider"`). That is the sr8
promise -- "switch the source of a library, keep everything else" -- executed
rather than described. Each lane hashes only its own files, so editing the conan
lock cannot age a vcpkg artifact and vice versa (asserted in the supplier test
family).

## Why there is no PROVENANCE.md here

The parent repo's `3rdparty/*/PROVENANCE.md` ledgers exist because vendored
binaries and archives need a hand-maintained origin record. The recipes here
carry no archive: identity comes from the pin island's content hash (which rides
the cache key) and from the pack's own `.sha256` + `.manifest.json` sidecars. A
second ledger would be a second truth to drift, so there is none. If a truly
offline runner ever needs vendored ecosystem archives, that belongs in the
parent's `3rdparty/` convention, not here.

## What must not move onto the ecosystem lane

SDL3 carries an environment patch and Dear ImGui tracks a docking branch. Both
stay on `provider: "source"`: the ecosystem lanes have no overlay-port channel
(sr8 O-13, a conditional entry that requires a real candidate before it gets
built). Nothing in the default build path changes for either library, and no CI
leg runs the ecosystem lane (network and duration; conditional).
