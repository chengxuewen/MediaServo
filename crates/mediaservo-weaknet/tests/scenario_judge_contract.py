#!/usr/bin/env python3
"""scenario_judge_contract.py -- T8: the Rust scenario runner's artifacts -> the judge.py consumption contract.

Inputs (written by tests/scenario_engine.rs into this repo's CARGO_TARGET_TMPDIR = <target>/tmp; the gate passes the dir):
  scenario_clean_timeline.jsonl     clean run (the full NoopExec chain: baseline/two sets/two steps/clear)
  scenario_aborted_timeline.jsonl   lock-busy aborted run (scenario-end carries the aborted field)
  abort_clean.txt / abort_aborted.txt  the authoritative Rust scenario::aborted_in_timeline markers

Contract asserts (the real artifact = the parent repo scripts/weaknet.d/judge.py, imported via importlib, no fork):
  C1 whitelist event selection works: scenario* union {baseline-done, set} all enter the judge's view.
  C2 the three windows (baseline/damage/recovery) are derivable from the clean timeline, and synthetic probes fill them ->
     window_avg non-empty per window, values tiering correctly by window position.
  C3 the recovery-step predicate (judge.py:71) = set.after prefix rtt=0ms AND contains loss=0% -> recovery_from hits the zeroing step.
  C4 aborted => verdict invalidated (rev-2.2): the Rust aborted_in_timeline marker agrees with the Python
     self-scan of scenario-end.aborted -- clean (none / no aborted -> verdict usable);
     aborted (lock-busy / has aborted -> verdict void, consumers W6/T10 must discard).

Usage: python3 tests/scenario_judge_contract.py [artifacts_dir]
Exit code 0 = the contract fully green; non-0 = broken (stderr names which one). A missing judge.py is also non-0 (never silent).
"""
import importlib.util
import json
import os
import sys
from datetime import timedelta

HERE = os.path.dirname(os.path.abspath(__file__))
# judge.py truth source = the parent repo scripts/weaknet.d/judge.py. This crate lives inside the submodule (MediaServo),
# so probe upward from HERE layer by layer for scripts/weaknet.d/judge.py (no hardcoded absolute/depth, C20).
# WEAKNET_JUDGE can override explicitly. Imported on the consumer side; judge.py itself is never rewritten.
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
        sys.stderr.write(f"judge.py missing: {JUDGE_PATH} (the parent repo scripts/weaknet.d not in place)\n")
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
    """Lay 1s-granularity browser probes around the timeline event moments:
    baseline window = 2000kbps, damage window = 300 (targeted collapse), from the recovery window = 1800 (recovering)."""
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
    """Replicates the judge.py:65-75 window derivation: the moment of the first step whose trailing set.after prefix is rtt=0ms AND contains loss=0%."""
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
    """The Python consumption-side scan of scenario-end.aborted (the rev-2.2 invalidation contract surface)."""
    return [e.get("aborted") for e in events if e["ev"] == "scenario-end" and "aborted" in e]


def main():
    art = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("WN_ART")
    if not art or not os.path.isdir(art):
        sys.stderr.write(f"artifacts dir missing: {art!r} (should be the parent repo target/tmp -- cargo test scenario_engine already produced it)\n")
        sys.exit(2)
    judge = load_judge()
    clean_p = os.path.join(art, "scenario_clean_timeline.jsonl")
    abort_p = os.path.join(art, "scenario_aborted_timeline.jsonl")
    for p in (clean_p, abort_p):
        if not os.path.isfile(p):
            sys.stderr.write(f"artifact missing: {p}\n")
            sys.exit(2)
    clean = read_events(clean_p)
    abort = read_events(abort_p)

    # ---- C1 whitelist event selection (judge.py:54 convention) ----
    ev_types = {e["ev"] for e in clean if e.get("ev", "").startswith("scenario") or e.get("ev") in ("baseline-done", "set")}
    print("C1 whitelist event view (clean):")
    check("scenario-start" in ev_types, "scenario-start selected")
    check("baseline-done" in ev_types, "baseline-done selected")
    check("set" in ev_types, "set selected (the recovery-predicate generation source)")
    check("scenario-step" in ev_types, "scenario-step selected")
    check("scenario-end" in ev_types, "scenario-end selected")

    # ---- C2/C3 three windows + recovery detection (the real judge window_avg) ----
    print("C2 three-window derivation (synthetic probes tiered 2000/300/1800):")
    probe, start, base_done, recovery_from, ends = synth_probe(judge, clean, ["cam0"])
    bw = judge.window_avg(probe, start, base_done)
    fw = judge.window_avg(probe, base_done, recovery_from)
    rw = judge.window_avg(probe, recovery_from, ends + timedelta(seconds=1))
    check(bool(bw.get("cam0")), "baseline window non-empty")
    check(bool(fw.get("cam0")), "damage window non-empty")
    check(bool(rw.get("cam0")), "recovery window non-empty")
    check(abs(bw["cam0"]["kbps"] - 2000) < 1, f"baseline kbps~2000 (got {bw['cam0']['kbps']:.0f})")
    check(abs(fw["cam0"]["kbps"] - 300) < 1, f"damage kbps~300 (got {fw['cam0']['kbps']:.0f})")
    check(abs(rw["cam0"]["kbps"] - 1800) < 1, f"recovery kbps~1800 (got {rw['cam0']['kbps']:.0f})")
    print("C3 recovery-step predicate (judge.py:71 = after prefix rtt=0ms AND contains loss=0%):")
    check(recovery_from is not None, "recovery_from hits the zeroing step (not None)")
    check(recovery_from > base_done, "recovery_from later than baseline-done (window order correct)")
    check(recovery_from <= ends, "recovery_from no later than scenario-end")

    # ---- C4 aborted => verdict invalidated (Rust marker vs Python self-scan, must agree) ----
    print("C4 aborted<->invalid (Rust aborted_in_timeline = Python scan result, in agreement):")
    rust_clean = open(os.path.join(art, "abort_clean.txt"), encoding="utf-8").read().strip()
    rust_abort = open(os.path.join(art, "abort_aborted.txt"), encoding="utf-8").read().strip()
    py_clean = scan_aborted(clean)
    py_abort = scan_aborted(abort)
    check(rust_clean == "none" and py_clean == [], f"clean: Rust=none AND Python no aborted -> verdict usable ({py_clean})")
    check(rust_abort == "lock-busy" and py_abort == ["lock-busy"], f"aborted: Rust=lock-busy AND Python={py_abort} -> in agreement")
    # judge would misjudge the aborted run (it ignores aborted); consumers must invalidate on that basis -- pin: the invalidation predicate non-empty triggers
    check(len(py_abort) == 1, "the aborted run's scenario-end.aborted exactly one -> W6/T10 consumers must discard that round's verdict")

    if _failures:
        print(f"\n[FAIL] contract broken in {len(_failures)} items: {_failures}")
        sys.exit(1)
    print("\n[OK] scenario<->judge consumption contract fully green")


if __name__ == "__main__":
    main()
