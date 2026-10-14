# mediasoup Official Source Navigation

> Created: 2026-08-05 | Purpose: the authoritative official-source baseline for AI analysis (cross-platform/cross-host development)
> Source dir: `.refinfo/` (git-ignored, script-managed) | Script: `scripts/sync-official-refs.sh`

## Why This Exists

MediaServo's Host SFU integration needs an authoritative baseline for the mediasoup client negotiation flow. The official client sources (mediasoup-client JS / libmediasoupclient C++ / mediasoup-demo) are **the only authoritative comparison for the standard offer/answer negotiation flow** (C18 official-usage-first). The script pulls these into the git-ignored `.refinfo/`, avoiding `/tmp` (no loss across hosts/reboots); shallow clones fetch source only.

**Deliberately excluded**:
- `mediasoup` (server) — we only call its API and never modify it; no source needed
- `mediasoup-client-android` — a third-party community fork (not official versatica), and not Rust

## Usage

```bash
# First time / new machine / want latest
bash scripts/sync-official-refs.sh          # idempotent; skips if present
bash scripts/sync-official-refs.sh --force  # force re-pull

# Cleanup
rm -rf .refinfo
```

Network note: GitHub interference in China (PIT-14) → the script auto-falls-back to `--http1.1`; on failure set `HTTPS_PROXY` and retry.

## Repository Inventory

| Repo | Directory | Version@commit | Purpose |
|------|------|-------------|------|
| mediasoup-client (JS) | `.refinfo/mediasoup-client` | @ 1d4d597 | `Chrome74.ts send()` — the JS baseline of the standard produce negotiation flow |
| libmediasoupclient (C++) | `.refinfo/libmediasoupclient` | @ 8f08b4a | `Handler.cpp Send()` — the official C++ client baseline |
| mediasoup-demo | `.refinfo/mediasoup-demo` | @ 6a578ac | `RoomClient.js` — the official demo end-to-end flow |

## Key File Navigation (for analysis)

### mediasoup-client (JS)
| File | Content | Corresponding MediaServo problem |
|------|------|------------------|
| `src/handlers/Chrome74.ts:335-563` | the complete `send()` produce negotiation | Host produce standard-flow baseline |
| `src/handlers/Chrome74.ts:360-364` | `addTransceiver(sendonly, sendEncodings)` | Standard direction + encoding parameters |
| `src/handlers/Chrome74.ts:394-397` | `ortc.getSendingRtpParameters` | rtpParameters derived from the offer |
| `src/handlers/Chrome74.ts:542-560` | the client builds its own answer + `setRemoteDescription` | **mediasoup never returns SDP** (pure ORTC) |
| `src/handlers/ortc/RTCRtpParameters.ts` | ortc parameter construction | ssrc/PT/fmtp derivation |

### libmediasoupclient (C++)
| File | Content | Corresponding MediaServo problem |
|------|------|------------------|
| `src/Handler.cpp:185-381` | the complete `SendHandler::Send()` produce | Host produce standard-flow C++ baseline |
| `src/Handler.cpp:191-200` | `RtpTransceiverInit{kSendOnly, send_encodings}` | Standard direction + encodings |
| `src/Handler.cpp:306-308` | `getRtpEncodings` parses the ssrc from the offer SDP | **The ssrc source** (not hand-made) |
| `src/Handler.cpp:365-369` | `remoteSdp->GetSdp()` builds the answer itself | The client builds its own answer |
| `src/sdp/RemoteSdp.cpp:157-185` | `RemoteSdp::Send` → `AnswerMediaSection` | How the answer uses the server's rtpParameters |
| `src/sdp/MediaSection.cpp:255-277` | codecOptions → fmtp (x-google-bitrate) | **The official keyframe/bitrate parameter path** |

### mediasoup-demo
| File | Content | Corresponding MediaServo problem |
|------|------|------------------|
| `app/src/RoomClient.js` | End-to-end produce/consume | The full signaling + media negotiation flow |

## Related Docs

- `docs/reference/webrtc/mediasoup-client.md` — official client architecture/usage/case notes (profile)
- `docs/reference/webrtc/webrtc-w3c-alignment.md` — W3C API alignment rework analysis (incl. §0 team-review corrections)
- `docs/reference/webrtc/keyframe-black-screen-analysis.md` — PIT-65 keyframe black-screen root-cause analysis
- `.agents/memorys/` — the C18 official-usage-first constraint, PIT-46/54/56/65 lessons
