//! config.rs -- the vehicle-surface param file `weaknet.yaml` (design §config.rs row + §vehicle M4').
//!
//! schema (strict, unknown keys rejected with the legal set listed -- the yaml port of the P0-era bash emit.py profile whitelist semantics @git ancestry):
//! ```yaml
//! iface: eth0            # default lo (documented constant)
//! ports: [40010, 40012]  # media UDP port enumeration -- anti-deadlock rule layer: a physical iface with no ports
//!                        # (flag/yaml both empty and stats unresolvable) -> exit2 with cause; no "all ports" tier
//! duration: 120          # fuse seconds (default 300 lives in the caller)
//! spec:                  # profile key shape (rtt_ms/jitter_ms/loss/loss_mode/gemodel/
//!   rtt_ms: 100          # reorder_pct/rate_mbps/seed/dir) -- one parse source shared with --profile
//!   dir: out
//! ```
//! Resolution: `--config` explicit path > env `WEAKNET_CONFIG` > probing (weaknet.yaml next to the binary -> cwd).
//! Priority: CLI explicit flag > env > yaml > documented default (iface chain note: yaml comes after env,
//! same shape as the server_url chain -- flag > env > file probe).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::engine::{Fail, Wn};
use crate::spec::ImpairSpec;

pub const CONFIG_FILE: &str = "weaknet.yaml";
/// env override slot (same family as WEAKNET_SERVER_YAML, an explicit-intent slot: set = must exist).
pub const CONFIG_ENV: &str = "WEAKNET_CONFIG";

/// The parsed weaknet.yaml (all fields optional = the defaults supplier; spec = a profile-key-shape parse result).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeaknetConfig {
    pub iface: Option<String>,
    /// Media UDP port enumeration (1-65535; 0/out-of-range rejected -- the parse-layer defense of ports-only matching).
    pub ports: Option<Vec<u16>>,
    pub duration: Option<u64>,
    pub spec: Option<ImpairSpec>,
    /// Source path (echoed in causes, not part of semantics).
    pub path: Option<PathBuf>,
}

impl WeaknetConfig {
    #[must_use]
    pub fn iface_or(&self) -> Option<&str> {
        self.iface.as_deref().filter(|s| !s.is_empty())
    }
}

/// Top-level strict schema (deny_unknown_fields' serde cause carries the legal key set).
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

/// Resolution chain (pure injection shape, tests touch zero environment): flag > env > exe sibling > cwd; all absent = Ok(None).
/// flag/env pointing at a missing file = exit2 with cause (explicit intent tolerates no silent degradation).
#[must_use = "all absent = Ok(None) (no config file) -- callers must take the CLI default chain accordingly"]
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
            return Err(Fail::env(format!(
                "{label} pointed at a missing {CONFIG_FILE}: {}",
                p.display()
            )));
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

/// CLI/serve assembly entry (env + current_exe dir + cwd, all real).
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
        .map_err(|e| Fail::env(format!("failed to read {}: {e}", path.display())))?;
    let mut cfg = parse_yaml(&raw)
        .map_err(|e| Fail::bad_param(format!("{} invalid: {e}", path.display())))?;
    cfg.path = Some(path);
    Ok(Some(cfg))
}

/// Text -> typed (strict parsing; the unit-test surface).
pub fn parse_yaml(raw: &str) -> Result<WeaknetConfig, String> {
    let doc: serde_yaml::Value =
        serde_yaml::from_str(raw).map_err(|e| format!("YAML syntax: {e}"))?;
    if doc.is_null() {
        return Ok(WeaknetConfig::default());
    }
    let strict: RawConfig = serde_yaml::from_value(doc)
        .map_err(|e| format!("top-level keys: {e} (legal keys: iface, ports, duration, spec)"))?;
    let iface = strict.iface.filter(|s| !s.trim().is_empty());
    let ports = match strict.ports {
        None => None,
        Some(list) => {
            for p in &list {
                if *p == 0 {
                    return Err("ports contains 0 (the port enumeration must be concrete media ports; no all-ports semantics)".to_string());
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
                .map_err(|e| format!("spec section conversion failed: {e} (spec must be a mapping with scalar/array values)"))?;
            Some(spec_from_profile_doc(&json)?)
        }
    };
    Ok(WeaknetConfig { iface, ports, duration: strict.duration, spec, path: None })
}

/// The single parse source of the profile / weaknet.yaml `spec:` section (the emit.py profile consumed key shape @git ancestry):
/// `rtt_ms/jitter_ms/rate_mbps/seed` taken directly, `loss` stringified numerically, `reorder_pct->reorder`,
/// `gemodel{r,h,k}->[r,h,k]`, `dir` passed through (from_flat_json validates out|in|both; used by the T12 vehicle yaml,
/// profile files can therefore carry dir for the first time -- existing profiles have no dir key, zero behavior change).
/// Non-param fields like `scope`/`clear` stripped (WARN when signaling=true -- the T5 consumption surface).
/// Keys outside the whitelist rejected with the legal set listed (the T12 strictness contract: typos surface loudly, never silently swallowed).
pub fn spec_from_profile_doc(doc: &Value) -> Result<ImpairSpec, String> {
    const ACCEPTED: &[&str] = &[
        "rtt_ms",
        "jitter_ms",
        "loss",
        "loss_mode",
        "gemodel",
        "reorder_pct",
        "rate_mbps",
        "seed",
        "dir",
        "scope",
        "clear",
    ];
    let obj = doc
        .as_object()
        .ok_or_else(|| format!("the spec/profile document must be a mapping, got {doc}"))?;
    for key in obj.keys() {
        if !ACCEPTED.contains(&key.as_str()) {
            return Err(format!("spec/profile unknown key \"{key}\" (legal set: {ACCEPTED:?})"));
        }
    }
    if doc.get("scope").and_then(|s| s.get("signaling")).and_then(Value::as_bool).unwrap_or(false) {
        eprintln!(
            "weaknet: WARN profile scope.signaling=true -- the signaling leg is consumed via --signaling-port explicitly (yaml/embedded do not supply it)"
        );
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
        let trio: Vec<Value> =
            ["r", "h", "k"].iter().filter_map(|k| g.get(*k)).map(stringify).collect();
        if trio.len() != 3 {
            return Err("gemodel needs dict{r,h,k}".to_string());
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

/// Anti-deadlock rule-layer cause enrichment (pure function, unit-test surface): a physical iface with no explicit port supply this round
/// (both --rtp-port/--config ports empty) gets enumeration guidance appended to targeting-resolution failures; other shapes return None as-is.
#[must_use]
pub fn physical_ports_hint(is_physical: bool, had_explicit_ports: bool) -> Option<&'static str> {
    if is_physical && !had_explicit_ports {
        Some(
            "-- vehicle anti-deadlock: a physical iface must explicitly enumerate media UDP ports (--rtp-port or weaknet.yaml ports; system ports like TCP/SSH/DNS never enter impairment by design)",
        )
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
        assert_eq!(cfg.ports.as_deref(), Some(&[40010u16, 40012][..])); // deduped ascending
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
        assert!(
            e.contains("iface")
                && e.contains("ports")
                && e.contains("duration")
                && e.contains("spec"),
            "must list the legal key set: {e}"
        );
    }

    #[test]
    fn unknown_spec_key_rejected_by_flat_parser() {
        // unknown key in the spec section -> from_flat_json KNOWN gate cause (legal set listed)
        let e = parse_yaml("spec:\n  rtt_m: 80\n").unwrap_err();
        assert!(e.contains("rtt_m"), "{e}");
    }

    #[test]
    fn ports_reject_zero_and_out_of_range_and_string() {
        assert!(parse_yaml("ports: [0]\n").unwrap_err().contains("0"));
        assert!(parse_yaml("ports: [70000]\n").is_err());
        assert!(parse_yaml("ports: [\"5000\"]\n").is_err());
        // empty list = None (same meaning as absent, doesn't trigger the enumeration gate's supply)
        assert_eq!(parse_yaml("ports: []\n").unwrap().ports, None);
    }

    #[test]
    fn spec_gemodel_and_duration_pass_through() {
        let cfg =
            parse_yaml("duration: 60\nspec:\n  loss: 5\n  gemodel: {r: 0.1, h: 0.02, k: 0.001}\n")
                .unwrap();
        assert_eq!(cfg.duration, Some(60));
        match cfg.spec.unwrap().loss {
            Some(LossSpec::GeModel { loss, r, .. }) => {
                assert_eq!(loss, "5");
                assert_eq!(r, "0.1");
            }
            other => panic!("expected gemodel, got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_chain_flag_env_probe() {
        let dir = std::env::temp_dir().join(format!("wnet-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(CONFIG_FILE);
        std::fs::write(&f, "iface: lo\n").unwrap();

        // flag wins and must exist
        assert_eq!(
            resolve_path_pure(Some(f.to_str().unwrap()), None, &dir, &dir).unwrap().unwrap(),
            f
        );
        assert!(resolve_path_pure(Some("/nonexistent/w.yaml"), None, &dir, &dir).is_err());
        // env next; missing also reports cause (explicit intent)
        assert_eq!(
            resolve_path_pure(None, Some(f.to_str().unwrap()), &dir, &dir).unwrap().unwrap(),
            f
        );
        assert!(resolve_path_pure(None, Some("/nope"), &dir, &dir).is_err());
        // probing: exe sibling before cwd
        let exe = dir.join("exe");
        std::fs::create_dir_all(&exe).unwrap();
        let other = dir.join("cwd");
        std::fs::create_dir_all(&other).unwrap();
        // a hit next to the binary (exe_dir itself contains weaknet.yaml) before cwd
        assert_eq!(resolve_path_pure(None, None, &dir, &other).unwrap().unwrap(), f);
        // exe sibling absent -> cwd fallback
        let f2 = other.join(CONFIG_FILE);
        std::fs::write(&f2, "iface: lo\n").unwrap();
        assert_eq!(resolve_path_pure(None, None, &exe, &other).unwrap().unwrap(), f2);
        // all absent -> None
        std::fs::remove_file(&f).unwrap();
        std::fs::remove_file(&f2).unwrap();
        assert_eq!(resolve_path_pure(None, None, &exe, &other).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn physical_hint_only_when_no_explicit_ports() {
        assert!(physical_ports_hint(true, false).is_some());
        assert!(physical_ports_hint(true, true).is_none());
        assert!(
            physical_ports_hint(false, false).is_none(),
            "the lo path unchanged (no enumeration-gate guidance)"
        );
    }
}
