# Plan: C++ bindings full migration to tl::expected — C++11 baseline + native API (2026-08-18)

> **Source**: hyperplan adversarial planning (4 perspectives: arch/integration/cost/creative + lead cross-attack + C++11 empirical verdict + plan agent two revision rounds)
> **Status**: Awaiting user confirmation to execute | **Related**: 23-binding-guide, D248, contract §7

## Verdict Summary

Abandon the compatibility layer (Result wrapper keeps ok()/operator bool), **fully migrate to the tl::expected native API**:
`Result<T>` = alias of `tl::expected<T, Error>` (not a wrapper class); factories success()/failure() are removed; misuse throws
`tl::bad_expected_access<Error>` (a std::exception subclass). Rationale: zero real consumers → future consumers learn a standard API
worth more; a half-migration is worse than no migration; after the Jetson toolchain upgrade it is a one-line swap to std::expected;
D241 only locks the C ABI, the C++ header API has no ABI promise, source-breaking is acceptable.

## Empirical Baseline (verified, `-std=c++11 -Wall -Wextra` zero errors zero warnings)

1. tl::expected 1.2.0 fully green on C++11; `expected<void,E>` default-ctor engaged (`Result<void>()` = success)
2. Value/error construction OK (`expected<T,E>(std::move(v))` / `(tl::unexpect, e)`)
3. Value semantics: copy-constructible/assignable/nothrow-move; move-only unique_ptr
4. Misuse of value() throws bad_expected_access, catch(std::exception) compatible; what() is generic text, details via .error()
5. Of the three headers only variant (being removed) + [[nodiscard]] (warning-level) were C++17 sources → full test suite can be -std=c++11
6. cxxkit `3rdparty/<lib>/` layout verified; source at .refinfo/cxxkit/3rdparty/expected-1.2.0.tar.gz (git-ignored → vendor copy must be committed)

## Design Final

### D1 vendor layout (mirrors cxxkit)
`bindings/cxx/include/mediaservo/3rdparty/tl/expected.hpp` (unmodified verbatim) + `3rdparty/NOTICE` (version/source/do-not-modify)

### D2 shared header (~20 lines, alias approach, nodiscard macro dropped)
`bindings/cxx/include/mediaservo/detail/result.hpp`:
```cpp
#include <mediaservo/3rdparty/tl/expected.hpp>
namespace mediaservo {
struct Error { int code; std::string message; };
template <typename T> using Result = tl::expected<T, Error>;
template <> using Result<void> = tl::expected<void, Error>;
}
```
Rationale for dropping the nodiscard macro: attributes cannot decorate aliases; tl::expected ships TL_EXPECTED_NODISCARD (C++17+), already covered.

### D3 rewrite pattern table (three headers + three tests, single source of truth)
| Current API | After migration |
|---|---|
| `Result<T>::success(v)` | `Result<T>(std::move(v))` |
| `Result<T>::failure(e)` | `Result<T>(tl::unexpect, e)` |
| `Result<void>::success()` | `Result<void>()` |
| `.ok()` / `.has_error()` | `.has_value()` / `!.has_value()` |
| `if (r)` / `if (!r)` | `if (r.has_value())` / `if (!r.has_value())` |
| catch std::logic_error (misuse) | catch tl::bad_expected_access<Error> or std::exception (details via e.error()) |
| `r.value()` / `.error().code/.message` | unchanged (exception type changes; Error struct preserved) |
| Header-comment misuse contract | Change to bad_expected_access + change note |

### D4 Test standard — full `-std=c++11`

## Execution Steps

- **T1** vendor: 3rdparty/tl/expected.hpp + NOTICE (unpack .refinfo/cxxkit/3rdparty/expected-1.2.0.tar.gz)
  verify: `g++ -std=c++11 -fsyntax-only -I bindings/cxx/include ...` zero errors zero warnings + version grep
- **T2** shared header detail/result.hpp (dual-standard C++11+C++17 smoke compile)
- **T3** contract tests bindings/cxx/tests/test_result_common.cpp (RED expected): static_assert value semantics/alias wiring/cross-SDK is_same + runtime + bad_expected_access + multi-SDK single-TU regression
- **T4** full migration (**atomic step**): three headers drop local implementations + include shared header + using + site-wide rewrite (field 19/link 39/deck 50 sites) + three tests in sync + test-cxx.sh (-std=c++11 + -I bindings/cxx/include + 4th block contract tests)
  verify: test-cxx 4 PASS + zero-residue grep (`.ok()\|::success(\|std::logic_error` site-wide 0 output) + `std::variant` 0
- **T5** parity_bindings.sh + e2e-bindings.sh: -std=c++11 + -I (e2e C++ step pushes to a real server = strongest consumer validation)
- **T6** install bindings: rglob copy of bindings/cxx/include/ (detail/ + 3rdparty/ + NOTICE); verify: three files exist after install
- **T7** docs 23-binding-guide: structure tree/compile commands/C++11 promise/contract change declaration/CC0 attribution/future std::expected swap path
- **T8** full gates: build-c + test-cxx + parity-bindings + abi-drift + installed-prefix consumer C++11 smoke; e2e-bindings (needs server)
- **T9** single atomic commit (all of T1-T8)

## Dependency Graph

```
T1 → T2 → T3(RED) → T4(atomic: T4a×3 ∥ T4b×3 ∥ T4c) → T5(2 parallel) → T6 ∥ T7 → T8 → T9(single commit)
```

## Risk Register

| Risk | Mitigation |
|---|---|
| Exception contract change | Explicit header comments + doc declaration + catch(std::exception) compatibility verified; zero consumers |
| Site missed after factory removal | T4 verify zero-residue grep, site-wide 0 output |
| what() is not Error.message | Docs note details via e.error().code/.message; contract tests only assert the catch type |
| Alias has no nodiscard | tl built-in TL_EXPECTED_NODISCARD |
| Multi-SDK single TU | T3 contract tests include all three headers in regression |
| Install layout lacks hierarchy | T6 rglob + mkdir + T8 consumer smoke |
| vendor drift | NOTICE do-not-modify declaration + T1 version grep; upgrades re-vendor |
