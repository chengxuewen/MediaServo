//! 品牌（branding）读取器 — 应用层可定制、可品牌化的统一入口。
//!
//! MediaServo 作为基石被第三方平台依赖时，host/client/server 应用层可白标，
//! 而 SDK bindings（C ABI 符号前缀）与 wire 协议保持固化（见
//! `docs/superpowers/plans/2026-08-21-app-branding-customization.md`）。
//!
//! 优先级：运行时 env `MEDIASERVO_BRAND` > 编译期 `option_env!` > 默认 "mediaservo"。
//!
//! **默认品牌映射 legacy 串（零行为变化硬约束）**：`product` 字段 ≠ 命名串——
//! 默认下 app 名前缀保持 `host-`、unit 前缀保持 `oxmgr-host-`、设备前缀保持 `ms-`，
//! 仅非默认品牌才用 `<brand>-` 派生。禁止按 `<product>-` 直推（撞 identity 单测断言）。

/// 应用层品牌 — 只影响用户可见层（字符串/命名/布局），永不进入 wire 类型/符号表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brand {
    /// CLI/二进制显示名（帮助文本/版本串/unit 提示）——默认 "mediaservo-host"。
    pub product: &'static str,
    /// 产品展示名（GUI/面板标题）——默认 "MediaServo"（品牌词），非默认 = brand。
    pub display: &'static str,
    /// 路径/包 id（install 布局/默认配置路径）——默认 "mediaservo"，非默认 = brand。
    pub id: &'static str,
    /// app 名前缀：默认保持 legacy `host-`；非默认 `<brand>-`（如 cp-agent）。
    pub app_prefix: &'static str,
    /// systemd unit 前缀：默认 `oxmgr-host-`；非默认 `oxmgr-<brand>-`。
    pub unit_prefix: &'static str,
    /// 设备 id 前缀（identity.json）：默认 `ms-`；非默认 `<brand>-`（仅新 key）。
    pub device_prefix: &'static str,
    /// namespace（oxfile/status 过滤）：默认 `mediaservo-host`；非默认 `<brand>-host`。
    pub namespace: &'static str,
    /// 快捷二进制前缀：默认 `mediaservo-`（`host` + `mediaservo-host`）；非默认 `<brand>-`。
    pub bin_prefix: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct RawBrand {
    product: &'static str,
    // 可选编译期覆盖（option_env 无法在 const 中 unwrap_or，故双阶段解析）
    env_product: Option<&'static str>,
}

const DEFAULT_RAW: RawBrand = RawBrand {
    product: "mediaservo",
    env_product: option_env!("MEDIASERVO_BRAND"),
};

/// 合法品牌串字符：`[a-z0-9-]`（kebab-case）。
fn valid_brand(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// 读取当前品牌。env > 编译期 option_env > 默认 "mediaservo"。
pub fn media_brand() -> Brand {
    let raw = instance();
    media_brand_from(raw.env_product.or(Some(raw.product)))
}

/// 纯函数映射（可确定性单测，不依赖外部 env）：
/// 品牌值 → Brand；`None`/非法 → 默认。
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
            // 默认品牌 — legacy 串硬映射（勿改成 mediaservo-*——identity 断言锁死 ms-）
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

/// env/编译期原始品牌值（进程内缓存一次——生命周期内不变）。
fn instance() -> RawBrand {
    use std::sync::OnceLock;
    static CACHE: OnceLock<RawBrand> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut raw = DEFAULT_RAW;
        if let Ok(v) = std::env::var("MEDIASERVO_BRAND") {
            if valid_brand(&v) {
                raw.env_product = Some(leak(v));
            } else {
                tracing::warn!("MEDIASERVO_BRAND={v:?} 非法（须 [a-z0-9-]，≤32 字符）——回落默认品牌");
            }
        }
        if let Some(p) = raw.env_product {
            if p == "mediaservo" {
                tracing::debug!("MEDIASERVO_BRAND=mediaservo——显式默认（等同缺省 legacy 映射）");
            } else {
                tracing::info!("品牌化模式: product={p}（app/unit/device/namespace 前缀按 <brand>- 派生）");
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
        // 零行为变化硬约束：默认 app 前缀是 host-、unit 是 oxmgr-host-、设备是 ms-
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
        // 显式 "mediaservo" = 等同缺省 legacy 映射
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