#!/usr/bin/env python3
"""scenario_judge_contract.py —— T8：Rust scenario runner 产物 → judge.py 消费合同。

输入（由 tests/scenario_engine.rs 写入本仓 CARGO_TARGET_TMPDIR = <target>/tmp，gate 传目录）：
  scenario_clean_timeline.jsonl     干净跑（NoopExec 全链，含 baseline/两 set/两 step/clear）
  scenario_aborted_timeline.jsonl   lock-busy abort 跑（scenario-end 带 aborted 字段）
  abort_clean.txt / abort_aborted.txt  Rust scenario::aborted_in_timeline 权威标记

合同断言（真件 = 主仓 scripts/weaknet.d/judge.py，importlib 载入，零 fork）：
  C1 白名单事件选择生效：scenario* ∪ {baseline-done, set} 全进 judge 视野。
  C2 三窗（baseline/damage/recovery）均可从 clean timeline 推导，且合成 probe 填满 →
     window_avg 每窗非空、数值随窗位正确分层。
  C3 恢复步判据（judge.py:71）= set.after 前缀 rtt=0ms ∧ 含 loss=0% → recovery_from 命中清零步。
  C4 aborted ⇒ 判据作废（rev-2.2）：Rust aborted_in_timeline 标记 与 Python 自扫
     scenario-end.aborted 一致——clean（none / 无 aborted → verdict 可用）；
     aborted（lock-busy / 有 aborted → verdict 作废，消费方 W6/T10 必须丢弃）。

用法: python3 tests/scenario_judge_contract.py [artifacts_dir]
退出码 0=合同全绿；非 0=断裂（stderr 指明哪条）。judge.py 缺失亦非 0（不静默）。
"""
import importlib.util
import json
import os
import sys
from datetime import timedelta

HERE = os.path.dirname(os.path.abspath(__file__))
# judge.py 真值源 = 主仓 scripts/weaknet.d/judge.py。本 crate 在 submodule（MediaServo）内，
# 从 HERE 逐层上溯探测 scripts/weaknet.d/judge.py（免硬编码绝对/深度，C20）。
# WEAKNET_JUDGE 可显式覆写。消费者侧 import，不改写 judge.py 本体。
def _find_judge():
    if os.environ.get("WEAKNET_JUDGE"):
        return os.environ["WEAKNET_JUDGE"]
    d = HERE
    for _ in range(8):
        cand = os.path.join(d, "scripts", "weaknet.d", "judge.py")
        if os.path.isfile(cand):
            return cand
        parent = os.path.dirname(d)
        if parent == d:
            break
        d = parent
    return os.path.join(HERE, *([".."] * 5), "scripts", "weaknet.d", "judge.py")


JUDGE_PATH = _find_judge()

_failures = []


def check(cond, label):
    if cond:
        print(f"  PASS  {label}")
    else:
        print(f"  FAIL  {label}")
        _failures.append(label)


def load_judge():
    if not os.path.isfile(JUDGE_PATH):
        sys.stderr.write(f"judge.py 缺失: {JUDGE_PATH}（主仓 scripts/weaknet.d 未就位）\n")
        sys.exit(2)
    spec = importlib.util.spec_from_file_location("wnet_judge", JUDGE_PATH)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def read_events(path):
    rows = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def synth_probe(judge, events, rooms):
    """围绕 timeline 事件时刻铺 1s 粒度 browser probe：
    baseline 窗=2000kbps，damage 窗=300（定向塌陷），recovery 窗起=1800（回升）。"""
    start = next(judge.ts(e["t_utc"]) for e in events if e["ev"] == "scenario-start")
    base_done = next(
        (judge.ts(e["t_utc"]) for e in events if e["ev"] == "baseline-done"),
        start + timedelta(seconds=10),
    )
    recovery_from = detect_recovery(judge, events) or (
        next(judge.ts(e["t_utc"]) for e in events if e["ev"] == "scenario-end")
    )
    ends = next(judge.ts(e["t_utc"]) for e in events if e["ev"] == "scenario-end")
    probe = []
    t = start - timedelta(seconds=1)
    horizon = ends + timedelta(seconds=2)
    while t <= horizon:
        if t < base_done:
            k = 2000
        elif t < recovery_from:
            k = 300
        else:
            k = 1800
        for room in rooms:
            probe.append({"src": "browser", "t_utc": t.strftime("%Y-%m-%dT%H:%M:%SZ"), "room": room, "kbps": k, "fps": 30})
        t = t + timedelta(seconds=1)
    return probe, start, base_done, recovery_from, ends


def detect_recovery(judge, events):
    """复刻 judge.py:65-75 窗推导：首个『末位 set.after 前缀 rtt=0ms ∧ 含 loss=0%』的 step 时刻。"""
    steps = [e for e in events if e["ev"] == "scenario-step"]
    for e in steps:
        t = judge.ts(e["t_utc"])
        after = next(
            (
                s.get("after", "")
                for s in reversed([x for x in events if x["ev"] == "set" and judge.ts(x["t_utc"]) <= t])
                if "after" in s
            ),
            "",
        )
        if after.startswith("rtt=0ms") and "loss=0%" in after:
            return t
    return None


def scan_aborted(events):
    """Python 消费侧扫 scenario-end.aborted（rev-2.2 作废合同面）。"""
    return [e.get("aborted") for e in events if e["ev"] == "scenario-end" and "aborted" in e]


def main():
    art = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("WN_ART")
    if not art or not os.path.isdir(art):
        sys.stderr.write(f"工件目录缺失: {art!r}（应为主仓 target/tmp——cargo test scenario_engine 已产）\n")
        sys.exit(2)
    judge = load_judge()
    clean_p = os.path.join(art, "scenario_clean_timeline.jsonl")
    abort_p = os.path.join(art, "scenario_aborted_timeline.jsonl")
    for p in (clean_p, abort_p):
        if not os.path.isfile(p):
            sys.stderr.write(f"工件缺失: {p}\n")
            sys.exit(2)
    clean = read_events(clean_p)
    abort = read_events(abort_p)

    # ---- C1 白名单事件选择（judge.py:54 口径）----
    ev_types = {e["ev"] for e in clean if e.get("ev", "").startswith("scenario") or e.get("ev") in ("baseline-done", "set")}
    print("C1 白名单事件视野（clean）:")
    check("scenario-start" in ev_types, "scenario-start 入选")
    check("baseline-done" in ev_types, "baseline-done 入选")
    check("set" in ev_types, "set 入选（恢复判据生成源）")
    check("scenario-step" in ev_types, "scenario-step 入选")
    check("scenario-end" in ev_types, "scenario-end 入选")

    # ---- C2/C3 三窗 + 恢复检测（真件 judge.ts/window_avg）----
    print("C2 三窗推导（合成 probe 2000/300/1800 分层）:")
    probe, start, base_done, recovery_from, ends = synth_probe(judge, clean, ["cam0"])
    bw = judge.window_avg(probe, start, base_done)
    fw = judge.window_avg(probe, base_done, recovery_from)
    rw = judge.window_avg(probe, recovery_from, ends + timedelta(seconds=1))
    check(bool(bw.get("cam0")), "baseline 窗非空")
    check(bool(fw.get("cam0")), "damage 窗非空")
    check(bool(rw.get("cam0")), "recovery 窗非空")
    check(abs(bw["cam0"]["kbps"] - 2000) < 1, f"baseline kbps≈2000（得 {bw['cam0']['kbps']:.0f}）")
    check(abs(fw["cam0"]["kbps"] - 300) < 1, f"damage kbps≈300（得 {fw['cam0']['kbps']:.0f}）")
    check(abs(rw["cam0"]["kbps"] - 1800) < 1, f"recovery kbps≈1800（得 {rw['cam0']['kbps']:.0f}）")
    print("C3 恢复步判据（judge.py:71 = after 前缀 rtt=0ms ∧ 含 loss=0%）:")
    check(recovery_from is not None, "recovery_from 命中清零步（非 None）")
    check(recovery_from > base_done, "recovery_from 晚于 baseline-done（窗序正确）")
    check(recovery_from <= ends, "recovery_from 不晚于 scenario-end")

    # ---- C4 aborted ⇒ 判据作废（Rust 标记 vs Python 自扫 一致）----
    print("C4 aborted↔作废（Rust aborted_in_timeline 标记 = Python 扫 result 一致）:")
    rust_clean = open(os.path.join(art, "abort_clean.txt"), encoding="utf-8").read().strip()
    rust_abort = open(os.path.join(art, "abort_aborted.txt"), encoding="utf-8").read().strip()
    py_clean = scan_aborted(clean)
    py_abort = scan_aborted(abort)
    check(rust_clean == "none" and py_clean == [], f"clean：Rust=none ∧ Python 无 aborted → verdict 可用（{py_clean}）")
    check(rust_abort == "lock-busy" and py_abort == ["lock-busy"], f"aborted：Rust=lock-busy ∧ Python={py_abort} → 一致")
    # judge 会误算 aborted 跑（它忽略 aborted），消费方须据此作废——钉作废判据非空即触发
    check(len(py_abort) == 1, "aborted 跑 scenario-end.aborted 恰一 → W6/T10 消费方必须丢弃该轮判据")

    if _failures:
        print(f"\n❌ 合同断裂 {len(_failures)} 项: {_failures}")
        sys.exit(1)
    print("\n✅ scenario↔judge 消费合同全绿")


if __name__ == "__main__":
    main()
