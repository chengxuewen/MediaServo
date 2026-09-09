//! T3/T4: dry-run argv 快照（design §Testing Strategy「引擎 dry-run argv 快照进默认门」）。
//! 钉 plan_skeleton 全量 argv：bash build_skeleton 同构 + §dir 表 protocol-17 AND 腿 + 信令腿。
//! 快照判据源 = 纯规划函数（零内核/零 docker 触达）；channel 前缀由 main 层拼（engine 测已钉形）。

use mediaservo_weaknet::engine::{StepPlan, iface_kind_of, plan_skeleton};
use mediaservo_weaknet::spec::{Dir, IfaceKind, ScopeSel, build_filter_legs};

/// 展示形：`run [|| alt] [[be]]`——与 main.rs print_dry_run 同规则（前缀除外）。
fn render(steps: &[StepPlan]) -> Vec<String> {
    steps
        .iter()
        .map(|s| {
            let mut line = s.run.join(" ");
            if let Some(alt) = &s.alt {
                line.push_str(&format!(" || {}", alt.join(" ")));
            }
            if s.best_effort {
                line.push_str("   [be]");
            }
            line
        })
        .collect()
}

#[test]
fn dry_argv_snapshot_media_both_two_ports() {
    // Media × dir=both × 两口 → sport 腿×2 + dport 腿×2（rev-2.1：每腿 protocol 17 合取）。
    let legs = build_filter_legs(
        ScopeSel::Media,
        IfaceKind::Loopback,
        Dir::Both,
        &[40010, 40011],
        &[],
    )
    .unwrap();
    let steps = plan_skeleton(
        "lo",
        true,
        "limit 100000 delay 40ms 7ms loss 2%",
        "1000gbit",
        &legs,
        None,
    );
    assert_eq!(
        render(&steps),
        vec![
            "qdisc del dev lo root   [be]",
            "qdisc add dev lo root handle 1: htb default 99",
            "class change dev lo parent 1: classid 1:99 htb rate 1000gbit || class add dev lo parent 1: classid 1:99 htb rate 1000gbit",
            "class change dev lo parent 1: classid 1:10 htb rate 1000gbit ceil 1000gbit || class add dev lo parent 1: classid 1:10 htb rate 1000gbit ceil 1000gbit",
            "qdisc change dev lo parent 1:10 handle 10: netem limit 100000 delay 40ms 7ms loss 2% || qdisc add dev lo parent 1:10 handle 10: netem limit 100000 delay 40ms 7ms loss 2%",
            "filter del dev lo parent 1:   [be]",
            "filter add dev lo parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport 40010 0xffff flowid 1:10",
            "filter add dev lo parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport 40011 0xffff flowid 1:10",
            "filter add dev lo parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip dport 40010 0xffff flowid 1:10",
            "filter add dev lo parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip dport 40011 0xffff flowid 1:10",
        ]
    );
}

#[test]
fn dry_argv_snapshot_stream_and_pair_out_in() {
    // Stream × both × 单对 (40010,50001)：每向一条**双 match AND** 单 filter（rev-2.2 并案形）。
    let legs = build_filter_legs(
        ScopeSel::Stream,
        IfaceKind::Loopback,
        Dir::Both,
        &[],
        &[(40010, 50001)],
    )
    .unwrap();
    let steps = plan_skeleton(
        "lo",
        false,
        "limit 100000 delay 80ms loss 5%",
        "4mbit",
        &legs,
        None,
    );
    assert_eq!(
        render(&steps),
        vec![
            "class change dev lo parent 1: classid 1:99 htb rate 1000gbit || class add dev lo parent 1: classid 1:99 htb rate 1000gbit",
            "class change dev lo parent 1: classid 1:10 htb rate 4mbit ceil 1000gbit || class add dev lo parent 1: classid 1:10 htb rate 4mbit ceil 1000gbit",
            "qdisc change dev lo parent 1:10 handle 10: netem limit 100000 delay 80ms loss 5% || qdisc add dev lo parent 1:10 handle 10: netem limit 100000 delay 80ms loss 5%",
            "filter del dev lo parent 1:   [be]",
            "filter add dev lo parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport 40010 0xffff match ip dport 50001 0xffff flowid 1:10",
            "filter add dev lo parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport 50001 0xffff match ip dport 40010 0xffff flowid 1:10",
        ]
    );
}

#[test]
fn dry_argv_snapshot_media_out_with_sig_leg_and_iface_param() {
    // Media × out × 单口 + 信令腿（prio 2 / protocol 6 / dport——bash --signaling 同形）；
    // --iface 参数化：dev 全链替换（物理口 M4′ 面的 argv 前置证据）。
    let legs = build_filter_legs(
        ScopeSel::Media,
        iface_kind_of("eth0"),
        Dir::Out,
        &[20000],
        &[],
    )
    .unwrap();
    let steps = plan_skeleton(
        "eth0",
        true,
        "limit 100000 delay 60ms",
        "1000gbit",
        &legs,
        Some(9800),
    );
    let got = render(&steps);
    assert_eq!(got.len(), 8);
    assert_eq!(
        got[6],
        "filter add dev eth0 parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport 20000 0xffff flowid 1:10"
    );
    assert_eq!(
        got[7],
        "filter add dev eth0 parent 1: protocol ip prio 2 u32 match ip protocol 6 0xff match ip dport 9800 0xffff flowid 1:10"
    );
    // 全链无绝对路径字面量（C20 巡检面顺带钉）。
    assert!(got.iter().all(|l| !l.contains('/')), "{got:?}");
}

// ---------- T9: ifb 镜像链 dry-run 快照（design §ifb 施加序 + §dir 物理口腿切分） ----------

/// 展示形（含 prog 前缀；print_dry_run 同规则）。
fn render2(steps: &[StepPlan]) -> Vec<String> {
    steps
        .iter()
        .map(|s| {
            let mut line = format!("{} {}", s.prog.str(), s.run.join(" "));
            if let Some(alt) = &s.alt {
                line.push_str(&format!(" || {}", alt.join(" ")));
            }
            if s.best_effort {
                line.push_str("   [be]");
            }
            line
        })
        .collect()
}

#[test]
fn dry_argv_snapshot_physical_in_pure_ifb_chain() {
    // 物理口 dir=In = 无骨架纯镜像链（replay: install_skeleton=false 同形）。
    use mediaservo_weaknet::engine::plan_ifb;
    let legs_in = build_filter_legs(
        ScopeSel::Media,
        IfaceKind::Physical,
        Dir::In,
        &[40010],
        &[],
    )
    .unwrap();
    let steps = plan_ifb("ens32", "limit 100000 delay 80ms loss 10%", &legs_in, true);
    assert_eq!(
        render2(&steps),
        vec![
            "ip link add ifb0 type ifb",
            "ip link set ifb0 up",
            "tc qdisc change dev ifb0 root netem limit 100000 delay 80ms loss 10% || qdisc add dev ifb0 root netem limit 100000 delay 80ms loss 10%",
            "tc qdisc add dev ens32 ingress   [be]",
            "tc filter del dev ens32 ingress   [be]",
            "tc filter add dev ens32 ingress protocol ip u32 match ip protocol 17 0xff match ip dport 40010 0xffff action mirred egress redirect dev ifb0",
        ]
    );
}

#[test]
fn dry_argv_snapshot_physical_both_egress_skeleton_plus_ingress_mirror() {
    // dir=Both 物理口 = egress 只装 Out 腿（root htb/netem/sport filter）+ ifb 镜像 In 腿；
    // dport 腿绝不进 egress root（§dir 表：in 腿改走 ifb）。
    use mediaservo_weaknet::engine::plan_ifb;
    let legs_out = build_filter_legs(
        ScopeSel::Stream,
        IfaceKind::Physical,
        Dir::Out,
        &[],
        &[(40010, 50001)],
    )
    .unwrap();
    let legs_in = build_filter_legs(
        ScopeSel::Stream,
        IfaceKind::Physical,
        Dir::In,
        &[],
        &[(40010, 50001)],
    )
    .unwrap();
    let mut steps = plan_skeleton("ens32", false, "limit 100000 delay 40ms", "1000gbit", &legs_out, None);
    steps.extend(plan_ifb("ens32", "limit 100000 delay 40ms", &legs_in, false));
    let shown = render2(&steps);
    // egress 段：仅 (sport=40010 ∧ dport=50001) 出向 AND 腿，无 link 步（ifb0 已在形）。
    assert!(shown.iter().any(|l| l.contains("filter add dev ens32 parent 1:")
        && l.contains("sport 40010") && l.contains("dport 50001")), "{shown:?}");
    assert!(shown.iter().all(|l| !l.contains("parent 1:") || !l.contains("sport 50001")),
        "in 向腿不得混入 egress root: {shown:?}");
    // ifb 段在骨架之后且无 link add（need_link=false）；up 恒在（ENETDOWN 红线）。
    let idx_up = shown.iter().position(|l| l == "ip link set ifb0 up").unwrap();
    let idx_mir = shown.iter().position(|l| l.contains("mirred egress redirect dev ifb0")).unwrap();
    assert!(shown.iter().all(|l| !l.contains("link add")));
    assert!(idx_up < idx_mir, "{shown:?}");
    // 镜像腿 = 反向 AND 对 (sport=50001 ∧ dport=40010)。
    assert!(shown[idx_mir].contains("match ip sport 50001 0xffff match ip dport 40010 0xffff"), "{}", shown[idx_mir]);
}
