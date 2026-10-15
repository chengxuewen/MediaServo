# Handoff note — ABI baseline execution kickoff (2026-10-15)

> From: TPHMI consumer-side session. For: SDK-side worker sessions (build/bindings owners).
> Companion authoritative spec: `docs/modules/25-abi-baseline.md` (rev3, rulings #1-#19). This note is a delivery-order
> digest + consumer constraints, NOT a spec amendment. If it conflicts with doc25, doc25 wins; report the conflict.

## Execution order (please follow exactly; rationale lives in doc25 §5/§8/§11)

1. **T9 first (tiny)**: gate the Linux-only link args in `bindings/node/rust/mediaservo-node/build.rs:18-19` behind
   `#[cfg(target_os = "linux")]`, same pattern the C bindings already use. Ride-along per ruling #6: the README:85
   win-64 claim vs `pixi.toml` (no win-64) — retract or downgrade to "planned"; do not leave the paper promise.
2. **T2 before T1** (ruling #12, hard order): per-`.so` version scripts first (shared single map would swallow
   field's bare `mediaservo_last_error` export — M①), then the static-embed trio on client-c/field:
   `-static-libstdc++ -static-libgcc -Wl,--exclude-libs,ALL` (+ `cargo:rustc-cdylib-link-arg`).
   Acceptance per artifact: `readelf --dyn-syms -W <so> | awk '/UND/ && /GLIBCXX|CXXABI/' | wc -l` == 0.
3. **T15 is not cosmetic-last**: the English conversion of doc25 **blocks T10b** (all-gates wiring per rulings
   #9/#11/#14). Sequence: T10a script-fix may go any time → T15 → T10b wire.
4. **T5 stays off until T12 + T10 fixes land** (C1 lesson: the gate as drafted was blind to the very incident class
   it shipped for). New `abibaseline` gate **coexists** with the live `abi-drift` gate
   (`scripts/check-abi-client.sh` / `check-abi-drift.sh`, wired at `all-gates.sh:52`) — additive, not a replacement.
5. **T4 = PAUSED** (ruling #17, kept not deleted). Reopen only if a build-side floor drift becomes unlocalizable or an
   18.04/JetPack4 customer list materializes. Do not implement it "while passing through".
6. Dual-family thresholds: x86_64 and aarch64 register their own floor numbers and must each independently hit zero
   (measured field.so: 80 UND x86 vs 106 aarch64 — one number does not cover both boxes).

## Consumer-side constraint (TPHMI) — respect this boundary

Ruling #19=A settles the **documentation posture**: the T13 delivery matrix carries the consumer-app
`-static-libstdc++` recommendation as the primary posture row. It does **not** authorize SDK sessions to add that
flag (or any link flag) into **TPHMI's** build files. Consumer adoption is decided by the TPHMI side (currently
deferred: fleet already runs 3.4.29-era apps; exe-level static embed carries an interposition caveat pending the
`nm --defined-only -D bin/TPHMI | grep -c ' std::'` probe). Please keep the recommendation in the matrix only.

## What we do when your embedded artifacts arrive (T7, consumer-side)

Authoritative acceptance runs in a **pristine ubuntu:20.04 container** (doc25 §11.1 H3 — our dev box's runtime was
manually upgraded to 3.4.35 in March and cannot certify acceptance). Checklist: `src/TPHMI/.sisyphus/plans/abi-exec/acceptance.md`
(TPHMI main repo, local). Includes per-artifact zero-C++-refs, glibc ceiling ≤2.31, export whitelist, NEEDED-closure,
clean-toolchain direct link, dlopen, RC010 + sanitizer smoke, then evidence backfill into doc25 §9 and
`changes/changes-1.4.1` tagged `(1.4.1.2)`.

## Known errata we spotted in your house (fix in your own batch, not blockers)

- doc25 self-count stale: §11.2 #9 says 123 CJK lines, §11.4 says 127 — actual is **132** (refresh during T15).
- §11.2 ruling table splits into two tables (blank line before row 12 → second block has no header row).
- SDK `.agents/memorys/status.md` header (2026-08-26, D196-D254) predates D293/rev3 entirely; D293 records
  #1-#11 only — the rev3 deltas (#12-#19: T4 PAUSED, #19 posture boundary, T2≺T1) deserve an amendment line.
- A shipped `changes-1.4.1` entry describes removing a build-time "pixi libstdc++ preload" — distinguish it from T0 (never built) and T1
  (static embed) when writing the receipt.

— end of handoff note; questions route through the user (single human between sessions).

---

## STATUS UPDATE 2026-10-15 ~16:20 — T9/T2/T1 IMPLEMENTED by the TPHMI session (uncommitted)

The wall is gone. Measured after official `./mediaservo.sh build bindings` (all 5 artifacts):
client/deck/field/link .so + mediaservo.node — **UND GLIBCXX/CXXABI = 0, NEEDED libstdc++ = 0,
exports ⊆ mediaservo_*, glibc ceiling 2.28**. Incident target `tp_tst_msrtcBackend` now LINKS (was red).

### Empirical finding that changes doc25's premise — E1:
**conda-forge gcc's `-static-libstdc++` is a SILENT NO-OP** (minimal repro: undefined `operator new` /
`__gxx_personality_v0` when linking C++ with it). The node precedent (build.rs:18, the 09-17 "fix") was
never actually binding libstdc++.a — .node measured UND=80 + dynamic NEEDED right up to today. The
plain `-lstdc++` that cc/cxx crates emit sits near the END of the rustc cc line, after which the
trailing `-Bstatic` sandwich also loses.

### The recipe that works (already wired into the code):
build.rs probe: `<CXX||CC> -print-file-name=libstdc++.a` → emit `cargo:rustc-link-search=native=<dir>` +
`cargo:rustc-link-lib=static=stdc++` (rustc emits root-crate user libs BEFORE dependency native -l flags;
verified: versioned refs resolve locally AND the DSO NEEDED disappears entirely). Fallback sandwich kept.
Applied: client-c / field-c / node build.rs (+ per-so .map files ×4, exclude-libs, link-c & deck-c keep
sandwich+exclude — they were already C++-free). `.cargo/config.toml` route was tried and REJECTED
(relative -L resolves against rustc CWD, broke rlib compile; reverted to pristine).

### Files touched (all uncommitted — user decides commit split):
bindings/c/mediaservo-{client,field}-c/build.rs (+2 .map), mediaservo-{link,deck}-c (+2 .map, sandwich+exclude only),
bindings/node/rust/mediaservo-node/build.rs (cfg gate + probe + exclude-libs), README.md:85 (win-64 → "planned, not supported"),
.cargo/config.toml (reverted, zero diff). RUSTFLAGS/`.cargo` NOT required — recipe is self-contained per crate.

### Still yours / open:
- T3 pure-C boundary test, T10 guard fix(+LC_ALL)+wiring after T15, T12 gate (now measurable: assert UND==0 & NEEDED-no-stdc per family),
  T13/T14/T15, doc25 §1 errata: the "static trio" wording is empirically dead on conda — replace with the probe recipe + this section as evidence.
- Jetson aarch64: system-toolchain path — probe is harmless there (finds system libstdc++.a at gcc-10 dir, binds static;
  measure field.so aarch64 after next build — if system gcc's -static-libstdc++ works, probe still wins deterministically).
