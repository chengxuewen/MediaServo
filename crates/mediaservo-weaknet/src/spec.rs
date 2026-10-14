//! spec.rs -- impairment parameter semantic core (the CLI is the core, serve/UI are shells; design.md §dir leg-definition table is the sole truth source).
//!
//! Rendering contract = character-by-character port of `scripts/weaknet.sh` `render_netem_spec` / `rate_arg`
//! (pinned by the 9 inherited golden-fixture cases, dir=both baseline); the 3 new dir-contract cases are carried by `build_filter_legs`.
//! Exit code 4 (invalid params) semantics are mirrored by this module's validation functions (table copied from weaknet.sh parse_opts).
//!
//! Note (bash-port fidelity): `render_netem_spec` **always halves** rtt/jitter (the lo two-pass-per-direction
//! convention); the dir-aware single-leg full-amount rule lives in [`effective_leg_delay_ms`], consumed by the install layer (T3/T5).

use serde::{Deserialize, Serialize};

/// Direction (same shape as the clap surface; lib semantic source, main.rs's CLI enum converts into it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Out,
    In,
    #[default]
    Both,
}

/// Scope selection (§dir table row dimension; Ports as escape hatch normalized to the Media form in T3 scope.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeSel {
    /// room level: the local media port set (server listen/local ports)
    Media,
    /// stream level: ordered pairs (local, remote) -- single filter, dual-match AND
    Stream,
    /// device level: the union of remote_ports over all streams of that owner (the pairless Stream form)
    Device,
}

/// Interface kind. Logical leg shape is iface-independent (the same function produces it); the difference is the **install point** --
/// a physical NIC with dir=in moves to ifb mirror ingress (design §ifb contract, T9); lo is always a port leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IfaceKind {
    Loopback,
    Physical,
}

/// Loss mode (Simple carries the raw percentage string -- "2%" or "0.5%"; "0"/"0%" skipped at render).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LossSpec {
    Simple(String),
    /// All four values are raw strings ("25%" allowed; the tc side always takes bare numbers = % stripped at render).
    GeModel {
        loss: String,
        r: String,
        h: String,
        k: String,
    },
}

/// Full impairment spec. limit defaults to 100000 (bash NETEM_LIMIT: netem defaults to 100 packets, which with delay silently self-harms with drops).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpairSpec {
    pub rtt_ms: u64,
    pub jitter_ms: u64,
    pub loss: Option<LossSpec>,
    /// Reorder percentage as a bare integer string ('%' appended at render, structurally same as bash `reorder ${REORDER_PCT}%`).
    pub reorder_pct: Option<String>,
    pub rate_mbps: Option<f64>,
    pub seed: Option<u64>,
    pub limit: u64,
    pub dir: Dir,
}

impl Default for ImpairSpec {
    fn default() -> Self {
        Self {
            rtt_ms: 0,
            jitter_ms: 0,
            loss: None,
            reorder_pct: None,
            rate_mbps: None,
            seed: None,
            limit: crate::spec::DEFAULT_NETEM_LIMIT,
            dir: Dir::Both,
        }
    }
}

pub const DEFAULT_NETEM_LIMIT: u64 = 100_000;
/// htb sentinel when rate unset (no limit = astronomic bandwidth; bash SENTINEL_RATE).
pub const SENTINEL_RATE: &str = "1000gbit";

impl ImpairSpec {
    /// Parse from flat JSON (the golden fixture `input` shape / the post-CLI-merge shape).
    /// Keys: rtt_ms, jitter_ms, loss, loss_mode("simple"|"gemodel"), gemodel([r,h,k]),
    /// reorder, rate_mbps, seed, limit, dir("out"|"in"|"both")。
    /// Validation mirrors the bash exit-4 table; unknown keys rejected (typos surface loudly, never silently swallowed).
    pub fn from_flat_json(v: &serde_json::Value) -> Result<Self, String> {
        let obj = v.as_object().ok_or_else(|| format!("spec input is not an object: {v}"))?;
        const KNOWN: &[&str] = &[
            "rtt_ms",
            "jitter_ms",
            "loss",
            "loss_mode",
            "gemodel",
            "reorder",
            "rate_mbps",
            "seed",
            "limit",
            "dir",
        ];
        for key in obj.keys() {
            if !KNOWN.contains(&key.as_str()) {
                return Err(format!("spec input unknown key \"{key}\" (legal set: {KNOWN:?})"));
            }
        }
        let get_u64 = |k: &str| -> Result<u64, String> {
            match obj.get(k) {
                None | Some(serde_json::Value::Null) => Ok(0),
                Some(serde_json::Value::Number(n)) => {
                    n.as_u64().ok_or_else(|| format!("{k} must be a non-negative integer, got {n}"))
                }
                Some(other) => Err(format!("{k} must be a number, got {other}")),
            }
        };
        let get_opt_str = |k: &str| -> Result<Option<String>, String> {
            match obj.get(k) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
                Some(other) => Err(format!("{k} must be a string, got {other}")),
            }
        };
        let rtt_ms = get_u64("rtt_ms")?;
        let jitter_ms = get_u64("jitter_ms")?;
        let loss = get_opt_str("loss")?;
        let loss_mode = get_opt_str("loss_mode")?.unwrap_or_else(|| "simple".into());
        let gemodel = match obj.get("gemodel") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Array(a)) => {
                let vals: Option<Vec<&str>> = a.iter().map(Value::as_str).collect();
                match vals {
                    Some(v) if v.len() == 3 => Some(v),
                    _ => return Err(format!("gemodel needs 3 strings [r,h,k], got {}", a.len())),
                }
            }
            Some(other) => return Err(format!("gemodel must be an array, got {other}")),
        };
        let loss_spec = match loss_mode.as_str() {
            "simple" | "uniform" => loss.map(LossSpec::Simple),
            "gemodel" => match (loss, gemodel) {
                (Some(l), Some(g)) if g.len() == 3 => Some(LossSpec::GeModel {
                    loss: l,
                    r: g[0].to_string(),
                    h: g[1].to_string(),
                    k: g[2].to_string(),
                }),
                _ => return Err("gemodel mode needs loss + gemodel[r,h,k]".to_string()),
            },
            other => return Err(format!("loss_mode must be simple|gemodel, got {other:?}")),
        };
        let rate_mbps = match obj.get("rate_mbps") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Number(n)) => {
                let f = n.as_f64().ok_or_else(|| format!("rate_mbps must be numeric, got {n}"))?;
                Some(f)
            }
            Some(other) => return Err(format!("rate_mbps must be a number, got {other}")),
        };
        let seed = match obj.get("seed") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Number(n)) => Some(
                n.as_u64()
                    .ok_or_else(|| format!("seed must be a non-negative integer, got {n}"))?,
            ),
            Some(other) => return Err(format!("seed must be a number, got {other}")),
        };
        let limit = match obj.get("limit") {
            None | Some(serde_json::Value::Null) => DEFAULT_NETEM_LIMIT,
            Some(_) => get_u64("limit")?,
        };
        let dir = match obj.get("dir") {
            None | Some(serde_json::Value::Null) => Dir::Both,
            Some(serde_json::Value::String(s)) => match s.as_str() {
                "out" => Dir::Out,
                "in" => Dir::In,
                "both" => Dir::Both,
                other => return Err(format!("dir must be out|in|both, got {other:?}")),
            },
            Some(other) => return Err(format!("dir must be a string, got {other}")),
        };
        let spec = Self {
            rtt_ms,
            jitter_ms,
            loss: loss_spec,
            reorder_pct: get_opt_str("reorder")?,
            rate_mbps,
            seed,
            limit,
            dir,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Range validation (mirrors the weaknet.sh parse_opts exit-4 table; messages include the offending value and the legal domain).
    pub fn validate(&self) -> Result<(), String> {
        validate_rtt(self.rtt_ms)?;
        validate_jitter(self.jitter_ms)?;
        if let Some(l) = &self.loss {
            match l {
                LossSpec::Simple(p) => validate_loss(p)?,
                LossSpec::GeModel { loss, r, h, k } => {
                    validate_loss(loss)?;
                    validate_gemodel_value(r)?;
                    validate_gemodel_value(h)?;
                    validate_gemodel_value(k)?;
                }
            }
        }
        if let Some(rp) = &self.reorder_pct {
            validate_reorder(rp)?;
        }
        if let Some(rate) = self.rate_mbps {
            validate_rate(rate)?;
        }
        Ok(())
    }

    /// Per-leg installed delay ms (design §dir "rtt calibration rule": both -> rtt/2 per leg;
    /// single out|in -> full amount -- keeps "observed RTT ~ rtt_ms" constant across dirs).
    /// Decoupled from [`render_netem_spec`] (the always-halving bash inherited baseline): the netem string itself still goes the
    /// render + install-layer-overrides-the-delay-segment route (T3/T5 argv assembly takes this value).
    #[must_use]
    pub fn effective_leg_delay_ms(&self) -> u64 {
        match self.dir {
            Dir::Both => self.rtt_ms / 2,
            Dir::Out | Dir::In => self.rtt_ms,
        }
    }

    /// jitter halves by the same rule (both halves inherit bash `j=$((JITTER_MS/2))`; single leg = full amount).
    #[must_use]
    pub fn effective_leg_jitter_ms(&self) -> u64 {
        match self.dir {
            Dir::Both => self.jitter_ms / 2,
            Dir::Out | Dir::In => self.jitter_ms,
        }
    }
}

use serde_json::Value;

/// netem parameter string (bash `render_netem_spec` character-by-character port: limit -> delay -> loss -> reorder -> seed).
/// Fidelity note: always halves (dir=both baseline); the jitter segment concatenates with single spaces like the delay segment (rtt=0:
/// jitter follows limit directly -- structurally same as bash `spec="$spec ${j}ms ..."`). Dir-aware delay in
/// [`ImpairSpec::effective_leg_delay_ms`] (consumed by the install layer T3/T5).
#[must_use]
pub fn render_netem_spec(s: &ImpairSpec) -> String {
    render_netem_spec_with(s, s.rtt_ms / 2, s.jitter_ms / 2)
}

/// dir-aware leaf form (design §dir rtt calibration: both = halved baseline (= render_netem_spec);
/// single out|in = full -- observed RTT constant across dirs). The 9 golden cases pin only the legacy form.
#[must_use]
pub fn render_netem_spec_leg(s: &ImpairSpec) -> String {
    match s.dir {
        Dir::Both => render_netem_spec(s),
        Dir::Out | Dir::In => render_netem_spec_with(s, s.rtt_ms, s.jitter_ms),
    }
}

fn render_netem_spec_with(s: &ImpairSpec, d: u64, j: u64) -> String {
    let mut spec = format!("limit {}", s.limit);
    if d > 0 {
        spec.push_str(&format!(" delay {d}ms"));
    }
    if j > 0 {
        spec.push_str(&format!(" {j}ms distribution normal"));
    }
    // gemodel with loss=0 -> downgrade to "loss 0%" (works around the kernel "no delay means reset" quirk, bash header note 3)
    match &s.loss {
        Some(LossSpec::GeModel { loss, r, h, k }) => {
            if loss == "0" || loss == "0%" {
                spec.push_str(" loss 0%");
            } else {
                spec.push_str(&format!(
                    " loss gemodel {} {} {} {}",
                    strip_pct(loss),
                    strip_pct(r),
                    strip_pct(h),
                    strip_pct(k)
                ));
            }
        }
        Some(LossSpec::Simple(p)) if !p.is_empty() && strip_pct(p) != "0" => {
            spec.push_str(&format!(" loss {p}"));
        }
        // "0"/"0%"/empty = skip the impairment segment (bash elif synonym); GeModel already exhausted in the branch above
        Some(_) | None => {}
    }
    if let Some(rp) = &s.reorder_pct
        && !rp.is_empty()
    {
        spec.push_str(&format!(" reorder {rp}%"));
    }
    if let Some(seed) = s.seed {
        spec.push_str(&format!(" seed {seed}"));
    }
    spec
}

fn strip_pct(v: &str) -> &str {
    v.strip_suffix('%').unwrap_or(v)
}

/// htb rate argument (bash `rate_arg`): unset -> sentinel; set -> "{n}mbit",
/// an integral f64 prints without a decimal point (4.0 -> "4mbit"; 4.5 -> "4.5mbit").
#[must_use]
pub fn render_rate_arg(rate_mbps: Option<f64>) -> String {
    match rate_mbps {
        Some(n) if n.fract() == 0.0 => format!("{}mbit", n as u64),
        Some(n) => format!("{n}mbit"),
        None => SENTINEL_RATE.to_string(),
    }
}

// ---------- §dir leg-definition table (rev-2.1 core contract) ----------

/// protocol conjunction for all media legs (UDP; TCP ephemeral-port collision defense). The signaling TCP leg (protocol 6)
/// carries the bash --signaling slot, listed separately at T3 -- not part of this media-leg set.
pub const UDP_PROTOCOL: u8 = 17;
/// Target class of all media legs (build_skeleton's 1:10).
pub const MEDIA_FLOWID: &str = "1:10";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegFilter {
    pub protocol: u8,
    /// multiple matches inside one filter = conjunction (AND); ("sport"/"dport", port). Never split into two single-match filters (= OR cross-hit).
    pub matches: Vec<(MatchKind, u16)>,
    pub flowid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    Sport,
    Dport,
}

/// Assemble direction legs per the §dir table:
/// - Media: out -> sport in P (one filter per port); in -> dport in P
/// - Stream/Device (rev-2.2 unified): the ordered-pair set (L,R), Device = (L,R) over all streams of the owner, flattened by the caller;
///   out=(sport=L AND dport=R) / in=(sport=R AND dport=L), single filter with dual-match AND per pair --
///   the remote-union single-match form retired (loosest contract; Momus ambiguities #1/#3 adjudicated together)
/// - both = install both directions (equivalent to current bash); protocol 17 conjunction always present.
///
/// iface does not change the logical leg shape (a physical NIC with dir=in installs at ifb ingress, §ifb contract T9 --
/// this function is the design's "dir-aware pure function", the predicate source of the T5/W1 dry-run snapshots).
pub fn build_filter_legs(
    scope: ScopeSel,
    iface: IfaceKind,
    dir: Dir,
    local_ports: &[u16],
    pairs: &[(u16, u16)],
) -> Result<Vec<LegFilter>, String> {
    let _ = iface;
    let mut legs = Vec::new();
    let mut push_media_leg = |kind: MatchKind, ports: &[u16]| {
        for &p in ports {
            legs.push(LegFilter {
                protocol: UDP_PROTOCOL,
                matches: vec![(kind, p)],
                flowid: MEDIA_FLOWID.to_string(),
            });
        }
    };
    let out = matches!(dir, Dir::Out | Dir::Both);
    let inbound = matches!(dir, Dir::In | Dir::Both);
    match scope {
        ScopeSel::Media => {
            if out {
                push_media_leg(MatchKind::Sport, local_ports);
            }
            if inbound {
                push_media_leg(MatchKind::Dport, local_ports);
            }
        }
        // Stream/Device unified ordered-pair AND legs (rev-2.2)
        ScopeSel::Stream | ScopeSel::Device => {
            for &(l, r) in pairs {
                if out {
                    legs.push(and_leg(&[(MatchKind::Sport, l), (MatchKind::Dport, r)]));
                }
                if inbound {
                    legs.push(and_leg(&[(MatchKind::Sport, r), (MatchKind::Dport, l)]));
                }
            }
        }
    }
    if legs.is_empty() {
        return Err(format!(
            "leg set is empty (scope={scope:?} dir={dir:?} ports/pairs unresolved -- empty stats reports cause exit2, never silently all ports)"
        ));
    }
    Ok(legs)
}

fn and_leg(matches: &[(MatchKind, u16)]) -> LegFilter {
    LegFilter {
        protocol: UDP_PROTOCOL,
        matches: matches.to_vec(),
        flowid: MEDIA_FLOWID.to_string(),
    }
}

// ---------- Range validation (exit-4 table; messages include the offending value + legal domain) ----------

pub fn validate_rtt(v: u64) -> Result<(), String> {
    if v > 2000 {
        return Err(format!("rtt invalid: {v} (legal domain 0-2000ms integer)"));
    }
    Ok(())
}

pub fn validate_jitter(v: u64) -> Result<(), String> {
    // bash: any non-negative integer passes (u64 is inherently non-negative); type placeholder only, against future reshaping.
    // ponytail: no upper bound = bash as-is; if a kernel cap is needed, fold into T3.
    let _ = v;
    Ok(())
}

pub fn validate_loss(s: &str) -> Result<(), String> {
    if parse_pct_bounded(s).is_none() {
        return Err(format!("loss invalid: {s:?} (legal domain 0-100%, decimals allowed)"));
    }
    Ok(())
}

pub fn validate_gemodel_value(s: &str) -> Result<(), String> {
    // bash tolerates the "25%" shape, profile readability first; the numeric domain convention is 0-100.
    if parse_pct_bounded(s).is_none() {
        return Err(format!("gemodel params invalid: {s:?} (legal domain 0-100, % allowed)"));
    }
    Ok(())
}

pub fn validate_reorder(s: &str) -> Result<(), String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("reorder invalid: {s:?} (needs a non-negative integer %)"));
    }
    Ok(())
}

pub fn validate_rate(v: f64) -> Result<(), String> {
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("rate invalid: {v} (needs a numeric Mbps > 0)"));
    }
    Ok(())
}

pub fn validate_duration(secs: u64) -> Result<(), String> {
    if secs < 5 {
        return Err(format!("duration invalid: {secs} (needs >=5 seconds; 0 not accepted)"));
    }
    Ok(())
}

/// "100" | "0.5%" shape (bash `^(100|[0-9]{1,2})(\.[0-9]+)?%?$`) and <=100.
fn parse_pct_bounded(s: &str) -> Option<f64> {
    let core = s.strip_suffix('%').unwrap_or(s);
    let (int_part, frac_part) = match core.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (core, None),
    };
    if int_part.is_empty() || int_part.len() > 3 || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(f) = frac_part
        && (f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    // integer part <= 2 digits (0-99) or exactly "100" (bash-regex synonym)
    if int_part.len() > 2 && int_part != "100" {
        return None;
    }
    let v: f64 = core.parse().ok()?;
    (v <= 100.0).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple(rtt: u64, jitter: u64, loss: Option<&str>) -> ImpairSpec {
        ImpairSpec {
            rtt_ms: rtt,
            jitter_ms: jitter,
            loss: loss.map(|l| LossSpec::Simple(l.to_string())),
            ..ImpairSpec::default()
        }
    }

    #[test]
    fn leg_delay_table_dir_aware() {
        // design §dir rtt calibration: both = halves, single leg = full (render_netem_spec unaffected by dir)
        let mut s = simple(80, 15, None);
        s.dir = Dir::Both;
        assert_eq!(s.effective_leg_delay_ms(), 40);
        assert_eq!(s.effective_leg_jitter_ms(), 7);
        s.dir = Dir::Out;
        assert_eq!(s.effective_leg_delay_ms(), 80);
        assert_eq!(s.effective_leg_jitter_ms(), 15);
        s.dir = Dir::In;
        assert_eq!(s.effective_leg_delay_ms(), 80);
        // the render baseline always halves (dir doesn't change the legacy render; the 9-fixture case uses the dir=both convention)
        assert_eq!(render_netem_spec(&s), "limit 100000 delay 40ms 7ms distribution normal");
    }

    #[test]
    fn rate_format_roundtrip() {
        assert_eq!(render_rate_arg(None), SENTINEL_RATE);
        assert_eq!(render_rate_arg(Some(4.0)), "4mbit");
        assert_eq!(render_rate_arg(Some(4.5)), "4.5mbit");
        assert_eq!(render_rate_arg(Some(0.25)), "0.25mbit");
    }

    #[test]
    fn gemodel_loss0_demotes_and_zero_rtt_omits_delay() {
        let s = ImpairSpec {
            loss: Some(LossSpec::GeModel {
                loss: "0%".into(),
                r: "0.1".into(),
                h: "0.02".into(),
                k: "0.001".into(),
            }),
            ..Default::default()
        };
        assert_eq!(render_netem_spec(&s), "limit 100000 loss 0%");
    }

    #[test]
    fn seed_reorder_appended_in_bash_order() {
        let mut s = simple(0, 0, Some("1%"));
        s.reorder_pct = Some("5".into());
        s.seed = Some(42);
        assert_eq!(render_netem_spec(&s), "limit 100000 loss 1% reorder 5% seed 42");
    }

    #[test]
    fn jitter_without_rtt_renders_like_bash_single_space() {
        // bash-port fidelity: `spec="$spec ${j}ms ..."` always concatenates with single spaces (rtt=0: the jitter segment follows limit directly).
        assert_eq!(render_netem_spec(&simple(0, 15, None)), "limit 100000 7ms distribution normal");
    }

    #[test]
    fn flat_json_parse_and_rejects() {
        let ok = ImpairSpec::from_flat_json(
            &serde_json::json!({"rtt_ms": 80, "jitter_ms": 15, "loss": "2%", "loss_mode": "simple"}),
        )
        .unwrap();
        assert_eq!(ok, simple(80, 15, Some("2%")));
        // unknown keys rejected
        assert!(ImpairSpec::from_flat_json(&serde_json::json!({"rtt_m": 80})).is_err());
        // rtt out of range (exit-4 table)
        let e = ImpairSpec::from_flat_json(&serde_json::json!({"rtt_ms": 3000})).unwrap_err();
        assert!(e.contains("3000") && e.contains("0-2000"), "{e}");
        // loss not a number
        assert!(ImpairSpec::from_flat_json(&serde_json::json!({"loss": "abc%"})).is_err());
        // loss upper bound 100 included, 100.1 rejected
        assert!(validate_loss("100").is_ok());
        assert!(validate_loss("100.1").is_err());
        assert!(validate_loss("99.99%").is_ok());
        // gemodel missing params
        assert!(
            ImpairSpec::from_flat_json(&serde_json::json!({
                "loss": "5%", "loss_mode": "gemodel", "gemodel": ["1", "2"]
            }))
            .is_err()
        );
    }

    #[test]
    fn duration_and_rate_validation() {
        assert!(validate_duration(4).is_err());
        assert!(validate_duration(5).is_ok());
        assert!(validate_rate(0.0).is_err());
        assert!(validate_rate(f64::NAN).is_err());
        assert!(validate_reorder("5").is_ok());
        assert!(validate_reorder("5%").is_err());
        assert!(validate_gemodel_value("25%").is_ok());
        assert!(validate_gemodel_value("250").is_err());
    }

    #[test]
    fn legs_stream_rejects_empty_and_media_keeps_port_order() {
        // empty set reports cause (never silently all ports)
        assert!(
            build_filter_legs(ScopeSel::Media, IfaceKind::Loopback, Dir::Both, &[], &[]).is_err()
        );
        // Media out: one filter per port, in input order
        let legs =
            build_filter_legs(ScopeSel::Media, IfaceKind::Loopback, Dir::Out, &[9, 3, 7], &[])
                .unwrap();
        let ports: Vec<u16> = legs
            .iter()
            .map(|l| match l.matches.as_slice() {
                [(MatchKind::Sport, p)] => *p,
                other => panic!("expected a single sport match, got {other:?}"),
            })
            .collect();
        assert_eq!(ports, vec![9, 3, 7]);
    }

    #[test]
    fn legs_device_uses_ordered_pair_and_like_stream() {
        // rev-2.2: Device same shape as Stream -- dual-match AND per pair, empty set reports cause
        let legs = build_filter_legs(
            ScopeSel::Device,
            IfaceKind::Physical,
            Dir::Both,
            &[],
            &[(40010, 5000), (40010, 5001)],
        )
        .unwrap();
        assert_eq!(legs.len(), 4);
        assert_eq!(legs[0].matches, vec![(MatchKind::Sport, 40010), (MatchKind::Dport, 5000)]);
        assert_eq!(legs[1].matches, vec![(MatchKind::Sport, 5000), (MatchKind::Dport, 40010)]);
        assert_eq!(legs[3].matches, vec![(MatchKind::Sport, 5001), (MatchKind::Dport, 40010)]);
        assert!(
            build_filter_legs(ScopeSel::Device, IfaceKind::Physical, Dir::Both, &[], &[]).is_err()
        );
    }

    /// T12C anti-deadlock rule-layer pin (design §vehicle 1): all media legs with protocol 17 conjunction +
    /// exact port matching (no wildcard shape: match non-empty, port non-0, flowid always 1:10) --
    /// a builder-layer guarantee that TCP/SSH/DNS never enter netem; the vehicle netns e2e proves the behavior surface.
    #[test]
    fn legs_anti_lockout_pins_protocol_and_exact_ports() {
        for scope in [ScopeSel::Media, ScopeSel::Stream, ScopeSel::Device] {
            for dir in [Dir::Out, Dir::In, Dir::Both] {
                for iface in [IfaceKind::Loopback, IfaceKind::Physical] {
                    let legs =
                        build_filter_legs(scope, iface, dir, &[40010, 40012], &[(40020, 5000)])
                            .unwrap_or_else(|e| panic!("{scope:?}/{dir:?}/{iface:?}: {e}"));
                    assert!(!legs.is_empty());
                    for leg in &legs {
                        assert_eq!(
                            leg.protocol, UDP_PROTOCOL,
                            "media legs must be protocol 17 (UDP)"
                        );
                        assert_eq!(leg.flowid, MEDIA_FLOWID);
                        assert!(
                            !leg.matches.is_empty(),
                            "empty matches forbidden (wildcard shape)"
                        );
                        for (kind, port) in &leg.matches {
                            assert!(*port > 0, "port 0 = wildcard shape, forbidden");
                            assert!(
                                matches!(kind, MatchKind::Sport | MatchKind::Dport),
                                "port-match dimension only (no address/prefix shapes)"
                            );
                        }
                        // shape pin: Media = single match (one leg per port); Stream/Device = dual-match AND
                        match scope {
                            ScopeSel::Media => assert_eq!(leg.matches.len(), 1),
                            _ => assert_eq!(leg.matches.len(), 2),
                        }
                    }
                    if dir == Dir::Both {
                        let expect = if scope == ScopeSel::Media { 4 } else { 2 };
                        assert_eq!(legs.len(), expect, "{scope:?} both leg count");
                    }
                }
            }
        }
    }
}
