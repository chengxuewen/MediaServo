---
name: security-hardening
description: "MediaServo security audit and hardening. OWASP Top 10 checks, hardcoded secrets/ports/URLs scan (merged review-hardcode), PSK/JWT auth flow review, WebSocket security, secrets management (PIT-10 lesson), mediasoup SFU transport security. Use before release, after auth changes, or when onboarding new PSK keys. Also accessible via /review-hardcode."
---

# security-hardening — Security Hardening

> OWASP Top 10 + PSK/JWT auth + WebSocket security + secrets management.
> Every rule has a check command. Every check must pass.

## Trigger Conditions

- before a release (release candidate)
- after auth-module changes
- new WebSocket endpoint
- new PSK key / JWT secret
- user says "security review" / "安全审计" (security audit) <!-- c49:trigger -->
- new FFI boundary (C/ObjC bridge)

## Mode A: Audit Mode

Full security audit, manually triggered. Runs all 6 Phases and generates an audit report.

## Mode B: Guard-while-building (PreToolUse)

> Auto-triggered on code changes, blocking insecure commits. Lightweight, target <5s.
> Reference: JoeyPatricio/security-hardening-skill — PreToolUse guard pattern

### B1: cargo-audit Quick Scan

```bash
# check only newly-appeared vulnerabilities (not a full audit)
cargo audit --quiet --deny unsound 2>&1 | head -20
```

| Check | Command | Pass criteria |
|--------|------|---------|
| known vulnerabilities | `cargo audit --quiet` | 0 RUSTSEC warnings |
| deny unsound | `cargo audit --deny unsound` | exit 0 |

### B2: Unsafe Block Scan

```bash
# every unsafe block must carry a // SAFETY: comment
UNSAFE_NO_COMMENT=$(grep -rn 'unsafe' crates/ --include='*.rs' | grep -v '// SAFETY:' || true)
if [ -n "$UNSAFE_NO_COMMENT" ]; then
  echo "ERROR: unsafe block without SAFETY comment:"
  echo "$UNSAFE_NO_COMMENT"
  exit 1
fi
```

### B3: gitleaks Secret Detection

```bash
# install (one-time)
# brew install gitleaks  # macOS
# or: go install github.com/gitleaks/gitleaks/v8@latest

# PreToolUse check: scan staged changes
gitleaks detect --source . --no-git --redact --verbose 2>&1 | head -30
```

### B4: Hardcode Quick Scan (lightweight)

```bash
# scan only changed files (not the whole repo)
git diff --cached --name-only -- '*.rs' | xargs -I{} grep -n 'token.*=\|password.*=\|api[_-]key' {} 2>/dev/null || true
```

### Guard Execution Order

```text
PreToolUse (on edit .rs/.toml):
  1. unsafe-block comment check  (<0.5s)
  2. hardcode quick scan         (<1s)
  3. gitleaks detect             (<2s)
  4. cargo audit                 (<3s)

any failure → block the operation
```

### gitleaks Config (.gitleaks.toml)

```toml
# .gitleaks.toml — project-level config
title = "MediaServo gitleaks config"

[extend]
useDefault = true

[allowlist]
description = "Known false positives"
paths = [
  "scripts/scan-hardcode.sh",  # test patterns
  ".env.example",               # placeholder values
]

[[rules]]
id = "custom-psk-pattern"
description = "MediaServo PSK keys"
regex = '''(?i)(psk|pre_shared_key)\s*=\s*["'][A-Za-z0-9+/]{32,}["']'''
```

### gitleaks pre-commit Hook (.git/hooks/pre-commit)

> Reference D201: the existing pre-commit hook runs `cargo fmt --check` + `cargo clippy -- -D warnings`.
> Below is the extension integrating gitleaks into the same hook:

```bash
#!/usr/bin/env bash
# .git/hooks/pre-commit — Rust quality gate + gitleaks secret detection
set -euo pipefail

STAGED_RS=$(git diff --cached --name-only --diff-filter=ACM -- '*.rs' || true)
STAGED_TOML=$(git diff --cached --name-only --diff-filter=ACM -- '*.toml' || true)

# ---- gitleaks secret detection (whole repo, only when .rs/.toml changed) ----
if [ -n "$STAGED_RS" ] || [ -n "$STAGED_TOML" ]; then
  if command -v gitleaks &>/dev/null; then
    echo "→ gitleaks: scanning staged changes..."
    { gitleaks detect --source . --no-git --redact --verbose 2>&1; } || {
      echo ""
      echo "ERROR: gitleaks detected secrets in staged files."
      echo "  Review findings above. False positive? Add to .gitleaks.toml allowlist."
      echo "  To bypass (emergency only): GITLEAKS_SKIP=1 git commit ..."
      exit 1
    }
  else
    echo "⚠ gitleaks not installed. Install: brew install gitleaks"
  fi
fi

# ---- Rust quality gate (only when .rs changed) ----
if [ -n "$STAGED_RS" ]; then
  echo "→ cargo fmt: checking..."
  cargo fmt --check
  echo "→ cargo clippy: checking..."
  cargo clippy -- -D warnings
fi

echo "✅ pre-commit checks passed"
```

### gitleaks Installation

| Platform | Command |
|------|------|
| macOS | `brew install gitleaks` |
| Linux | `go install github.com/gitleaks/gitleaks/v8@latest` or download a [release binary](https://github.com/gitleaks/gitleaks/releases) |
| Docker | `docker run -v $(pwd):/path zricethezav/gitleaks detect --source /path` |

## Phase 1: Secrets Scan (PIT-10 enforced)

```bash
# run the hardcode scan
./scripts/scan-hardcode.sh

# supplementary manual checks
grep -rn 'api[_-]key\|api[_-]secret\|token.*=\|password.*="' crates/ --include='*.rs' | grep -v '//.*TODO' | grep -v 'env::var'
grep -rn 'sk-\|pk-\|AKID\|SecretId' crates/ --include='*.rs'
grep -rn 'apiKey\|api_key\|API_KEY' .opencode/ --include='*.json' --include='*.jsonc'
```

### PIT-10 Rule

> Hardcoded API keys in global config leak. Use environment-variable interpolation: `"apiKey": "{env:NEW_API_KEY}"`.

| Check | Command | Pass criteria |
|--------|------|---------|
| no hardcoded secrets | `scripts/scan-hardcode.sh` | 0 CRITICAL |
| env-var interpolation | `grep -rn '{env:' .opencode/` | every apiKey uses interpolation |
| .gitignore coverage | `grep '\.env' .gitignore` | includes .env, .env.local |
| example-file placeholders | `grep 'EXAMPLE_KEY\|your-key-here' .env.example` | no real keys |

### Hardcoded-Value Severity (formerly review-hardcode)

This skill absorbed the full scanning capability of `review-hardcode`. The `/review-hardcode` command points here.

| Pattern | Severity | Notes |
|------|--------|------|
| `token="..."` / `password="..."` / `secret="..."` / `api_key="..."` | 🔴 CRITICAL | hardcoded secrets/tokens |
| hardcoded ports like `:9800` | 🟠 HIGH | production ports must not be hardcoded |
| `localhost:PORT` / `127.0.0.1:PORT` | 🟠 HIGH | address+port should come from config |
| `http://IP` hardcoded-IP URLs | 🟡 MEDIUM | use config or DNS instead |

### Exclusion Rules

The scan auto-excludes: `target/`, `node_modules/`, `.git/`, `.pixi-cache/`.
Hardcoded values marked with `TODO:` (tolerated temporarily) require manual review to decide whether to ignore.

## Phase 2: Auth Flow Audit

### PSK Authentication (Host↔Server)

```rust
// MediaServo PSK flow:
// Host ──[PSK in WS header]──> Server ──[validate]──> Session token

// checkpoints:
// 1. Is the PSK injected via environment variable? (not plaintext in a config file)
// 2. Is the PSK >= 32 bytes?
// 3. Does the server rate-limit PSK validation? (brute-force protection)
// 4. Does the session token expire?
// 5. Are PSK-error responses vague? (never leak "user exists" or "key is close")
```

```bash
# check commands
grep -rn 'pre_shared_key\|psk' crates/mediaservo-server/src/ --include='*.rs'
grep -rn 'env::var.*PSK\|env::var.*SECRET' crates/ --include='*.rs'
grep -rn 'session.*ttl\|token.*expir\|jwt.*exp' crates/mediaservo-common/src/auth/ --include='*.rs'
grep -rn 'rate.limit\|429\|too.many' crates/mediaservo-server/src/ --include='*.rs'
```

### JWT Authentication (Admin UI)

```bash
# check commands
# 1. is the JWT secret >= 256 bit?
grep -rn 'jwt.*secret\|JWT_SECRET' crates/ --include='*.rs' | grep -v env::var

# 2. confirm the algorithm is HS256 or RS256 (never none)
grep -rn 'Algorithm\|alg.*HS\|alg.*RS' crates/mediaservo-common/src/auth/ --include='*.rs'

# 3. confirm an exp claim exists
grep -rn 'exp\|expir' crates/mediaservo-common/src/auth/ --include='*.rs'

# 4. confirm refresh-token rotation exists
grep -rn 'refresh_token\|refresh' crates/mediaservo-server/src/ --include='*.rs'
```

## Phase 3: WebSocket Security

```bash
# check commands
# 1. message-size limit (OOM protection)
grep -rn 'max.*message\|max.*frame\|message.*size' crates/mediaservo-server/src/ --include='*.rs'

# 2. connection rate limiting
grep -rn 'connection.*limit\|max_connections\|concurrent' crates/mediaservo-server/src/ --include='*.rs'

# 3. Origin validation
grep -rn 'origin\|allowed_origin\|verify_origin' crates/mediaservo-server/src/ --include='*.rs'

# 4. TLS (production)
grep -rn 'wss://\|tls\|ssl_config' crates/mediaservo-server/src/ --include='*.rs'
```

### Message Injection Defense

```rust
// all WS message deserialization must use serde strict mode
// WRONG: serde_json::from_str(&raw) — extra fields unvalidated
// CORRECT: serde_json::from_str::<StrictSchema>(&raw) or deny_unknown_fields
```

```bash
grep -rn '#\[serde(deny_unknown_fields)\|unknown_fields\|additional_properties' crates/ --include='*.rs'
grep -rn 'serde_json::from_slice\|serde_json::from_str' crates/mediaservo-server/src/ --include='*.rs' --include='*.rs'
```

## Phase 4: mediasoup SFU Security

```bash
# check commands
# 1. is the RTP port range bounded (not 0-65535)?
grep -rn 'rtc_min_port\|rtc_max_port\|rtp_port' crates/mediaservo-server/src/ --include='*.rs'

# 2. do WebRTC transports require auth?
grep -rn 'web_rtc_server\|webRtcTransport\|transport.*auth' crates/mediaservo-server/src/ --include='*.rs'

# 3. does room creation require permission?
grep -rn 'create_room\|router.*create' crates/mediaservo-server/src/sfu/ --include='*.rs'

# 4. producer/consumer permission isolation
grep -rn 'peer_id\|producer.*peer\|consumer.*peer' crates/mediaservo-server/src/sfu/ --include='*.rs'
```

## Phase 5: Dependency Audit

```bash
# run cargo-deny for known advisories
cargo deny check advisories

# run cargo-audit
cargo audit

# check unsafe usage
grep -rn 'unsafe' crates/ --include='*.rs' | grep -v '// SAFETY:'
# rule: every unsafe block must carry a // SAFETY: comment explaining why it is sound
```

## Phase 6: Admin UI Security

```bash
# check commands
# 1. does the Dashboard require authentication?
grep -rn 'auth\|login\|redirect.*login' crates/mediaservo-server/src/admin/ --include='*.rs'

# 2. is there CSRF protection?
grep -rn 'csrf\|xsrf\|same_site' crates/mediaservo-server/src/ --include='*.rs'

# 3. is CORS strict?
grep -rn 'access-control\|allow_origin\|cors' crates/mediaservo-server/src/ --include='*.rs'

# 4. Content Security Policy
grep -rn 'content-security\|CSP\|frame-ancestors' crates/mediaservo-server/src/admin/ --include='*.rs'
```

## Security Checklist (OWASP Top 10 aligned)

| # | Check | MediaServo counterpart | Command | Required |
|---|--------|-------------|------|:---:|
| A01 | broken access control | Auth trait impls complete | `grep -rn 'TODO\|FIXME' crates/*/src/auth/` | ✅ |
| A02 | cryptographic failures | PSK >=32B, TLS wss:// | `grep -rn 'wss://' crates/` (production check) | ✅ |
| A03 | injection | WS messages strict deserialization | `grep -rn 'deny_unknown' crates/` | ✅ |
| A04 | insecure design | rate limiting + timeouts | `grep -rn 'rate.limit\|timeout' crates/` | ✅ |
| A05 | misconfiguration | no debug mode in production | `grep -rn 'debug_assert\|cfg(debug)' crates/` | ✅ |
| A06 | vulnerable components | cargo-audit passes | `cargo audit` | ✅ |
| A07 | authentication failures | PSK + JWT dual mode | all Phase 2 checks | ✅ |
| A08 | software/data integrity | FlatBuffers verification | (Phase 2+) | ✅ |
| A09 | logging/monitoring failure | audit log | `grep -rn 'audit\|security.log' crates/` | ⚠️ |
| A10 | SSRF | no server-side HTTP fetches | `grep -rn 'reqwest\|hyper::Client' crates/mediaservo-server/` | ✅ |

## Report Format

```
## Security Audit Report — [date]

### Phase 1: Secrets Scan
✅ scan passed: 0 CRITICAL, 0 HIGH, 0 MEDIUM

### Phase 2: Auth Flow
✅ PSK comes from env var (OMSP_PSK)
✅ PSK length: 64 bytes
⚠️ Session TTL: 24h (recommend 2h)
❌ no rate limiting (P0)

### Phase 3: WebSocket
✅ message size limit: 1MB
❌ no Origin validation (CSWSH-attackable)
⚠️ TLS enabled only in Docker environments

### Phase 4: mediasoup
✅ RTP ports: 40000-40100
✅ Room requires auth token
⚠️ Producer has no bandwidth cap

### Phase 5: Dependencies
✅ cargo-audit: 0 vulnerabilities
✅ cargo-deny: 0 unlicensed

### Summary
CRITICAL: 0 | HIGH: 1 | MEDIUM: 2
Fix plan: [sorted by severity]
```

## Forbidden

- skipping the PIT-10 anti-pattern: hardcoded keys are never acceptable
- silently swallowing errors: auth failures must be logged but must never leak the secret
- ignoring cargo-audit warnings: every RUSTSEC advisory requires a decision record
- using debug configuration in production: `debug_assert!` doesn't execute in release builds
- plaintext HTTP WebSockets: production must use `wss://` or a controlled internal network
