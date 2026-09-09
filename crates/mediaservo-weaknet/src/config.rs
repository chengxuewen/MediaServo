//! config.rs —— 车端面参数文件 `weaknet.yaml`（design §config.rs 行 + §车端面 M4′）。
//!
//! schema（严格，未知键拒绝并列合法集——emit.py profile 白名单语义的 yaml 平移）：
//! ```yaml
//! iface: eth0            # 缺省 lo（文档化常量）
//! ports: [40010, 40012]  # 媒体 UDP 口枚举——反锁死规则层：物理 iface 无 ports
//!                        # （flag/yaml 皆空且 stats 不可解析）→ exit2 报因，无「全集」档
//! duration: 120          # 保险丝秒（缺省 300 在调用方）
//! spec:                  # profile 键形（rtt_ms/jitter_ms/loss/loss_mode/gemodel/
//!   rtt_ms: 100          # reorder_pct/rate_mbps/seed/dir）——与 --profile 单一解析源
//!   dir: out
//! ```
//! 寻径：`--config` 显式路径 > env `WEAKNET_CONFIG` > 探测（二进制同级 weaknet.yaml → cwd）。
//! 优先级：CLI 显式 flag > env > yaml > 文档化缺省（iface 链注记：yaml 位于 env 之后，
//! 与 server_url 链同形——flag > env > 文件探测）。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::engine::{Fail, Wn};
use crate::spec::ImpairSpec;

pub const CONFIG_FILE: &str = "weaknet.yaml";
/// env 覆写位（与 WEAKNET_SERVER_YAML 同族显式意图位：设了就必须存在）。
pub const CONFIG_ENV: &str = "WEAKNET_CONFIG";

/// 解析后的 weaknet.yaml（全字段可选=缺省供给源；spec 为 profile 键形解析结果）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeaknetConfig {
    pub iface: Option<String>,
    /// 媒体 UDP 口枚举（1-65535；0/越界拒绝——ports-only 匹配的 parse 层防御）
    pub ports: Option<Vec<u16>>,
    pub duration: Option<u64>,
    pub spec: Option<ImpairSpec>,
    /// 来源路径（报因回显用，不参与语义）
    pub path: Option<PathBuf>,
}

impl WeaknetConfig {
    #[must_use]
    pub fn iface_or(&self) -> Option<&str> {
        self.iface.as_deref().filter(|s| !s.is_empty())
    }
}

/// 顶层严格 schema（deny_unknown_fields 的 serde 报因自带合法键集）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    iface: Option<String>,
    #[serde(default)]
    ports: Option<Vec<u16>>,
    #[serde(default)]
    duration: Option<u64>,
    #[serde(default)]
    spec: Option<serde_yaml::Value>,
}

/// 寻径链（纯注入形，测试零环境触碰）：flag > env > exe 同级 > cwd；皆空=Ok(None)。
/// flag/env 指向不存在的文件 = exit2 报因（显式意图不容静默降级）。
#[must_use = "皆无=Ok(None)（无配置文件）——调用方需据此走 CLI 缺省链"]
pub fn resolve_path_pure(
    cli: Option<&str>,
    env_val: Option<&str>,
    exe_dir: &Path,
    cwd: &Path,
) -> Wn<Option<PathBuf>> {
    for (label, cand) in [("config", cli), (CONFIG_ENV, env_val)] {
        let Some(raw) = cand.map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let p = PathBuf::from(raw);
        if !p.is_file() {
            return Err(Fail::env(format!("{label} 指定的 {CONFIG_FILE} 不存在: {}", p.display())));
        }
        return Ok(Some(p));
    }
    for dir in [exe_dir, cwd] {
        let cand = dir.join(CONFIG_FILE);
        if cand.is_file() {
            return Ok(Some(cand));
        }
    }
    Ok(None)
}

/// CLI/serve 装配入口（env + current_exe 目录 + cwd 实取）。
pub fn load(cli: Option<&str>) -> Wn<Option<WeaknetConfig>> {
    let env_val = std::env::var(CONFIG_ENV).ok().filter(|s| !s.is_empty());
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let cwd = std::env::current_dir().unwrap_or_default();
    let Some(path) = resolve_path_pure(cli, env_val.as_deref(), &exe_dir, &cwd)? else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| Fail::env(format!("读 {} 失败: {e}", path.display())))?;
    let mut cfg = parse_yaml(&raw)
        .map_err(|e| Fail::bad_param(format!("{} 非法: {e}", path.display())))?;
    cfg.path = Some(path);
    Ok(Some(cfg))
}

/// 文本 → 类型化（严格解析；单测面对象）。
pub fn parse_yaml(raw: &str) -> Result<WeaknetConfig, String> {
    let doc: serde_yaml::Value =
        serde_yaml::from_str(raw).map_err(|e| format!("YAML 语法: {e}"))?;
    if doc.is_null() {
        return Ok(WeaknetConfig::default());
    }
    let strict: RawConfig = serde_yaml::from_value(doc)
        .map_err(|e| format!("顶层键: {e}（合法键: iface, ports, duration, spec）"))?;
    let iface = strict.iface.filter(|s| !s.trim().is_empty());
    let ports = match strict.ports {
        None => None,
        Some(list) => {
            for p in &list {
                if *p == 0 {
                    return Err("ports 含 0（端口枚举必须是具体媒体口，无全口语义）".to_string());
                }
            }
            let dedup = {
                let mut v = list;
                v.sort_unstable();
                v.dedup();
                v
            };
            (!dedup.is_empty()).then_some(dedup)
        }
    };
    let spec = match strict.spec {
        None => None,
        Some(s) => {
            let json = serde_json::to_value(&s)
                .map_err(|e| format!("spec 节转换失败: {e}（spec 必须是映射，值为标量/数组）"))?;
            Some(spec_from_profile_doc(&json)?)
        }
    };
    Ok(WeaknetConfig {
        iface,
        ports,
        duration: strict.duration,
        spec,
        path: None,
    })
}

/// profile / weaknet.yaml `spec:` 节的单一解析源（emit.py `profile` 消费键形）：
/// `rtt_ms/jitter_ms/rate_mbps/seed` 直取、`loss` 数值 stringify、`reorder_pct→reorder`、
/// `gemodel{r,h,k}→[r,h,k]`、`dir` 透传（from_flat_json 校验 out|in|both；T12 车端 yaml 用，
/// profile 文件因此键首次可带 dir——既有 profile 无 dir 键，行为零变更）。
/// `scope`/`clear` 等非参数字段剥离（signaling=true 时 WARN——T5 消费面）。
/// 白名单外键拒绝并列合法集（T12 严格性合同：typos 显性化，禁静默吞）。
pub fn spec_from_profile_doc(doc: &Value) -> Result<ImpairSpec, String> {
    const ACCEPTED: &[&str] = &[
        "rtt_ms", "jitter_ms", "loss", "loss_mode", "gemodel", "reorder_pct", "rate_mbps",
        "seed", "dir", "scope", "clear",
    ];
    let obj = doc
        .as_object()
        .ok_or_else(|| format!("spec/profile 文档需为映射，得 {doc}"))?;
    for key in obj.keys() {
        if !ACCEPTED.contains(&key.as_str()) {
            return Err(format!("spec/profile 未知键 \"{key}\"（合法集: {ACCEPTED:?}）"));
        }
    }
    if doc
        .get("scope")
        .and_then(|s| s.get("signaling"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        eprintln!("weaknet: WARN profile scope.signaling=true——信令腿消费走 --signaling-port 显式（yaml/内嵌不供给）");
    }
    let mut flat = serde_json::Map::new();
    for k in ["rtt_ms", "jitter_ms", "rate_mbps", "seed", "dir"] {
        if let Some(v) = doc.get(k) {
            flat.insert(k.to_string(), v.clone());
        }
    }
    if let Some(l) = doc.get("loss") {
        flat.insert("loss".to_string(), stringify(l));
    }
    if let Some(r) = doc.get("reorder_pct") {
        flat.insert("reorder".to_string(), stringify(r));
    }
    if let Some(g) = doc.get("gemodel") {
        let trio: Vec<Value> = ["r", "h", "k"]
            .iter()
            .filter_map(|k| g.get(*k))
            .map(stringify)
            .collect();
        if trio.len() != 3 {
            return Err("gemodel 需 dict{r,h,k}".to_string());
        }
        flat.insert("gemodel".to_string(), Value::Array(trio));
        flat.insert("loss_mode".to_string(), Value::String("gemodel".into()));
    }
    ImpairSpec::from_flat_json(&Value::Object(flat))
}

fn stringify(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    }
}

/// 反锁死规则层报因增强（纯函数，单测面）：物理 iface + 本次无显式端口供给
/// （--rtp-port/--config ports 皆空）时，为定向解析失败补枚举指引；其余形原样 None。
#[must_use]
pub fn physical_ports_hint(
    is_physical: bool,
    had_explicit_ports: bool,
) -> Option<&'static str> {
    if is_physical && !had_explicit_ports {
        Some("——车端反锁死：物理 iface 必须显式枚举媒体 UDP 口（--rtp-port 或 weaknet.yaml ports；TCP/SSH/DNS 等系统口结构性不进损伤）")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Dir, LossSpec};

    #[test]
    fn parse_full_document() {
        let cfg = parse_yaml(
            r#"
iface: veth0
ports: [40012, 40010, 40010]
duration: 120
spec:
  rtt_ms: 100
  jitter_ms: 20
  loss: 10%
  dir: out
"#,
        )
        .unwrap();
        assert_eq!(cfg.iface.as_deref(), Some("veth0"));
        assert_eq!(cfg.ports.as_deref(), Some(&[40010u16, 40012][..])); // 去重升序
        assert_eq!(cfg.duration, Some(120));
        let s = cfg.spec.unwrap();
        assert_eq!(s.rtt_ms, 100);
        assert_eq!(s.jitter_ms, 20);
        assert_eq!(s.loss, Some(LossSpec::Simple("10%".into())));
        assert_eq!(s.dir, Dir::Out);
    }

    #[test]
    fn empty_doc_is_all_defaults() {
        let cfg = parse_yaml("").unwrap();
        assert_eq!(cfg, WeaknetConfig::default());
    }

    #[test]
    fn unknown_top_level_key_rejected_listing_accepted_set() {
        let e = parse_yaml("iface: lo\nportz: [5000]\n").unwrap_err();
        assert!(e.contains("portz"), "{e}");
        assert!(e.contains("iface") && e.contains("ports") && e.contains("duration") && e.contains("spec"), "需列合法键集: {e}");
    }

    #[test]
    fn unknown_spec_key_rejected_by_flat_parser() {
        // spec 节未知键 → from_flat_json KNOWN 门报因（列合法集）
        let e = parse_yaml("spec:\n  rtt_m: 80\n").unwrap_err();
        assert!(e.contains("rtt_m"), "{e}");
    }

    #[test]
    fn ports_reject_zero_and_out_of_range_and_string() {
        assert!(parse_yaml("ports: [0]\n").unwrap_err().contains("0"));
        assert!(parse_yaml("ports: [70000]\n").is_err());
        assert!(parse_yaml("ports: [\"5000\"]\n").is_err());
        // 空列表 = None（与缺省同义，不触发枚举门供给）
        assert_eq!(parse_yaml("ports: []\n").unwrap().ports, None);
    }

    #[test]
    fn spec_gemodel_and_duration_pass_through() {
        let cfg = parse_yaml(
            "duration: 60\nspec:\n  loss: 5\n  gemodel: {r: 0.1, h: 0.02, k: 0.001}\n",
        )
        .unwrap();
        assert_eq!(cfg.duration, Some(60));
        match cfg.spec.unwrap().loss {
            Some(LossSpec::GeModel { loss, r, .. }) => {
                assert_eq!(loss, "5");
                assert_eq!(r, "0.1");
            }
            other => panic!("期望 gemodel，得 {other:?}"),
        }
    }

    #[test]
    fn resolve_path_chain_flag_env_probe() {
        let dir = std::env::temp_dir().join(format!("wnet-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(CONFIG_FILE);
        std::fs::write(&f, "iface: lo\n").unwrap();

        // flag 优先且必须存在
        assert_eq!(
            resolve_path_pure(Some(f.to_str().unwrap()), None, &dir, &dir)
                .unwrap()
                .unwrap(),
            f
        );
        assert!(resolve_path_pure(Some("/nonexistent/w.yaml"), None, &dir, &dir).is_err());
        // env 次之；缺失同样报因（显式意图）
        assert_eq!(
            resolve_path_pure(None, Some(f.to_str().unwrap()), &dir, &dir)
                .unwrap()
                .unwrap(),
            f
        );
        assert!(resolve_path_pure(None, Some("/nope"), &dir, &dir).is_err());
        // 探测：exe 同级先于 cwd
        let exe = dir.join("exe");
        std::fs::create_dir_all(&exe).unwrap();
        let other = dir.join("cwd");
        std::fs::create_dir_all(&other).unwrap();
        // 二进制同级命中（exe_dir 自身含 weaknet.yaml）先于 cwd
        assert_eq!(
            resolve_path_pure(None, None, &dir, &other).unwrap().unwrap(),
            f
        );
        // exe 同级缺位 → cwd 兜底
        let f2 = other.join(CONFIG_FILE);
        std::fs::write(&f2, "iface: lo\n").unwrap();
        assert_eq!(
            resolve_path_pure(None, None, &exe, &other).unwrap().unwrap(),
            f2
        );
        // 皆无 → None
        std::fs::remove_file(&f).unwrap();
        std::fs::remove_file(&f2).unwrap();
        assert_eq!(resolve_path_pure(None, None, &exe, &other).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn physical_hint_only_when_no_explicit_ports() {
        assert!(physical_ports_hint(true, false).is_some());
        assert!(physical_ports_hint(true, true).is_none());
        assert!(physical_ports_hint(false, false).is_none(), "lo 路径不变（无枚举门指引）");
    }
}
