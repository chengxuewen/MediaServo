# oxmgr upstream feedback — vendor integration gaps observed in production

> Status: feedback letter (informational; filed 2026-10-19 from the msrtv302 vehicle
> round). oxmgr 0.5.0, archived per D281 (tarball + sha256 pin). Nothing here blocks
> us — every gap is worked around in our own layer (see PLAN §6.2 k12/k14/k16 and
> `.agents/memorys/pitfalls.md` PIT-223/224). This letter exists so the vendor can
> fix the root causes and we can retire our workarounds on the next version bump.

## 1. `service install` — not usable for multi-instance deployments

`oxmgr service install` takes no arguments: fixed unit name `oxmgr.service`, bare
`ExecStart=oxmgr daemon run`, and no way to inject environment (OXMGR_HOME /
OXMGR_DATA_DIR / OXMGR_DAEMON_ADDR / OXMGR_API_ADDR) or override the unit name and
ExecStart. Consequences we hit:

- Two instances (host tree + server tree) would fight over one unit file.
- Without OXMGR_DATA_DIR the daemon lands in the XDG shared directory while CLI
  calls target the instance directory — registry split-brain (PIT-223).
- Without an instance-pinned daemon port, CLI "daemon alive" probes look at a port
  the unit daemon never bound (our F19 finding; see §3).

Ask: `service install --name <unit> --env KEY=VAL ... [--exec-start <cmd>]`, or at
minimum an `--environment-file` passthrough.

## 2. `startup` — prints instructions instead of installing

`oxmgr startup` only prints a template for the user to hand-create. Combined with §1
this means every embedder re-implements unit rendering. We did (per-instance unit
name, four env keys baked from a single identity source, post-install probe); we
would rather not.

Ask: make `startup`/`service` actually install, parameterized per §1.

## 3. Daemon liveness truth + auto-spawn parasitism (the sharpest one)

When the vendor CLI cannot reach the daemon address it reports "Daemon is not
running" and silently spawns a new daemon. Observed failure chain on one machine
(full write-up: our F19 + PIT-224):

1. A daemon was alive for the SAME OXMGR_HOME but on a DIFFERENT port (our render
   bug; any deployment mistake reproduces this).
2. CLI: "Daemon is not running" (false — a same-home daemon existed) → auto-spawn.
3. The spawned daemon unlink-rebinds `events.sock` in the shared home (socket
   hijack) — now two daemons share one registry directory.
4. CLI stop/apply then hit the newcomer's EMPTY registry while the real flock keeps
   running: `stop` printed "verified" with 12 live processes; `apply` mutated the
   wrong registry. Our k12 fix (upstream-applicable): probe the derived addr; if
   dead, scan same-uid processes for environ OXMGR_HOME equal to ours — found =
   refuse with the pid/ports, never auto-spawn onto an occupied home.

Ask: before auto-spawning, (a) treat "same-home daemon reachable on another addr"
as a hard error naming the pid, and (b) make the socket bind fail (or at least
warn) when `events.sock` already has a live listener instead of unlink-rebind.

## 4. Daemon SIGTERM orphans all supervised children

`kill <daemon pid>` (and systemctl stop of a unit wrapping it) leaves every managed
app running with ppid→1 — they keep streaming. Ops-correct shutdown today is
apps-first (`our msrtc-host stop` verb order), or systemd KillMode=control-group
cgroup sweep. Vendor-side ask: on daemon SIGTERM, stop children gracefully
(stop_timeout respected) before exit — or document loudly that the daemon must
never be signaled directly.

## 5. Minor

- `oxmgr list` table truncates NAME to 15 chars in some renders (ps comm
  truncation leaks through); the JSON form is fine.
- No `--version` JSON / build-info field we can pin in our doctor drift checks
  (we archive the tarball, but a runtime identity string would harden C49-style
  exe verification).

## What we ship meanwhile (cross-reference)

- k12: single-source daemon identity (unit env ≡ CLI env, canonicalize-once) +
  same-home foreign-daemon refusal (§3) — `crates/mediaservo-host/src/bin/host.rs`
  (`startup_daemon_env`, `daemon_spawn_action`).
- k14: our `startup on` enables linger itself (polkit no-sudo) — vendor `service
  install` could adopt the same pattern.
- k16: this letter; revisit at oxmgr version bump (D281 archive pin).

## Addendum (2026-10-19, second field reboot): cold-boot daemon on an unsynced clock discards the registry

Repro (real Jetson, no RTC battery): boot at fake clock `2023-11-22`; the unit (ExecStart=oxmgr
daemon run) starts ~2s later, still pre-sync; NTP lands 73s later. Outcome: `state.json` rewritten
empty (30 B) at daemon start — the 8 "running" entries from the previous session are discarded, no
respawn, no log. The same daemon restarted warm (clock sane, same state.json shape) restores all
entries fine. Suspect the restore path's timestamp/consistency check treating future-dated entries
as corrupt. Request: tolerate clock jumps (defer registry GC until clock sanity, or retry restore
on first successful time sync). Our mitigation meanwhile: ExecStartPost idempotent apply hook
(k19) — boot semantics no longer depend on the restore path.
