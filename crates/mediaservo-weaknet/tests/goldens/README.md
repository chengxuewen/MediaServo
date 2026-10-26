# measure goldens (SR6 P3-half-B, N3 golden-fixture discipline)

Frozen (input, expected-stdout, expected-rc) triples. The Rust `measure`
subcommand must reproduce expected-stdout byte-for-byte (including `nan`
spellings and CRLF-free trailing newline) and expected-rc exactly, per pair.

| dir | provenance |
|-----|-----------|
| v4-live | BYTE-COPY of the real 2026-09-08 V4 harness run: /tmp/fb2/docs/plans/_archive/weaknet-harness/evidence/V4-{probe,timeline}.jsonl (weaknet-harness P0-era live capture, 6 rooms 16:9, remote-burst collapse; judge.py verdict FAIL rc=1, retention ~0.12, nan recovery on 4 rooms). Files committed unmodified. |
| recovery-pass | SYNTHESIZED from the real V4 row schema (browser probe row {t_utc,src:"browser",room,st,fps,res,kbps,jitter_ms} + timeline event shapes copied from V4-timeline.jsonl): 2 rooms, baseline 10s, one damage step, one zeroing (recovery) step; room-b retention 0.94 = WARN band edge; recover_ratio 0.95 lands exactly on default tol. Generator scratch (not committed): /tmp/opencode/gen_goldens.py recipe in git history of this README. |
| nan-recovery | SYNTHESIZED same schema: single room, damage step, NO zeroing step -> recovery window collapses -> `nan` outputs (byte-exact nan formatting branch), rc=1 via retention FAIL. |
| tolerance-pass | SYNTHESIZED same schema: retention 0.97 PASS + 0.80 WARN-band edge, rc=0 (WARN does not flip rc; only FAIL does). |
| tol-flag | reuses recovery-pass inputs with `--tol 0.90`: room-b flips WARN->PASS proving the flag reaches the verdict predicate; footer echoes 0.90. |

Provenance comments in this README = the N3 provenance record; judge.py (deleted
in the same round) was the python oracle that produced expected-stdout.txt at
freeze time (2026-10-23).
