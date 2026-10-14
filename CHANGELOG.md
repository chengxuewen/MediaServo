# Changelog

This file records behavior changes aimed at consumers (deployment, ops, cockpit/vehicle-side
teams); for development process details, consult git log directly. Newest version first.

## Unreleased

### ⚠ Upgrade Notes
- [host][server][sdk-client][sdk-field] english-only-migration (C49): all artifacts and
  user-visible text are now English — log/error/audit message WORDING switched from Chinese to
  English. Operational greps must switch keywords (e.g. streamer subscription line now
  "FrameBus subscribed", no-frame timeout now "no frames for ..."; "acl denied: <topic>" kept).
  Historical keyword forms: submodule git history before this change.
- [sdk-client] client-cxx `client.hpp` was missing `#include <chrono>` — every consumer of
  `Consumer::on_qoe` failed to compile since the 2026-10-09 QOE merge (latent debt, caught by the
  migration gate rerun of test-cxx).

### Added
- [protocol] wire-compat-governance (C50, user-ratified 2026-10-14): four-plane compatibility
  governance convention — P1 signaling = dialect integer negotiation (min-clamp, 4101), P2
  media = capabilities content negotiation (no version numbers), P3 IPC/SHM = FrameMeta
  WIRE_VERSION hard gate + same-machine pairing, P4 SDK ABI = soname + manifest tuple.
  Codifies: three hard additive rules, permanent back-compat window (MIN=1; legacy-shape
  removal = user ruling + semver major + warning section), upgrade order (server-first),
  announcement lanes keyed on release events with the behavior-identical same-release
  bridge form legal, security lane (immediate, user-gated, minor+warning exempt from major),
  interop-down plane-attribution SOP, and design forbidden zones. Gate `
  scripts/gate-wire-changelog.sh` now pairs protocol.rs wire-shape diffs with a
  CHANGELOG [protocol] entry (fixture-verified); packaging (G1/D-H14): manifest.json +
  {target}-version.txt carry the signaling tuple {signaling_protocol_version,
  signaling_min_supported} next to frame_meta + token_schema, all derived from Rust source.

### Fixed
- [server] admin dashboard play grid was stuck at one column regardless of the Columns
  buttons: the english-only migration dropped the opening `/*` of a two-line comment in
  `Dashboard.css`, and the browser CSS parser swallowed the whole `.video-grid { display: grid }`
  block with the stray `*/` junk. Repaired; full-site CSS comment-balance sweep shows no
  other occurrences. Build/vitest/playwright-computed-string checks all passed over it —
  visual layout smoke is now a required step for asset-wording rounds (see status ledger).
- [host] K3 v1 regression (same-day follow-up): the explicit-dir gate collected tail args
  only for `logs`, so `monit`/`ps` never saw the directory argument — they failed with
  "explicit instance-dir required" even when one was passed (commands unusable), and the
  wrapper handed `fixed` to oxmgr twice (`ui ui`). Unified tail collection + single pass;
  `logs` keeps the process-name passthrough (last pathlike positional = dir).
- [server] `monit`/`ps` accept the same optional dir form (strict resolution, `.server`
  guessing sealed) — completes the K3 promise that both sides tightened consistently.

### Changed
- [host] root fix for oxmgr CLI IPC daemon spawning (startup-stop-followups K3): (1) full
  instance env coverage — every oxmgr subprocess from the host CLI and the translate layer now
  sets both `OXMGR_HOME`+`OXMGR_DATA_DIR` (all four keys, same shape as server);
  (2) `monit`/`ps`/`logs` instance-dir resolution (K3 v2): the old cwd-default `.` is gone (it
  let any directory resolve a `<cwd>/run/oxmgr` daemon address and silently spawn a
  default-location daemon via oxmgr CLI IPC); the dir is now an optional trailing pathlike
  positional (e.g. `msrtc-host ps out/host`, `msrtc-host logs cam0 out/host`) or a strict
  smart default (cwd instance / instance beside the binary). No provable instance → exit 2
  with a hint — the legacy `.host`/`.server` guessing is sealed for these proxy commands.
- [server] `stop -g` summary line reports skipped/failed separately (direct-kill gate skips =
  a distinct Skipped state, no longer lumped in with kill failures):
  "Reap summary: via daemon N / direct-killed M / skipped K / failed J". **Exit code behavior
  change** — pure skips (daemon still alive / entry present, no overstep into direct kill)
  with a clean final-state signature scan change exit code 1→0; skips where the final state
  still has a signature-hit survivor still exit 1 (final-state leftovers count as failures,
  existing semantics unchanged); kill issued but process still alive (EPERM/D-state) still
  counts as failed.

### Added
- [server][host] startup conflict gate v2 (startup-hardening S1): `startup on` now scans every
  autostart anchor unit of this product (server+host dual domains, brand-aware exact prefixes
  — including the legacy `oxmgr-host-`+OXMGR_DATA_DIR old form and both default/branded
  forms), parses the other instance's ports (server=server.yaml/Caddyfile/oxfile three
  sources; host=host.yaml gateway port+oxfile) and intersects them with this instance's.
  Same-kind collision: interactive three-way choice on tty (don't install / uninstall the
  other and take over / handle manually); on non-tty it refuses and lists both sides' units/
  instance directories/conflicting ports in detail; cross-domain collision (server↔host): no
  longer skips the comparison, the intersection downgrades to a warning confirmation
  (overwrite / cancel / allow parallel); no intersection = coexisting install. The other
  side's uninstall action details are shown before executing; unrecognized `oxmgr-*` units
  are only WARN'd, never taken over (guard against TalkServo same-family misidentification).
- [host] startup platform wording fix + non-Linux thin adaptation (startup-hardening S4):
  help text "three platforms" changed to "Linux=systemd user unit; macOS/Windows=use the
  oxmgr service (not verified built-in)"; non-Linux `startup on/off/status` changed from a
  stub message to forwarding `oxmgr service install/uninstall/status --system auto` (exit
  code passthrough; singleton form — instance env not injected, multi-instance autostart is
  Linux-only, WARN noted).
- [server][host] post-install health probe for `startup on` (startup-hardening S3): after
  `enable --now`, poll `systemctl --user is-active` for ≤5s (10×500ms, same stepping as the
  stop self-check), then re-check for 2s after the first active that it is still active
  before reporting "daemon started ✓"; a failed seen during polling fails immediately (no
  idle waiting, with a port-conflict quick-guess line), a failed re-check = late bind failure
  warning, not active within 5s = timeout — all three failures print a `journalctl --user -u
  <unit> -n 20` pointer and exit 1 (no more false success). The probe checks unit state only,
  not application readiness (ports left to doctor).
- [server][host] `startup status` full inventory + no-argument `off` form (startup-hardening
  S2): no-argument `status` (the new default) prints a four-line health panel per unit —
  enabled/daemon/linger/instance bin alive — with linger state via
  `loginctl show-user $(id -u) -p Linger` (falling back to /var/lib/systemd/linger on
  failure), disabled gets an enable-linger pointer; deleted bin (legacy install-host form) is
  marked "will fail at boot" + cleanup pointer; daemon failed gets a journalctl diagnostic
  pointer; unknown `oxmgr-*` units WARN'd one by one. Exit codes are script-consumable:
  0=all healthy, 1=any zombie/failed/no unit. `status <dir>` kept as a single-instance filter
  (same four-line panel). No-argument `off`: exactly one unit stops it directly; multiple
  units = tty list choice (including [0] all) / non-tty refuses with the listing; zero units
  = idempotent 0; `off <dir>` unchanged.
- [server] `stop -g|--all` global reap (global-reap T3): sweeps every one of this product's
  server-cluster processes across directories and daemons by process identity (argv signature
  + caddy disk marker guarding against cross-product misfire) — processes under an instance
  daemon are gracefully reaped via their own daemon (preventing `restart_policy=always`
  resurrection), orphans are direct-killed as the last resort with a banner; `-g` combined
  with a directory argument = exit 2. Summary table "via daemon N / direct-killed M / failed
  K" + final-state signature line + an "instance daemons untouched (not targets)" note; exit
  code = failure-count semantics, same as the existing stop. Directory `stop` (with sweep
  fallback and port-attribution self-check, T2) is behaviorally unchanged.
- [server] `stop` now sweeps leftovers + ownership-attributed self-check (global-reap T2):
  after the registry stops its processes, a /proc scan finds this instance's leftover
  processes (including the split form registered under a shared/default daemon — the
  2026-10-12 "stop said stopped but 9800 was still held" incident shape), reaping them
  gracefully via their owning daemon (preventing `restart_policy=always` resurrection); only
  when both registry lookups miss does it SIGTERM→SIGKILL direct-kill and print a banner;
  daemon reap failure automatically degrades to direct kill. The completion criterion is
  upgraded to port-level ownership attribution (9800/8080/9810): this instance's leftover
  holding a port = failure with per-case identification; a stranger (this product's instance
  from another directory / an unrelated process) holding a port = WARN+success, not a false
  failure. Instance daemon death / missing oxmgr no longer exits early — the sweep runs all
  the same. Default-daemon operations are env-isolated: the four OXMGR_* keys are explicitly
  cleared, so operator shell residue can no longer silently route "default daemon" operations
  at an instance daemon.

### ⚠ Breaking Changes
- [sdk-client] ⚠ breaking: the cockpit C ABI is fully renamed `ms_client_*` →
  `mediaservo_client_*` (S6 batch 1b, zero aliases, old names not kept); `login` switches to
  the flat parameter form (the old config-struct form is retired, the struct definition kept
  for one cycle); `control_on_ack` switches to cumulative form + token out-parameter (new
  `control_off_ack(token)` to unregister; the old "re-register replaces" semantics are void);
  `control_producer_ids` unified to the needed-overflow feedback form. C++ side in sync:
  `Control::on_ack` returns `Result<uint64_t>` (token), `Error` gains `wire_code`/
  `retryable` machine-readable fields (the other three families stay 0/false, source
  compatible). ⊘ old forms kept for one cycle (behavior byte-identical, batch 2 retires
  them): `last_error` process-global / `consume_video` first route / `video_stats` session
  union. Migration = mechanical prefix replacement + the three signature points above.

### Added
- [server] doctor/contention dual-daemon registry split detection (T5, identify-don't-kill):
  doctor gains a [warn] — if the shared/legacy daemon registry ($XDG_DATA_HOME else
  ~/.local/share/oxmgr/state.json; the plan text's ~/.oxmgr was a misquote) contains a row
  whose cwd belongs to this instance, report name/id/pid with the "pin OXMGR_DATA_DIR +
  reconcile via `oxmgr list`" remedy (not counted as failed); at start, when a port is
  occupied and /proc cannot locate an old instance, the same criterion identifies "the
  occupier sits under the old shared daemon". The criterion = cwd ownership
  (status-independent), targeting the 2026-10-12 "stop said stopped but the port was still
  held" incident shape (instance table empty + default holding everything); no automatic
  kill/migration (PIT-98).
- [server] two new doctor checks — web port drift WARN (server.yaml/env intent vs Caddyfile
  artifact; after the D-A re-ruling the only drift alarm channel; parse failure skips per
  contract) and an `announced_ips` staleness WARN (a yaml-configured IP ∉ local interfaces in
  bare-metal form; container posture exempted as INFO; reuses the sfu
  `use_bare_metal_listen`/`detect_all_ips` same-source criteria, no second enumeration copy)
  — both WARN only, the doctor exit code contract unchanged. The docker server.yaml template
  drops the zero-consumer dead key `rtc_ports_range`; the `DEFAULT_WEB_PORT`/
  `WEAKNET_SERVE_PORT` default constants are consolidated into `mediaservo-common::config`
  as a single source (T6b(2), same pattern as `DEFAULT_SFU_PORT`; weaknet `DEFAULT_LISTEN`
  does not depend on common and keeps its hand-copied constant + cross-reference comment
  anchor).
- [server] init-generated `server.yaml` template comments completed (T6a): added comment
  examples for the four fields `tls`/`ws_max_message_size`/`api_keys_file`/
  `api_token_ttl_secs` (three elements: purpose/default value/activation stage; `tls`
  annotated with required sub-keys + yaml shape), and clarified `announced_ips` (unset =
  auto-detect, bare metal may omit the whole block, containers must set it) and `web.port`
  (after changing it, delete etc/Caddyfile and re-init; rendered once) — correct
  configuration achievable from the comments alone, no source reading required; a
  ServerConfig field coverage audit test guards the comments against rot.
- [server] manual `[apps.env]` lines no longer lost after redeploy/rebuild: on oxfile
  re-render, manual env keys are auto-merged from the old file (falling back to the
  run/oxfile.toml.bak snapshot when live is missing) — system keys `MEDIASERVO_SFU_*` always
  follow the current render, old values never resurrect; on manual-key conflicts the old
  value wins (user intent); an unrecognizable old file degrades to a plain render with a
  warning, never blocking start. The notice line also reports "merged N manual env entries"
  (PIT-160 root fix).
- [server] `sfu.port` in `server.yaml` now takes effect (fake-key defused): the SFU listen
  port resolution order = yaml `sfu.port` > env `MEDIASERVO_SFU_PORT` > default 20000 (D-B
  ruling, same shape as `web.port`; previously only env was honored, a yaml value was
  silently ignored). The default 20000 converges to the single constant
  `mediaservo_common::config::DEFAULT_SFU_PORT`, also referenced by the env help master
  table render.
- [deploy] `deploy bindings` prefix default = `<out>/bindings` (bindings are purely static
  and stateless; the out root follows the calling shell — msrtc.sh=main repo out,
  mediaservo.sh/bare CLI=submodule out; an explicit `--prefix` overrides). host/server
  required-prefix semantics unchanged.
- [host][server] start/stop conflict doctor (start-conflict-doctor): when a port is
  occupied, the occupier is automatically identified (pid/binary path/stale-upgrade leftover
  (exe deleted)/parent process) with handling advice; after stop completes, a self-check
  verifies the process table is empty ("reap self-check ✓"), preventing a false success from
  poisoning later operations. Identify-don't-kill — the handling decision is left to ops.
- [sdk-client] video_stats JSON contract extended (additive): five new fields
  jitter/frame_dropped/nack_count/pli_count/fir_count — always carried by libwebrtc
  inbound-rtp, the abstraction layer previously forwarded only 10 fields; union semantics =
  jitter takes max, counters sum (aligned with the web play detail layer).
- [sdk-client] cockpit C ABI/C++ batch 1b new surface (28 symbols): multi-route `Consumer`
  (`session_consume`→id/stats/close handles, each route with an independent frame pump, no
  cross-route coupling, close bounded ≤250ms); session state observation (`session_state`
  snapshot + `session_on_state` cumulative callback, Disconnected/Connected/Reconnecting/
  Failed wire values 0..3); `wait_video` pure wait (targeted entry for multi-route
  orchestration); handle-level error slot (`session_error` machine-readable
  code/wire_code/retryable + `session_last_error` text; `strerror` static message table);
  control backpressure observation (`ready_state`/`buffered_amount`); user_free contract =
  exactly once per registration (on_state→session_close, consume→consumer_close,
  on_ack→off_ack pump reclaim or control_close). The C++ mirror = the `Consumer` RAII class
  + `State`/`DcState` enums + `wait_video`/`consume`/`off_ack`/`ready_state`/
  `buffered_amount`; the `consume_video`/`video_stats` old bridges kept.
- [sdk-client] cockpit SDK (Rust surface, S6 batch 1a): automatic reconnect on signaling
  link loss (backoff + resume-ticket priority, auth family terminal, Consumer frame streams
  survive reconnects, `connection_state()`/`set_auto_reconnect()`); multi-route consumption
  via `Consumer` objects (one handle per producer, pause deferred to batch 2);
  machine-readable errors (`wire_code()`/`is_retryable()`); control channel backpressure
  reads (`ready_state()`/`buffered_amount()`). C/C++ surfaces open in later S6 batch 1 cuts.

- [server] consumer-facing room discovery `GET /api/rooms`: the room list is visible with an
  account JWT (admin/dispatcher see all online rooms; viewer/operator filtered by allowlist,
  edits hot-apply); offline/unowned rooms are not listed; the response carries only room_id
  and kind (video/audio), replacing "type the room name by hand" onboarding. First cut for
  the p3 GUI examples; the C++/Rust SDKs will provide a `list_rooms` convenience wrapper
  (later cut).
- [sdk-client] `mediaservo_client::list_rooms(http_base, jwt)`: room discovery convenience
  function (GET /api/rooms → `Vec<RoomInfo>{room_id, kind}`, usable as a free function
  before a session); login/discovery share the same hand-written HTTP surface (zero new
  dependencies); authorization rejections return `ClientError::RestRejected{code, message}`.
- [sdk-client] C/C++ binding `ms_client_list_rooms` (C ABI, JSON array passthrough +
  `needed` overflow feedback — a new buffer contract; the `control_producer_ids` cap blind
  spot is not repeated) and C++ `client::list_rooms()` (header-only, auto-grows and retries
  once on overflow, 64KiB cap). ABI table 12→13 symbols reconciled green.
- [sdk-client] C++ example command surface `./mediaservo.sh list/build/run/test example
  [name]` (p3 W0): artifacts in `target/examples/bin/` (not in the delivery tree); GUI
  dependency archives SDL3 release-3.4.16 + Dear ImGui v1.92.9b (3rdparty/ local zip +
  sha256 ledger); the new example directories `imgui_viewer` (empty-window skeleton) and
  `imgui_shell` (rendering shell) in place, `control_demo` migrated into the unified
  aggregator.
- [host][sdk-client] e-stop key file channel aligned across vehicle and cockpit: new
  `MEDIASERVO_CONTROL_HMAC_KEY_FILE` (vehicle-side controller, 0600 permission gate +
  panic-on-read-failure early death = a security feature refusing silent degradation, same
  discipline as the cockpit `hmac_key_file`); both ends share the common single-source
  loader. The plaintext env form is kept for migration compatibility.
- [host][deploy] e-stop key config joins the render chain: host.yaml gains
  `[control].hmac_key_file` (relative paths resolved against the instance root) → a missing
  file at deploy time fails the render (before any process is launched), translated into the
  host-controller `--hmac-key-file` argument (explicit > env var, read failure = error
  early-exit, no silent downgrade); unconfigured = the dev migration form, byte-identical
  (zero disturbance on private/dev networks). Production e-stop signature verification
  converges from the "manual env three-piece set" to one config line.
- [deploy] delivery size root fix: the .so binaries assembled for bindings and the
  wheel-embedded _libs are `strip --strip-unneeded`ed before leaving the build (debug
  symbols were the entire cause of the fat 528MB→**80MB** wheel). Zero impact on the
  exported ABI (.dynsym untouched; symbol audit / consumer linking / find_package all
  re-run green).
- [deploy] release packages gain `manifest.json` (N2 discovery surface): a mechanically
  derived component enumeration (lib/header/pc/cmake component names + `requires_ffmpeg`
  **per readelf DT_NEEDED evidence** — deck=true, field/link=false) and protocol pairing
  (frame_meta wire version source-generated); ROS/packagers/CI can judge the capability
  surface without parsing CMake. The sdk-field/sdk-client two packages each take shape.
- [deploy] release packages split into the F12 dual halves: the `package` target now forms
  **sdk-field** (device half: field/link/deck libs + C/cxx headers + Python wheel + node)
  and **sdk-client** (cockpit half: client lib + headers + pc + cmake as an independent
  component set, zero device-surface mixing); the old `bindings` target is renamed
  `sdk-field` (one-cycle alias, package name still sdk with a WARN). Package names
  `<brand>-<target>-<version>.tar.gz`, all four forms; each package's CMake component
  surface is rendered per domain (the cockpit package's find_package only recognizes
  client; mistakenly requesting field errors out explicitly).
- [deploy] release package CHANGES.md mechanical slicing (F11-X): the repo-root single
  source routes entries into host/server/sdk packages by `[scope]` tag (protocol rides
  all three packages, deploy rides the two cluster packages; untagged historical entries
  are carried in full = prefer completeness over omission); CI enforces that new
  Unreleased entries carry an in-vocabulary tag (missing/out-of-vocabulary = red).
- [sdk-client] CI `test-gui` job: GUI example compile gate + `ctest -R core`; SDL3/ImGui
  in-repo archives, zero network; aggregator SDL audio backend default OFF guard
  (clean-ubuntu prevention). The examples directory gains a README (IN/OUT + **cockpit
  dual-room convention** (media `<vehicle-room>_<stream>` / control `<vehicle-room>`) +
  headless acceptance usage).
- [sdk-client] session video stats surface `ms_client_session_video_stats` / C++
  `Session::video_stats()`: inbound-rtp folded into flat JSON (bytes/frames/resolution/
  fps/loss), the data source for mini-stats rendering (Rust-side fold rules pinned by unit
  tests; C surface uses the same needed-overflow contract).
- [sdk-client] example shell `viewer_core`: RateEstimator (Δt=0 holds, counter regression
  re-anchors, never a negative rate) and dependency-free JSON scalar extraction; ctest
  `-R core` acceptance in place.
- [sdk-client] C binding video-consumption crash fix: reactor panic in the frame-pump
  thread (argument construction outside block_on) — before the fix, the C/cxx
  consume_video receive thread died silently (always reproducible with multi-route
  video; four consecutive panics confirmed in the spike).
- [sdk-client] **Fix: non-browser SDK video playback stops refreshing ~1 second in**
  (known issue closed). Root cause = the track wrapper object that registered the sink is
  destroyed on scope exit, and its destructor takes the just-registered sink down with it
  — all previous "track rebuild / thread context" hypotheses were falsified; in practice
  the wrapper is newly created on every fetch and its pointer constantly changes. The fix
  = hold the wrapper paired with the sink from registration; a 90-second dual-source
  cross-check (callback frame count vs libwebrtc decode count) matched 2700/2699 with zero
  decay. Also removed the rescue/track-rehook patch machinery (its premises were dead and
  it caused duplicate delivery).
- [server][sdk-client] room discovery completion (W4d): `GET /api/rooms` previously
  reported only registered rooms — the media per-stream rooms
  (`<vehicle-room>_<stream-id>`, PIT-140 v2) are produced by the streamer without a
  RoomJoin, so SDK consumers could not discover playable rooms from the list. Now stream
  rooms are derived from StatusReport(connected), owner inherited from the base room
  (allowlist filter applied in the same direction; offline streams not derived), kind in
  three facets = video (stream room) / control (vehicle room, no video producer) / audio.
  `imgui_viewer` in sync: checking a stream room auto-merges the vehicle control room tile
  (the dual-room convention as the product form), auto-discovery fallback, and a fix for
  the room-name parsing off-by-one (a latent bug carried for years by the explicit env
  channel, now surfaced).
- [sdk-client] e-stop signing surface exposed in C/C++ (W4b): config gains
  `hmac_key_file` (key file channel, 0600 permission gate + non-empty + trailing newline
  stripped, G13 = keys never in argv/env plaintext); `ms_client_session_emergency_stop` /
  C++ `Session::emergency_stop(ctl,…)` dual-path delivery (DC fast path signed + signaling
  audit copy). No key configured = unsigned (vehicle side without a key = migration
  allow form; vehicle side with a key rejects unsigned = correct ruling). ABI 14→15;
  `imgui_viewer` e-stop button switches to the signed entry (signed/unsigned wording
  follows the config state).
- [sdk-client] GUI example `imgui_viewer` W3b first cut: login → join → consume →
  **I420 texture to screen, full chain** (new shell components: FrameStaging pump
  thread → main-thread latest-frame slot + VideoTexture SDL IYUV station). Headless
  acceptance `SDL_VIDEODRIVER=dummy` + `MSRTC_RUN_SECS` self-exit + `[frame] cb=/tex=`
  dual counters — CI can accept the video chain without a display.
- [sdk-client] GUI example W4 merge round: login panel (password input, G13) →
  `/api/rooms` multi-route selection (one session per room) → Tiles page 1-3 column grid
  + mini-stats row (kbps/fps/resolution, consumer of the W3a asset) → Control page
  (chassis steer sliders + ack echo + RTT; e-stop button in unsigned form, the signing
  cut is on record). Headless CI channel: MSRTC_PASS+MSRTC_ROOM auto login/join,
  `[tile]` anomalies printed explicitly.

- [sdk-client][host][protocol] e-stop command chain (teleop safety): the cockpit SDK
  `emergency_stop` dual-path delivery — the data-channel fast path carries an HMAC-SHA256
  signature (deploy-time pre-shared key MEDIASERVO_CONTROL_HMAC_KEY, same value on vehicle
  and cockpit) + a signaling-channel audit copy; the vehicle-side executor has a signature
  verification gate (when a key is configured, unsigned/mis-signed e-stops are always
  rejected with an estop_signature_rejected receipt), and each execution result is written
  to the actuation audit (jsonl or log line; the audit never blocks execution). Key not
  configured = behavior identical to the previous version (migration period).
- [deploy] compatibility matrix script scripts/e2e-compat.sh: four dialect probes
  (missing field → v1 / declared clamp / reject-too-old 4101) + a new-SDK full-chain
  loopback + a dispatcher 4012 control-rejection negative case; the old-server quadrant
  requires b34f3a1 artifacts (default SKIP, accounted).
- [deploy] weak-network debt repayment (loss × Cmd/ACK): remote-burst 5/5 all pass, RTT
  254-301ms; degraded outage-level with zero brittle breaks; conclusions and evidence in
  main-repo docs/plans/client-dual-form/evidence/s4-weaknet.md.

### Added
- [sdk-client] the cockpit SDK fourth family (C ABI + C++): new library mediaservo-client-c
  providing the stable `ms_client_*` C interface (login/join/video callbacks/teleop
  channel), new header mediaservo/client.hpp providing a C++11-compatible RAII wrapper;
  with a pure-C++ teleop sample control_demo and a ROS2 bridge sample node (built at
  device-day). CI gains the test-cxx job: four-SDK C++ test compile+run + C ABI symbol
  table audit (including the client's new 12-export surface).

### Fixed
- [deploy] the build/deploy/package bindings delivery surface now ships the SDK usage
  manuals: the four sdk-cxx manuals are copied into out/bindings/docs/sdk-cxx/ (in-directory
  cross-links preserved; each round removes-then-copies to prevent stale manual leftovers);
  the sdk-field/sdk-client release packages split the docs per half (the client package
  carries only client.md, the field package carries the field/link/deck three manuals),
  pinned by four positive/negative e2e-package assertions.
- [deploy] build:deploy bindings same-tree crash fix: the build stage had already assembled
  the delivery layout into out/bindings, so a deploy copying to the same prefix aborted with
  a "same file" error (triggered on the very first use of that command combination); now a
  same-tree deploy skips the copy idempotently (same guard as server/host deploys).
- [host] signaling link-loss awareness completion: WebSocket abnormal closure (RST triggered
  by the peer process being killed/crashed, read errors) previously emitted no link-loss
  event, so downstream consumers (vehicle gateway/streamer) had to wait the full 15-second
  heartbeat silence before declaring death; read errors now emit the disconnect notice
  immediately (same standing as a normal close).
- [host] subscriber capacity cap when multiple streams share one capture source: the
  in-process frame bus previously kept the iceoryx2 default cap of 8 subscribers (fixed at
  service creation), so pointing 8 streams at one source in host.yaml (a legal form under
  D282 defaults) left one stream forever failing to subscribe — and the old cap survived
  restarts of that service ("restarting didn't help either"); the per-topic subscriber cap
  is now raised to 32. Recovery for existing deployments: stop host → clear /tmp/iceoryx2
  and /dev/shm/iox2_* → start host (old service files carry the old cap).
- [server][host][sdk-client] teleop data-channel resource leak (production-grade): after a
  cockpit disconnects, its data-channel registrations and the peer's consumer handles were
  previously never reclaimed; long-running servers would exhaust available channel numbers
  so new cockpits could not connect (previously also visible as the dashboard's data-source
  count growing monotonically). Now disconnection triggers targeted reaping and notifies the
  vehicle side to tear down its local channels. Dashboard fix in step: data-channel close
  events no longer falsely trigger a video stream restart.
- [host] the vehicle-side controller no longer enters a crash-restart storm when the server
  is not yet ready at cold start: the signaling connection now retries with exponential
  backoff (covering the whole cluster bring-up window), and the guardian policy remains the
  last-resort backstop.
- [server][host][sdk-client] root fix for teleop data-channel one-sided open (vehicle/
  cockpit could not receive each other's messages): the data-channel consume receipt now
  carries the server-assigned SCTP stream parameters, and both ends receive forwarded
  messages via the out-of-band negotiated channel (the official mediasoup-client contract)
  — the previously awaited in-band handshake never happens under the proxy form. Wire
  compatibility with old versions preserved (fields read with defaults).

### Added
- [sdk-client] new mediaservo-client v2 consumer SDK (Rust library): account login (JWT),
  join, SFU video consumption, teleop data channel (4012 control rejection / protocol too
  old are explicit typed errors); the old client demo form (built-in WS / :9101 forwarder /
  HMAC channel) is retired.
- [host][protocol] vehicle-side teleop receive chain generational upgrade: host-controller
  moves from point-to-point negotiation (a dead path since 2026-08-25) to server SFU data
  channels — DTLS/SCTP registration completes at power-on (chassis/gimbal/light/ack four
  channels), and command mirrors are published to the local bus control/cmd (receipts on
  control/ack) for zero-change consumption by ROS/playback nodes.
- [protocol][server][host] signaling channel hardening: the server actively probes for dead
  links via heartbeat (zombie connections reaped in roughly 10-15 seconds; connections no
  longer killed by mistake during network jitter); after a brief vehicle-side outage, a
  one-time resume ticket restores quickly (authentication is always re-run in full, so
  revocation takes effect immediately); heartbeat and timeout parameters are tunable via
  MEDIASERVO_WS_PING_SECS / WS_PONG_MISS / WS_PSK_WAIT_SECS / WS_JOIN_WAIT_SECS /
  WS_RESUME_HOLD_SECS.
- [protocol][server] loss limiting for high-frequency status reports under signaling
  congestion (drop counts self-reported); authentication/control/terminal messages are no
  longer dragged down queued behind a status flood (priority dual queue). Protocol dialect
  raised to v3 (new session resume domain; v1/v2 endpoint behavior byte-identical).
- [protocol] signaling protocol version negotiation: endpoints declare their dialect version
  when joining a room, the server negotiates and echoes the effective value; versions too old
  are explicitly rejected (error 4101) instead of silently downgraded. Old clients that
  don't declare are treated as v1 — wire traffic is byte-identical, upgrade ordering is
  unconstrained. The control data channel (teleop) requires negotiated version ≥2.
- [host][server][deploy] device public-key fingerprint enrollment: the host answers the
  server challenge with the device private key already generated at init, and registration
  only needs one "approve" click in the admin console's "pending devices" queue; on private/
  dev networks set `ALLOW_DEV_ENROLL=1` and new devices enroll with zero human steps (no
  more copying/configuring any secret).
- [server] the admin console devices page gains a "pending devices" queue (one-click
  approve + optional naming).
- [server] the web player negotiation core switches to the official mediasoup-client
  (hand-spliced SDP / hardcoded payload type tech debt repaid; outward behavior and UI
  unchanged, weak-network resilience semantics preserved as-is).
- [deploy] deploy help gains an "environment variable master table": `msrtc.sh -h` and
  `msrtc-server -h` / `msrtc-host -h` all three surfaces share the single source
  `crates/mediaservo-common/assets/env-usage.md` ([A] script-injected / [B] oxfile manual
  lines / [C] startup env); after a full-tree redeploy loses manual env, restore per the
  table.

### ⚠ Upgrade Notes
- [protocol] the PSK discovery surface (psk-discover) is a same-batch feature: a new client
  (list_rooms_psk) against an old server gets 401 — upgrade the whole batch; old clients are
  unaffected (no Psk scheme, new branch never taken).
- [protocol] auth-failure semantics tightened (fail-closed): a presented JWT that fails
  signature verification → Error 4013 and immediate disconnect (old behavior = fall back to
  PSK, and a correct PSK silently gained the roleless permission matrix). In-repo Rust/TS
  clients already classify 4013 into the auth terminal family; integrators signing their own
  tokens must sync — not treating 4013 as terminal will cause infinite retries on bad
  credentials.
- [host][server] device enrollment generational change: vehicles registered under the old
  "device secret" scheme will be refused on first connect after upgrading the host — delete
  the old entry in the admin console, then let the device re-enroll (auto mode enrolls
  instantly; default mode needs one approval click). The device-secret path is kept for one
  version cycle, removed next release.
- [host] the `identity.json` generated by `host init` no longer contains key material (old
  files remain readable).

### Fixed
- [deploy] root fix for the examples build chain conda/system toolchain mixing (the Ubuntu
  20.04 windowed-run incident family): the `build example` configure/build subprocesses now
  pin the system compiler + strip conda build env (CFLAGS/PKG_CONFIG*/CONDA_PREFIX etc.) +
  three SDL peripherals explicitly OFF (liburing/dbus/libusb) + the `-isystem /usr/include`
  hack made conditional (only under conda compilers); integrators on older systems
  (glibc<2.33) can now build the GUI examples natively, while the CI/Jetson conda route is
  byte-identical.
- [sdk-client] imgui_viewer window resizable/maximizable: CreateWindow adds
  SDL_WINDOW_RESIZABLE (SDL3 defaults to non-resizable; the earlier note "already draggable"
  was falsified); the initial 80% adaptive sizing and the ini layout are unaffected.
- [server] /api/rooms visibility fix for apikey-form tokens: the allowlist previously only
  consulted the accounts registry (an exchanged token has no account row by design) →
  authorization for `apikey:`-prefixed subs now reads claims.vehicles directly (same source
  and same standing as the RoomJoin gate; the human-account path is verbatim unchanged).
  Symptom = viewer key exchanged successfully but the room list always empty (caught live in
  acceptance V1).

### Added
- [server] room ownership semantics on the wire (room-grouping-semantics): `GET /api/rooms`
  entries gain `parent` (stream room pointing back to its vehicle) / `streams` (online
  streams embedded in the vehicle room), both additive; **derived stream rooms are no
  longer flattened as top-level entries** (previously 8 streams showed as 8 separate
  rooms). Consumer UIs group by vehicle room with streams as children (1 vehicle, N
  streams); the underlying join/produce/token granularity is unchanged.
  ⚠ Upgrade note: an old viewer against a new server will not see stream room entries (they
  are merged into streams) — upgrade client and server as one batch; a new client against
  an old server automatically falls back to `_`-prefix grouping, imperceptibly.
- [server][sdk-client] PSK room discovery (psk-discover, closing out cross-room pulls):
  `GET /api/rooms` gains an `Authorization: Psk <secret>` auth branch — a Legacy full view
  (aligned with the PSK join gate's capabilities: it was already all-powerful; "able to join
  but not to look" was an accident of history; constant-time comparison, wrong key or no
  PSK configured both give an explicit 401). SDK `list_rooms_psk` in all three languages;
  in viewer PSK mode, leaving the room name empty = check multiple rooms in the discovery
  tree (filling a room name = direct-connect override).
- [sdk-client] imgui_viewer five-mode auth test panel (viewer-auth-matrix): the login panel
  gets a Radio to pick account/API-Key/PSK/device identity/pasted JWT, with input fields
  shown/hidden per mode; after connecting a bottom `identity:` line echoes the effective
  identity; PSK/device paths skip room discovery and join directly (Legacy discovery 401 /
  Device has no REST issuance — semantics, not shortcuts); a bad pasted JWT = the 4013 red
  card as a GUI acceptance point. Headless injection `MSRTC_AUTH_MODE=account|apikey|psk|
  device|jwt` + the matching credential keys. C ABI additive: an `identity_dir` field at
  the tail of config_t (MIN_SIZE = old shape, boundary-checked reads) + the
  `mediaservo_client_session_identity` symbol; cxx `Config::identity_dir` +
  `identity_label()`.
- [server] admin console new "API Keys" page (same discipline as /devices /accounts):
  registration (key_id/role/vehicle allowlist/remark; secret shown once + copy, closing the
  dialog discards it = revoke and re-register to see it again), revocation (blocks only
  future exchanges; already-issued JWTs die naturally at TTL); pairs with
  `POST /api/auth/exchange`, non-admin guard same as devices.
- [sdk-client] ⚠ the web player no longer implicitly sends the shared PSK: connections
  without a token are refused outright (the `mediaservo-dev` hardcoded fallback is retired,
  PIT-171 family / S2 repayment) — browser-side auth = login-and-exchange is the only path;
  the true PSK form remains for host/link clients configured directly.
- [sdk-client] cockpit SDK device identity wiring (accountless-client-auth T3, D283 public
  key chain): `ClientConfig::identity_dir` points at the instance directory
  (`identity.json` + `etc/link/signing.pem`, same layout as `msrtc-host init`) → RoomJoin
  carries device_pubkey and passes the Ed25519 challenge (identity = Device, no account, no
  key needed). The loader is consolidated upstream into the single-source
  `DeviceIdentity::load_from_instance_dir` (host assembly thinned into a wrapper; behavior
  and error wording verbatim unchanged).
- [sdk-client] API-key exchange across all three families (accountless-client-auth T2):
  Rust `mediaservo_client::exchange(http_base, key_id, secret)`, C
  `mediaservo_client_exchange` (ABI 29 symbols), C++ `mediaservo::client::exchange` — the
  output JWT fills the existing `jwt` config slot as-is, zero changes on the consumption
  side; the imgui_viewer login panel gains a "use api key" mode (account or key, G13
  password discipline same form; MSRTC_KEY_ID/MSRTC_KEY_SECRET as the headless CI preset
  channel). Bad key = 401 → InvalidCredentials terminal (no retry).
- [server] accountless client credentials (accountless-client-auth T1):
  `POST /api/auth/exchange {key_id, secret}` exchanges for a short JWT (same shape as
  LiveKit apikey/apiSecret) — issuance reuses the existing admin key infrastructure, and
  every existing role gate on `/ws` and REST is reused with zero changes; admin surface
  `/api/admin/apikeys` CRUD (secret plaintext only once in the registration response,
  revocation blocks new issuance, already-issued JWTs die naturally at TTL); the registry
  `api_keys.yaml` hot-applies (same atomic save / anti-enumeration / constant-time as
  devices). server.yaml new keys `api_keys_file` (default api_keys.yaml in the same
  directory) and `api_token_ttl_secs` (default 43200 = 12h).
- [sdk-client] imgui_viewer login panel server address now editable:
  `MSRTC_WS_URL/MSRTC_HTTP_BASE` demoted to preset values (demo/bring-up machines can swap
  addresses without restart); the server address lives in the model, and the three consumer
  points — login/discovery/session creation — all read the panel value.

## v0.1.1 (2026-09-10)

### Added
- Full weak-network simulation toolset: the admin console sidebar gains a "Weak Network"
  panel (admin-visible) — apply/inspect network impairments straight from the browser; the
  command line `./msrtc.sh weaknet apply|set|scenario|status|clear` is available in step;
  the weak-network service auto starts/stops with the server deploy cluster, no manual
  process launching.
- Three weak-network streaming policy tiers (host config etc/host.yaml gains stream_mode per
  stream): smooth keeps frame rate (recommended for teleop) / quality keeps clarity
  (recommended for forensics) / balanced is the default middle ground; under weak networks
  the frame rate no longer falls off a cliff.
- Playback resilience: after a vehicle or server restart, the web page recovers the picture
  automatically (no manual refresh); when the source stops it honestly shows "source
  offline" instead of a false connection-failure.
- Weak-network simulation vehicle-side standalone form: impairments can be applied to the
  physical NIC on the Jetson vehicle itself.

### Changed
- The server deploy cluster consists of 3 services (main service / web frontend / weak
  network); start, stop, status manage the whole cluster with one command.
- Version numbers globally unified: all components ship on a single workspace version;
  release packages `{brand}-{target}-{version}.tar.gz` have a versioned top-level directory
  containing this file (CHANGES.md) and the version contract version.txt.
- Release package output converges on out/packages/.

### Upgrade Notes
- Old deployed instances' etc/host.yaml comments do not cover stream_mode usage; refer to
  the comments in the latest release package or the docs.
- After upgrading the server cluster, the first start rewrites the run manifest (old file
  auto-backed up as .bak); manually added runtime env vars are preserved automatically.
- The weak-network panel requires an admin account login; applying impairments to a given
  video stream requires that stream to be actively publishing.

## v0.1.0 (2026-08-20 → 09-09, unversioned internal baseline)
- First internal integration baseline: multi-type video sources (camera/generator/desktop) →
  SFU forwarding → web playback, full chain; unified device/account admin console (changes
  hot-apply, no manual config edits + restarts); PSK/JWT dual-track auth with key rotation;
  audio conference rooms; H.264/VP8/VP9/AV1 encoder selectable; disconnect self-healing and
  weak-network-friendly playback; native / single-container / compose three deploy forms.
