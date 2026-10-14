# 24. Application-Layer Branding Guide (App Branding)

> When MediaServo is depended on as a foundation by third-party platforms (docs/architecture.md §1.2), the
> host/client/server application layers can be white-labeled as needed, while the SDK bindings and the wire
> protocol stay frozen. This guide lists the **customizable surface** and the **frozen surface**.

## Quick Start

```bash
# 1. Install the branded host package (layout + branded app symlinks + identity prefix)
./mediaservo.sh deploy host --brand cp --prefix /opt/cp-host

# 2. Runtime brand (translation/app names/units/device prefixes all read this env)
export MEDIASERVO_BRAND=cp
/opt/cp-host/cp start /opt/cp-host
```

Priority: **runtime env `MEDIASERVO_BRAND` > compile-time `option_env!` > default `mediaservo`**.
Valid values: `[a-z0-9-]`, ≤32 chars; invalid values fall back to the default with a `warn` (branding is a display-layer concern; it never blocks startup).

## Brand Mapping Table (default → legacy strings, hard-mapped)

| Surface | Default (zero change) | Branded (MEDIASERVO_BRAND=cp) |
|---------|----------------------|-------------------------------|
| App names (processes) | `host-agent`/`host-capturer-cam0`/... | `cp-agent`/`cp-capturer-cam0`/... |
| Namespace (oxfile/status filter) | `mediaservo-host` | `cp-host` |
| systemd unit prefix | `oxmgr-host-*` | `oxmgr-cp-*` |
| Device id prefix (identity.json) | `ms-<12hex>` | `cp-<12hex>` (**new keys only** — existing ones not migrated; G2 provisioning requires re-registration) |
| CLI display name/help/version | `mediaservo-host` | `cp` |
| Product display name (client logs/panels) | `MediaServo` | `cp` |
| Path/package id (default config path) | `mediaservo` | `cp` |
| bin shortcuts | `host` + `mediaservo-host` | `cp` + `cp-host` |
| bin/ branded app symlinks | none (binaries are host-*) | `bin/cp-agent → host-agent` (created automatically at install) |

> **Do not derive directly from `<product>-`**: under the default brand `product="mediaservo-host"` but the app
> prefix must be **`host-`** (legacy) and the device prefix **`ms-`** (locked by identity unit-test assertions)
> — see the `brand.rs` mapping table.

## Customizable Items (cross-surface parameters)

| Surface | Location | Notes |
|---------|----------|-------|
| [signaling] local_port | host.toml | Multi-instance coexistence (C32 trinity) |
| [signaling] room | host.toml | Room names are brand-independent (default `vehicle`; the `audio-` prefix convention C29 is not rewritten) |
| PSK / server_url | host.toml | Deployment config surface |
| Process topology | `translate.rs` | Vehicle-side trimming (e.g. dropping controller/audio) — the `enabled_apps` evolution slot |
| admin title | `VITE_APP_TITLE` (vite build env) | Requires `pnpm build` + server rebuild (C24) |
| Ports | config surface | daemon/API ports derived from the instance directory (oxmgr's three envs, C32) |

## Frozen Surface (branding forbidden)

| Surface | Reason |
|---------|--------|
| **bindings/*** (C ABI symbols `mediaservo_*`, include/mediaservo/, cxx/py/node) | Multiple products coexist in one process — symbol conflicts mean link failure (D247) |
| **Wire protocol** (signaling SignalingMessage/SFU/RTP parameters, FrameMeta wire format) | The server may serve multiple brands of host simultaneously — branding would fragment the ecosystem |
| **soname/ABI discipline** (additive-only) | Dynamic library upgrade compatibility (D240) |
| **Crate names** (`mediaservo-*` workspace members) | Dependency-surface identity (independent release goes via fork + D209-style rename) |
| admin localStorage key (`mediaservo_admin_token`) | Keep the original key (rebranding would log users out) |
| Room naming (the `audio-` prefix convention) | Server-side room semantics (C29) |

## Verification Commands

```bash
# Frozen gate: bindings zero diff (branding work must not touch)
git diff --stat bindings/            # should be empty

# Default zero-change gate: full regression under the default brand
pixi run cargo test -p mediaservo-host --lib    # 50 all green
bash scripts/e2e-deploy-host.sh                 # PASS (incl. start/status/stop roundtrip)

# Branding verification
MEDIASERVO_BRAND=cp ./target/debug/mediaservo-host version   # cp 0.1.0
MEDIASERVO_BRAND=cp ./target/debug/mediaservo-host startup on <dir>  # unit oxmgr-cp-*.service
```

## Design Locations

- `crates/mediaservo-common/src/brand.rs`: the Brand struct + `media_brand()`/`media_brand_from()` readers
- `crates/mediaservo-host/src/bin/host.rs`: USAGE/version/namespace filter/unit names
- `crates/mediaservo-host/src/translate.rs`: `app_name()` (app prefix) + oxfile namespace
- `crates/mediaservo-host/src/identity.rs`: `generate_identity()` device prefix
- `scripts/mediaservo_cli.py`: `deploy host --brand` / `package host --brand`
- `www/apps/admin/vite.config.ts`: `__APP_TITLE__` define

Plan: `docs/superpowers/plans/2026-08-21-app-branding-customization.md`
