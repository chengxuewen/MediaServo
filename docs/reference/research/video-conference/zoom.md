# Zoom Reference Analysis

> Generated: 2026-07-16 | Category: Video Conference

## 1. Product Profile

- **Name**: Zoom Workplace (formerly Zoom Video Communications; brand upgraded to an AI work system in late 2025)
- **Developer**: Zoom Communications, Inc. (NASDAQ: ZM)
- **Founder**: Eric S. Yuan (Yuan Zheng), former VP of Engineering at Cisco WebEx. Denied a US visa 9 times before the 10th attempt was approved. Founded Zoom in 2011; NASDAQ IPO in 2019
- **First release**: Zoom Meetings v1.0 in January 2013 (beta August 2012). IPO in 2019. During the 2020 pandemic, daily meeting participants peaked at 300 million (including duplicates)
- **Product positioning**: AI-driven modern work system (System of Action). Upgraded from a pure video conferencing platform to a full UCaaS/CCaaS suite covering Meetings, Phone, Contact Center, Webinars, Team Chat, Whiteboard, and Revenue Accelerator
- **Target users**: The full spectrum, from individual entrepreneurs to Fortune 500. Enterprise customers ($100K+ annual contracts), 4,350+, are the core growth driver. Online self-serve churn rate is near its all-time low
- **Licensing / business model**: Fully closed-source commercial SaaS. Subscription tiers (Basic/Pro/Business/Enterprise). FY2026 full-year revenue $4.868B (+4.4% YoY). Cash and investments $7.8B. FY2027 expected to break $5B

## 2. Technical Characteristics

### 2.1 Overall Architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│                         Zoom Global Infrastructure                     │
│                                                                        │
│  ┌───────────────────────────────────────────────────────────────┐    │
│  │               Global Cloud Controller (GCC)                    │    │
│  │  ─────────────────────────────────────────────────────────────│    │
│  │  • Awareness of state across dozens of global data centers     │    │
│  │  • Routes to the optimal Zone based on user geolocation        │    │
│  │    + network quality                                           │    │
│  │  • Cross-Zone meeting coordination (decides cascade topology)  │    │
│  │  • Global load balancing and failover                          │    │
│  └────────────┬───────────────────────────┬──────────────────────┘    │
│               │ Zone A                    │ Zone B                     │
│  ┌────────────┴───────────┐ ┌────────────┴───────────┐                │
│  │    Zone Controller     │ │    Zone Controller     │                │
│  │  ────────────────────  │ │  ────────────────────  │                │
│  │  • In-zone load        │ │  • In-zone load        │                │
│  │    balancing           │ │    balancing           │                │
│  │  • MMR pool health     │ │  • MMR pool health     │                │
│  │    checks              │ │    checks              │                │
│  │  • New meeting → best  │ │  • New meeting → best  │                │
│  │    MMR                 │ │    MMR                 │                │
│  │  • Signaling routing   │ │  • Signaling routing   │                │
│  └────────────┬───────────┘ └────────────┬───────────┘                │
│               │ MMR Pool                │ MMR Pool                    │
│  ┌────────────┴───────────┐ ┌────────────┴───────────┐                │
│  │  MMR (Meeting Server)  │ │  MMR (Meeting Server)  │                │
│  │  ────────────────────  │ │  ────────────────────  │                │
│  │                        │ │                        │                │
│  │  Media Forwarding      │ │  Media Forwarding      │                │
│  │  ┌──────────────────┐  │ │  ┌──────────────────┐  │                │
│  │  │ RTP selective    │  │ │  │ RTP selective    │  │                │
│  │  │ forwarding       │  │ │  │ forwarding       │  │                │
│  │  ├──────────────────┤  │ │  ├──────────────────┤  │                │
│  │  │ Sub 1: 720p(L3)  │  │ │  │ Sub A: 360p(L2)  │  │                │
│  │  │ Sub 2: 360p(L2)  │  │ │  │ Sub B: 720p(L3)  │  │                │
│  │  │ Sub 3: 180p(L1)  │  │ │  │ Sub C: 180p(L1)  │  │                │
│  │  └──────────────────┘  │ │  └──────────────────┘  │                │
│  └────────────────────────┘ └────────────────────────┘                │
│               │                                                        │
│               │ Cross-Zone cascade: Zoom private fiber backbone        │
│               │ (not public BGP — only 10-30ms incremental latency     │
│               │  across regions)                                       │
│               │                                                        │
│  ┌───────────────────────────────────────────────────────────────┐    │
│  │                Zoom Node (Hybrid Deployment)                  │    │
│  │  ──────────────────────────────────────────────────────────── │    │
│  │  Enterprise DC runs a local MMR coexisting with Zoom Cloud    │    │
│  │  Same region: low latency via the local MMR                   │    │
│  │  Cross-region: automatic fallback to Zoom Cloud MMR           │    │
│  └───────────────────────────────────────────────────────────────┘    │
│                                                                        │
│  ┌───────────────────────────────────────────────────────────────┐    │
│  │                    Auxiliary Service Cluster                   │    │
│  │  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐    │    │
│  │  │ Cloud     │ │ AI        │ │ Webinar   │ │ PSTN      │    │    │
│  │  │ Recording │ │ Companion │ │ CDN       │ │ Gateway   │    │    │
│  │  │ ───────── │ │ 3.0       │ │ (HLS/     │ │           │    │    │
│  │  │ Multi-    │ │ 7 product │ │  DASH)    │ │           │    │    │
│  │  │ track     │ │ lines,    │ │           │ │           │    │    │
│  │  │ split +   │ │ unified   │ │           │ │           │    │    │
│  │  │ post-     │ │ engine    │ │           │ │           │    │    │
│  │  │ synthesis │ │           │ │           │ │           │    │    │
│  │  └───────────┘ └───────────┘ └───────────┘ └───────────┘    │    │
│  └───────────────────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────────────────────┘

                         Client Layer
┌────────────────────────────────────────────────────────────────────────┐
│  Desktop (native C++)  Mobile (native)      Web (Wasm + WebRTC)        │
│  ───────────────────   ─────────────        ─────────────────────      │
│  • GPU hw codec        • iOS (Swift)        • Wasm custom codec        │
│  • Multi-monitor       • Android (KT)       • WebRTC API wrapper       │
│  • Virtual bg/avatars  • Background PiP     • TCP RTCDataChannel       │
│  • Full Recording      • Cellular opt.      (video — freezing issues)  │
└────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Key Technical Capabilities

| Capability | Details |
|------|------|
| Architecture pattern | Pure SFU (MMR — Multi-Media Router). Three-tier scheduling: GCC→ZC→MMR. No transcoding or mixing. Cascaded over a global dedicated fiber backbone |
| Video encoding | Proprietary adaptive codec (not a WebRTC standard). Multiple simultaneous streams (2-3 streams encoded and sent concurrently). H.264/H.265 on desktop. WASM proprietary codec on Web, replacing the browser's built-in codec |
| Transport protocols | Entirely proprietary (not the standard WebRTC protocol stack). UDP first → TCP fallback → TLS 443 as last resort. Screen sharing uses Reliable UDP |
| Recording | Cloud: audio/video/screen/whiteboard/chat/captions separated into independent tracks. Asynchronous GPU/CPU post-processing synthesis. Local recording is desktop-only. AI Companion generates summaries and timelines automatically |
| Platform support | Windows, macOS, Linux (native C++). iOS, Android (native). Web (Chrome/Firefox/Edge/Safari). Zoom Rooms hardware. Meeting SDK + Video SDK |
| Adaptive transport | Four-dimensional decision: network layer (bandwidth/loss/latency/jitter) + device layer (CPU/GPU/memory/battery) + application layer (layout/screen sharing) + decision engine (auto-adjust framerate → resolution → quality) |
| Audio priority | Guarantees audio continuity even at ~45% packet loss — the most competitive technical metric in the industry. When bandwidth drops sharply, video is degraded before audio. FEC + PLC co-optimized |
| Security | SRTP + DTLS + AES-256 GCM. Optional E2EE (GCM mode, based on the Insertable Streams API). SOC2/HIPAA/FedRAMP/GDPR/CCPA compliant |
| Webinar CDN | At large scale, Speakers go uplink through MMR (SFU) while all viewers go through the HLS/DASH CDN. Speakers and viewers sit on different technology stacks and do not interfere with each other |
| Hybrid deployment | Zoom Node: enterprise DC runs a local MMR coexisting with the cloud MMR. Same region goes local (low latency + data residency); cross-region falls back to cloud |

### 2.3 Technology Stack

**Core languages**: C/C++ (core MMR, codecs, transport layer). The founder confirmed "a lot of code still C++ code"
**Codecs**: Proprietary adaptive codec (not a standard VP8/VP9/H.264 implementation, optimized specifically for video conferencing)
**Transport layer**: Proprietary RTP implementation. Multi-mode switching across UDP/TCP/TLS. Multiple simultaneous streams connected and sent at once
**Signaling**: Proprietary protocol, HTTPS/WebSocket hybrid. Zone Controller assigns the optimal MMR
**Web client**: WebAssembly (proprietary codec replacing the browser's built-in codec) + WebRTC API wrapper. In some scenarios RTCDataChannel carries video
**Global network**: Dozens of data centers + a dedicated fiber backbone connecting the DCs (not public BGP). Provides a 99.9% availability SLA
**AI Companion 3.0**: A unified AI engine across 7 product lines. The technology stack is not public (speculated to mix proprietary models with third-party LLM APIs)
**Security compliance**: SRTP encryption by default + DTLS. SOC 2 Type II, HIPAA, FedRAMP Moderate/High, GDPR, CCPA. UI localization in 32 languages

### 2.4 Deep Dive into the Adaptive Transport Layer

Zoom's "it just works" reputation is built on its adaptive transport engine. This is Zoom's most fundamental technical advantage over competitors built on standard WebRTC.

**Multi-Simultaneous Streams architecture**:
- Each video publisher simultaneously encodes and sends 2-3 streams at different resolutions and framerates
- The MMR does not transcode — it selects the best existing stream for each subscriber
- Subscribers dynamically switch received streams based on their own network and device conditions
- Same philosophy as WebRTC Simulcast, but Zoom's proprietary implementation allows finer-grained control

**Four-dimensional adaptive decision engine**:
1. **Network layer**: real-time bandwidth estimation, packet loss rate, RTT, jitter
2. **Device layer**: CPU load, available memory, GPU encode/decode load, battery state
3. **Application layer**: current meeting layout (Active Speaker vs Gallery View), whether screen sharing is active, whether the window is minimized
4. **Decision engine**: synthesizes the above dimensions to execute a gradual degradation — framerate down (30→15→7.5fps) → resolution down (720p→360p→180p) → encoding quality down → close inactive views → audio down last

**Audio-First strategy**:
- This is Zoom's core technical philosophy — audio matters an order of magnitude more than video
- Guarantees audio continuity even at 45% packet loss (an industry record)
- When bandwidth drops sharply, video framerate/resolution is sacrificed before audio
- Proprietary audio codec parameter tuning (not standard Opus), FEC + PLC co-optimized
- "If you can't see clearly you can still hear; if you can't hear clearly the meeting is ruined" — a fundamental law of user experience

**Historical lesson**:
- Zoom Web SDK carries video over RTCDataChannel (SCTP over TCP) rather than the standard WebRTC UDP media channel
- This is historical debt in Zoom's architecture — the Web SDK needed to work around certain browser limitations
- Consequence: observable video freezing in moderate WiFi environments
- Zoom is gradually migrating the Web SDK to standard WebRTC UDP channels

## 3. Feature Overview

### 3.1 Core Product Lines

| Product line | Features | Key metrics |
|--------|------|---------|
| **Zoom Meetings** | Core video conferencing. Up to 1000 interactive + 10000 view-only. HD audio/video, screen sharing (with remote control + annotation), whiteboard collaboration, break-out rooms (50 groups), virtual backgrounds/filters/avatars, waiting room, live polling, AI translated captions (30+ languages), sign language translation view | Flagship product, core of platform traffic |
| **Zoom Phone** | Cloud PBX replacement. $10-20/month/seat. PSTN inbound/outbound, number porting, voicemail-to-text, call queues, IVR | Mid-teens% growth in FY2026. Phone AI feature users +35% QoQ |
| **Zoom Contact Center** | Omnichannel routing (voice/chat/email/SMS). AI-assisted agents. ZVA (Virtual Agent) — an AI voice agent that resolves customer issues 24/7 on its own | Fastest-growing business. High double-digit YoY in Q4. 10/10 of the top 10 deals included AI, 4/10 included ZVA |
| **AI Companion 3.0** | Released December 2025. Unified AI engine across 7 product lines. Real-time meeting transcription (30+ languages). Automatic summaries (key decisions + action items). My Notes (AI personalized notes, 1.5M licenses in 4 months). Smart chapter segmentation. Action item extraction and tracking. Real-time queries in the side panel | MAU up 3x+ YoY. Side panel MAU more than doubled QoQ. AI paying users +184% YoY |
| **Custom AI Companion** | No-Code visual workflow builder. Connects to enterprise SaaS such as Salesforce/ServiceNow/Jira. AI search over enterprise knowledge bases. Task-loop Agents from composition to completion | Core engine of AI monetization |
| **Zoom Webinars** | Large-scale seminars. Speakers get low-latency interaction over SFU, viewers get CDN HLS/DASH | Technology stack separated from Meetings SFU |
| **Revenue Accelerator** | AI sales acceleration. Conversational intelligence analysis. Automatic CRM updates | Customers +50%+ YoY |

### 3.2 Special Features

1. **AI Companion 3.0 work system**: meeting → summary → task → CRM update → scheduling follow-up as a fully automated closed loop. Founder Eric Yuan has had his own AI Avatar attend some meetings in his place for 3 quarters. Custom AI Companion lets enterprises define their own Agent workflows — a visual builder connects enterprise SaaS. This is the core engine of Zoom's transformation from a "video conferencing tool" into an "AI work system"
2. **MMR three-tier scheduling + dedicated fiber backbone**: GCC→ZC→MMR three-tier scheduling has been validated at hyper-scale across dozens of data centers worldwide. Dedicated fiber (not public BGP) connects the data centers — cross-region cascading adds only 10-30ms. 99.9% availability
3. **UCaaS + CCaaS dual engines + AI shared knowledge base**: one platform covers internal collaboration (Meetings/Phone/Team Chat) and external customer service (Contact Center). AI shares one knowledge base across both scenarios — AI insights from internal meetings and customer calls can flow within the same system
4. **Audio priority + 45% packet loss tolerance**: call continuity is maintained even under extremely poor network conditions. FEC + PLC co-optimization reaches industry-leading metrics. This is the lowest-level technical guarantee behind Zoom's "it just works" experience
5. **Unified AI engine across 7 product lines**: Meetings/Phone/CX/Webinar/Revenue Accelerator/Whiteboard/TeamChat share the same AI Companion engine. AI paying users +184% YoY validates that AI monetization across product lines is feasible

### 3.3 Scalability and Developer Ecosystem

- **Zoom App Marketplace**: more than 2500 third-party integrations (Slack, Salesforce, M365, Google Workspace, etc.). Sandbox review mechanism
- **Developer Platform**: REST API (meetings/users/recording/reports/Phone/CX); Webhooks (event notifications); Meeting SDK (embed Zoom into third-party apps — Windows/macOS/iOS/Android/Web); Video SDK (low-level audio/video API with a custom UI, positioned against the Agora/LiveKit PaaS model); Zoom Apps SDK (run third-party Web apps inside a meeting)
- **Zoom Node**: a local MMR deployed in the enterprise's own data center, cascaded with the cloud MMR over the backbone. Satisfies data residency compliance
- **Phone BYOC**: Bring Your Own Carrier — connects to an enterprise's existing PSTN carrier

## 4. Status & Ecosystem

### 4.1 Core Financial Metrics (FY2026 Q4, through 2026-01-31)

| Metric | Value | YoY change |
|------|------|---------|
| Q4 revenue | $1.247B | +5.3% |
| Full-year revenue | $4.868B | +4.4% (growth accelerated 130 bps) |
| Enterprise customers ($100K+/year) | 4,350+ | +3.1% |
| Online churn rate | All-time low | — |
| Enterprise churn rate | All-time low | — |
| Cash + investments | $7.8B | — |
| FY2027 projected revenue | Breaks $5B | — |

### 4.2 AI Adoption Metrics

| Metric | Data |
|------|------|
| AI Companion MAU | Up 3x+ YoY |
| Side panel MAU | More than doubled QoQ |
| AI Companion paying users | +184% YoY |
| Phone AI feature users | +35% QoQ |
| My Notes licensed users | 1.5M in 4 months |
| ZCX ARR | High double-digit YoY growth |
| Top 10 CX deals with AI | 10/10 |
| Top 10 CX deals with ZVA | 4/10 |

### 4.3 Customers and Market

- **Coverage across all industries**: finance, technology, healthcare, education, government, manufacturing, legal. 7 of the top 10 CX deals were taken from competitors (Genesys, NICE, Five9, Talkdesk)
- **Top global banks**: adopted AI Companion 3.0 in Q4
- **International revenue**: continued growth. Multi-country localized product compliance deployments

### 4.4 Known Defects and Limitations

1. **Fully closed-source, cannot be self-hosted**: no code can be audited or modified. Zoom Node permits only limited localization — the local MMR still needs to interoperate with Zoom Cloud
2. **Non-WebRTC-standard protocols**: proprietary codecs, proprietary transport protocols — completely isolated from the open-source WebRTC ecosystem. Cannot interoperate directly with standard WebRTC endpoints
3. **Web SDK video travels over TCP RTCDataChannel**: observable freezing in moderate WiFi. This is the largest technical debt in Zoom's Web SDK, but constrained by browser API limits it is hard to root-fix in the short term
4. **Limited E2EE features**: after enabling E2EE, many advanced features become unavailable (cloud recording, AI Companion, phone access). This is the inherent contradiction between E2EE and an SFU architecture — the server needs to access RTP headers to make routing decisions
5. **Cost grows linearly with scale**: large enterprises can spend millions of dollars per year on licenses. Lock-in effect — migration cost is very high after deep integration of Phone + Rooms + CX
6. **40-minute limit for small and mid-size users**: the free-tier policy of 40 minutes per meeting looks increasingly harsh in the AI era. Competitors (Google Meet) already offer 60-minute free meetings

## 5. Market Positioning

### 5.1 Main Application Industries

- **Enterprise collaboration** (coverage across all industries, SMB to Fortune 500). Zoom has redefined itself from a "meeting tool" to an "AI work system"
- **Education**: remote teaching, virtual classrooms, hybrid teaching. Deep integration with LMS (Canvas/Moodle/Blackboard)
- **Healthcare**: telemedicine, HIPAA-compliant editions. Integration with Epic/Cerner EHR systems
- **Financial services**: compliant meetings, client consultations. FedRAMP Moderate/High certified
- **Government**: FedRAMP certified. Zoom Node hybrid deployment meets data sovereignty requirements

### 5.2 Competitor Comparison

| Dimension | Zoom | Teams | Meet | Jitsi | LiveKit |
|------|------|-------|------|-------|---------|
| Type | Closed-source SaaS | Closed-source SaaS | Closed-source SaaS | Open-source full stack | Open-source + Cloud |
| Key technology | Proprietary full stack | Azure Media | WebRTC + SVC | JVB (Java) | Go + Pion |
| AI capability | Companion 3.0 (strongest) | Copilot | Gemini | None | Agents |
| Protocol standards | Proprietary (not WebRTC) | Partial WebRTC | Standard WebRTC | Standard WebRTC | Proprietary signaling + WebRTC media |
| PSTN | Phone built-in | Teams Phone | Sold separately | Jigasi (limited) | SIP (limited) |
| Contact Center | ZCX high growth | Dynamics 365 | None | None | None |
| Self-hosted deployment | No (Node limited) | No | No | Yes (100%) | Yes (100%) |
| E2EE | Yes (features limited) | No | No | Yes (full) | Supported (no key management) |
| Annual revenue | $4.87B | Included in M365 | Included in Workspace | Free/JaaS | Cloud $ |
| Web performance | Three stars | Three stars | Four stars | Four stars | Four stars |
| Open-source self-hosting cost | N/A | N/A | N/A | Free | Free |

### 5.3 Pricing

- **Basic**: free (40 minutes per meeting, 100-person cap)
- **Pro**: $14.99/user/month (recommended for 1-9 users)
- **Business**: $21.99/user/month (minimum 10 users, includes SSO, recording transcription, branding)
- **Enterprise**: custom quote (minimum 50 users, includes unlimited cloud storage, dedicated CSM, FedRAMP)
- **Phone**: $10-20/user/month (tiered by region and features)
- **Contact Center**: custom per agent (entry/advanced/expert tiers)
- **AI Companion**: free users get basic AI features. Custom AI Companion sold separately

## 6. Product Highlights

1. **The most aggressive AI work system transformation**: Zoom is not satisfied with a surface-level integration of "meetings + AI summaries". AI Companion 3.0 turns meetings into actionable workflows (meeting → summary → task → CRM update → scheduling follow-up as a fully automated closed loop). Founder Eric Yuan has had his own AI Avatar attend some meetings in his place for 3 quarters. Custom AI Companion lets enterprises build Agent workflows in No-Code. This strategy of "eating your own dog food" and "making AI a platform rather than a feature" is unprecedented in the video conferencing industry
2. **MMR three-tier scheduling + dedicated fiber backbone**: GCC→ZC→MMR three-tier scheduling is the result of hyper-scale validation across dozens of data centers worldwide. Dedicated fiber (not public BGP) connects the data centers — cross-region cascading adds only 10-30ms of latency. This is Zoom's core infrastructure moat, and no self-built solution can directly replicate it
3. **Audio priority + 45% packet loss tolerance**: the only large-scale commercial solution that maintains call continuity under extremely poor network conditions. FEC + PLC co-optimization is the accumulation of many years of proprietary codec and transport protocol work. This is the lowest-level technical guarantee behind Zoom's "it just works" reputation
4. **UCaaS + CCaaS dual-engine synergy**: one platform covers internal collaboration and external customer service. AI shares a knowledge base across both scenarios — AI insights from internal meetings and customer calls can flow within the same system. ZCX's high double-digit growth proves the platform's synergy
5. **From video conferencing pioneer to AI work system pioneer**: Zoom's brand perception shift is one of the most decisive strategic pivots in the tech industry. A $4.87B annual revenue commercial scale validates that the ceiling of the video conferencing market is far higher than imagined — provided you can keep expanding the product boundary

## 7. Reference Value for MediaServo

### [Adopt] Directly Adoptable

1. **GCC→ZC→SFU Node three-tier scheduling model**: MediaServo's Conference Controller should implement similar hierarchical routing — `GlobalRouteManager` (region selection) → `ZoneRouter` (node selection) → `SfuWorkerNode` (media forwarding). Each tier scales independently with clear responsibilities
2. **Adaptive transport strategy**: UDP first → TCP fallback → TLS 443 as last resort. MediaServo's WebRTC Transport should implement this degradation chain to guarantee connectivity through enterprise firewalls
3. **Audio-priority QoS strategy**: under bandwidth contention, drop framerate → resolution → audio only last. MediaServo's PipelineEngine QoS controller should have this priority ordering built in
4. **Cloud recording two-layer architecture**: in-meeting real-time track separation with independent storage (one track each for audio/video/screen), followed by asynchronous GPU/CPU batch synthesis after the meeting. MediaServo's recording plugin should adopt the same architecture
5. **Webinar CDN separation**: at large scale, Speakers go through SFU (low latency) and viewers go through the HLS/DASH CDN. MediaServo should design this traffic-splitting capability from the architecture stage
6. **Zoom Node hybrid deployment**: local SFU + cloud SFU coexistence. MediaServo's multi-form deployment (Embed/Sidecar/Standalone/AUDEBase) should natively support a Hybrid mode — same region goes local, cross-region falls back to cloud

### [Adapt] Needs Modification Before Adoption

1. **Proprietary transport protocol → standard WebRTC**: Zoom's proprietary protocol cannot be borrowed as code, but the design philosophy is worth keeping. MediaServo should use the standard WebRTC protocol stack (str0m or webrtc-rs) to guarantee interoperability with browsers and standard WebRTC endpoints
2. **Closed-source AI Companion → open AI pipeline**: Zoom AI Companion's core architecture is not visible. MediaServo's PipelineEngine should provide a `trait MediaProcessor` as a pluggable AI node — connecting open LLM/ASR/TTS services without locking in any single vendor
3. **Cloud Controller → self-hosted orchestration**: GCC/ZC is Zoom proprietary technology. MediaServo should use etcd/NATS for service discovery + a simplified Zone Router. Open orchestration > closed black box
4. **Reserve a Contact Center path**: ZCX's success validates the strategic value of a unified platform. MediaServo's meeting room architecture should reserve an extension point for a `mediaservo-contact` crate — even if Phase 0 does not implement it
5. **Custom AI Companion → PipelineEngine Node Graph**: the concept of Zoom's No-Code workflow builder is worth borrowing. MediaServo's PipelineEngine should support DAG (directed acyclic graph) definitions of media processing chains — Audio→ASR→LLM→TTS→Audio is a 5-node pipeline

### [Avoid] Known Pitfalls / Non-Applicable

1. **Web SDK RTCDataChannel carrying video = freezing hell**: the biggest technical debt in Zoom's Web SDK. MediaServo Web clients must use the standard WebRTC UDP SRTP media channel — never carry video over RTCDataChannel
2. **Proprietary non-standard protocols = ecosystem isolation**: Zoom's moat is also its shackle — it cannot interoperate with standard WebRTC endpoints. MediaServo must be based on standard protocols (SDP/JSEP/ICE/DTLS/SRTP) to guarantee interoperability
3. **Closed-source and unauditable = a security blind box**: Zoom's security incident history (Zoombombing, E2EE false advertising) proves the limits of closed-source security transparency. MediaServo is open source + auditable
4. **AI features over-bound to a commercial ecosystem**: Zoom's AI Companion is entirely part of its commercial SaaS. MediaServo's AI pipeline stays Provider-agnostic — the LLM can be OpenAI/Claude/self-hosted, ASR can be Deepgram/Whisper/proprietary
5. **Enterprise license cost model does not apply to self-hosted scenarios**: the essence of Zoom's business model is "locking customers in with licenses". MediaServo's core commercial value is the opposite — offering a self-hosted solution with no license fees

**Overall rating**: ★★★★★ (5/5)

> **Comment**: Zoom is the pinnacle of commercial video conferencing. MMR three-tier scheduling, audio-priority QoS, Webinar CDN traffic splitting, hybrid cloud deployment, and AI Companion integration across product lines — every design decision is a direct source of inspiration for the MediaServo project. Although closed-source makes its code unavailable, its architectural design philosophy and business strategy evolution are universal best practices for any large-scale real-time communication system. MediaServo should digest Zoom's design wisdom item by item, while taking the open-standards route in implementation: WebRTC instead of a proprietary protocol, open-source Rust instead of closed-source C++, a pluggable AI pipeline instead of a closed AI Companion, and no license fees instead of per-seat subscriptions.

---

> **References**
> Zoom FY2026 10-K Annual Report (SEC Filing, Feb 2026)
> Zoom Q4 FY2026 Earnings Call Transcript (Feb 2026)
> Zoom Q1 FY2027 Earnings Press Release (May 2026)
> Zoom Investor Relations (investors.zoom.us)
> Zoom Developer Platform (developers.zoom.us)
> Zoom AI Companion 3.0 Press Release (Dec 2025)
> Zoom Node Architecture: Zoom Blog, "Hybrid Deployment with Zoom Node" (2023)
> Zoom Web SDK Architecture: WebRTC Hacks, "How Zoom's Web Client Works" (2020)
> MediaServo: docs/research/video-conference.md

---

**Related decisions**: D-QOS-AUDIO, D50

## Appendix A: Zoom Web SDK Architecture and MediaServo Lessons

The technical architecture of Zoom's Web SDK is the best textbook for "what not to do" in MediaServo's Web client design:

**Zoom Web SDK media path**:
```
Browser                             Zoom Server
┌────────────┐                      ┌────────────┐
│ getDisplay │                      │            │
│ Media()    │ ──MediaStream──►     │            │
│            │                      │            │
│ Peer-      │ ──SCTP/RTCDataChannel─► │ Signal     │
│ Connection │    (TCP, port 443)   │ Server     │
│            │    ▲ video takes     │            │
│            │    │ this path!      │            │
│            │    │                 │            │
│ Audio      │ ──WebAudio API──►   │            │
│ handled    │    (UDP? unsure)     │            │
│ separately │                      │            │
└────────────┘                      └────────────┘
```

**Problem analysis**:
1. Video data travels over WebRTC RTCDataChannel (SCTP over DTLS over UDP... but it actually falls back to TCP)
2. SCTP was not designed for real-time video transport — it was designed for reliable, ordered data transfer (like TCP)
3. In WiFi packet-loss scenarios: lost SCTP packets are retransmitted → subsequent packets are blocked → video freezes → a disaster for user experience
4. Zoom did this because the early WebRTC standard did not support the combination of screen sharing + standard video encoding on some browsers

**The correct path for MediaServo**:
1. **Never carry real-time video over RTCDataChannel**. Use RTCDataChannel for signaling, control messages, and teleoperation commands
2. **Video always travels over the standard WebRTC UDP SRTP media channel**. Rely on FEC/PLC/NACK on packet loss rather than TCP retransmission
3. **Use standard WebRTC codecs (VP8/VP9/H.264/AV1)** — hardware encode/decode support is built into browsers, no WASM needed
4. **Consider the WebCodecs API** (Chrome 94+) — lower-level than WebRTC, allowing fine-grained control of encoding parameters
5. **Use a WASM codec only in special scenarios** (e.g. needing AV1 in older browsers that do not support it)

This is the most important lesson learned from Zoom's Web SDK technical debt — take the standard path; even a closed-source commercial giant falls over the non-standard one.

---

## Appendix B: Mapping Zoom's MMR Three-Tier Scheduling into MediaServo

Zoom's GCC→ZC→MMR three-tier scheduling can be mapped directly onto MediaServo's Conference Controller:

```rust
// MediaServo Conference Controller architecture

// Layer 1: Global Route Manager (corresponds to Zoom GCC)
struct GlobalRouteManager {
    zones: HashMap<RegionId, ZoneState>,
    zone_selector: Box<dyn ZoneSelector>,  // geolocation / latency-based
}

impl GlobalRouteManager {
    fn route_meeting(&self, user_location: GeoLocation) -> ZoneId {
        self.zone_selector.select_zone(user_location, &self.zones)
    }
}

// Layer 2: Zone Router (corresponds to Zoom ZC)
struct ZoneRouter {
    region: RegionId,
    sfu_nodes: Vec<SfuNodeHandle>,  // all SFU nodes in the same region
    node_selector: Box<dyn NodeSelector>,  // load balancing strategy
    health_checker: HealthChecker,
}

impl ZoneRouter {
    fn assign_sfu(&self, meeting: &Meeting) -> SfuNodeHandle {
        let healthy = self.health_checker.filter_healthy(&self.sfu_nodes);
        self.node_selector.select(meeting, &healthy)  // round-robin / least-connections
    }
}

// Layer 3: SFU Worker Node (corresponds to Zoom MMR)
struct SfuWorkerNode {
    node_id: NodeId,
    workers: Vec<WorkerHandle>,  // one Worker per CPU core (see mediasoup)
    router_trait: Box<dyn Router>,  // Local or Redis (see LiveKit)
    qos_controller: QosController,  // audio priority (see Zoom)
}

impl SfuWorkerNode {
    fn create_room(&self, config: RoomConfig) -> Room {
        let worker = self.select_worker();        // least loaded Worker
        let router = worker.create_router();       // mediasoup style
        let room = Room::new(router, config);
        self.qos_controller.attach(&room);         // audio priority + adaptive bitrate
        room
    }

    fn select_worker(&self) -> WorkerHandle {
        self.workers.iter().min_by_key(|w| w.load()).cloned()
    }
}
```

**Design principles**:
- Single responsibility per tier — Global Route Manager handles routing, Zone Router handles assignment, Worker handles media
- Each tier scales independently — adding Workers does not require changing the Zone Router
- Trait-based interfaces — the selection strategy of each tier is pluggably replaceable (see Jitsi BridgeSelectionStrategy)
- Rust traits rather than Java interfaces (consistent with MediaServo's technology stack)

---

## Appendix C: Strategic Transformation Lessons from Zoom

Zoom's transformation from a "video conferencing company" into an "AI work system company" carries important implications for MediaServo's strategic positioning:

**Key data points**:
- AI Companion paying users +184% YoY — there is genuine willingness to pay for AI features
- AI Companion MAU up 3x+ YoY — there is genuine user demand for AI features (not hype)
- 10/10 of the top 10 CX deals include AI — enterprise customers treat AI as a must-have for procurement
- Custom AI Companion lets enterprises define their own Agent workflows — platform over feature

**Strategic recommendations for MediaServo**:
1. **Reserve AI interfaces in Phase 0** — even without implementing concrete AI features. PipelineEngine's `trait MediaProcessor` should support:
   - ASR Node (audio → text)
   - LLM Node (text → text)
   - TTS Node (text → audio)
   - Video Analysis Node (video frames → metadata)
2. **Do not lock in any single AI provider** — Zoom is bound to its own AI Companion. MediaServo should stay Provider-agnostic
3. **AI as a platform capability rather than a specific feature** — AI is a node type within the pipeline, not a separate "AI feature" switch
4. **Recording + AI summary = a post-meeting value loop** — this is the core value of Zoom AI Companion 3.0
   - During the meeting: real-time transcription + action item extraction
   - After the meeting: AI summary + timeline annotations + automatically generated to-dos
5. **An evolution path from narrow scenarios to a broad platform**: Phase 0 video conferencing → Phase 1 AI transcription → Phase 2 Agent → Phase 3 work system

---

## Appendix D: The Technical Trade-offs of Zoom's End-to-End Encryption (E2EE)

After the 2020 security crisis (Zoombombing + E2EE false advertising), Zoom released real E2EE in 2021:

**How E2EE is implemented**:
- Based on the WebRTC Insertable Streams API (a W3C standard proposal)
- GCM (Galois/Counter Mode) encryption. Keys are generated and stored only on client devices
- Zoom Server only forwards encrypted RTP packets — it cannot decrypt media content

**The technical cost of E2EE** (why many features become unavailable once E2EE is on):
1. **Cloud recording unavailable**: the server cannot decrypt content, so it cannot record. E2EE meetings can only be recorded locally
2. **AI Companion unavailable**: real-time transcription and AI summaries both require server-side access to decrypted audio
3. **PSTN phone access unavailable**: the phone gateway needs to decrypt audio for mixing and codec conversion
4. **Livestream pushing unavailable**: pushing requires access to unencrypted media content for RTMP/HLS encoding
5. **Break-out rooms limited**: creating sub-rooms requires server-coordinated signaling, which is constrained under E2EE

**This is the universal E2EE contradiction under any SFU architecture**:
- The SFU needs to read the RTP header (SSRC, SN, timestamp, PT) for selective forwarding and Simulcast layer selection
- E2EE requires that the server not access plaintext payloads
- Solution: encrypt only the RTP payload and leave the header in plaintext. This is the common practice of both Zoom and Jitsi
- This is called "E2EE-Lite" or "Selective Header Encryption"

**Recommended E2EE strategy for MediaServo**:
1. Default mode (SRTP over DTLS): the server can decrypt content, all features available
2. Optional E2EE mode: Insertable Streams API encrypts the payload. Enabling it disables cloud recording, AI analysis, and PSTN bridging
3. Recording uses local recording + encrypted post-meeting upload as a substitute for cloud recording
4. AI analysis happens on the client (browser-side Whisper WASM) rather than on the server
5. Provide user-facing prompts when switching between the two modes — so users understand the trade-offs

---

## Appendix E: History and Lessons from Zoom's Security Incidents

Zoom went through a series of security crises in 2020; these lessons are of reference value for all video conferencing products (including MediaServo):

**Timeline of Zoom's 2020 security crisis**:
1. **Zoombombing**: uninvited users guessed meeting IDs (9-11 digits) to join meetings and harass participants. Cause: the default configuration allowed anyone to join, and meeting IDs could be brute-force enumerated
2. **E2EE false advertising**: Zoom claimed to offer end-to-end encryption, but in reality it only encrypted between the client and Zoom servers. The servers could decrypt everything. The FTC opened an investigation
3. **macOS installer used an auto-install script**: it exploited Apple's auto-install script mechanism to bypass user confirmation during installation. Triggered privacy controversy
4. **Data routing to China**: some non-Chinese users' data was routed to Chinese servers, triggering data sovereignty controversy
5. **Windows client vulnerability**: UNC path injection allowed a remote attacker to steal Windows credentials

**Zoom's response measures (2020-2021)**:
- 90-day feature freeze — all engineering resources redirected to security and privacy fixes
- Hired former Facebook CSO Alex Stamos as a security advisor
- Acquired Keybase (the E2EE technology team) and recruited cryptographers
- Enabled security features such as waiting rooms, meeting passwords, and host approval for admission by default
- Released real E2EE (2021)
- Published transparency reports and security white papers

**Security lessons for MediaServo**:
1. **Security cannot be a post-hoc patch** — design for security in Phase 0. AuthProvider trait, Bearer Token, Permission Matrix, Rate Limiter — built in from the architecture stage
2. **Secure by default > open by default** — waiting rooms, passwords, and approval for admission on by default. Users actively choose to lower the security level
3. **Honest marketing** — "peer-to-peer encryption" and "end-to-end encryption" have strict technical definitions. Do not slap the E2EE label onto SRTP
4. **Data routing transparency** — if MediaServo has cross-region nodes, let users know explicitly which regions data passes through. Provide data residency options
5. **Dependency management security** — Zoom's Windows UNC vulnerability exposed the risk of third-party library dependencies. MediaServo's `cargo audit` / `cargo deny` are mandatory CI/CD steps


---

## Appendix F: Overall Rating Matrix for the Four Reference Products

| Dimension | mediasoup | LiveKit | Jitsi Meet | Zoom |
|------|-----------|---------|------------|------|
| SFU performance | ★★★★★ | ★★★★☆ | ★★★☆☆ | ★★★★★ (estimated) |
| Signaling flexibility | ★★★★★ | ★★☆☆☆ | ★★★☆☆ | ★☆☆☆☆ |
| Deployment simplicity | ★☆☆☆☆ | ★★★★★ | ★★★☆☆ | N/A (SaaS) |
| Documentation quality | ★★★☆☆ | ★★★★★ | ★★★★☆ | ★★★★☆ |
| AI capability | ★☆☆☆☆ | ★★★★★ | ★☆☆☆☆ | ★★★★★ |
| Standards compatibility | ★★★★☆ | ★★★☆☆ | ★★★★★ | ★☆☆☆☆ |
| Community size | ★★★☆☆ | ★★★★☆ | ★★★★★ | N/A (closed-source) |
| Self-hosting feasibility | ★★★★★ | ★★★★★ | ★★★★★ | ☆☆☆☆☆ |
| Rust ecosystem | ★★★★★ | ★★★★☆ | ☆☆☆☆☆ | ☆☆☆☆☆ |
| Production validation | ★★★★☆ | ★★★☆☆ | ★★★★★ | ★★★★★ |
| Reference value for MediaServo | ★★★★★ | ★★★★★ | ★★★★☆ | ★★★★☆ |

Recommended strategy for MediaServo:
- **SFU engine**: borrow from mediasoup (performance model) + LiveKit (architectural abstractions)
- **Signaling protocol**: borrow from Colibri2 (RESTful resource modeling) + a proprietary WebSocket
- **Distributed architecture**: borrow from Zoom MMR three-tier scheduling + Jitsi Pools star topology
- **Recording scheme**: borrow from Zoom's two-layer recording + mediasoup PlainTransport export
- **AI integration**: borrow from the LiveKit Agent framework + Zoom AI Companion's full-line integration
- **Security compliance**: borrow from Jitsi's fully open and auditable source + Zoom's security lessons
- **Deployment experience**: borrow from LiveKit's single binary + Jitsi's Docker Compose

Take the best design idea from every reference project and build MediaServo's own optimal architecture.

### 7.9 [Adapt] Deep reference: Rust implementation of adaptive transport

Zoom's adaptive transport decision logic can be implemented in Rust within PipelineEngine:
- BandwidthMonitor component: monitors in real time each Consumer's received bandwidth, packet loss rate, latency, and jitter
- DecisionEngine component: makes unified decisions based on multi-dimensional input (drop layer / drop framerate / drop resolution / stop video)
- AudioProtector component: marks audio packets as highest priority so they are never dropped during bandwidth contention
- Adaptive decisions use configurable strategies (aggressive/conservative/auto) to suit different scenarios (meetings/remote desktop/vehicle streaming)

### 7.10 [Adopt] Deep reference: Rust implementation of Zoom Node hybrid deployment

Design points of MediaServo's Hybrid deployment mode:
- The local SFU and the cloud SFU use the same code (a single Rust binary), with configuration deciding the role
- The local SFU automatically discovers the nearest cloud SFU node (via DNS SRV or a Redis registry)
- Connection migration mechanism: when a participant switches from the local network to the internet, the ICE connection migrates automatically to the cloud SFU
- Data synchronization: only meeting metadata (participant list, track info) is synced between local and cloud; media content is not transferred

### 7.11 [Avoid] Deep reference: engineering lessons of closed-source commercial products

Lessons of Zoom's closed-source strategy for MediaServo:
- Interoperability: the lesson of Zoom's non-standard protocols — MediaServo must maintain full compatibility with standard WebRTC
- Vendor lock-in: the migration predicament of Zoom's customers — MediaServo uses open protocols to keep users in control
- Unauditable: the security risk of closed-source code — MediaServo's open-source strategy lets users audit and harden security

### 7.12 [Adopt] Deep reference: the meeting integration model of AI Companion

The integration model of Zoom AI Companion provides direction for MediaServo's AI capability design:
- Real-time captions: real-time speech-to-text based on ASR, displayed in the meeting UI
- Smart summaries: an automatically generated meeting summary sent to all participants after the meeting
- Action item extraction: automatically identifying to-dos in a meeting based on an LLM
- Real-time translation: real-time cross-language speech translation in a meeting
- MediaServo should reserve an AI node interface in PipelineEngine, define the API in Phase 0, and implement it in later versions
