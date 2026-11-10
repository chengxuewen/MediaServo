# 2026-08-21 app-branding-customization — Application-layer branding customization

**Status**: Plan pending review | **Boundary**: SDK bindings and the wire protocol are frozen; host/client/server get customizable branding
**Dependencies**: D247 (C ABI symbol prefix), D240 (soname/ABI discipline), D243 (FrameMeta wire format), C32 (instance isolation trinity)

---

## 1. Proposal

### What

When MediaServo is consumed by third-party platforms as a foundation dependency (docs/architecture.md §1.2: static linking / standalone deployment embedding), the three application layers — host/client/server — should be customizable and brandable (white-label) without forking and renaming. Introduce a unified **Brand mechanism** (compile-time `MEDIASERVO_BRAND` + runtime env override of the same name, default = the current "mediaservo"): all user-visible strings, naming, layouts, and process topology of the application layers read through the Brand reader.

### Why

- The current `mediaservo-` prefix / fixed layouts are scattered across host.rs inline strings, translate.rs namespaces, cli.py install layouts, the host.toml template, and identity.json device prefixes — a white-label consumer today must fork and do a D209-style full rename (259 files / 1 day of cost, plus losing upstream evolution sync).
- The tension: bindings (C ABI symbol prefix D247) **must stay frozen** (multiple products coexist in one process); earlier design never distinguished "SDK contract layer frozen / application layer customizable" — this plan makes that boundary explicit.

### Scope

**In scope**:
- `mediaservo-common::brand` reader (env > compile-time option_env > default "mediaservo")
- host: help text/version strings, namespaces (oxfile app names `host-agent` → `<brand>-agent`), status filtering (host.rs:408), systemd unit name prefix (`oxmgr-<brand>-*`), identity.json device prefix (`ms-` → `<brand>-`, new keys only, no migration of existing), install layout (`/opt/<brand>-host`), default psk comment
- client: GUI window title/product name, default server_url
- server: admin panel title (www/apps/admin), default port, default PSK placeholder comment
- cli.py: `install host --brand` / `package host --brand` parameterization
- docs/modules branding guide

**Out of scope (frozen iron rules, zero changes in this plan)**:
- All of `bindings/*`: C ABI symbols `mediaservo_*` (D247), include/mediaservo/ header layout, cxx/py/node bindings, soname/ABI discipline (D240)
- Wire protocol: signaling SignalingMessage/SFU/RTP parameters, FrameMeta wire format (D243), DeviceStream SDP frames
- Crate names (`mediaservo-*` workspace members — dependency-surface identity; independent release goes via fork + D209-style rename)
- oxmgr itself (external tool) and the preserved docs/.agents surfaces

### Layers Affected

| Layer | Location | Impact |
|-------|----------|--------|
| Host application | `crates/mediaservo-host/src/bin/host.rs`, `translate.rs` | Strings/namespaces/units/device prefix → Brand |
| Client application | `crates/mediaservo-client/src/` | Title/default server_url |
| Server application | `crates/mediaservo-server/src/` + `www/apps/admin/` | Panel title/ports/PSK placeholder |
| SDK bindings | `bindings/*` | 🔒 frozen — verification gate only |
| Release scripts | `scripts/mediaservo_cli.py` | `--brand` parameter |

### Risks

1. **Device prefix change vs the G2 provisioning chain**: after branding, new instances use `<brand>-<12hex>` in identity.json — devices.yaml entries like the registered `ms-3d37e51f7703` stay compatible (default brand unchanged); branded deployments require re-provisioning (documented as additive — prefix migration not supported)
2. **Unit name switch residue**: after branding, the auto-stop enumeration prefix changes — old-brand units linger — install auto-stop enumerates and cleans both old and new prefixes (`oxmgr-host-*` + `oxmgr-<brand>-*`)
3. **Status filter semantics**: status (host.rs:408) filters by the current brand — old oxfiles (old namespace) don't show under a branded instance (correct behavior, documented); do not attempt mixed-reading compatibility
4. **Default behavior regression**: the default brand must cause zero behavior change — use existing test assertions (the `grep mediaservo-host` string assertions) as the regression gate

### Success Criteria

- [ ] Default (no env): `pixi run cargo test -p mediaservo-host` (~104) all green + `install host` succeeds + `git diff bindings/` **is empty** (frozen-boundary verification gate) + `scripts/e2e-install-host.sh` (:54 asserts version contains mediaservo-host) + `scripts/e2e-package.sh` (dist names + SDK lib/include names — frozen boundary)
- [ ] Under `MEDIASERVO_BRAND=cp`: `ps/status/monit` show `cp-agent` etc. app names, units `oxmgr-cp-*.service`, new identity keys `cp-<12hex>`, help text contains the brand name, install `--prefix /opt/cp-host` layout complete
- [ ] bindings four languages (c/cxx/py/node) symbols zero diff
- [ ] docs/modules branding guide written (checklist: customizable list vs frozen list)

### References

- docs/architecture.md §1.2 (third-party platform relationship: static linking / standalone deployment)
- D247/D248 (symbol prefix + manual maintenance gate), D240/D241 (soname/ABI), D243 (FrameMeta), D209 (rename precedent cost)

---

## 2. Design

### Architecture

```
env MEDIASERVO_BRAND ──┐
option_env!(compile)  ─┼──▶ common::brand::media_brand() -> &'static Brand
default "mediaservo"  ─┘        │
                                ▼
                     Brand { product, bin_prefix, unit_prefix,
                             device_prefix, default_psk }
                                │
        ┌───────────┬───────────┼───────────┬───────────┐
        ▼           ▼           ▼           ▼           ▼
   host.rs strings  translate.rs cli.py      client     server
   help/namespace   app names/   --brand    title/     title/
   /unit/status     oxfile      install    server_url  port/PSK
```

- Priority: runtime env (branded deployments skip recompilation) > compile-time option_env (CI one-shot white-label builds) > default "mediaservo" (current behavior)
- Brand is `&'static` (cached after first env read — constant for the process lifetime, avoiding concurrent env-read races)

**Default brand → legacy string mapping table (hard constraint of the zero-behavior-change gate — deriving directly from `<product>-` is forbidden)**:

| Brand field | Default "mediaservo" (legacy strings) | Non-default brand (e.g. cp) |
|---|---|---|
| App name prefix | `host-` (host-agent/.../host-streamer, 7 of them, translate.rs:107) | `<brand>-` (cp-agent) |
| Unit prefix | `oxmgr-host-` (host.rs:1039; cli.py:210) | `oxmgr-<brand>-` |
| Device prefix | `ms-` (identity.rs:21 — locked by identity.rs:92-94 unit test + tests/identity_cli.rs:28 assertion) | `<brand>-` (new keys only) |
| Namespace | `mediaservo-host` (host.rs:408) | `<brand>-host` |
| Product display name | `mediaservo-host` | the env value |

> I.e. the `product` field ≠ the legacy naming strings (by default product is a display name; the naming strings map independently) — otherwise the default-zero-change gate (~104 host tests + identity_cli assertions) would self-break.

### Files to Touch

| Op | File | Purpose |
|----|------|---------|
| Create | `crates/mediaservo-common/src/brand.rs` | Brand struct + reader + unit tests |
| Modify | `crates/mediaservo-host/src/bin/host.rs` | Help text/version strings/namespace filter/unit names/device prefix → Brand |
| Modify | `crates/mediaservo-host/src/translate.rs` | App name prefixes (host-agent → cp-agent), oxfile namespace |
| Modify | `crates/mediaservo-host/src/identity.rs` (DEVICE_ID_PREFIX) + bin/host.rs cmd_init | identity.json device prefix (new keys use brand; existing keys not migrated) |
| Modify | `scripts/mediaservo_cli.py` | `--brand` parameter, install layout, dual-prefix unit enumeration, dual shortcut names |
| Modify | `crates/mediaservo-client/src/` | Window title/default server_url |
| Modify | `crates/mediaservo-server/src/` + `www/apps/admin/` | Panel title/default port/PSK placeholder |
| Create | `docs/modules/24-app-branding.md` | Branding guide (customizable/frozen lists) |

### Integration Points

- **Brand boundary**: affects only the user-visible layer (strings/naming/layout) — never enters wire types, never enters FrameMeta, never enters the symbol table
- **host.rs:408 filter**: `p["namespace"] == brand.app_namespace()` (default "mediaservo-host")
- **Install layout**: `--brand cp` → default prefix `/opt/cp-host`, dual bin shortcuts `cp` + `cp-host` (symlinks to the mediaservo-host binary — one binary, multiple brands coexisting)
- **Unit names**: `oxmgr-<brand>-<abs-dir>.service` (host CLI self-managed startup — the name prefix is controllable)

### Error Handling

- env parse failure (invalid brand string: not [a-z0-9-]): `warn!` + fall back to default (does not block startup — branding is a display-layer concern)
- identity prefix: newly generated keys use the brand prefix; existing keys preserved as-is (idempotent — never overwritten) — consistent with existing `host init` idempotency semantics (C15-related; explicit error on failure)

### Testing Strategy

- Unit tests: `brand.rs` (env override priority, invalid-value fallback, default correctness) 3-4 cases
- Regression gate (default = zero behavior change): `cargo test -p mediaservo-host` all green + `git diff bindings/` empty + `install host` succeeds
- Branding verification (manual/scripted): `MEDIASERVO_BRAND=cp` start → ps/status/monit/unit/identity/help assertions (playwright `vehicle-*` spec or shell assertions)
- e2e: existing e2e under the default brand (e2e_sfu/field push/streamer) — branding does not touch the wire

### Dependencies

No new dependencies. Pure in-crate rework of existing crates.

---

## 3. Tasks

## Phase 1: Brand foundation (common)

- [ ] **`common/src/brand.rs` — Brand struct + media_brand() reader**
  - File: `crates/mediaservo-common/src/brand.rs` (+ lib.rs export)
  - Content: `Brand { product, bin_prefix, unit_prefix, device_prefix, default_psk }` + priority (env > option_env > default) + warn on invalid fallback
  - Verify: `cargo test -p mediaservo-common brand` (3-4 unit tests: default/override/invalid)

## Phase 2: host application-layer rework

- [ ] **host.rs user strings → Brand**
  - File: `crates/mediaservo-host/src/bin/host.rs`
  - Content: help text/version strings/`Usage:` prefix, shortcut name list (`host`/`<bin_prefix>-host`), status `namespace` filter (line 408)
  - Verify: `cargo check -p mediaservo-host` + under the default brand `-h` output matches the current state (diff comparison)

- [ ] **translate.rs app names/namespace → Brand**
  - File: `crates/mediaservo-host/src/translate.rs`
  - Content: app-name prefix generation (`host-agent` etc.), namespace injection (default "mediaservo-host")
  - Verify: all 14 translate tests green + under `MEDIASERVO_BRAND=cp` the oxfile generates `cp-agent`

- [ ] **identity device prefix (new keys)**
  - File: `crates/mediaservo-host/src/identity.rs` (DEVICE_ID_PREFIX) + bin/host.rs cmd_init
  - Content: new keys use `brand.device_prefix + "-<12hex>"`; existing keys are not migrated (idempotent)
  - Verify: `host init` idempotent (repeat generation yields the same key) + new instances under brand become `cp-<12hex>` + unit test

- [ ] **startup unit names + auto-stop dual-prefix enumeration**
  - File: `crates/mediaservo-host/src/bin/host.rs` startup_install + `scripts/mediaservo_cli.py` install auto-stop
  - Content: unit names `oxmgr-<brand>-<abs-dir>.service`; install enumerates and cleans both `oxmgr-host-*` + `oxmgr-<brand>-*`
  - Verify: startup on under brand → unit file exists; reinstall stops the old unit

- [ ] **cli.py `--brand` release parameter**
  - File: `scripts/mediaservo_cli.py`
  - Content: `install host --brand cp` → `/opt/cp-host` layout + dual shortcuts (`cp` + `cp-host` symlinks), `package host` package names
  - Verify: `install host --brand cp --prefix /tmp/cp-test` full flow + layout assertions

## Phase 3: client/server light branding

- [ ] **client title/default server_url**
  - File: `crates/mediaservo-client/src/`
  - Verify: `cargo check -p mediaservo-client` + defaults unchanged

- [ ] **server admin title/port/PSK placeholder**
  - File: `crates/mediaservo-server/src/` + `www/apps/admin/` (title constants)
  - Mechanism (C24 constraint — admin dist is compile-time include_bytes! embedded, **runtime env can't reach it**): compile-time `option_env!("MEDIASERVO_BRAND")` → build.rs injection / vite define; Docker deployments pass it through via build-arg (C13 — compose/Dockerfile add `ARG MEDIASERVO_BRAND` + build.rs rerun-if-changed)
  - Verify: `pnpm build` + after server rebuild the 9800 admin title contains the brand name; default unchanged (default branch untouched)

## Phase 4: verification regression

- [ ] **Default zero-behavior-change regression gate**
  - Verify: `cargo test -p mediaservo-host` (~104) + `install host` succeeds + **`git diff bindings/` is empty** (frozen-boundary verification)
- [ ] **Branding manual verification matrix**
  - Verify: `MEDIASERVO_BRAND=cp ./mediaservo-host ps/status/monit/startup/init` all assertions (app names/units/device prefix/help)
- [ ] **e2e default-brand regression**
  - Verify: e2e_sfu 4/4 + field push_e2e 6/6 + streamer_e2e (wire unaffected)

## Phase 5: docs & memory

- [ ] **docs/modules/24-app-branding.md branding guide**
  - Content: customizable list (strings/naming/layout/topology) × frozen list (bindings symbols/wire/FrameMeta/crate names); `MEDIASERVO_BRAND` usage + release parameters
- [ ] **Memory capture**
  - `.agents/memorys/`: new decision (D number — Brand mechanism + boundary division), status update
