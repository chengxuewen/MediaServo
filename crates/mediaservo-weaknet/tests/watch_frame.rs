//! watch 帧快照测（design §测试策略「watch 帧快照」，T12）：render_frame 纯函数面——
//! 参数/倒计时/dropped 阶梯/尾事件行（sig_lines 注入形）逐钉，不触内核不读环境。

use mediaservo_weaknet::engine::LeafStats;
use mediaservo_weaknet::spec::{Dir, ImpairSpec, LossSpec, ScopeSel};
use mediaservo_weaknet::state::{ChannelSer, State, Teardown};
use mediaservo_weaknet::watch;

fn state(iface: &str, expires_at_ms: u64) -> State {
    State {
        schema: mediaservo_weaknet::state::STATE_SCHEMA.to_string(),
        spec: ImpairSpec {
            rtt_ms: 80,
            jitter_ms: 0,
            loss: Some(LossSpec::Simple("2%".into())),
            ..ImpairSpec::default()
        },
        dir: Dir::Both,
        scope: ScopeSel::Media,
        iface: iface.to_string(),
        ports: vec![40010, 40012],
        pairs: vec![],
        rooms: vec![],
        devices: vec![],
        sig_port: None,
        expires_at_ms,
        created_root: true,
        ifb_used: false,
        created_ifb: false,
        job: None,
        teardown: Teardown {
            channel: ChannelSer::LocalRoot,
            sidecar: None,
            steps: vec![],
        },
    }
}

#[test]
fn frame_active_session_pins_all_blocks() {
    let st = state("lo", 100_000);
    let leaf = LeafStats {
        sent_pkt: 1000,
        dropped: 250,
    };
    let sig = vec![
        "channel=local-root watchdog=ok".to_string(),
        "scope=Media dir=Both ports=[40010 40012]".to_string(),
        "{\"ev\":\"apply\",\"spec\":\"rtt=80ms\"}".to_string(),
    ];
    let frame = watch::render_frame(Some(&st), Some(leaf), &sig, 100, 40_000);
    let lines: Vec<&str> = frame.lines().collect();
    assert_eq!(
        lines[0],
        "weaknet watch — lo 剩余 60s",
        "头行=iface+倒计时（(100000-40000)/1000）"
    );
    assert!(lines[1].starts_with("rtt=80ms"), "参数行=param_summary: {}", lines[1]);
    assert!(lines[1].contains("loss=2%"), "param_summary 全形: {}", lines[1]);
    assert!(lines[2].contains("sent=1000"), "计数行: {}", lines[2]);
    assert!(lines[2].contains("dropped=250"), "计数行: {}", lines[2]);
    assert!(lines[2].contains("阶梯["), "阶梯条: {}", lines[2]);
    assert!(lines[2].contains("25%"), "dropped 占比: {}", lines[2]);
    for s in &sig {
        assert!(frame.contains(s), "sig 行回显缺失: {s}");
    }
    // 快照恒定性：同输入逐字节同输出（重绘稳定 = 无内部状态泄漏）
    assert_eq!(frame, watch::render_frame(Some(&st), Some(leaf), &sig, 100, 40_000));
}

#[test]
fn frame_inactive_and_boundary_countdowns() {
    let frame = watch::render_frame(None, None, &["watchdog=none".to_string()], 80, 0);
    assert!(frame.starts_with("weaknet watch — 无活跃现场"), "{frame}");
    assert!(frame.contains("watchdog=none"));

    // forever 档（expires=0）与已过期档（自愈将清措辞）
    let st = state("lo", 0);
    assert!(watch::render_frame(Some(&st), None, &[], 80, 5).contains("剩余 forever"));
    let st = state("lo", 10);
    assert!(
        watch::render_frame(Some(&st), None, &[], 80, 20).contains("已过期（惰性自愈将清）")
    );

    // 零 sent 除零防御 + 无 leaf（读失败）= sent=0 dropped=0 阶梯 0%
    let st = state("lo", 100);
    let f = watch::render_frame(Some(&st), None, &[], 80, 0);
    assert!(f.contains("sent=0 dropped=0"), "{f}");
    assert!(f.contains("0%"), "{f}");
}

#[test]
fn frame_truncates_long_sig_lines_to_width() {
    let long = "x".repeat(500);
    let f = watch::render_frame(None, None, std::slice::from_ref(&long), 40, 0);
    let line = f.lines().find(|l| l.starts_with('x')).unwrap();
    assert_eq!(line.chars().count(), 40, "超长尾行按 w 截断");
}
