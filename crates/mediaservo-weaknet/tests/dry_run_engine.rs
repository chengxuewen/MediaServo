//! T3/T4: dry-run argv snapshots (design §Testing Strategy: engine dry-run argv snapshots enter the default gate).
//! Pins plan_skeleton's full argv: bash build_skeleton structural parity + §dir table protocol-17 AND legs + the signaling leg.
//! Snapshot predicate source = the pure planning functions (zero kernel/zero docker touching); channel prefixes assembled at the main layer (pinned in the engine tests).

use mediaservo_weaknet::engine::{StepPlan, iface_kind_of, plan_skeleton};
use mediaservo_weaknet::spec::{Dir, IfaceKind, ScopeSel, build_filter_legs};

/// Display form: `run [|| alt] [[be]]` -- same rules as main.rs print_dry_run (prefixes aside).
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
    // Media x dir=both x two ports -> sport legs x2 + dport legs x2 (rev-2.1: every leg carries the protocol 17 conjunction).
    let legs =
        build_filter_legs(ScopeSel::Media, IfaceKind::Loopback, Dir::Both, &[40010, 40011], &[])
            .unwrap();
    let steps =
        plan_skeleton("lo", true, "limit 100000 delay 40ms 7ms loss 2%", "1000gbit", &legs, None);
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
    // Stream x both x one pair (40010,50001): one **dual-match AND** single filter per direction (rev-2.2 merged shape).
    let legs =
        build_filter_legs(ScopeSel::Stream, IfaceKind::Loopback, Dir::Both, &[], &[(40010, 50001)])
            .unwrap();
    let steps = plan_skeleton("lo", false, "limit 100000 delay 80ms loss 5%", "4mbit", &legs, None);
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
    // Media x out x one port + signaling leg (prio 2 / protocol 6 / dport -- same shape as bash --signaling);
    // --iface parameterization: dev replaced across the whole chain (argv evidence for the M4' physical-port surface).
    let legs =
        build_filter_legs(ScopeSel::Media, iface_kind_of("eth0"), Dir::Out, &[20000], &[]).unwrap();
    let steps =
        plan_skeleton("eth0", true, "limit 100000 delay 60ms", "1000gbit", &legs, Some(9800));
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
    // no absolute-path literals anywhere in the chain (C20 inspection surface pinned in passing).
    assert!(got.iter().all(|l| !l.contains('/')), "{got:?}");
}

// ---------- T9: ifb mirror chain dry-run snapshot (design §ifb apply order + §dir physical-port leg split) ----------

/// Display form (with prog prefixes; same rules as print_dry_run).
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
    // physical port dir=In = a pure mirror chain without the skeleton (replay: install_skeleton=false same shape).
    use mediaservo_weaknet::engine::plan_ifb;
    let legs_in =
        build_filter_legs(ScopeSel::Media, IfaceKind::Physical, Dir::In, &[40010], &[]).unwrap();
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
    // dir=Both physical port = egress installs only Out legs (root htb/netem/sport filter) + the ifb mirror for In legs;
    // dport legs never enter the egress root (§dir table: in legs move to ifb).
    use mediaservo_weaknet::engine::plan_ifb;
    let legs_out =
        build_filter_legs(ScopeSel::Stream, IfaceKind::Physical, Dir::Out, &[], &[(40010, 50001)])
            .unwrap();
    let legs_in =
        build_filter_legs(ScopeSel::Stream, IfaceKind::Physical, Dir::In, &[], &[(40010, 50001)])
            .unwrap();
    let mut steps =
        plan_skeleton("ens32", false, "limit 100000 delay 40ms", "1000gbit", &legs_out, None);
    steps.extend(plan_ifb("ens32", "limit 100000 delay 40ms", &legs_in, false));
    let shown = render2(&steps);
    // egress section: only the (sport=40010 AND dport=50001) outbound AND leg, no link step (ifb0-already-present form).
    assert!(
        shown.iter().any(|l| l.contains("filter add dev ens32 parent 1:")
            && l.contains("sport 40010")
            && l.contains("dport 50001")),
        "{shown:?}"
    );
    assert!(
        shown.iter().all(|l| !l.contains("parent 1:") || !l.contains("sport 50001")),
        "an in-direction leg must not mix into the egress root: {shown:?}"
    );
    // the ifb section comes after the skeleton and has no link add (need_link=false); up is always present (the ENETDOWN red line).
    let idx_up = shown.iter().position(|l| l == "ip link set ifb0 up").unwrap();
    let idx_mir = shown.iter().position(|l| l.contains("mirred egress redirect dev ifb0")).unwrap();
    assert!(shown.iter().all(|l| !l.contains("link add")));
    assert!(idx_up < idx_mir, "{shown:?}");
    // the mirror leg = the reversed AND pair (sport=50001 AND dport=40010).
    assert!(
        shown[idx_mir].contains("match ip sport 50001 0xffff match ip dport 40010 0xffff"),
        "{}",
        shown[idx_mir]
    );
}
