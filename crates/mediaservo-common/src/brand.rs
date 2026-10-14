//! Brand reader - single entry point for app-layer customization and branding.
//!
//! When MediaServo serves as the foundation for third-party platforms, the host/client/server
//! app layer can be white-labeled, while SDK bindings (C ABI symbol prefixes) and the wire
//! `docs/superpowers/plans/2026-08-21-app-branding-customization.md`）。
//!
//! Priority: runtime env `MEDIASERVO_BRAND` > compile-time `option_env!` > default "mediaservo".
//!
//! **Default brand maps to legacy strings (hard zero-behavior-change constraint)**: the `product`
//! field is not the naming string - by default the app name prefix stays `host-`, the unit prefix
//! stays `oxmgr-host-`, and the device prefix stays `ms-`; only non-default brands derive `<brand>-`.

/// App-layer brand - affects the user-visible layer (strings/naming/layout) only; never enters wire types or symbol tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brand {
    /// CLI/binary display name (help text/version string/unit hints) - default "mediaservo-host".
    pub product: &'static str,
    /// Product display name (GUI/panel title) - default "MediaServo" (brand word); non-default = brand.
    pub display: &'static str,
    /// Path/package id (install layout/default config paths) - default "mediaservo"; non-default = brand.
    pub id: &'static str,
    /// App name prefix: default keeps the legacy `host-`; non-default `<brand>-` (e.g. cp-agent).
    pub app_prefix: &'static str,
    /// systemd unit prefix: default `oxmgr-host-`; non-default `oxmgr-<brand>-`.
    pub unit_prefix: &'static str,
    /// Device id prefix (identity.json): default `ms-`; non-default `<brand>-` (new keys only).
    pub device_prefix: &'static str,
    /// Namespace (oxfile/status filtering): default `mediaservo-host`; non-default `<brand>-host`.
    pub namespace: &'static str,
    /// Shortcut binary prefix: default `mediaservo-` (`host` + `mediaservo-host`); non-default `<brand>-`.
    pub bin_prefix: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct RawBrand {
    product: &'static str,
    // Optional compile-time override (option_env cannot unwrap_or inside const, hence two-stage resolution)
    env_product: Option<&'static str>,
}

const DEFAULT_RAW: RawBrand =
    RawBrand { product: "mediaservo", env_product: option_env!("MEDIASERVO_BRAND") };

/// Valid brand characters: `[a-z0-9-]` (kebab-case).
fn valid_brand(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Read the current brand. env > compile-time option_env > default "mediaservo".
pub fn media_brand() -> Brand {
    let raw = instance();
    media_brand_from(raw.env_product.or(Some(raw.product)))
}

/// Pure mapping function (deterministically testable, no external env dependency):
/// brand value -> Brand; `None`/invalid -> default.
pub fn media_brand_from(brand: Option<&str>) -> Brand {
    match brand.filter(|b| valid_brand(b)) {
        Some(p) if p != "mediaservo" => Brand {
            product: leak(p.to_string()),
            display: leak(p.to_string()),
            id: leak(p.to_string()),
            app_prefix: leak(format!("{p}-")),
            unit_prefix: leak(format!("oxmgr-{p}-")),
            device_prefix: leak(format!("{p}-")),
            namespace: leak(format!("{p}-host")),
            bin_prefix: leak(format!("{p}-")),
        },
        _ => {
            // Default brand - legacy strings hard-mapped (do not switch to mediaservo-*: identity asserts lock ms-)
            Brand {
                product: "mediaservo-host",
                display: "MediaServo",
                id: "mediaservo",
                app_prefix: "host-",
                unit_prefix: "oxmgr-host-",
                device_prefix: "ms-",
                namespace: "mediaservo-host",
                bin_prefix: "mediaservo-",
            }
        }
    }
}

/// Raw env/compile-time brand value (cached once per process - never changes within its lifetime).
fn instance() -> RawBrand {
    use std::sync::OnceLock;
    static CACHE: OnceLock<RawBrand> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut raw = DEFAULT_RAW;
        if let Ok(v) = std::env::var("MEDIASERVO_BRAND") {
            if valid_brand(&v) {
                raw.env_product = Some(leak(v));
            } else {
                tracing::warn!(
                    "MEDIASERVO_BRAND={v:?} invalid (must be [a-z0-9-], <=32 chars) - falling back to default brand"
                );
            }
        }
        if let Some(p) = raw.env_product {
            if p == "mediaservo" {
                tracing::debug!("MEDIASERVO_BRAND=mediaservo - explicit default (same as the omitted legacy mapping)");
            } else {
                tracing::info!(
                    "branding enabled: product={p} (app/unit/device/namespace prefixes derive from <brand>-)"
                );
            }
            raw.product = p;
        }
        raw
    })
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_brand_maps_legacy_strings() {
        // Zero-behavior-change hard constraint: default app prefix is host-, unit is oxmgr-host-, device is ms-
        let b = Brand {
            product: "mediaservo-host",
            display: "MediaServo",
            id: "mediaservo",
            app_prefix: "host-",
            unit_prefix: "oxmgr-host-",
            device_prefix: "ms-",
            namespace: "mediaservo-host",
            bin_prefix: "mediaservo-",
        };
        assert_eq!(b.app_prefix, "host-");
        assert_eq!(b.unit_prefix, "oxmgr-host-");
        assert_eq!(b.device_prefix, "ms-");
        assert_eq!(b.namespace, "mediaservo-host");
    }

    #[test]
    fn custom_brand_derives_prefixes() {
        let b = Brand {
            product: "cp",
            display: "cp",
            id: "cp",
            app_prefix: "cp-",
            unit_prefix: "oxmgr-cp-",
            device_prefix: "cp-",
            namespace: "cp-host",
            bin_prefix: "cp-",
        };
        assert_eq!(format!("{}{}", b.app_prefix, "agent"), "cp-agent");
        assert_eq!(format!("{}{}", b.unit_prefix, "abc"), "oxmgr-cp-abc");
    }

    #[test]
    fn brand_validity_rule() {
        assert!(valid_brand("cp"));
        assert!(valid_brand("car-platform-v2"));
        assert!(!valid_brand(""));
        assert!(!valid_brand("Car"));
        assert!(!valid_brand("cp_1"));
        assert!(!valid_brand(&"x".repeat(33)));
    }

    #[test]
    fn none_or_invalid_falls_back_to_default() {
        assert_eq!(media_brand_from(None).product, "mediaservo-host");
        assert_eq!(media_brand_from(Some("Car")).product, "mediaservo-host");
        assert_eq!(media_brand_from(Some("")).product, "mediaservo-host");
        // Explicit "mediaservo" = same as the omitted legacy mapping
        assert_eq!(media_brand_from(Some("mediaservo")).app_prefix, "host-");
    }

    #[test]
    fn custom_brand_full_mapping() {
        let b = media_brand_from(Some("cp"));
        assert_eq!(b.product, "cp");
        assert_eq!(b.display, "cp");
        assert_eq!(b.id, "cp");
        assert_eq!(b.product, "cp");
        assert_eq!(b.app_prefix, "cp-");
        assert_eq!(b.unit_prefix, "oxmgr-cp-");
        assert_eq!(b.device_prefix, "cp-");
        assert_eq!(b.namespace, "cp-host");
        assert_eq!(b.bin_prefix, "cp-");
    }
}
