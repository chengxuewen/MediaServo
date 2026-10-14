# mediasoup Rust API Reference (v0.24.2)

> **Source**: [versatica/mediasoup v3/rust](https://github.com/versatica/mediasoup)  
> **Generated**: 2026-07-31  
> **Applies to**: mediasoup 0.24.x (Rust crate)

---

## 1. Architecture Overview

```
WorkerManager
  └── Worker (C++ thread, one CPU core, hosts Router instances)
       ├── WebRtcServer (carries multiple WebRtcTransports on one port, ICE Lite)
       └── Router (media routing)
            ├── WebRtcTransport (ICE/DTLS network path, sends/receives media)
            │    ├── Producer (injects audio/video sources)
            │    └── Consumer (forwards to endpoints)
            ├── PlainTransport (RTP/RTCP plain)
            ├── PipeTransport (communication between Routers on the same host)
            └── DirectTransport (in-process direct transfer)
```

The mediasoup Rust API is **ICE Lite** (server side); it does not initiate ICE connections and waits for the client's Binding Request.

---

## 2. Worker Creation

### 2.1 WorkerSettings

```rust
use mediasoup::worker::{WorkerSettings, WorkerLogTag, WorkerLogLevel};
use mediasoup::worker_manager::WorkerManager;
use std::net::{IpAddr, Ipv4Addr};
use std::ops::RangeInclusive;

let worker_manager = WorkerManager::new();

let worker_settings = WorkerSettings {
    log_level: WorkerLogLevel::Debug,  // Debug | Warn | Error | None
    log_tags: vec![
        WorkerLogTag::Info,
        WorkerLogTag::Ice,
        WorkerLogTag::Dtls,
        WorkerLogTag::Rtp,
        WorkerLogTag::Rtcp,
        // ...
    ],
    rtc_port_range: 10000..=59999,  // ICE/DTLS/RTP port range
    dtls_files: None,  // None = auto-generated certificates
    libwebrtc_field_trials: None,
    thread_initializer: None,
    app_data: AppData::default(),
};

let worker = worker_manager
    .create_worker(worker_settings)
    .await
    .expect("Failed to create worker");
```

### 2.2 Worker Lifecycle Callbacks

```rust
worker.on_new_router(|router| { /* Router created */ });
worker.on_new_webrtc_server(|server| { /* WebRtcServer created */ });
worker.on_dead(|result| { /* Worker thread exited unexpectedly */ });
worker.on_close(|| { /* Worker closed */ });
```

---

## 3. Router Creation & RouterOptions

### 3.1 RouterOptions + media_codecs

```rust
use mediasoup::router::RouterOptions;
use mediasoup_types::rtp_parameters::{
    RtpCodecCapability, MimeTypeAudio, MimeTypeVideo,
    RtpCodecParametersParameters,
};
use std::num::{NonZeroU32, NonZeroU8};

fn media_codecs() -> Vec<RtpCodecCapability> {
    vec![
        // Opus audio
        RtpCodecCapability::Audio {
            mime_type: MimeTypeAudio::Opus,
            preferred_payload_type: None,  // None = auto-assigned
            clock_rate: NonZeroU32::new(48000).unwrap(),
            channels: NonZeroU8::new(2).unwrap(),
            parameters: RtpCodecParametersParameters::from([
                ("useinbandfec", 1_u32.into()),
            ]),
            rtcp_feedback: vec![],
        },
        // VP8 video
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::Vp8,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(90000).unwrap(),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: vec![],
        },
        // H264 video
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::H264,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(90000).unwrap(),
            parameters: RtpCodecParametersParameters::from([
                ("level-asymmetry-allowed", 1_u32.into()),
                ("packetization-mode", 1_u32.into()),
                ("profile-level-id", "4d0032".into()),
            ]),
            rtcp_feedback: vec![],
        },
    ]
}

let router = worker
    .create_router(RouterOptions::new(media_codecs()))
    .await
    .expect("Failed to create router");
```

**Key**: `RouterOptions::default()` creates `media_codecs: vec![]` → an **empty codec list**, and produce will report "Unsupported codec". You must pass `RouterOptions::new(media_codecs)` explicitly.

### 3.2 Router rtp_capabilities (returned to the client)

```rust
let rtp_capabilities = router.rtp_capabilities();
// Serialize rtp_capabilities and send it to the client, which uses it for computeSendingRtpParameters
```

### 3.3 Router::can_consume

```rust
let can_consume = router.can_consume(&producer.id(), &client_rtp_capabilities);
```

---

## 4. WebRtcTransport

### 4.1 Option A: Individual (independent ports)

```rust
use mediasoup::webrtc_transport::{
    WebRtcTransportOptions, WebRtcTransportListenInfos,
};
use mediasoup_types::data_structures::{ListenInfo, Protocol};
use std::net::{IpAddr, Ipv4Addr};

let transport = router
    .create_webrtc_transport(WebRtcTransportOptions::new(
        WebRtcTransportListenInfos::new(ListenInfo {
            protocol: Protocol::Udp,
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            announced_address: Some("public IP or domain".to_string()),
            expose_internal_ip: false,
            port: None,               // None = auto-assigned
            port_range: None,         // Some(40000..=40100)
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        }),
    ))
    .await?;
```

### 4.2 Option B: WebRtcServer (shared single port) — recommended

```rust
use mediasoup::webrtc_server::{WebRtcServerOptions, WebRtcServerListenInfos};

// 1. Create the WebRtcServer (Worker level)
let webrtc_server = worker
    .create_webrtc_server(WebRtcServerOptions::new(
        WebRtcServerListenInfos::new(ListenInfo {
            protocol: Protocol::Udp,
            ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),  // 0.0.0.0
            announced_address: None,
            expose_internal_ip: false,
            port: Some(20000),  // Fixed single port!
            port_range: None,
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        }),
    ))
    .await?;

// 2. Create the Transport using the WebRtcServer
let transport = router
    .create_webrtc_transport(
        WebRtcTransportOptions::new_with_server(webrtc_server.clone())
    )
    .await?;
```

`new_with_server()` vs `new()`:
- `new_with_server`: `enable_tcp: true` (default), `enable_udp: true`
- `new`: `enable_tcp: false` (default), `enable_udp: true`

### 4.3 Retrieving ICE/DTLS Parameters (sent to the client)

```rust
// Fetch the parameters immediately after creating the Transport and send them to the browser
let id = transport.id();
let ice_parameters = transport.ice_parameters();  // usernameFragment + password
let ice_candidates = transport.ice_candidates();  // Vec<IceCandidate>
let dtls_parameters = transport.dtls_parameters(); // role + fingerprints
```

### 4.4 ICE State Change Callbacks

```rust
transport.on_ice_state_change(|ice_state| {
    match ice_state {
        IceState::New => {},
        IceState::Connected => { /* ICE connected! */ },
        IceState::Completed => {},
        IceState::Disconnected => {},
        IceState::Failed => {},
        _ => {},
    }
});

transport.on_dtls_state_change(|dtls_state| {
    match dtls_state {
        DtlsState::Connected => { /* DTLS handshake complete! */ },
        _ => {},
    }
});
```

### 4.5 Connecting with Client DTLS Parameters (connect)

```rust
use mediasoup::webrtc_transport::WebRtcTransportRemoteParameters;
use mediasoup_types::data_structures::{DtlsParameters, DtlsRole, DtlsFingerprint};

// After the browser sends its DTLS parameters, call connect:
transport
    .connect(WebRtcTransportRemoteParameters {
        dtls_parameters: DtlsParameters {
            role: DtlsRole::Client,  // Browser side
            fingerprints: vec![
                DtlsFingerprint::Sha256 {
                    value: [/* 32 bytes of SHA-256 hash */],
                }
            ],
        },
    })
    .await
    .expect("Failed to connect WebRTC transport");

// connect can only be called once; a repeated call returns Err(RequestError::Response{..})
```

### 4.6 Bandwidth Control

```rust
transport.set_max_incoming_bitrate(1_000_000).await?;  // bps
transport.set_max_outgoing_bitrate(2_000_000).await?;
transport.set_min_outgoing_bitrate(100_000).await?;
// Pass 0 to remove the limit
```

### 4.7 ICE Restart

```rust
let new_ice_params = transport.restart_ice().await?;
// Returns new IceParameters, which must be forwarded to the client
```

---

## 5. Producer — Full RtpParameters Format

### 5.1 Audio Producer (Opus)

```rust
use mediasoup::producer::ProducerOptions;
use mediasoup_types::rtp_parameters::{
    MediaKind, MimeTypeAudio, RtcpParameters, RtpCodecParameters,
    RtpCodecParametersParameters, RtpEncodingParameters,
    RtpHeaderExtensionParameters, RtpHeaderExtensionUri, RtpParameters,
};

let audio_producer = transport.produce(ProducerOptions::new(
    MediaKind::Audio,
    RtpParameters {
        mid: Some("AUDIO".to_string()),
        codecs: vec![
            RtpCodecParameters::Audio {
                mime_type: MimeTypeAudio::Opus,
                payload_type: 0,  // Must match the Opus payloadType assigned in the Router
                clock_rate: NonZeroU32::new(48000).unwrap(),
                channels: NonZeroU8::new(2).unwrap(),
                parameters: RtpCodecParametersParameters::from([
                    ("useinbandfec", 1_u32.into()),
                    ("usedtx", 1_u32.into()),
                ]),
                rtcp_feedback: vec![],
            }
        ],
        header_extensions: vec![
            RtpHeaderExtensionParameters {
                uri: RtpHeaderExtensionUri::Mid,
                id: 10,
                encrypt: false,
            },
            RtpHeaderExtensionParameters {
                uri: RtpHeaderExtensionUri::SsrcAudioLevel,
                id: 12,
                encrypt: false,
            },
        ],
        encodings: vec![
            RtpEncodingParameters {
                ssrc: Some(11111111),
                ..RtpEncodingParameters::default()
            }
        ],
        rtcp: RtcpParameters {
            cname: Some("audio-1".to_string()),
            ..RtcpParameters::default()
        },
        msid: None,
    },
)).await?;
```

### 5.2 Video Producer (H264 + Simulcast)

```rust
let video_producer = transport.produce(ProducerOptions::new(
    MediaKind::Video,
    RtpParameters {
        mid: Some("VIDEO".to_string()),
        codecs: vec![
            RtpCodecParameters::Video {
                mime_type: MimeTypeVideo::H264,
                payload_type: 112,
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::from([
                    ("packetization-mode", 1_u32.into()),
                    ("profile-level-id", "4d0032".into()),
                ]),
                rtcp_feedback: vec![
                    RtcpFeedback::Nack,
                    RtcpFeedback::NackPli,
                    RtcpFeedback::GoogRemb,
                ],
            },
            RtpCodecParameters::Video {
                mime_type: MimeTypeVideo::Rtx,
                payload_type: 113,
                clock_rate: NonZeroU32::new(90000).unwrap(),
                parameters: RtpCodecParametersParameters::from([("apt", 112u32.into())]),
                rtcp_feedback: vec![],
            },
        ],
        header_extensions: vec![
            RtpHeaderExtensionParameters {
                uri: RtpHeaderExtensionUri::Mid,
                id: 10,
                encrypt: false,
            },
            RtpHeaderExtensionParameters {
                uri: RtpHeaderExtensionUri::VideoOrientation,
                id: 13,
                encrypt: false,
            },
        ],
        encodings: vec![
            RtpEncodingParameters {
                ssrc: Some(22222222),
                rtx: Some(RtpEncodingParametersRtx { ssrc: 22222223 }),
                scalability_mode: "L1T5".parse().unwrap(),
                ..RtpEncodingParameters::default()
            },
            RtpEncodingParameters {
                ssrc: Some(22222224),
                rtx: Some(RtpEncodingParametersRtx { ssrc: 22222225 }),
                scalability_mode: "L1T5".parse().unwrap(),
                ..RtpEncodingParameters::default()
            },
            RtpEncodingParameters {
                ssrc: Some(22222226),
                rtx: Some(RtpEncodingParametersRtx { ssrc: 22222227 }),
                scalability_mode: "L1T5".parse().unwrap(),
                ..RtpEncodingParameters::default()
            },
        ],
        rtcp: RtcpParameters {
            cname: Some("video-1".to_string()),
            ..RtcpParameters::default()
        },
        msid: None,
    },
)).await?;
```

### 5.3 Producer Type Determination

| Number of encodings | scalability_mode | ProducerType |
|-------------|-----------------|--------------|
| 1 | None | `Simple` |
| 1 | `"L1T3"` etc. | `Svc` |
| ≥2 | any | `Simulcast` |

### 5.4 Producer Events

```rust
producer.on_pause(|| {});
producer.on_resume(|| {});
producer.on_score(|scores: &[ProducerScore]| {
    for s in scores {
        println!("encoding={} ssrc={} score={}", s.encoding_idx, s.ssrc, s.score);
    }
});
producer.on_close(|| {});
```

---

## 6. Consumer — Full RtpCapabilities Format

### 6.1 RtpCapabilities (client device capabilities)

```rust
use mediasoup_types::rtp_parameters::{
    MediaKind, MimeTypeAudio, MimeTypeVideo, RtcpFeedback,
    RtpCapabilities, RtpCodecCapability, RtpCodecParametersParameters,
    RtpHeaderExtension, RtpHeaderExtensionDirection, RtpHeaderExtensionUri,
};

let device_capabilities = RtpCapabilities {
    codecs: vec![
        RtpCodecCapability::Audio {
            mime_type: MimeTypeAudio::Opus,
            preferred_payload_type: Some(100),  // Client-preferred payloadType
            clock_rate: NonZeroU32::new(48000).unwrap(),
            channels: NonZeroU8::new(2).unwrap(),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: vec![RtcpFeedback::Nack],
        },
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::H264,
            preferred_payload_type: Some(101),
            clock_rate: NonZeroU32::new(90000).unwrap(),
            parameters: RtpCodecParametersParameters::from([
                ("level-asymmetry-allowed", 1_u32.into()),
                ("packetization-mode", 1_u32.into()),
                ("profile-level-id", "4d0032".into()),
            ]),
            rtcp_feedback: vec![
                RtcpFeedback::Nack,
                RtcpFeedback::NackPli,
                RtcpFeedback::CcmFir,
                RtcpFeedback::GoogRemb,
            ],
        },
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::Rtx,
            preferred_payload_type: Some(102),
            clock_rate: NonZeroU32::new(90000).unwrap(),
            parameters: RtpCodecParametersParameters::from([("apt", 101_u32.into())]),
            rtcp_feedback: vec![],
        },
    ],
    header_extensions: vec![
        RtpHeaderExtension {
            kind: MediaKind::Audio,
            uri: RtpHeaderExtensionUri::Mid,
            preferred_id: 1,
            preferred_encrypt: false,
            direction: RtpHeaderExtensionDirection::default(),
        },
        RtpHeaderExtension {
            kind: MediaKind::Video,
            uri: RtpHeaderExtensionUri::Mid,
            preferred_id: 1,
            preferred_encrypt: false,
            direction: RtpHeaderExtensionDirection::default(),
        },
        RtpHeaderExtension {
            kind: MediaKind::Audio,
            uri: RtpHeaderExtensionUri::SsrcAudioLevel,
            preferred_id: 6,
            preferred_encrypt: false,
            direction: RtpHeaderExtensionDirection::default(),
        },
        RtpHeaderExtension {
            kind: MediaKind::Video,
            uri: RtpHeaderExtensionUri::VideoOrientation,
            preferred_id: 8,
            preferred_encrypt: false,
            direction: RtpHeaderExtensionDirection::default(),
        },
    ],
};
```

### 6.2 Consumer Creation

```rust
use mediasoup::consumer::{ConsumerOptions, ConsumerLayers};

let consumer = transport.consume(ConsumerOptions::new(
    producer.id(),                    // The Producer ID to consume
    device_capabilities,              // Client RTP capabilities
)).await?;

// With options:
let consumer = transport.consume({
    let mut options = ConsumerOptions::new(producer.id(), device_capabilities);
    options.paused = true;                           // Pause first, wait for the client to be ready
    options.preferred_layers = Some(ConsumerLayers {
        spatial_layer: 0,
        temporal_layer: Some(1),
    });
    options.enable_rtx = Some(true);                 // Enable NACK retransmission
    options.mid = Some("custom-mid".to_owned());     // Custom MID
    options.pipe = false;                            // PipeTransport mode
    options.ignore_dtx = false;                      // DTX packet handling
    options
}).await?;
```

### 6.3 Consumer RtpParameters (forwarded to the client)

```rust
let consumer_rtp_params = consumer.rtp_parameters();
// Contains: mid, codecs, header_extensions, encodings, rtcp
// The browser uses this to create the local RTCRtpReceiver
```

### 6.4 Consumer Control

```rust
consumer.pause().await?;
consumer.resume().await?;
consumer.set_preferred_layers(ConsumerLayers {
    spatial_layer: 2, temporal_layer: Some(3),
}).await?;
consumer.set_priority(2).await?;   // 1-255, default 1
consumer.request_key_frame().await?;
consumer.get_stats().await?;       // ConsumerStat
```

---

## 7. DtlsParameters Structure

```rust
pub struct DtlsParameters {
    pub role: DtlsRole,  // Auto | Client | Server
    pub fingerprints: Vec<DtlsFingerprint>,
}

pub enum DtlsFingerprint {
    Sha1 { value: [u8; 20] },
    Sha224 { value: [u8; 28] },
    Sha256 { value: [u8; 32] },
    Sha384 { value: [u8; 48] },
    Sha512 { value: [u8; 64] },
}

// Sent by the client for connect:
// {
//   "role": "client",
//   "fingerprints": [{ "algorithm": "sha-256", "value": "82:5A:68:..." }]
// }
```

---

## 8. WebRtcServer Single-Port Mode (Full Flow)

```rust
use mediasoup::webrtc_server::{WebRtcServerOptions, WebRtcServerListenInfos};
use mediasoup::webrtc_transport::WebRtcTransportOptions;

// Step 1: Worker creates the WebRtcServer (port 20000)
let webrtc_server = worker
    .create_webrtc_server(WebRtcServerOptions::new(
        WebRtcServerListenInfos::new(ListenInfo {
            protocol: Protocol::Udp,
            ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            announced_address: None,
            expose_internal_ip: false,
            port: Some(20000),
            port_range: None,
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        }),
    ))
    .await?;

// Step 2: Each Peer creates a Transport reusing the same port
let transport_1 = router
    .create_webrtc_transport(WebRtcTransportOptions::new_with_server(webrtc_server.clone()))
    .await?;

let transport_2 = router
    .create_webrtc_transport(WebRtcTransportOptions::new_with_server(webrtc_server))
    .await?;

// WebRtcServer distinguishes different Transports via local_ice_username_fragment
```

---

## 9. Complete Signaling Flow

```
Browser                          Server (Rust/mediasoup)
  │                                    │
  │──── WS: getRouterRtpCapabilities ──→│
  │←── router.rtp_capabilities() ──────│
  │                                    │
  │──── WS: createWebRtcTransport ─────→│
  │       (clientRtpCapabilities)      │ router.create_webrtc_transport(...)
  │←── transport.id() +                │
  │     iceParameters +                │
  │     iceCandidates +                │
  │     dtlsParameters ────────────────│
  │                                    │
  │─── WebRTC setRemoteDescription ───→│ (browser local)
  │←── createAnswer ──────────────────│
  │─── WebRTC setLocalDescription ────│
  │                                    │
  │──── WS: connectWebRtcTransport ────→│
  │       {dtlsParameters: browser}    │ transport.connect(remote_dtls_params)
  │←── "transport connected" ──────── │ (DTLS + ICE complete)
  │                                    │
  │──── WS: produce ──────────────────→│
  │       {kind, rtpParameters}        │ transport.produce(producer_options)
  │←── producer.id() ──────────────────│
  │                                    │
  │──── WS: consume ───────────────────→│ (another Peer)
  │       {producerId, rtpCapabilities}│ transport.consume(consumer_options)
  │←── consumer.rtp_parameters() ─────│ → used to create the RTCRtpReceiver
```

---

## 10. Common Errors & Solutions

### E1: `RouterOptions::default()` creates an empty codec list

**Symptom**: `produce()` returns "Unsupported codec"

**Root cause**: `RouterOptions::default()` yields `media_codecs: vec![]`

**Solution**: You must use `RouterOptions::new(media_codecs)` to pass the codec list explicitly

### E2: `RtpCodecParameters` untagged enum deserialization failure

**Symptom**: "data did not match any variant of untagged enum RtpCodecParameters"

**Root cause**: `RtpCodecParameters` is a `#[serde(untagged)]` enum with `Audio` and `Video` variants. Each variant requires specific fields. Video does not need `channels`, Audio does. Missing fields cause variant matching to fail.

**Solution**: Ensure type-safe construction with `RtpCodecParameters::Audio{...}` / `RtpCodecParameters::Video{...}` rather than JSON deserialization.

### E3: payloadType mismatch

**Symptom**: produce succeeds but the client cannot decode

**Solution**: The payloadType in the Producer RtpParameters must match the value assigned to that codec in the Router media_codecs. You can get the payloadType actually in use from the Consumer rtp_parameters.

### E4: Incorrect apt parameter on the RTX codec

**Symptom**: `ProduceError::FailedRtpParametersMapping`

**Root cause**: The RTX codec's `apt` parameter must point at the associated primary codec payloadType

**Solution**:
```rust
RtpCodecParameters::Video {
    mime_type: MimeTypeVideo::Rtx,
    payload_type: 113,
    // ...
    parameters: RtpCodecParametersParameters::from([("apt", 112u32.into())]),
    // apt must = the primary H264 codec's payloadType
}
```

### E5: Inter-Router pipe must use different Routers

**Symptom**: `PipeProducerToRouterError::SameRouter`

**Solution**: The target Router must differ from the source Router

### E6: Second connect on a WebRtcTransport fails

**Symptom**: `RequestError::Response { reason: "..." }`

**Solution**: `transport.connect()` can only be called once; repeated calls raise an error

### E7: Empty encodings cause produce to fail (video)

**Symptom**: `ProduceError::Request`

**Solution**: A video Producer must provide at least 1 encoding, and each encoding must have an ssrc or rid

### E8: MID conflict

**Symptom**: A duplicate MID causes produce to fail

**Solution**: Each Producer's `mid` must be unique within the same Transport

---

## 11. Serde JSON Mapping

mediasoup Rust types use `#[serde(rename_all = "camelCase")]`; field names are automatically converted to camelCase when serialized to JSON:

| Rust field | JSON field |
|-----------|----------|
| `payload_type` | `payloadType` |
| `clock_rate` | `clockRate` |
| `mime_type` | `mimeType` |
| `header_extensions` | `headerExtensions` |
| `rtcp_feedback` | `rtcpFeedback` |
| `ice_parameters` | `iceParameters` |
| `media_codecs` | `mediaCodecs` |
| `preferred_payload_type` | `preferredPayloadType` |
| `scalability_mode` | `scalabilityMode` |

**Note**: The `Audio`/`Video` variants of `RtpCodecParameters` use `#[serde(untagged)]`, with no tagging — serialization does not include a `"type": "audio"` field; the mediasoup worker distinguishes them by `mimeType` and the presence of the `channels` field.

---

## 12. Key Type Path Quick Reference

| Type | Import path |
|------|---------|
| `WorkerManager` | `mediasoup::worker_manager::WorkerManager` |
| `Worker` | `mediasoup::worker::Worker` |
| `WorkerSettings` | `mediasoup::worker::WorkerSettings` |
| `Router` | `mediasoup::router::Router` |
| `RouterOptions` | `mediasoup::router::RouterOptions` |
| `WebRtcServer` | `mediasoup::webrtc_server::WebRtcServer` |
| `WebRtcServerOptions` | `mediasoup::webrtc_server::WebRtcServerOptions` |
| `WebRtcTransport` | `mediasoup::webrtc_transport::WebRtcTransport` |
| `WebRtcTransportOptions` | `mediasoup::webrtc_transport::WebRtcTransportOptions` |
| `ProducerOptions` | `mediasoup::producer::ProducerOptions` |
| `ConsumerOptions` | `mediasoup::consumer::ConsumerOptions` |
| `RtpParameters` | `mediasoup_types::rtp_parameters::RtpParameters` |
| `RtpCapabilities` | `mediasoup_types::rtp_parameters::RtpCapabilities` |
| `RtpCodecCapability` | `mediasoup_types::rtp_parameters::RtpCodecCapability` |
| `RtpCodecParameters` | `mediasoup_types::rtp_parameters::RtpCodecParameters` |
| `MediaKind` | `mediasoup_types::rtp_parameters::MediaKind` |
| `DtlsParameters` | `mediasoup_types::data_structures::DtlsParameters` |
| `DtlsFingerprint` | `mediasoup_types::data_structures::DtlsFingerprint` |
| `ListenInfo` | `mediasoup_types::data_structures::ListenInfo` |
| `Protocol` | `mediasoup_types::data_structures::Protocol` |
| `AppData` | `mediasoup_types::data_structures::AppData` |

---

## 13. prelude Shortcut Imports

```rust
use mediasoup::prelude::*;
```

Covers: `WorkerManager`, `Worker`, `Router`, `Transport`, `Producer`, `Consumer`, `WebRtcTransport`, `WebRtcServer`, and the common exports of the related types.

## 14. mediasoup-sys Build Requirements

- **Linux x86_64 only** — the mediasoup C++ Worker does not build on macOS/Windows
- `cargo check --features sfu-mediasoup` passes on macOS (type checking only)
- `cargo build` / `cargo test` require a Linux environment
- Requires meson, ninja-build, libuv1-dev, libssl-dev
- `rtc_port_range` defaults to 10000-59999; Docker must map the corresponding UDP port range

---

**See also**:
- [mediasoup official docs](https://mediasoup.org/documentation/v3/)
- [RTP Parameters and Capabilities](https://mediasoup.org/documentation/v3/mediasoup/rtp-parameters-and-capabilities/)
- [GitHub integration tests](https://github.com/versatica/mediasoup/tree/v3/rust/tests/integration)
