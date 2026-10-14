# mediaservo C++ SDK examples (aggregated build)

Runnable examples for one of the three Rust SDK families (`mediaservo::client` / C shim / this directory's C++17 header-only).
The aggregation root `CMakeLists.txt` consumes `target/{debug,release}/`'s `libmediaservo_client_c.*` +
`include/mediaservo/client.h` guarded by `MEDIASERVO_SDK_DIR`; SDL3/ImGui use in-repo archives
(`3rdparty/*.zip` file:// FetchContent; zero network in CI).
The ImGui archive = `v1.92.9b-docking` (the first-party docking branch; imgui_viewer's four-pane IDE layout depends on it).

## IN (what this directory guarantees)

| Example | Form | Judgment |
|---|---|---|
| `imgui_viewer` | The cockpit GUI acceptance piece (**docking four panes**, see below) | `ctest -R core` + the dummy headless `[frame] cb=/tex=` double count |
| `imgui_shell` | The rendering shell (an SDL3+ImGui frame loop, `ShellOptions{docking, ini_path}`) + `viewer_core` (RateEstimator/JSON extraction) + `FrameStaging`/`VideoTexture` (I420→texture with zero re-packing) | fail-soft exit 0 without a display; ini writing disabled under the dummy driver |
| `control_demo` | A headless teleop closed loop (login→join→open_control→steer→ack) | the `build example` compile gate + the device-day live run |

## imgui_viewer layout (docking four panes, IDE form)

```
┌───────────┬──────────────────────────┬──────────────┐
│ Streams   │      Video Grid          │ Stream Info  │
│ device▸   │   (1-6 columns, fit)     │ (stream      │
│ stream tree│                         │  details)    │
│ click→Pull│                          ├──────────────┤
│           │                          │   Control    │
├───────────┴──────────────────────────┴──────────────┤
│ Log (500-line process ring · follow auto-scroll · reset layout) │
└──────────────────────────────────────────────────────┘
```

Dragging panes = grab a border; double-click a title = maximize/restore that window; drag a title out of the canvas = a floating window.
Positions and pane widths are stored in `imgui_viewer.ini` (deleting the file or the Log area's "reset layout" = restore the factory four-pane form).
Under dummy headless mode the ini is never written (zero pollution of CI build directories).

### Reading guide when copying as a template

1. **The `imgui_viewer/src/app_model.hpp` header note** = the layering iron rules (main beats / model / sessions actions / stateless panel rendering; one-way dependencies).
2. **The `imgui_viewer/src/dock_layout.hpp` header note** = the five docking elements at a glance (switch/canvas/DockBuilder/first-run guard/persistence), with all three real-world pitfalls flagged: `DockBuilder*` lives in `imgui_internal.h`; `SetNodeSize` must precede Split; the first-run guard uses an **ini probe** (`App::had_saved_layout()`) rather than the folk WasActive form (that belongs to the window, not the node).
3. **`imgui_viewer/src/app_log.{hpp,cpp}`** = the minimal form for GUI-side multi-threaded logging (mutex + ring + add_fmt; the split of duties between stdout machine-judgment lines and UI log lines).

### Manual test checklist (first-round acceptance, 5 minutes)

- [ ] The grid cell's top-right x / a live tree row's x → that session closes, the cell disappears, the tree row becomes selectable again; re-checking can Pull again
- [ ] Login → the left tree shows device groups; **click rows/checkboxes to multi-select** (selected rows highlight yellow) → press "Pull selected ▶" → the center cells show video, the right-top details show fps/kbps, the tree rows turn live-grey
- [ ] Switch the cols dropdown to 1/2/3/6 columns → **the grid really changes columns** (the legacy Columns single-column bug is root-fixed); drag the Video Grid pane bigger/smaller → cell widths adapt
- [ ] Pulling a stream room auto-pairs the whole-vehicle control room (the `●` group in the tree + the "1 control session (auto-paired)" note line at the grid window's bottom; **the control room takes no video cell**), and Control (right-bottom) steer→ack shows a live RTT
- [ ] Grab any pane border and drag → exit and reopen → **the widths persist** (the ini working)
- [ ] Double-click the Video Grid title = maximize, double-click again = restore; drag the Streams title out of the canvas = floating
- [ ] The Log area's "reset layout" → the four panes return to the factory split
- [ ] **The five auth modes** (viewer-auth-matrix): the Radio switch is mutually exclusive, irrelevant fields grey out; Account/API-Key logins show video;
  PSK fills psk + room name and presses "Connect" to go straight in (no discovery step); Device fills the identity directory the same way; pasting a bad JWT →
  the red card 4013 without a reconnect storm; after login the bottom `identity:` line matches the server log's identity

## Run

```bash
# Headless acceptance (CI/server machines): the dummy driver + a timed self-exit + env injection (the viewer-auth-matrix five modes)
# Common: SDL_VIDEODRIVER=dummy MSRTC_RUN_SECS=<N> MSRTC_WS_URL=ws://<s>:9800/ws MSRTC_HTTP_BASE=http://<s>:9800
#   Judgment line: [frame] t=..s cb=<callback frames> tex=<texture frames> WxH — at steady state both are equal and grow at ~30fps
# M1 account: MSRTC_AUTH_MODE=account MSRTC_USER=<u> MSRTC_PASS=<p> MSRTC_ROOM=<video room>
# M2 API-Key: MSRTC_AUTH_MODE=apikey MSRTC_KEY_ID=<id> MSRTC_KEY_SECRET=<s> MSRTC_ROOM=<video room>
# M3 PSK (full discovery; check tree rows to pull across rooms): MSRTC_AUTH_MODE=psk MSRTC_PSK=<psk> (no ROOM needed; the server needs a same-batch upgrade including psk-discover)
# M4 device identity (identity_dir joins directly): MSRTC_AUTH_MODE=device MSRTC_IDENTITY_DIR=<instance dir> MSRTC_ROOM=<room name>
# M5a/M5b paste a JWT: MSRTC_AUTH_MODE=jwt MSRTC_JWT=<token> MSRTC_ROOM=<room name>
#   The M5b bad-token judgment = [tile] join: auth rejected [4013] (a fail-closed red card, no reconnect storm)
# The GUI face's five Radios mirror these five paths (the login panel's bottom identity: line = the effective-identity echo)

# Build (the CLI auto-runs build-c; if the Rust binding surface changed, first cargo build -p mediaservo-client-c)
./mediaservo.sh build example && ./mediaservo.sh test example

# GUI (a real display; DISPLAY is already set inside an Xwayland session)
./target/examples/bin/imgui_viewer
# Emergency-stop signing (optional): a pre-shared key file (0600, the same value on vehicle and cockpit; no file = the unsigned form)
export MSRTC_ESTOP_KEY_FILE=/path/to/control-hmac.key
```

## The cockpit dual-room convention (read this before the API)

The media plane and the control plane use **different rooms** (PIT-140 v2 + the W4c ruling; a semantics note from 2026-09-29: the per-stream room = the underlying isolation granularity, not product semantics — the UI/discovery layer should group by the whole-vehicle room with streams as children; see the pitfalls PIT-140 note):

| Plane | Room | Discovery |
|---|---|---|
| Video/audio streams | `<vehicle room>_<stream id>` (e.g. `vehicle_test`) | A `GET /api/rooms` derived entry, `kind:"video"/"audio"` |
| Teleop/e-stop | `<vehicle room>` (e.g. `vehicle`) | The same list, `kind:"control"` |

Hitting both planes from one room = one side is always empty (a stream room has no chassis producer / a vehicle room has no video producer).
This viewer already implements the dual-room pairing; home-grown cockpits should wire it the same way.

## OUT (deliberately not done / not yet done)

- Touch/portrait-specific layouts and the GPU texture backend (sdlgpu3) — after docking, dragging already covers the desktop feel; touch is another round
- Android/Windows assembly — device-day (CI = the Linux `test-gui` job; mac pending)
- The audio-conference face (the `audio-` room kind is discoverable; consumption is not wired)

## CI

The `test-gui` job (Linux) = `build example` + `ctest -R core`; live video and mac/win/Android
= self-hosted/device-day (needs a live server + producer). See `.github/workflows/ci.yml`.
