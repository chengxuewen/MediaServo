// SFU Consumer Client — mediasoup-client core (P1/D270-R2 core swap)
// Flow: get_router_rtp_capabilities → Device.load → create_web_rtc_transport(recv)
//       → device.createRecvTransport(opts) → 'connect'→connect_web_rtc_transport(dtls)
//       → new_producer → consume(rtp_capabilities=device.rtpCapabilities) → consumed
//       → transport.consume → track。
// The resilience layer (W1-W5/F2/H1/H3/V2) stays structurally isomorphic to the old implementation — hand-rolled SDP/ORTC logic (hand-built offers / hardcoded PT /
// videoRtpCapabilities, the PIT-173/PIT-55/PIT-56 debt cluster) is now handled by the official mediasoup-client implementation (C18).
// P2P fallback removed: the all-SFU decision (2026-08-25 — server rooms are uniformly DeviceStream, SDP/ICE relay dropped)
// means the old P2P path never worked anyway — sdp/ice broadcasts are silently ignored (noise); negotiation failures go through the round engine.

import { Device } from 'mediasoup-client';
import type { Consumer, Transport as MsTransport } from 'mediasoup-client/types';
import type { DtlsParameters as MsDtlsParameters, RtpParameters } from 'mediasoup-client/types';

// ── server wire (snake_case SignalingMessage) ↔ mediasoup-client (camel) mapping pure functions ──
// Single convergence point (design §3): vitest replays the P1.1 message wire shapes directly.

interface WireIceParams { username_fragment: string; password: string }
interface WireDtlsParams { fingerprints: { algorithm: string; value: string }[]; role: string }
// The server IceCandidate struct uses rename_all=camelCase (candidateType); handwritten candidate_type kept for compatibility.
interface WireIceCandidate {
  ip: string; port: number; protocol: string; foundation: string; priority: number;
  candidateType?: string; candidate_type?: string;
}
interface TransportCreated {
  transport_id: string;
  ice_parameters: WireIceParams;
  dtls_parameters: WireDtlsParams;
  ice_candidates?: WireIceCandidate[];
}

export function mapIceParameters(p: WireIceParams): { usernameFragment: string; password: string; iceLite: true } {
  // iceLite always true = a design invariant of mediasoup server transports (worker-side ICE-Lite); the server wire's
  // IceParameters does not carry the flag; the handler emits a=ice-lite in the remote SDP based on iceParameters.iceLite —
  // if missing, the browser treats the peer as full-ICE and STUN checks get misdirected/mistimed (located via A/B packet capture: req 95/resp 0).
  return { usernameFragment: p.username_fragment, password: p.password, iceLite: true };
}

export function mapDtlsParameters(p: WireDtlsParams): MsDtlsParameters {
  return {
    role: p.role as MsDtlsParameters['role'],
    fingerprints: p.fingerprints.map((f) => ({ algorithm: f.algorithm, value: f.value })),
  } as MsDtlsParameters;
}

export function mapIceCandidates(list: WireIceCandidate[]): Array<Record<string, unknown>> {
  return list.map((c) => ({
    foundation: c.foundation,
    priority: c.priority,
    ip: c.ip,
    port: c.port,
    protocol: c.protocol,
    type: c.candidateType ?? c.candidate_type ?? 'host',
  }));
}

type StreamCallback = (stream: MediaStream) => void;
type StatusCallback = (status: 'connecting' | 'connected' | 'playing' | 'stalled' | 'disconnected' | 'error') => void;
type MetricsCallback = (metrics: StreamMetrics) => void;
/// F2: consecutive zero-growth ticks (2s/tick) before declaring 'source offline' — ~6s, conservative for the slowest-fps stream and never earlier than any signaling self-heal.
const STALL_TICKS = 3;
/// W2: first-frame round watchdog — 30s without a first frame counts as a failed round; restartStream gets at most 3 rounds, then 'source offline' waiting.
const PLAY_WATCHDOG_MS = 30000;
const PLAY_ROUNDS_MAX = 3;
/// PIT-76 observability: logT is silent by default (T1.2 debug-output cleanup); re-enable with the localStorage switch `mediaservo_sfu_debug=1`.
const SFU_DEBUG = typeof localStorage !== 'undefined' && localStorage.getItem('mediaservo_sfu_debug') === '1';

/// W4: error-code routing — auth/authorization family = terminal (the only legal source of the red banner); 4031 (permissions hot-editable, C33),
/// 5000 (SFU internal failure, typical during server restarts) and 5001 (gateway upstream switch, host-emitted via forwarding) = retryable.
/// P1/T1.2 intentional change (design §1): 4012 (control DC denial, F8 role-gate explicit rejection) joins the terminal family.
/// accountless-client-auth T0: 4013 (JWT presented but signature failed = auth-family terminal; red banner prompts re-login).
export function classifySfuError(code: number): 'terminal' | 'retry' {
  switch (code) {
    case 4000: case 4001: case 4002: case 4003: case 4010: case 4011: case 4012: case 4013: case 4101:
      return 'terminal';
    default:
      return 'retry';
  }
}
/// W1: exponential backoff sequence 1s→cap (default 30s) pure function; jitter is applied by the caller (assertable).
export function nextBackoff(attempt: number, cap = 30000): number {
  const base = Math.max(1, Math.floor(attempt));
  return Math.min(1000 * 2 ** (base - 1), cap);
}

export interface StreamMetrics {
  rtt: number;          // ms
  packetLoss: number;   // percentage
  fps: number;
  bitrate: number;      // kbps
  jitter: number;       // ms
  resolution: string;
  // v2 (web-stream-stats T4): codec diagnostics — merged from browser side + Host EncoderStatus reports
  decoderImplementation?: string;   // browser decoder (inbound-rtp; real Chrome has it, headless lacks it)
  decoderCodec?: string;            // fallback: getStats codec report mimeType (the format the browser actually decodes)
  codec?: string;                   // encode format (Host EncoderStatus, e.g. video/H264)
  encoderBackend?: string;          // Host backend requested value (auto/software/hardware...)
  encoderImplementation?: string;   // Host encoder actually in use (get_stats, e.g. OpenH264/libvpx)
  hostFps?: number;                 // Host outbound fps
  hostResolution?: string;          // Host encode resolution
  avgEncodeMs?: number;             // v3 (encode-time-stats T4): Host average encode time per frame (ms/frame)
}

export class SfuConsumerClient {
  private ws: WebSocket | null = null;
  private closed = false;  // PIT-50: no reconnect after close() (StrictMode double-mount race)
  // P1 core swap: the media plane belongs to mediasoup-client (Transport provides getStats/connectionstatechange).
  private device: Device | null = null;
  private transport: MsTransport | null = null;
  private consumer: Consumer | null = null;
  // v2 (web-stream-stats fix): dual-source merged accumulator — getStats and encoder_status alternated
  // overwrote each other and made the panel flicker (values one second, "-" the next); merged uniformly before the callback
  private mergedMetrics: StreamMetrics | null = null;
  // bitrate delta computation: treating cumulative bytesReceived as an instantaneous value was the unit-error root cause
  private lastBytes = 0;
  private mutedTicks = 0; // H1: consecutive muted tick counter (2s tick)
  // F2: media freshness watchdog (second pillar of the double guarantee — no fake LIVE even if signaling is fully lost)
  private playingSeen = false;
  /** S0: dialect version negotiated with the server (room_joined.protocol; default = 1 for old servers). */
  private negotiatedProtocol = 1;

  /** Public read surface of the S0 negotiation result (consumed by control-DC capability bits / the S0.5 resume gate). */
  get negotiated(): number {
    return this.negotiatedProtocol;
  }
  private stallTicks = 0;
  private stalled = false;
  // W1/W2: resilience state-machine fields
  private playTimer: ReturnType<typeof setTimeout> | null = null;
  private playRounds = 0;
  private waitingForProducer = false;
  private reconnecting = false;
  private lastTs = 0;
  private onTrack: StreamCallback;
  private onStatus: StatusCallback;
  private onMetrics: MetricsCallback;
  private transportId: string | null = null;
  /** SFU mode flag: set as soon as startPlay sends create_web_rtc_transport —
      legacy P2P negotiation broadcasts in the room (sdp/rtc_ice_candidate) are always silently ignored. */
  private sfuMode = false;
  // PIT-65: unique SFU peer_id per connection — multiple pages sharing a peer_id make SfuManager recv_transports overwrite each other
  private sfuPeerId: string;
  private transportResolver: ((params: TransportCreated) => void) | null = null;
  // P1 core-swap RPC single-shot side channel (consumed first by handleMessage; one in-flight request at a time — negotiation is inherently serial):
  private capsResolver: ((caps: unknown) => void) | null = null;
  private consumeResolver: ((msg: { consumer_id: string; producer_id: string; kind: string; rtp_parameters: unknown }) => void) | null = null;
  /** H1: producer_closed self-heal re-entry guard (broadcast storms/consecutive messages restart only once). */
  private restarting = false;
  private metricsTimer: ReturnType<typeof setInterval> | null = null;
  // PIT-76: first-frame/render timestamp observability — timed from startPlay (SFU_DEBUG switch, silent by default)
  private t0 = 0;
  private logT(msg: string): void {
    if (!SFU_DEBUG) return;
    const t = performance.now();
    console.info(`[T+${Math.round(t - this.t0)}ms] ${msg}`);
  }

  /** producer-label (§12-1 device room + stream label): only consume producers with label===expectedLabel.
   *  Unset = legacy behavior (derived-room mode: consume every producer in the room), fully compatible. */
  private expectedLabel: string | undefined;
  private pendingProducer: { producer_id: string; kind: string } | null = null;

  constructor(
    private serverUrl: string,
    private roomId: string,
    private token: string,
    callbacks: {
      onTrack: StreamCallback;
      onStatus: StatusCallback;
      onMetrics: MetricsCallback;
    },
    options?: { expectedLabel?: string },
  ) {
    this.onTrack = callbacks.onTrack;
    this.onStatus = callbacks.onStatus;
    this.onMetrics = callbacks.onMetrics;
    this.expectedLabel = options?.expectedLabel;
    // PIT-65: unique SFU peer_id per connection (pages sharing a peer_id → SfuManager transports overwrite each other)
    this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`;
  }

  async connect(): Promise<void> {
    this.closed = false;  // PIT-50: reset the closed flag on every connect
    this.onStatus('connecting');

    // W1: detach the old socket and its onclose-reconnect semantics before connecting — prevents the old socket from re-triggering reconnect oscillation after a successful reconnect.
    if (this.ws) {
      try {
        this.ws.onclose = null;
        this.ws.onerror = null;
        this.ws.onmessage = null;
        this.ws.close();
      } catch { /* noop */ }
      this.ws = null;
    }

    const protocol = this.serverUrl.startsWith('wss:') ? 'wss:' : 'ws:';
    const host = this.serverUrl.replace(/^wss?:\/\//, '');
    const wsUrl = `${protocol}//${host}/ws`;

    // Auth: JWT via the sec-websocket-protocol subprotocol (RFC 6455 tokens cannot contain spaces — no "Bearer " prefix allowed)
    // PIT-49: browser subprotocol = raw JWT; the server parser accepts a "Bearer " prefix for compatibility
    // fail-closed (accountless-client-auth T4; PIT-171-family hardcoded-value cleanup): no token = no connection;
    // the old 'mediaservo-dev' shared-PSK fallback is retired — browser-side auth = login credentials are the only path; the server's
    // T0 semantics (PSK reserved for host/link clients that truly hold a key) are aligned.
    if (!this.token) {
      throw new Error('sfu-terminal:no-token');
    }
    this.ws = new WebSocket(wsUrl, [this.token]);

    const authPromise = new Promise<void>((resolve, reject) => {
      this.ws!.onopen = () => {
        // JWT goes via the sec-websocket-protocol subprotocol; no PSK frame to send
      };
      this.ws!.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.code === 0 || msg.type === 'error' && msg.code === 0) {
            this.onStatus('connected');
            resolve();
          } else if (typeof msg.code === 'number' && msg.code !== 0) {
            // W4: handshake-phase errors — auth/authorization family = terminal (retrying is pointless); 5000/5001 family = keep waiting for ack (Auth timeout 10s backstops the reconnect)
            if (classifySfuError(msg.code) === 'terminal') {
              reject(new Error(`sfu-terminal:${msg.code}`));
            }
          }
        } catch (err) {
          console.warn('SfuClient: auth message parse failed', err);
          // Non-JSON message, skip
        }
      };
      this.ws!.onerror = () => reject(new Error('WS error'));
      setTimeout(() => reject(new Error('Auth timeout')), 10000);
    });
    await authPromise;

    // Set up signaling message handler
    this.ws.onmessage = (event) => {
      this.handleMessage(event.data);
    };

    // Join room
    this.ws.send(JSON.stringify({
      type: 'room_join',
      room_id: this.roomId,
      peer_role: 'consumer',
      protocol: 3, // S0.5: declare this endpoint's dialect ceiling (v3 = resume domain; server negotiates the min)
    }));

    // Reconnect on WS close
    this.ws.onclose = () => {
      if (this.closed) return;  // PIT-50: no reconnect after close()
      this.onStatus('disconnected');
      this.stopMetrics();
      this.reconnect();
    };
  }

  async startPlay(): Promise<void> {
    this.sfuMode = true; // entering the SFU flow: all later-arriving sdp/ice broadcasts are ignored
    if (!this.ws) throw new Error('Not connected');

    this.t0 = performance.now();
    this.logT('startPlay: Device.load(routerRtpCapabilities)');

    // Core-swap step one: pull router caps from the P1.1 server surface → Device.load (official negotiation order, C18;
    // the old hardcoded-PT videoRtpCapabilities surface dies here). Failures share the negotiation-failure path → round engine.
    this.device = new Device();
    try {
      const caps = await this.rpcRouterRtpCapabilities();
      await this.device.load({ routerRtpCapabilities: caps as never });
    } catch (err) {
      this.device = null;
      const m = String((err as Error)?.message ?? err);
      if (m.startsWith('sfu-terminal:')) { this.device = null; throw err; }
      this.logT(`Device.load failed → advance round: ${m}`);
      void this.playDeadlineHit();
      return;
    }

    // recv transport: server create → option mapping → device.createRecvTransport.
    // resolver armed before sending (lesson from an old race); no response in 3s = round failure (the P2P fallback was removed with the all-SFU reality).
    const sfuPromise = new Promise<TransportCreated | null>((r) => { this.transportResolver = r; });
    this.logT('sending create_web_rtc_transport');
    this.ws.send(JSON.stringify({ type: 'create_web_rtc_transport', room_id: this.roomId, peer_id: this.sfuPeerId, direction: 'recv' }));
    this.armPlayWatchdog(); // W2: first-frame round deadline
    const sfuResult = await Promise.race([
      sfuPromise,
      new Promise<null>((r) => setTimeout(() => r(null), 3000)),
    ]);
    if (!sfuResult) {
      this.transportResolver = null;
      this.logT('SFU create got no response → advance round');
      void this.playDeadlineHit();
      return;
    }

    this.logT('received web_rtc_transport_created, id: ' + sfuResult.transport_id);
    this.transportId = sfuResult.transport_id;
    const transport = this.device.createRecvTransport({
      id: sfuResult.transport_id,
      iceParameters: mapIceParameters(sfuResult.ice_parameters),
      iceCandidates: mapIceCandidates(sfuResult.ice_candidates ?? []) as never,
      dtlsParameters: mapDtlsParameters(sfuResult.dtls_parameters),
      // sctpParameters omitted — P1 has no browser SFU-DC (the control plane joins with the P2 host-controller migration).
    });
    this.transport = transport;
    // PIT-64 observability surface (headless debugging): expose the transport under the debug switch.
    if (SFU_DEBUG) ((window as unknown as { __sfuTransports?: MsTransport[] }).__sfuTransports ??= []).push(transport);

    // The 'connect' event = mediasoup-client generates the local DTLS fingerprint (the old hand-spliced-answer fingerprint-extraction PIT-56 debt dies).
    transport.on('connect', ({ dtlsParameters }, ok, fail) => {
      this.logT('transport connect → connect_web_rtc_transport');
      const ws = this.ws;
      if (!ws || ws.readyState !== WebSocket.OPEN) { fail(new Error('ws closed')); return; }
      ws.send(JSON.stringify({
        type: 'connect_web_rtc_transport',
        room_id: this.roomId,
        peer_id: this.sfuPeerId,
        transport_id: this.transportId,
        dtls_parameters: {
          fingerprints: dtlsParameters.fingerprints.map((f) => ({ algorithm: f.algorithm, value: f.value })),
          role: dtlsParameters.role,
        },
      }));
      ok();
    });
    // ICE/DTLS aggregate state (semantics-equivalent migration from the old pc.oniceconnectionstatechange)
    let iceEver = false; // ever-connected marker — distinguishes "initial connect failed" from "dropped after connecting"
    transport.on('connectionstatechange', (state) => {
      this.logT(`connectionstatechange = ${state}`);
      if (state === 'connected') iceEver = true;
      if (state === 'failed') {
        this.onStatus(iceEver ? 'disconnected' : 'error');
        this.stopMetrics();
      } else if (state === 'disconnected' && iceEver) {
        // ignore transients before the connection completes — the 30s connect watchdog backstops
        this.onStatus('disconnected');
        this.stopMetrics();
      }
    });

    // transport ready: queued producers (arrived before the transport) are consumed immediately; later new_producer goes through handleMessage.
    if (this.pendingProducer) {
      const p = this.pendingProducer;
      this.pendingProducer = null;
      void this.consumeProducer(p.producer_id, p.kind);
    }
  }

  /** get_router_rtp_capabilities → router_rtp_capabilities single-shot side channel (routed by handleMessage). */
  private rpcRouterRtpCapabilities(): Promise<unknown> {
    const ws = this.ws;
    if (!ws) return Promise.reject(new Error('Not connected'));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.capsResolver = null; reject(new Error('router caps timeout')); }, 5000);
      this.capsResolver = (caps) => { clearTimeout(timer); this.capsResolver = null; resolve(caps); };
      ws.send(JSON.stringify({ type: 'get_router_rtp_capabilities', room_id: this.roomId }));
    });
  }

  /** consume request + consumed response (server Consumer parameters land) → local transport.consume attaches the track
   *  (the negotiation completion point = the old ontrack semantic anchor). rtp_capabilities uses the Device negotiation result —
   *  the old PIT-55 hand-spliced full-codec-field debt dies with the core swap. */
  private async consumeProducer(producerId: string, kind: string): Promise<void> {
    const transport = this.transport;
    const device = this.device;
    const ws = this.ws;
    if (!transport || !device || !ws) return;
    this.logT(`sending consume (producer=${producerId})`);
    const result = await new Promise<{ consumer_id: string; producer_id: string; kind: string; rtp_parameters: unknown } | 'timeout'>((resolve) => {
      const timer = setTimeout(() => { this.consumeResolver = null; resolve('timeout'); }, 8000);
      this.consumeResolver = (msg) => { clearTimeout(timer); this.consumeResolver = null; resolve(msg); };
      ws.send(JSON.stringify({
        type: 'consume',
        room_id: this.roomId,
        peer_id: this.sfuPeerId,
        transport_id: this.transportId,
        producer_id: producerId,
        kind,
        rtp_capabilities: device.rtpCapabilities,
      }));
    });
    if (result === 'timeout') {
      this.logT('consume timeout → advance round');
      if (!this.playingSeen && !this.restarting && !this.waitingForProducer) void this.playDeadlineHit();
      return;
    }
    try {
      const consumer = await transport.consume({
        id: result.consumer_id,
        producerId: result.producer_id,
        kind: result.kind as 'audio' | 'video',
        rtpParameters: result.rtp_parameters as RtpParameters,
      });
      this.consumer = consumer;
      this.clearPlayWatchdog();
      this.playRounds = 0;
      this.waitingForProducer = false;
      this.playingSeen = true;
      this.stalled = false;
      this.stallTicks = 0;
      this.onTrack(new MediaStream([consumer.track]));
      this.onStatus('playing');
      this.startMetrics();
    } catch (err) {
      console.warn('SfuClient: transport.consume failed', err);
      if (!this.playingSeen && !this.restarting) void this.playDeadlineHit();
    }
  }

  /** v2: merged metrics reporting — partial fields overwrite only their own keys, never reset the others (flicker fix) */
  private emitMetrics(partial: Partial<StreamMetrics>): void {
    this.mergedMetrics = { ...(this.mergedMetrics ?? { rtt: 0, packetLoss: 0, fps: 0, bitrate: 0, jitter: 0, resolution: '' }), ...partial };
    this.onMetrics(this.mergedMetrics);
  }

  handleMessage(data: string): void {
    try {
      const msg = JSON.parse(data);

      // S0: store the negotiation result (missing protocol = v1) — read point for control-DC / (future) resume gating.
      if (msg.type === 'room_joined' && typeof msg.protocol === 'number') this.negotiatedProtocol = msg.protocol;

      // RPC single-shot side channel takes priority (caps/consumed) — removed immediately after consumption.
      if (msg.type === 'router_rtp_capabilities' && this.capsResolver) { this.capsResolver(msg.capabilities); return; }
      if (msg.type === 'consumed' && this.consumeResolver) { this.consumeResolver(msg); return; }

      if (msg.type === 'web_rtc_transport_created' && this.transportResolver) {
        this.transportResolver({
          transport_id: msg.transport_id,
          ice_parameters: msg.ice_parameters,
          dtls_parameters: msg.dtls_parameters,
          ice_candidates: msg.ice_candidates ?? [],
        });
        this.transportResolver = null;
      } else if (msg.type === 'new_producer') {
        // producer-label filter (§12-1): when expectedLabel is set, non-target streams are always ignored
        // (in device-room mode, multiple streams broadcast in one room — each VideoPlayer consumes only its own label).
        if (this.expectedLabel !== undefined && msg.label !== this.expectedLabel) {
          this.logT(`new_producer label=${msg.label} ≠ expected=${this.expectedLabel} — ignored`);
          return;
        }
        // W2: producer signal while in the 'source offline waiting' state → wake up and reopen the round budget (full re-negotiation, including late-join).
        if (this.waitingForProducer) {
          this.waitingForProducer = false;
          this.playRounds = 0;
          this.logT('new_producer → waking source-offline wait');
          void this.restartStream();
          return;
        }
        if (this.transportId && this.transport) {
          void this.consumeProducer(msg.producer_id, msg.kind);
        } else {
          this.logT('new_producer before transport, queuing');
          this.pendingProducer = { producer_id: msg.producer_id, kind: msg.kind };
        }
      } else if (msg.type === 'producer_closed') {
        // S4′: data-producer (remote-control DC domain) death is unrelated to the video stream — do not trigger a media-plane restart,
        // otherwise every cockpit/vehicle DC flip fake-restarts the whole dashboard.
        if (msg.kind === 'data') {
          this.logT('producer_closed(data) — unrelated to the media plane, ignored');
        } else if ((this.sfuMode || this.waitingForProducer) && !this.closed && !this.restarting) {
          this.logT('producer_closed → auto resubscribe (refresh-free self-heal)');
          void this.restartStream();
        }
      } else if (msg.type === 'encoder_status') {
        // v2 (web-stream-stats T4): Host encoder status (room broadcast) → merged into metrics (browser fields untouched)
        this.emitMetrics({
          codec: msg.codec,
          encoderBackend: msg.encoder_backend,
          encoderImplementation: msg.encoder_implementation ?? undefined,
          hostFps: msg.frames_per_second,
          hostResolution: msg.frame_width && msg.frame_height ? `${msg.frame_width}x${msg.frame_height}` : undefined,
          avgEncodeMs: msg.avg_encode_ms ?? undefined,
        });
      } else if (msg.type === 'error' && msg.code === 0) {
        // transport_connected / RPC ack — negotiation cadence is driven by consumed/track; observation only here.
        this.logT('ack code:0 ' + msg.message);
      } else if (msg.type === 'error') {
        // W4 (C16 client compliance): server failures must not be silent — classification drives the state machine.
        const code = Number(msg.code) || 0;
        console.warn('SfuClient: error', code, msg.message);
        if (classifySfuError(code) === 'terminal') {
          this.clearPlayWatchdog();
          this.onStatus('error');
        } else if (!this.playingSeen && !this.waitingForProducer && !this.restarting) {
          this.logT(`retryable error ${code} → advance round`);
          void this.playDeadlineHit();
        }
      } else if (msg.type === 'sdp' || msg.type === 'rtc_ice_candidate' || msg.type === 'r_t_c_ice_candidate') {
        // After the all-SFU decision (2026-08-25) the server drops P2P relay — legacy negotiation broadcasts in the room are silently ignored (noise).
      }
    } catch (err) {
      console.warn('SfuClient: message handling failed', err);
    }
  }

  // startMetrics polls Transport.getStats() (= the underlying RTCPeerConnection raw report) every 2s
  private startMetrics(): void {
    this.stopMetrics();
    this.metricsTimer = setInterval(async () => {
      const transport = this.transport;
      if (!transport) return;
      // H1 backstop observability: producer dies mid-stream and ProducerClosed never arrives (H3 worker-notification silence) → warn after 10s of muted
      const track = this.consumer?.track;
      if (track?.muted) {
        this.mutedTicks++;
        if (this.mutedTicks === 5) console.warn('SfuClient: video track muted ≥10s — producer may have died with no ProducerClosed notification');
      } else {
        this.mutedTicks = 0;
      }
      try {
        const stats = await transport.getStats();
        let rtt = 0, packetsLost = 0, packetsReceived = 0, fps = 0, bitrate = 0, jitter = 0;
        let width = 0, height = 0, decoderImpl: string | undefined, decoderCodec: string | undefined;
        let growing = false; // F2: any byte growth this tick is a media-freshness signal
        const pendingInbound: (RTCInboundRtpStreamStats & { decoderImplementation?: string })[] = [];

        stats.forEach((report) => {
          if (report.type === 'candidate-pair' && report.state === 'succeeded') {
            rtt = Math.round((report as unknown as { currentRoundTripTime?: number }).currentRoundTripTime ?? 0 * 1000) || 0;
          }
          // v2 (decoder fix): headless shell inbound-rtp lacks the decoderImplementation field →
          // fall back to the codec report mimeType (the format the browser actually decodes)
          if (report.type === 'codec') {
            const mime = (report as unknown as { mimeType?: string }).mimeType;
            if (mime?.startsWith('video/')) decoderCodec = mime;
          }
          if (report.type === 'inbound-rtp' && report.kind === 'video') {
            // PIT-212 continued: two rows coexist (frozen old row + live row) — statically locking the first-seen id would lock onto the frozen row.
            // Dynamic row selection: only process rows whose timestamp is newer than the seen row with the same id, and across rows take the globally latest timestamp.
            // Simplified to two passes: collect first, then pick the row with the max timestamp.
            pendingInbound.push(report as RTCInboundRtpStreamStats & { decoderImplementation?: string });
            return;
          }
        });

        // Row selection (PIT-212 final form): measured — among multiple rows on one pc, the real video row's ssrc/id **changes on every sample**
        // (mediasoup dynamic demux), so id/ssrc/timestamp criteria all fail. Liveness criteria:
        // framesDecoded>0 first (only the truly-decoding row is >0); on equal liveness take the largest bytes (cumulative monotonic).
        let r: (RTCInboundRtpStreamStats & { decoderImplementation?: string }) | null = null;
        for (const cand of pendingInbound) {
          if (!r) { r = cand; continue; }
          const rLive = (r.framesDecoded || 0) > 0;
          const cLive = (cand.framesDecoded || 0) > 0;
          if (cLive && !rLive) { r = cand; continue; }
          if (cLive === rLive && (cand.bytesReceived || 0) > (r.bytesReceived || 0)) r = cand;
        }
        if (r) {
          packetsLost = r.packetsLost || 0;
          packetsReceived = r.packetsReceived || 0;
          fps = r.framesPerSecond || 0;
          // v2 (unit fix): bitrate = byte delta / time window
          const bytes = r.bytesReceived || 0;
          const now = performance.now();
          if (this.lastTs > 0 && bytes >= this.lastBytes) {
            const elapsed = (now - this.lastTs) / 1000;
            if (elapsed > 0) bitrate = Math.round(((bytes - this.lastBytes) * 8) / elapsed / 1000); // kbps
          }
          if (bytes > this.lastBytes) growing = true;
          this.lastBytes = bytes;
          this.lastTs = now;
          jitter = Math.round((r.jitter || 0) * 1000);
          width = r.frameWidth || 0;
          height = r.frameHeight || 0;
          decoderImpl = r.decoderImplementation || undefined;
        }

        this.emitMetrics({
          rtt,
          packetLoss: packetsReceived > 0 ? Math.round((packetsLost / (packetsLost + packetsReceived)) * 10000) / 100 : 0,
          fps,
          bitrate,
          jitter,
          resolution: width && height ? `${width}x${height}` : 'unknown',
          decoderImplementation: decoderImpl,
          decoderCodec,
        });
        // F2: freshness verdict — STALL_TICKS consecutive zero-growth ticks → 'stalled' (source offline); growth resumes → 'playing'.
        if (this.playingSeen) {
          if (growing) {
            this.stallTicks = 0;
            if (this.stalled) { this.stalled = false; this.logT('media growth resumed → playing'); this.onStatus('playing'); }
          } else {
            this.stallTicks++;
            if (!this.stalled && this.stallTicks >= STALL_TICKS) {
              this.stalled = true;
              this.logT(`media ${STALL_TICKS * 2}s zero growth → source offline (stalled)`);
              this.onStatus('stalled');
            }
          }
        }
      } catch (err) {
        console.warn('SfuClient: getStats failed', err);
        // getStats() may fail; non-critical
      }
    }, 2000);
  }

  /** W2: first-frame watchdog arm/clear + round advance (shared entry for timeouts/retryable errors). */
  private armPlayWatchdog(): void {
    this.clearPlayWatchdog();
    this.playTimer = setTimeout(() => { void this.playDeadlineHit(); }, PLAY_WATCHDOG_MS);
  }
  private clearPlayWatchdog(): void {
    if (this.playTimer) { clearTimeout(this.playTimer); this.playTimer = null; }
  }
  /** W2: rounds exhausted → 'source offline (waiting for stream)': re-send one room_join to get the late-join replay
   *  (plugging the deadlock of "producer came online exactly inside the round window and the broadcast was missed"); later new_producer/producer_closed events wake it. */
  private async playDeadlineHit(): Promise<void> {
    if (this.closed || this.playingSeen || this.restarting || this.waitingForProducer) return;
    this.playRounds++;
    if (this.playRounds >= PLAY_ROUNDS_MAX) {
      this.waitingForProducer = true;
      this.ws?.send(JSON.stringify({ type: 'room_join', room_id: this.roomId, peer_role: 'consumer', protocol: 3 }));
      this.logT(`${PLAY_ROUNDS_MAX} consecutive rounds without a first frame → source offline (waiting for a stream to wake it)`);
      this.onStatus('stalled');
      return;
    }
    this.logT(`first-frame timeout (round ${this.playRounds}/${PLAY_ROUNDS_MAX}) → restartStream`);
    await this.restartStream();
  }

  private stopMetrics(): void {
    if (this.metricsTimer) {
      clearInterval(this.metricsTimer);
      this.metricsTimer = null;
    }
  }

  /**
   * W1: unlimited backoff reconnect — under multi-host stress the server crash-revive cycle (oxmgr backoff can reach minutes) must never leave a permanent red banner.
   *  Only two exits: unmount (closed) / auth terminal.*/
  async reconnect(): Promise<void> {
    if (this.reconnecting) return; // guard against double loops from concurrent onclose/retries
    this.reconnecting = true;
    try {
      let attempt = 0;
      while (!this.closed) {
        attempt++;
        const delay = nextBackoff(attempt) * (0.75 + Math.random() * 0.5); // jitter: prevents a thundering herd of 8 aligned tiles the instant the server revives
        this.logT(`reconnecting (attempt ${attempt}, ~${Math.round(delay)}ms)...`);
        await new Promise(r => setTimeout(r, delay));
        if (this.closed) return;
        this.onStatus('connecting');
        try {
          await this.connectAndDrive();
          return;
        } catch (err) {
          const m = String((err as Error)?.message ?? err);
          if (m.startsWith('sfu-terminal:')) {
            console.error('SfuClient: auth terminal, stopping retries:', m);
            this.onStatus('error');
            return;
          }
          console.warn(`SfuClient: reconnect attempt ${attempt} failed (keep retrying):`, m);
          // on connect failure, if the play-watchdog is not armed (no first frame in flight) → restart the round engine, preventing the "infinite connecting, no exit" deadlock
          if (!this.playTimer && !this.restarting) void this.playDeadlineHit();
          continue;
        }
      }
    } finally {
      this.reconnecting = false;
    }
  }

  /** W1: direct re-run — old-socket oscillation is already eliminated by "detach handlers + close" inside connect(); when the server is down,
   *  connect fails in milliseconds (onerror→WS error), when half-dead the Auth timeout is 10s; the backoff loop absorbs everything. */
  private async connectAndDrive(): Promise<void> {
    await this.connect();
    if (this.closed) return;
    this.logT('reconnected successfully');
    if (this.sfuMode) void this.restartStream();
    else void this.startPlay();
  }

  /**
   * H1: producer_closed self-heal — tear down the media plane (transport/device fully discarded and rebuilt) then re-run the complete startPlay
   * (equivalent to a page refresh, but invisible to the user). Re-sending room_join triggers the server late-join replay of existing
   * producers (otherwise, when the host finishes re-producing before us, there is no new_producer broadcast to receive).
   */
  private async restartStream(): Promise<void> {
    this.restarting = true;
    try {
      this.stopMetrics();
      this.transport?.close();
      this.transport = null;
      this.device = null;
      this.consumer = null;
      this.transportId = null;
      this.pendingProducer = null;
      this.capsResolver = null;
      this.consumeResolver = null;
      // PIT-65: new peer_id — the old SfuPeer remnant was invalidated when the server-side producer died.
      this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`;
      this.onStatus('connecting');
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        await this.connect(); // the WS also dropped (rare) → do a full reconnect first
      }
      this.ws?.send(JSON.stringify({
        type: 'room_join', room_id: this.roomId, peer_role: 'consumer', protocol: 3,
      }));
      this.playingSeen = false; this.stalled = false; this.stallTicks = 0; // F2: reset the watchdog for the new first frame
      this.waitingForProducer = false; // W2: proactively re-running exits the waiting state
      await this.startPlay();
    } catch (err) {
      const m = String((err as Error)?.message ?? err);
      if (m.startsWith('sfu-terminal:')) {
        console.error('SfuClient: restartStream auth terminal:', m);
        this.onStatus('error');
      } else {
        console.warn('SfuClient: restartStream failed (advancing round):', m);
        this.onStatus('connecting');
        // W2: failures also burn a round — executed in finally after restarting is cleared, preventing lost-event deadlocks
        setTimeout(() => { void this.playDeadlineHit(); }, 1000);
      }
    } finally {
      this.restarting = false;
    }
  }

  close(): void {
    this.closed = true;  // PIT-50: set the flag first to prevent onclose reconnect
    if (this.playTimer) { clearTimeout(this.playTimer); this.playTimer = null; } // W2: tear down the watchdog on unmount (prevents ghost rounds)
    this.stopMetrics();
    this.transport?.close();
    this.transport = null;
    this.device = null;
    this.consumer = null;
    this.capsResolver = null;
    this.consumeResolver = null;
    this.ws?.close();
    this.ws = null;
    this.transportId = null;
    this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`; // PIT-65
    this.transportResolver = null;
  }
}

/**
 * P1/T1.2 minimal uplink surface (the meeting-mic root; SDK prerequisite for the D270-R2 'meeting' acceptance):
 * send transport + produce(track). Server gate = account exemption in audio-* rooms (D-H11 revision / landed in P1.1).
 * Control-plane data-produce joins with the P2 host-controller SFU-DC migration (design §4) — out of scope for this class.
 */
export class SfuMicProducer {
  private ws: WebSocket | null = null;
  private transport: MsTransport | null = null;
  /** Response type key → one-shot resolver (enough for the serial auth/join/caps/transport/produced flow). */
  private pending = new Map<string, (m: Record<string, unknown>) => void>();

  constructor(
    private serverUrl: string,
    private roomId: string,
    private token: string,
  ) {}

  private rpcWait<T>(type: string, map: (m: Record<string, unknown>) => T, ms = 8000): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(type); reject(new Error(`${type} timeout`)); }, ms);
      this.pending.set(type, (m) => { clearTimeout(timer); this.pending.delete(type); resolve(map(m)); });
    });
  }

  private route(m: Record<string, unknown>): void {
    // error code:0 = ack (transport_connected/produced are exceptions — produced is its own type); terminal errors interrupt the in-flight RPC.
    if (m.type === 'error' && Number(m.code) !== 0 && classifySfuError(Number(m.code)) === 'terminal') {
      for (const [, cb] of this.pending) { void cb; }
      this.pending.clear(); // semantics: callers perceive it via timeout/throw (this class's flow is short; the timeout form is accepted)
    }
    const type = String(m.type ?? '');
    const cb = this.pending.get(type);
    if (cb) { this.pending.delete(type); cb(m); }
  }

  /** Full uplink chain: auth → join → caps → load → send transport → produce(track). */
  async produceAudio(track: MediaStreamTrack): Promise<void> {
    const protocol = this.serverUrl.startsWith('wss:') ? 'wss:' : 'ws:';
    const host = this.serverUrl.replace(/^wss?:\/\//, '');
    const ws = new WebSocket(`${protocol}//${host}/ws`, this.token ? [this.token] : []);
    this.ws = ws;
    ws.onmessage = (ev) => {
      try { this.route(JSON.parse(String(ev.data))); } catch { /* ignore non-JSON */ }
    };
    const send = (m: Record<string, unknown>) => ws.send(JSON.stringify(m));
    await new Promise<void>((resolve, reject) => {
      ws.onopen = () => resolve();
      ws.onerror = () => reject(new Error('WS error'));
      setTimeout(() => reject(new Error('mic connect timeout')), 10000);
    });
    const peerId = `${this.roomId}-mic-${Math.random().toString(36).slice(2, 8)}`;
    await this.rpcWait('error', () => true, 10000); // auth ack (code:0 authenticated)
    send({ type: 'room_join', room_id: this.roomId, peer_role: 'consumer', protocol: 3 });
    await this.rpcWait('room_joined', () => true);
    const capsP = this.rpcWait('router_rtp_capabilities', (m) => m.capabilities);
    send({ type: 'get_router_rtp_capabilities', room_id: this.roomId });
    const device = new Device();
    await device.load({ routerRtpCapabilities: (await capsP) as never });

    const createdP = this.rpcWait('web_rtc_transport_created', (m) => m);
    send({ type: 'create_web_rtc_transport', room_id: this.roomId, peer_id: peerId, direction: 'send' });
    const created = await createdP;
    const transport = device.createSendTransport({
      id: String(created.transport_id),
      iceParameters: mapIceParameters(created.ice_parameters as WireIceParams),
      iceCandidates: mapIceCandidates((created.ice_candidates ?? []) as WireIceCandidate[]) as never,
      dtlsParameters: mapDtlsParameters(created.dtls_parameters as WireDtlsParams),
    } as never);
    this.transport = transport;
    transport.on('connect', ({ dtlsParameters }, ok, fail) => {
      if (ws.readyState !== WebSocket.OPEN) { fail(new Error('ws closed')); return; }
      send({
        type: 'connect_web_rtc_transport', room_id: this.roomId, peer_id: peerId,
        transport_id: String(created.transport_id),
        dtls_parameters: {
          fingerprints: dtlsParameters.fingerprints.map((x) => ({ algorithm: x.algorithm, value: x.value })),
          role: dtlsParameters.role,
        },
      });
      ok();
    });
    transport.on('produce', ({ kind, rtpParameters }, cb, errCb) => {
      if (ws.readyState !== WebSocket.OPEN) { errCb(new Error('ws closed')); return; }
      this.rpcWait('produced', (m) => { cb({ id: String(m.producer_id) }); return true; })
        .catch((err: Error) => errCb(err));
      send({
        type: 'produce', room_id: this.roomId, peer_id: peerId,
        transport_direction: kind === 'audio' ? 'send' : 'send',
        transport_id: String(created.transport_id),
        kind, rtp_parameters: rtpParameters as Record<string, unknown>,
      });
    });
    await transport.produce({ track });
  }

  /** Un-mic (idempotent): close the transport hosting the producer plus the WS — if meeting P4a needs finer granularity, switch to producer.pause(). */
  close(): void {
    this.transport?.close();
    this.transport = null;
    if (this.ws) { this.ws.onmessage = null; this.ws.close(); this.ws = null; }
    this.pending.clear();
  }
}
