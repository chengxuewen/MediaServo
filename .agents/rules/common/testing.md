# Testing Requirements

## Minimum Test Coverage: 80%

Test Types (ALL required):
1. **Unit Tests** - Individual functions, utilities, components
2. **Integration Tests** - API endpoints, database operations
3. **E2E Tests** - Critical user flows (framework chosen per language)

## Test-Driven Development

MANDATORY workflow:
1. Write test first (RED)
2. Run test - it should FAIL
3. Write minimal implementation (GREEN)
4. Run test - it should PASS
5. Refactor (IMPROVE)
6. Verify coverage (80%+)

## Troubleshooting Test Failures

1. Use **tdd-guide** agent
2. Check test isolation
3. Verify mocks are correct
4. Fix implementation, not tests (unless tests are wrong)

## Agent Support

- **tdd-guide** - Use PROACTIVELY for new features, enforces write-tests-first

## Test Structure (AAA Pattern)

Prefer Arrange-Act-Assert structure for tests:

```typescript
test('calculates similarity correctly', () => {
  // Arrange
  const vector1 = [1, 0, 0]
  const vector2 = [0, 1, 0]

  // Act
  const similarity = calculateCosineSimilarity(vector1, vector2)

  // Assert
  expect(similarity).toBe(0)
})
```

### Test Naming

Use descriptive names that explain the behavior under test:

```typescript
test('returns empty array when no markets match query', () => {})
test('throws error when API key is missing', () => {})
test('falls back to substring search when Redis is unavailable', () => {})
```

## Executable Checks

```bash
# Verify: tests pass
cargo test --workspace
# Verify: coverage
cargo tarpaulin --workspace 2>/dev/null || echo "tarpaulin not installed"
```

## Zero-Count Assertions Need a Positive Control First

**Rule**: before reporting any "0 warnings / 0 errors / 0 matches" result, first prove the
target path was actually exercised (a positive control: `grep -c '<execution-marker>' > 0`,
or a seeded defect that must be caught); only then does a `== 0` check count as a pass.
A bare zero count is not verification -- the scan may simply not have run.

**Precedent**: the zero-defined-symbols verdict for `libmediaservo_field.so` was only
trustworthy because the same round had already matched a known symbol as a positive control --
luck got it right, the rule exists so it does not depend on luck.

**Verify**: the positive control's output (`> 0`) is captured in the same transcript as the
zero assertion; no control, no claim.

**Blocking condition**: declaring a zero-count pass without a positive control in the same round.
