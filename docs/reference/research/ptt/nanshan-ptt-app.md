# Nanshan PTT app — competitive dossier

> Status: research archive (inert, historical reference only)
> Date: 2026-11-09 | Method: public app-store listings + a public demo video; **no app reverse engineering**
> Vendor: Deyang Yunge Technology (德阳云鸽科技有限公司, c1:allow-zh) | Package: `com.nawensoft.nsptt` | Filing: Sichuan ICP 20001135-3A
> Purpose: ground the mobile-intercom discussion for the TalkServo port; facts are cited, inferences are labeled

## 1. Positioning

A consumer/team **push-to-talk over cellular** app: "your phone is a walkie-talkie".
Network-based, unlimited distance wherever data coverage exists (Wi-Fi / 2G-5G), free to use.
Cross-platform: Android 6.0+, iOS, Windows (Microsoft Store), macOS. Bluetooth headsets and
vehicle kits are advertised; a PNC-series-adapted APK exists for Hytera (海能达, c1:allow-zh) PoC radios.

Marketing claims (not independently verified): standard voice at ~1 kb/s and HD at ~2 kb/s,
"high-compression audio", RNN neural noise suppression.

## 2. Feature inventory (as listed by stores)

| Class | Features |
|-------|----------|
| Voice | real-time PTT (hold to talk), voice-activated mic (VOX), lock-screen reception, RNN denoise, standard/HD voice modes |
| Channel | create your own channel, member management, timed mute of a member, talk priority (话权优先, c1:allow-zh), channel password, per-channel custom talk-time limit, queue mode (排麦, c1:allow-zh), listen to up to 10 channels simultaneously, public-channel discovery |
| Social | friends list (up to 1000), one-to-one call, group call, invite friends into a channel |
| Extras | text and image messages, map location sharing, Morse-code telegraph |
| Platform | Android/iOS/Windows/macOS, Bluetooth headset, vehicle kit, PNC radio adaptation |

## 3. Interaction model (observable)

1. Log in with an account (username/e-mail/ID).
2. Pick or search a channel (public list) or a friend.
3. Inside a channel, press and hold the bottom PTT button to speak; release to listen.
4. The channel owner can mute members, grant talk priority, set a password, and cap talk time.
5. A member may stay tuned to several channels at once (up to 10) and keeps receiving while
   the phone screen is locked.
6. Friends can be called directly (one-to-one) or rung as a group.

## 4. Technical inferences (labeled; not vendor-confirmed)

| Observation | Inference |
|-------------|-----------|
| ~1-2 kb/s advertised voice | narrowband codec profile (Opus/Speex class) + VAD/DTX + aggressive compression; far below WebRTC's habitual 16-24 kb/s mono Opus. Intelligibility-first trade-off |
| RNN denoise | on-device pre-encode suppression (models shipped with the app) |
| Lock-screen reception | Android foreground service + audio focus; likely push wake-up (FCM / vendor channels) for incoming talk bursts |
| 10-channel simultaneous listening | multiple concurrent receive streams with per-channel gain/mixing; a single-transport design would need server-side fan-out or client multi-consume |
| Bluetooth headset / PNC radio | hardware PTT key mapped to the same request/release path as the on-screen button |
| Channel password | room-level join credential; not per-message encryption as far as public material shows |
| Transport | not publicly documented; do not assume WebRTC or UDP vs TCP |

## 5. Mapping onto the TalkServo floor model

| Nanshan feature | TalkServo / MediaServo primitive |
|-----------------|----------------------------------|
| Hold-to-talk PTT | `FloorMode::Exclusive` + hold gesture -> `FloorRequest` / `FloorRelease` |
| Queue mode (排麦, c1:allow-zh) | `FloorState.queue` + `FloorQueued{position}` |
| Talk priority (话权优先, c1:allow-zh) | request/user priority + strict-preempt in Exclusive |
| Timed mute | `MuteSet` (privilege revocation; does not hand the floor back) |
| Channel password | room-scoped join token (MS JWT/PSK audience) |
| 10-channel listening | multi-room consume fan-out (out of PoC scope; a monitoring feature) |
| One-to-one / group call | direct room / group room (server rooms already exist) |
| Lock-screen reception | Android foreground service in a native client (browser cannot) |
| Voice-activated mic | client-side VAD that issues a floor request automatically (optional) |
| Text/image/location/Morse | product extras; explicit non-goals for the first PTT milestone |

## 6. Consequences for the MediaServo port

- **Browser-first PTT covers foreground use only.** Lock-screen/background reception and
  hardware PTT keys are native-client requirements; they belong to the Android track, not the SPA.
- **Android delivery is a separate decision** with materially different costs:
  - PWA/WebView shell: fastest, keeps one UI codebase; limited background audio, no OS-level PTT key, weaker codec control.
  - Native Kotlin + `libmediasoupclient` (mediasoup-client-android) + a WS protocol client: full parity, heaviest, a second client implementation to keep on the wire.
  - React Native / Flutter + WebRTC: one UI codebase, but the mediasoup client protocol and background-audio plumbing are custom work.
  - Rust client + UniFFI (TalkServo D8/D15 path): most aligned with the sister design, gated on a working native media stack and on the PIT-105-class outbound-audio risk.
- **Ultra-low bitrate is a product decision, not a default.** The MS audio plane runs standard
  Opus profiles today; a 1-2 kb/s narrowband profile would be a new, measured tuning exercise.
- **Feature trims for the first milestone**: no text/image/location/Morse/friends, no
  multi-channel monitoring. The core value to port is the floor + dispatch/field loop.

## 7. Sources

- Apple App Store listing (id1444030698), 2025-12 / 2026-01 snapshots.
- Xiaomi app store listing (`com.nawensoft.nsptt`), 2025-11 snapshot.
- Tencent MyApp listing, Wandoujia listing, Lenovo app store listing, 2025-2026 snapshots.
- Bilibili public demo of the Hytera PNC-adapted APK, 2026-01.
- Store metadata only; no binary analysis, no network-protocol capture.
