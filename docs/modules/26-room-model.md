# 26. Room Model (talk / meet / desk / view / ctrl)

> Status: **normative design** — lands with the talkservo-port P2 slice (shell plan `docs/plans/talkservo-port/PLAN.md` §4c, rulings RM1-RM12 user-adjudicated 2026-11-10). The wire part (dialect 4, floor variants, error 4032) is already merged; the kind-based gating below lands with P2.

## 26.1 Why rooms are typed by purpose

One room carries one purpose; composition happens on the client (a page joins several rooms), never server-side. The kind is a **data field on room metadata** — not a name prefix and not a new Rust `RoomType` enum (ruling RM3; Twilio `Type` / Discord `type` precedent). Room ids stay opaque (`fleet1`, `carA/view`).

Three behavioral models sit under the five kinds: **Talk** (enforced speaking rights), **Conference** (`meet`/`desk` — many mics, no enforced floor), **Monitor** (`view` — one-way), plus the non-media `ctrl` room.

## 26.2 The five kinds

| kind | purpose | publishers | floor | data channel | typical |
|---|---|---|---|---|---|
| `talk` | intercom / voice channel | channel members (accounts, audio) | **enforced** (Exclusive/Hybrid + queue/preempt) | — | PTT, 1:1 |
| `meet` | meeting / voice conference | members (audio/video/screen) | none (full-duplex; `video=false` = voice-only chat) | — | meetings, group chat |
| `desk` | remote desktop | controlled-end screen + controller input | none | ✅ control ACL | remote assistance |
| `view` | monitoring | **device only** (cameras + ambient mic) | none | — | surveillance, livestream variant |
| `ctrl` | device control | data | — | ✅ | vehicle control, status |

Kind rules: `talk` never carries video; `meet` never injects input; `view` never carries floor; `ctrl` never carries audio/video producers. Floor commands are **kind-gated**: only `talk` rooms accept them; a `Floor*` command elsewhere is refused with 4032 and audited.

## 26.3 How the kind is resolved

- `RoomMeta`/`RoomKind` live in `mediaservo-common/src/room.rs` (lands P2).
- The server resolves the kind via `room_kind(room_id)` (replaces the `audio-` prefix check `is_audio_room()`): explicit metadata first, then legacy fallbacks (`audio-<id>` → `meet, video=false`; `<device>_<stream>` → `stream`, deprecated), then device-registry derivation.
- Legacy forms are **deprecated, not removed**: resolution still works, emits a once-per-room warning, and the discovery `RoomEntry` carries a `deprecated` field.

## 26.4 Lifecycle and storage

- **Derived rooms** (`view`, `ctrl`, legacy `stream`) follow the device registry — they appear/disappear with the device, no admin action.
- **Managed rooms** (`talk`, `meet`, `desk`) are created/edited/deleted by the admin API and persist in `etc/rooms.yaml` (atomic write-back tmp+rename, one audit line per mutation, refuse-on-corrupt). `meet` rooms may instead be ephemeral with an empty-timeout. Single-instance boundary: `rooms.yaml` is not shared across server instances.

## 26.5 Media topology defaults

- Uplink: **device host** — one PC per camera process (process/fault isolation), all producers into the device's `view` room; **interactive clients** — one PC with several producers (mic+camera+screen+data on one transport).
- Downlink: the client chooses per its UI — **M1** one PC with N consumers filtered by producer `label` (default; multi-screen subscribes `front` only, teleop subscribes all + ambient); **M2** one PC, one consumer by label (legacy one-stream apps); **M3** the composition — an isolated label (e.g. `front`) on its own transport plus the rest on a shared one (hard bandwidth/failure isolation for safety video).
- Producer labels are the subscription address: `front` / `left` / `right` / `rear` / `ambient` (O19).
- The server never gates a producer that is not a floor participant in a `talk` room: `view`/`ctrl` media is untouched by intercom, and the vehicle device is a media peer, never a PTT participant.

## 26.6 Compatibility notes

- Signaling dialect 4 is additive (`MIN_SUPPORTED` stays 1); native clients ignore unknown variants (landed).
- One WebSocket per room today; browsers cap same-origin WS (~6-30) — pages needing more rooms go through single-WS multiplexing or an aggregated wall room (open decision, plan O17).
- The per-stream room shape (`<device>_<stream>`) keeps working during the deprecation window.

Related: `07-protocols.md` §5 (dialect 4 + floor variants + room kind on the wire), `10-signaling-architecture.md` §10.12 (delivery), `error-model.md` 4032, plan §4c (rulings), `evidence/room-model-architecture.md` in the shell repo (pictures).
