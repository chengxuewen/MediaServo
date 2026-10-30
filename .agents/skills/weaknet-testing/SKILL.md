---
name: weaknet-testing
description: "Weak-network reproduction and measurement discipline for MediaServo (tc netem + the mediaservo-weaknet agent, optional web panel). Use when reproducing weak-network behavior (latency/jitter/loss/bandwidth cap), running scenario tests, verifying D274 stream_mode presets under real impairment, debugging bitrate collapse, or when any weaknet command is involved. Triggers: 'weak net', 'weak network', 'netem', 'packet loss', 'delay jitter', 'rate cap test', 'stream_mode'."
---

# Weak-Network Testing -- weaknet agent discipline

The binary is `mediaservo-weaknet`; the thin wrapper is `scripts/weaknet.sh` in
this repo. In the msrtc workspace the same wrapper is reached as
`./msrtc.sh weaknet ...`; in that workspace, prefix the relative paths below
with the submodule checkout path.

## Tool surface

The bash `weaknet.sh` / `probe.sh` helpers were retired at T10; git history keeps
the anchor.

```bash
# standalone (this repo)
pixi run cargo build -p mediaservo-weaknet
./scripts/weaknet.sh serve
./scripts/weaknet.sh apply --profile smoke
./scripts/weaknet.sh apply --rtt 160 --loss 8% --gemodel 25% 0.2 0.05 --rate 4 --stream vehicle_test1
./scripts/weaknet.sh apply --profile cell-edge --device truck07 --dir in
./scripts/weaknet.sh set --loss 20%
./scripts/weaknet.sh scenario run example-cell-edge-40s [--auto-clear]
./scripts/weaknet.sh status | clear

# sampling: panel curves, or sampler + agent measure
# (judge.py retired at SR6 P3-half-B; the in-crate frozen goldens are the truth)
node scripts/weaknet.d/sampler.mjs <seconds> <interval> > probe.jsonl
out/server/bin/mediaservo-weaknet measure probe.jsonl <statedir>/timeline
```

Panel notes:

- The panel ships with the server cluster (third oxfile unit after deploy); do not start it twice.
- Cluster form (D278): admin:8080 sidebar "weak network panel"; token at
  `out/server/run/weaknet.token`; CLI and panel share the statedir.
- Browser acceptance MUST use the LAN-IP form `http://<server>:8080/weaknet/`;
  localhost masks the Host gate (403) and produces false green.
- Vehicle panel: `ssh -L 9810:127.0.0.1:9810 <jetson>` (run `mediaservo-weaknet serve`
  on the vehicle first, T13).

## Hard rules (each has an incident behind it)

1. **Three stimulus classes, never conflated**: network injection (weaknet) !=
   sender ceiling (D274 bitrate/stream_mode) != in-stack knobs. Reports and
   evidence must name the class; a mixed wording forced a README rework once.
2. **Always read back and prove the hit**: `apply` asserts on tc -s fingerprints
   plus leaf Sent deltas. A hand-written tc run must also read back
   (PIT-184: a container without iproute2 silently no-ops).
3. **`rtt_ms` input semantics**: with `--dir both` (default) lo crosses the qdisc
   twice, so each leg is halved -> observed RTT ~= input. A single leg (`out|in`)
   is NOT halved. Do not hand-write netem delay and then wonder why RTT doubled.
   Off-box topologies (cross-host, container bridge) invalidate lo probes.
4. **Liveness truth = stats list mode**: `/api/admin/sfu/stats` (no query) entries
   carrying `remote_port` are live; `closed()` and reachable stats lie
   (PIT-185: a disconnected transport stays "alive" forever). `--stream` takes
   the ROOM name.
5. **Fuses**: 300 s by default at three layers (lazy self-heal, watchdog, serve
   timer; scenarios may `--auto-clear` the signaling layer). Long experiments pass
   `--duration` (floor 5 s; there is no `--forever`), and a leftover lo qdisc on a
   shared dev box poisons all 127.0.0.1 traffic.
6. **Judge = baseline pairing**: the control-stream ratio must stay > 0.95 (J2);
   collapse on non-target streams is a bug, collapse on the target is expected.
   Recovery uses `recover_ratio` (BWE ramp-up 15-25 s; scenarios hold 25 s).
7. **dev-credentials only**: `WEAKNET_ADMIN_PASS` is for the local dev server;
   never run this skill against production.
8. **CI vs local**: the `test-weaknet` job is the simplified lo form;
   direction- and stream-specific impairment is local-only.

## Failure quick reference

| Symptom | Check first | Likely cause |
|---|---|---|
| apply says "media-port observation empty" | is the server producing? is `--stream` correct (the error lists valid names)? | no host / wrong room name |
| apply says "no live transport" | `--stream` needs the room name `vehicle_<stream-id>` (gateway prefix), not the bare id | used `vehicle_test1` vs `test1` |
| no video reaction after injection | apply's built-in verify rolls back on a no-delta; then `weaknet status` | wrong media port set / cross-host / no stream |
| whole box gets slow | `weaknet status` shows a qdisc without state? | external residue -- `clear` (lazy self-heal also collects) |
| set says "no state" | expired or cleared? | re-apply |
| loss 0 behaves oddly | -- | gemodel auto-degrades to uniform; just pass `--loss 0%` |

## Environment facts

- Privileged path: docker group + rootful daemon (docker group ~= root is a given
  here; do not report it as a vulnerability).
- Image `gaiadocker/iproute2` = 2015 tc: no seed, no `qdisc show parent`, cannot
  replace a non-empty root tree; the Rust engine adapts both sides.
- Single-port model: WebRtcServer fixed port (`MEDIASERVO_SFU_PORT`, default 20000);
  compose 40000-40100 is a legacy dead mapping.
- Suggested matrix order: smoke (alive) -> remote-burst (D274 three-tier
  discrimination) -> cell-edge (extreme) -> bandwidth-wall (pure bandwidth).
