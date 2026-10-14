---
name: test-harness
description: "MediaServo multi-language automated test harness. Generates test skeletons from SDD specs (Rust/TS/Python/C++/C), enforces AAA pattern, test→spec reverse tracing, coverage reports. Interactive menu-driven. Supports Phase awareness (skips not-ready modules)."
---

# Test Harness

Automated test generation and verification for the multilingual MediaServo project. Test code is produced directly from SDD specs, ensuring AAA pattern, Phase alignment, and consistent language conventions.

**Philosophy**: tests are not an afterthought bolted onto code — they grow directly from the spec. A good test file = an executable copy of the spec.

---

## Entry: Test Task Types

### `/test-harness` (no arguments)
Pops the task-type menu:

```
[1] SDD→tests — generate test files from specs (stubs / AAA skeletons / full fill)
[2] tests→spec — reverse tracing (verify coverage / missing scenarios)
[3] run & fix — run tests and fix failing cases
[4] incremental tests — generate incremental tests from the current git diff
[5] project init — set up test infrastructure for a new module
[6] coverage report — generate coverage analysis and gaps
[7] selective testing — run only tests affected by changes (git diff driven)
```

### `/test-harness generate`
Skip the menu, go straight to SDD→test generation mode.

### `/test-harness run`
Skip the menu, run tests directly.

### `/test-harness quick`
Only fix currently failing tests; do not generate new ones.

---

## Multi-Language Strategy

MediaServo is a multi-language project (D19, D21). Test generation must adapt to each language's conventions.

### Language Detection

Auto-detect the target language from project structure:

| Signal | Detected language |
|------|---------|
| `Cargo.toml` present | Rust |
| `package.json` + `tsconfig.json` | TypeScript |
| `pyproject.toml` / `setup.py` | Python |
| `CMakeLists.txt` + `.cpp`/`.hpp` | C++ |
| `CMakeLists.txt` + `.c`/`.h` (no .cpp) | C |
| multiple signals | interactive choice |

### Per-Language Test Conventions

#### Rust
- **Unit tests**: `#[cfg(test)] mod tests { ... }` inline in source files
- **Integration tests**: `tests/` directory, each file a separate crate
- **Mocks**: `mockall` crate, `mock!` macro
- **Assertions**: `assert_eq!` / `assert!` / `assert!(matches!(...))`
- **Run**: `cargo test`
- **Conventions**:
  - function naming: `test_<module>_<scenario>`
  - `unwrap()` / `expect()` forbidden — use `?` or `assert!(result.is_ok())`
  - test modules start with `use super::*;`

#### TypeScript
- **Unit tests**: `*.test.ts` or `*.spec.ts` alongside source files
- **Framework**: Vitest (preferred) / Jest
- **Mocks**: `vi.mock()` / `jest.mock()`
- **Assertions**: `expect(x).toBe(y)` / `expect(x).toEqual(y)`
- **Run**: `npx vitest` / `npx jest`
- **Conventions**:
  - `describe('ModuleName', () => { it('should ...', () => { ... }) })`
  - AAA comments: `// Arrange` / `// Act` / `// Assert`

#### Python
- **Unit tests**: `test_*.py` alongside source or in `tests/`
- **Framework**: pytest
- **Mocks**: `unittest.mock` / `pytest-mock`
- **Assertions**: plain `assert` (pytest style)
- **Run**: `pytest`
- **Conventions**:
  - function naming: `test_<module>_<scenario>`
  - class organization: `class Test<Module>:`
  - fixtures: `@pytest.fixture`

#### C++
- **Unit tests**: `*_test.cpp` in `tests/`
- **Framework**: GoogleTest (gtest/gmock)
- **Mocks**: `MOCK_METHOD` macro
- **Assertions**: `EXPECT_EQ` / `ASSERT_TRUE`
- **Run**: `ctest` or `cmake --build build && ctest --test-dir build`
- **Conventions**:
  - `TEST(TestSuiteName, TestName) { ... }`
  - AAA comments

#### C
- **Unit tests**: `test_*.c` in `tests/`
- **Framework**: Unity / CMock
- **Run**: `ctest` or `make test`
- **Conventions**:
  - `TEST_ASSERT_EQUAL(expected, actual)`
  - `setUp() / tearDown()` lifecycle

---

## Workflow

### Mode 1: SDD → Test Generation

```
/test-harness generate
```

#### Generation Levels (asked every time)

1. **stubs**: function signatures only + `todo!()` / `fail()` / `pytest.fail()` — compiles, tests fail
2. **AAA skeletons**: Arrange/Act/Assert comments + placeholders → structure ready, assertions to fill
3. **full fill**: concrete values extracted from the spec, runnable complete assertions — expected to pass directly

#### Step 1: Identify Spec Sources

Auto-scan the `openspec/specs/` directory:
```
openspec/specs/
├── hal-type-system-spec.md       → 30 items (S-TYPE-*)
├── hal-qos-spec.md               → 30 items (S-QOS-*)
├── config-barrier-spec.md        → 24 items (S-CB-*)
└── hal-protocol-spec.md          → 37 items (S-PROTO-*)
```

Let the user choose spec files (single or multi-select).

#### Step 2: Phase-Aware Filtering

Read `docs/plans/p0-milestone-roadmap.md`, determine the current Phase:

| Phase | Available specs |
|-------|---------|
| Phase 0 (CI) | type system (S-TYPE) — pure logic, no trait dependencies |
| Phase 1 (hal-core) | S-TYPE + S-QOS + S-CB + S-PROTO — traits ready |
| Phase 2+ | everything |

Auto-filter spec items:
- items whose Phase isn't ready → mark ⏭️ skip, generate an explanatory comment
- priority A (P0) → generate first, full fill
- priority B/B+/C → stubs or AAA skeletons

#### Step 3: Generate Test Files

For each spec item, generate a test function in the matching language:

```
Input: S-TYPE-001
  - preconditions: Bool = true / false
  - operation: encode → decode
  - expected: true ↔ true, false ↔ false
  - test mapping: test_type_01_bool_roundtrip

Output (Rust):
  #[test]
  fn test_type_01_bool_roundtrip() {
      // Arrange
      let values = vec![true, false];
      // Act
      for val in values {
          let encoded = encode_bool(val);
          let decoded = decode_bool(&encoded);
          // Assert
          assert_eq!(val, decoded, "Bool roundtrip failed");
      }
  }
```

#### Step 4: Write and Verify

1. write the test files (unit tests inline / integration tests in new files)
2. run compile check (no syntax errors)
3. run tests (expected: some pass, some fail → mark pending implementation)
4. report: `generated N test functions → M pass / K fail / P skip`

---

### Mode 2: Tests → Spec Reverse Tracing

```
/test-harness trace
```

Check existing test coverage, cross-referenced against SDD specs:

1. scan all test files
2. extract test function names → map to spec IDs (e.g. `test_type_01_*` → S-TYPE-001)
3. produce a coverage matrix:

```
Spec ID     | Test function                          | Status
-----------|------------------------------------|------
S-TYPE-001 | test_type_01_bool_roundtrip        | ✅
S-TYPE-002 | test_type_02_s8_roundtrip          | ✅
S-TYPE-003 | —                                  | ❌ missing
S-CB-007   | test_cb_07_partial_failure          | ⚠️ incomplete
```

4. mark:
   - ❌ missing → recommend generating from SDD
   - ⚠️ incomplete → boundary conditions not covered
   - ✅ complete

---

### Mode 3: Run & Fix Failing Cases

```
/test-harness run
```

1. run the current project test suite
2. collect the failure list
3. for each failure:
   - read the test source
   - read the corresponding implementation source
   - determine root cause: test bug vs implementation bug
   - auto-fix test bugs (wrong assertions, missing mocks)
   - flag implementation bugs → report to the user

**Root-cause rules**:
- test logic disagrees with the spec → test bug
- wrong assertion value (expects 1 but the spec says 2) → test bug
- implementation incomplete / interface changed → implementation bug

**Never**: modify implementation code to make a test pass (unless the user explicitly asks).

---

### Mode 4: Incremental Tests

```
/test-harness incremental
```

Identify changes from `git diff` → generate matching tests:

1. `git diff --name-only` to get changed files
2. reverse-map to SDD specs (file path → module → spec ID)
3. generate incremental tests only for spec items related to the changes
4. if a changed file has no tests yet → initialize a test file
(note: Mode 7 runs affected tests; Mode 4 generates incremental tests)

---

### Mode 5: Project Initialization

```
/test-harness init
```

Create test infrastructure for a new module:

**Rust**:
- `tests/` directory + integration-test entry points
- crate-level `#[cfg(test)]` helper module
- `mockall` dependency check

**TypeScript**:
- `vitest.config.ts` / `jest.config.ts`
- `__tests__/` directory
- test setup files

**Python**:
- `tests/__init__.py` + `conftest.py`
- pytest config (pyproject.toml)

**C++**:
- `CMakeLists.txt` test targets
- `tests/` directory + CMakeLists.txt
- gtest integration

**C**:
- `CMakeLists.txt` test targets
- Unity/CMock framework download

---

### Mode 6: Coverage Report

```
/test-harness coverage
```

1. run tests with coverage
2. aggregate and display per module:

```
Module              | Line cov  | Branch cov | Spec cov
-------------------|----------|----------|----------
hal-type-system    | 92%      | 85%      | 28/30
hal-qos            | 78%      | 71%      | 22/30
config-barrier     | 65%      | 58%      | 18/24
hal-protocol       | 88%      | 82%      | 34/37
───────────────────|──────────|──────────|────────
Total              | 81%      | 74%      | 102/121
```

3. rank gaps: uncovered spec items / low-branch-coverage functions
4. recommend: the N tests to add first

---

### Mode 7: Selective Test Runner

```
/test-harness selective
```

> Note: mediaservo and hal-flatbuffers are Phase 1 stub crates (M0.3) with no real test targets today. Mode 7 uses `cargo metadata` to check crate existence before suggesting test commands.
Identify changes from `git diff` → run only the affected tests (not a gate — a scheduling tool):

1. `git diff --name-only HEAD` to get changed files
2. map files to test targets:

| Changed path matches | Test command |
|---|---|
| `crates/mediaservo-common/` | `cargo test -p mediaservo-common` |
| `crates/mediaservo/` | `cargo test -p mediaservo` (Phase 1, stub only) |
| `crates/hal-flatbuffers/` | `cargo test -p hal-flatbuffers` (Phase 1, stub only) |
| `*.fbs` (FlatBuffers schema) | `cargo test -p hal-flatbuffers` |
| `Cargo.toml` or `Cargo.lock` | `cargo test --workspace` |
| cross-crate (3+ crates) | `./scripts/qa-fast.sh` |

3. run tests (one layer at a time, avoid cargo lock contention)
4. report: layer / command / test count / passed / failed / duration

**Never**: run `cargo test` for multiple crates simultaneously. Do not diagnose failures — report only. >20 changed files → suggest `cargo test --workspace`.

---

## MediaServo-Specific Test Patterns

### HAL Trait Tests

All HAL core trait tests use `MockHalTransport` (`mediaservo`):

```rust
// Rust pattern
#[test]
fn test_signal_write_read() {
    // Arrange
    let mut transport = MockHalTransport::new();
    let signal = Signal::new("test.value", HalValue::S32(42));

    // Act
    transport.write_signal(&signal).unwrap();
    let result = transport.read_signal("test.value").unwrap();

    // Assert
    assert_eq!(result, HalValue::S32(42));
}
```

### HalQoS Security-Domain Tags

```rust
#[test]
fn test_qos_security_domain_hierarchical() {
    let tag = SecurityDomain::parse("l1.control.reactor_a").unwrap();
    assert!(tag.matches("l1.*"));
    assert!(tag.matches("l1.control.*"));
    assert!(!tag.matches("l2.*"));
}
```

### Config Barrier State Machine

```rust
#[test]
fn test_config_barrier_state_machine() {
    let mut barrier = ConfigBarrier::new();
    assert_eq!(barrier.state(), BarrierState::Idle);

    // Queue config change
    barrier.queue(ConfigChange::UpdateSignal { ... });
    assert_eq!(barrier.state(), BarrierState::Pending);

    // Apply at cycle boundary
    barrier.apply().unwrap();
    assert_eq!(barrier.state(), BarrierState::Idle);
}
```

### FlatBuffers Round-Trip

```rust
#[test]
fn test_halvalue_fbs_roundtrip() {
    let original = HalValue::S32(42);
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let offset = original.serialize(&mut builder);
    builder.finish(offset, None);
    let buf = builder.finished_data();
    let restored = HalValue::deserialize(buf).unwrap();
    assert_eq!(original, restored);
}
```

---

## Interactive Mode

All test-generation operations show a diff preview before writing files, for user confirmation:

```
Will generate the following changes:
  crates/mediaservo-common/src/types.rs +45 (inline test module)
  tests/integration/test_type_roundtrip.rs (new file, 150 lines)
  tests/integration/test_qos_security.rs (new file, 80 lines)

Total: 3 files, +275 lines, 30 test functions

Execute? [Y/n]
```

---

## Phase-Awareness Rules

Auto-read `docs/plans/p0-milestone-roadmap.md` to confirm the current milestone.

| Checkpoint | Behavior |
|--------|------|
| trait required by the spec not yet defined | generate a stub or skip, mark "⏭️ Phase 2" |
| priority A (P0 mandatory) | full fill + assertions |
| priority B/B+ | AAA skeleton |
| priority C | stub placeholder |

When the Phase changes, re-run `generate` → previously skipped tests get auto-filled.

---

## Quality Rules

1. **One assertion target per test** — one test function validates one spec item
2. **AAA comments must be explicit** — `// Arrange` / `// Act` / `// Assert` are not optional
3. **Never bend a test to fit the implementation** — spec > test > implementation (precedence)
4. **No unwrap/expect** — avoid them in tests too; use `assert!(result.is_ok())`
5. **Boundary conditions first** — every entry in the spec's boundary-conditions field → its own test
6. **Traceable naming** — test function names include the spec ID (e.g. `test_type_01_*`)
7. **Phase alignment** — never generate tests that can't run in the current Phase

---

## Quick Reference

```
/test-harness               → choose task type
/test-harness generate      → SDD→test generation
/test-harness run           → run and fix tests
/test-harness trace         → tests→spec reverse tracing
/test-harness incremental   → incremental tests (git diff)
/test-harness init          → initialize test infrastructure
/test-harness coverage      → coverage report
/test-harness selective    → selective test runner (git diff)
```

### Language Command Cheat Sheet

| Language | Run tests | Coverage | Mock library |
|------|---------|---------|---------|
| Rust | `cargo test` | `cargo tarpaulin` | mockall |
| TS | `npx vitest` | `npx vitest --coverage` | vi.mock() |
| Python | `pytest` | `pytest --cov` | pytest-mock |
| C++ | `ctest` | `gcov + lcov` | gmock |
| C | `ctest` | `gcov + lcov` | CMock |
