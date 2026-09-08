//! spec.rs —— 损伤参数语义本体（CLI 是本体、serve/UI 是壳；design.md §dir 腿定义表为唯一真值源）。
//!
//! 渲染合同 = `scripts/weaknet.sh` 的 `render_netem_spec` / `rate_arg` 逐字符平移
//! （golden fixture 9 继承案钉死，dir=both 基准）；dir 新契约 3 案由 `build_filter_legs` 承接。
//! 退出码 4（参数非法）语义由本模块校验函数镜像（表抄自 weaknet.sh parse_opts）。
//!
//! 注意（bash 平移保真）：`render_netem_spec` **恒对半折算** rtt/jitter（lo 双向各过一次
//! qdisc 的口径）；dir 感知的单腿全额规则在 [`effective_leg_delay_ms`]，安装层（T3/T5）取用。

use serde::{Deserialize, Serialize};

/// 方向（与 clap 面同形；lib 语义源，main.rs 的 CLI enum 向其转换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Out,
    In,
    #[default]
    Both,
}

/// 作用域选择（§dir 表行维；Ports 逃生门在 T3 scope.rs 归一为 Media 形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScopeSel {
    /// 段级：本地媒体口集（server listen/local ports）
    Media,
    /// 流级：有序对 (local, remote)——单 filter 双 match AND
    Stream,
    /// 设备级：该 owner 全部流 remote_port 并集（无配对的 Stream 形）
    Device,
}

/// 接口种类。逻辑腿形状与 iface 无关（同一函数产出）；差别在**安装点**——
/// 物理网卡 dir=in 改走 ifb 镜像 ingress（design §ifb 合同，T9），lo 恒端口腿。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IfaceKind {
    Loopback,
    Physical,
}

/// 丢包模式（Simple 携带原始百分串——"2%" 或 "0.5%"；"0"/"0%" 渲染时跳过）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LossSpec {
    Simple(String),
    /// 四值皆原始串（允许 "25%" 形态，tc 侧一律裸数 = 渲染时剥 %）。
    GeModel {
        loss: String,
        r: String,
        h: String,
        k: String,
    },
}

/// 完整损伤规格。limit 缺省 100000（bash NETEM_LIMIT：netem 默认 100 包，配 delay 会静默自伤丢包）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImpairSpec {
    pub rtt_ms: u64,
    pub jitter_ms: u64,
    pub loss: Option<LossSpec>,
    /// 乱序百分比裸整数串（渲染时补 '%'，bash `reorder ${REORDER_PCT}%` 同构）。
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
/// 未设速率时 htb 哨兵（不限速=天文带宽；bash SENTINEL_RATE）。
pub const SENTINEL_RATE: &str = "1000gbit";

impl ImpairSpec {
    /// 从 flat JSON（golden fixture `input` 形 / CLI 合并后形）解析。
    /// 键：rtt_ms, jitter_ms, loss, loss_mode("simple"|"gemodel"), gemodel([r,h,k]),
    /// reorder, rate_mbps, seed, limit, dir("out"|"in"|"both")。
    /// 校验镜像 bash exit-4 表；未知键拒绝（typos 显性化，禁静默吞）。
    pub fn from_flat_json(v: &serde_json::Value) -> Result<Self, String> {
        let obj = v
            .as_object()
            .ok_or_else(|| format!("spec input 不是对象: {v}"))?;
        const KNOWN: &[&str] = &[
            "rtt_ms", "jitter_ms", "loss", "loss_mode", "gemodel", "reorder", "rate_mbps",
            "seed", "limit", "dir",
        ];
        for key in obj.keys() {
            if !KNOWN.contains(&key.as_str()) {
                return Err(format!("spec input 未知键 \"{key}\"（合法集: {KNOWN:?}）"));
            }
        }
        let get_u64 = |k: &str| -> Result<u64, String> {
            match obj.get(k) {
                None | Some(serde_json::Value::Null) => Ok(0),
                Some(serde_json::Value::Number(n)) => n
                    .as_u64()
                    .ok_or_else(|| format!("{k} 需非负整数，得 {n}")),
                Some(other) => Err(format!("{k} 需数字，得 {other}")),
            }
        };
        let get_opt_str = |k: &str| -> Result<Option<String>, String> {
            match obj.get(k) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
                Some(other) => Err(format!("{k} 需字符串，得 {other}")),
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
                    _ => return Err(format!("gemodel 需 3 个字符串 [r,h,k]，得 {}", a.len())),
                }
            }
            Some(other) => return Err(format!("gemodel 需数组，得 {other}")),
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
                _ => return Err("gemodel 模式需 loss + gemodel[r,h,k]".to_string()),
            },
            other => return Err(format!("loss_mode 需 simple|gemodel，得 {other:?}")),
        };
        let rate_mbps = match obj.get("rate_mbps") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Number(n)) => {
                let f = n.as_f64().ok_or_else(|| format!("rate_mbps 需数值，得 {n}"))?;
                Some(f)
            }
            Some(other) => return Err(format!("rate_mbps 需数字，得 {other}")),
        };
        let seed = match obj.get("seed") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Number(n)) => {
                Some(n.as_u64().ok_or_else(|| format!("seed 需非负整数，得 {n}"))?)
            }
            Some(other) => return Err(format!("seed 需数字，得 {other}")),
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
                other => return Err(format!("dir 需 out|in|both，得 {other:?}")),
            },
            Some(other) => return Err(format!("dir 需字符串，得 {other}")),
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

    /// 值域校验（镜像 weaknet.sh parse_opts 的 exit-4 表；消息含越界值与合法域）。
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

    /// 单腿安装延迟毫秒（design §dir「rtt 标定规则」：both → 各腿 rtt/2；
    /// 单腿 out|in → 全额——保「观测 RTT ≈ rtt_ms」跨 dir 恒定）。
    /// 与 [`render_netem_spec`]（恒折半的 bash 继承基准）解耦：netem 字符串本身仍走
    /// render + 安装层覆盖 delay 段的路线（T3/T5 argv 组装取此值）。
    #[must_use]
    pub fn effective_leg_delay_ms(&self) -> u64 {
        match self.dir {
            Dir::Both => self.rtt_ms / 2,
            Dir::Out | Dir::In => self.rtt_ms,
        }
    }

    /// jitter 同规则对半（both 折半承袭 bash `j=$((JITTER_MS/2))`；单腿全额）。
    #[must_use]
    pub fn effective_leg_jitter_ms(&self) -> u64 {
        match self.dir {
            Dir::Both => self.jitter_ms / 2,
            Dir::Out | Dir::In => self.jitter_ms,
        }
    }
}

use serde_json::Value;

/// netem 参数串（bash `render_netem_spec` 逐字符平移：limit → delay → loss → reorder → seed）。
/// 保真注：恒对半折算（dir=both 基准）；jitter 段与 delay 段同法单空格拼接（rtt=0 时
/// jitter 直接跟 limit——bash `spec="$spec ${j}ms ..."` 同构）。dir 感知延迟见
/// [`ImpairSpec::effective_leg_delay_ms`]（安装层 T3/T5 取用）。
#[must_use]
pub fn render_netem_spec(s: &ImpairSpec) -> String {
    render_netem_spec_with(s, s.rtt_ms / 2, s.jitter_ms / 2)
}

/// dir 感知叶形（design §dir rtt 标定：both=对半基准（= render_netem_spec）；
/// 单腿 out|in=全额——观测 RTT 跨 dir 恒定）。golden 9 案只钉 legacy 形。
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
    // gemodel 且 loss=0 → 降级 "loss 0%"（内核「无 delay 即 reset」怪癖规避，bash 头注 3）
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
        // "0"/"0%"/空 = 无损伤跳段（bash elif 同义）；GeModel 已在上方分支穷尽
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

/// htb rate 实参（bash `rate_arg`）：未设 → 哨兵；设了 → "{n}mbit"，
/// f64 回打整数不带小数点（4.0 → "4mbit"；4.5 → "4.5mbit"）。
#[must_use]
pub fn render_rate_arg(rate_mbps: Option<f64>) -> String {
    match rate_mbps {
        Some(n) if n.fract() == 0.0 => format!("{}mbit", n as u64),
        Some(n) => format!("{n}mbit"),
        None => SENTINEL_RATE.to_string(),
    }
}

// ---------- §dir 腿定义表（rev-2.1 核心合同） ----------

/// 所有媒体腿的 protocol 合取（UDP；TCP 临时端口碰撞防御）。信令 TCP 腿（protocol 6）
/// 是 bash --signaling 承接位，T3 单列——不在本媒体腿集合内。
pub const UDP_PROTOCOL: u8 = 17;
/// 全部媒体腿的目标 class（build_skeleton 的 1:10）。
pub const MEDIA_FLOWID: &str = "1:10";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegFilter {
    pub protocol: u8,
    /// 同 filter 内多 match = 合取(AND)；("sport"/"dport", port)。禁拆两条单 match filter（=OR 连坐）。
    pub matches: Vec<(MatchKind, u16)>,
    pub flowid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    Sport,
    Dport,
}

/// 按 §dir 表装配方向腿：
/// - Media：out → sport∈P（每口一 filter）；in → dport∈P
/// - Stream/Device（rev-2.2 统一）：有序对集 (L,R)，Device=owner 全部流的 (L,R) 由调用方拍平；
///   out=(sport=L ∧ dport=R) / in=(sport=R ∧ dport=L) 每对单 filter 双 match AND——
///   remote-union 单 match 形废弃（最松合同，Momus 歧义#1/#3 并案裁决）
/// - both = 两向各装（=bash 现状等价）；协议 17 合取恒在。
///
/// iface 不改变逻辑腿形状（物理网卡 dir=in 的安装点改 ifb ingress，§ifb 合同 T9——
/// 本函数即 design 所称「dir-aware 纯函数」，T5/W1 dry-run 快照的判据源）。
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
        // Stream/Device 统一有序对 AND 腿（rev-2.2）
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
            "腿集合为空（scope={scope:?} dir={dir:?} 端口/配对未解析——stats 空集报因 exit2，禁静默全口）"
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

// ---------- 值域校验（exit-4 表；消息含越界值 + 合法域） ----------

pub fn validate_rtt(v: u64) -> Result<(), String> {
    if v > 2000 {
        return Err(format!("rtt 非法: {v}（合法域 0-2000ms 整数）"));
    }
    Ok(())
}

pub fn validate_jitter(v: u64) -> Result<(), String> {
    // bash：非负整数即过（u64 天然非负）；仅类型占位防未来改型。
    // ponytail: 不设上界 = bash 原样；若内核上限要收，归 T3 一并。
    let _ = v;
    Ok(())
}

pub fn validate_loss(s: &str) -> Result<(), String> {
    if parse_pct_bounded(s).is_none() {
        return Err(format!("loss 非法: {s:?}（合法域 0-100%，可小数）"));
    }
    Ok(())
}

pub fn validate_gemodel_value(s: &str) -> Result<(), String> {
    // bash 容忍 "25%" 形态、profile 可读性优先；数值域文档口径 0-100。
    if parse_pct_bounded(s).is_none() {
        return Err(format!("gemodel 参数非法: {s:?}（合法域 0-100，可带%）"));
    }
    Ok(())
}

pub fn validate_reorder(s: &str) -> Result<(), String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("reorder 非法: {s:?}（需非负整数%）"));
    }
    Ok(())
}

pub fn validate_rate(v: f64) -> Result<(), String> {
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("rate 非法: {v}（需 > 0 的 Mbps 数值）"));
    }
    Ok(())
}

pub fn validate_duration(secs: u64) -> Result<(), String> {
    if secs < 5 {
        return Err(format!("duration 非法: {secs}（需 ≥5 秒；0 不接受）"));
    }
    Ok(())
}

/// "100" | "0.5%" 形态（bash `^(100|[0-9]{1,2})(\.[0-9]+)?%?$`）且 ≤100。
fn parse_pct_bounded(s: &str) -> Option<f64> {
    let core = s.strip_suffix('%').unwrap_or(s);
    let (int_part, frac_part) = match core.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (core, None),
    };
    if int_part.is_empty()
        || int_part.len() > 3
        || !int_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    if let Some(f) = frac_part
        && (f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    // 整数部分 ≤ 2 位（0-99）或恰为 "100"（bash 正则同义）
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
        // design §dir rtt 标定：both=对半，单腿=全额（render_netem_spec 不受 dir 影响）
        let mut s = simple(80, 15, None);
        s.dir = Dir::Both;
        assert_eq!(s.effective_leg_delay_ms(), 40);
        assert_eq!(s.effective_leg_jitter_ms(), 7);
        s.dir = Dir::Out;
        assert_eq!(s.effective_leg_delay_ms(), 80);
        assert_eq!(s.effective_leg_jitter_ms(), 15);
        s.dir = Dir::In;
        assert_eq!(s.effective_leg_delay_ms(), 80);
        // render 基准恒折半（dir 不改 legacy 渲染，fixture 9 案 dir=both 口径）
        assert_eq!(
            render_netem_spec(&s),
            "limit 100000 delay 40ms 7ms distribution normal"
        );
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
        // bash 平移保真：`spec="$spec ${j}ms ..."` 恒单空格拼接（rtt=0 时 jitter 段直接跟 limit）。
        assert_eq!(
            render_netem_spec(&simple(0, 15, None)),
            "limit 100000 7ms distribution normal"
        );
    }

    #[test]
    fn flat_json_parse_and_rejects() {
        let ok = ImpairSpec::from_flat_json(
            &serde_json::json!({"rtt_ms": 80, "jitter_ms": 15, "loss": "2%", "loss_mode": "simple"}),
        )
        .unwrap();
        assert_eq!(ok, simple(80, 15, Some("2%")));
        // 未知键拒绝
        assert!(ImpairSpec::from_flat_json(
            &serde_json::json!({"rtt_m": 80})
        )
        .is_err());
        // rtt 越界（exit-4 表）
        let e = ImpairSpec::from_flat_json(&serde_json::json!({"rtt_ms": 3000})).unwrap_err();
        assert!(e.contains("3000") && e.contains("0-2000"), "{e}");
        // loss 非数字
        assert!(ImpairSpec::from_flat_json(
            &serde_json::json!({"loss": "abc%"})
        )
        .is_err());
        // loss 上界 100 含、100.1 拒
        assert!(validate_loss("100").is_ok());
        assert!(validate_loss("100.1").is_err());
        assert!(validate_loss("99.99%").is_ok());
        // gemodel 缺参
        assert!(ImpairSpec::from_flat_json(&serde_json::json!({
            "loss": "5%", "loss_mode": "gemodel", "gemodel": ["1", "2"]
        }))
        .is_err());
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
        // 空集报因（禁静默全口）
        assert!(build_filter_legs(ScopeSel::Media, IfaceKind::Loopback, Dir::Both, &[], &[]).is_err());
        // Media out 每口一 filter，按输入序
        let legs = build_filter_legs(
            ScopeSel::Media,
            IfaceKind::Loopback,
            Dir::Out,
            &[9, 3, 7],
            &[],
        )
        .unwrap();
        let ports: Vec<u16> = legs
            .iter()
            .map(|l| match l.matches.as_slice() {
                [(MatchKind::Sport, p)] => *p,
                other => panic!("期望单 sport match，得 {other:?}"),
            })
            .collect();
        assert_eq!(ports, vec![9, 3, 7]);
    }

    #[test]
    fn legs_device_uses_ordered_pair_and_like_stream() {
        // rev-2.2：Device 与 Stream 同形——每对双 match AND，缺省空集报因
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
        assert!(build_filter_legs(ScopeSel::Device, IfaceKind::Physical, Dir::Both, &[], &[]).is_err());
    }
}
