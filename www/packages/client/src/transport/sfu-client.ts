// SFU Consumer Client — mediasoup Server-Offer transport
// Flow: CreateWebRtcTransport(recv) → WebRtcTransportCreated → buildRemoteSdp → setRemoteDescription → createAnswer → ConnectWebRtcTransport
// After consume, ontrack delivers remote stream.

// PIT-55: mediasoup consume 匹配要求完整 codec 字段 (clockRate/parameters/preferredPayloadType)，
// 缺任一 → match_codecs strict 匹配失败 → "No compatible media codecs"
// 参数与 Router/Producer 一致 (4d0032 Main, packetization-mode=1)
// PIT-55: mediasoup consume 匹配要求完整 codec 字段 (clockRate/parameters/preferredPayloadType)，
// 缺任一 → match_codecs strict 匹配失败 → "No compatible media codecs"
// P3 (2026-08-07): router 默认 VP8 (PT 96) — capabilities 必须含 VP8 才能匹配 Host produce
// (Host 标准协商 answer 选 VP8 96); H264 保留作备选
function videoRtpCapabilities() {
  return {
    codecs: [{
      kind: 'video', // serde(tag="kind") 必需 (PIT-55)
      mimeType: 'video/VP8',
      clockRate: 90000,
      preferredPayloadType: 96,
      parameters: {},
      rtcpFeedback: [],
    }, {
      kind: 'video',
      mimeType: 'video/H264',
      clockRate: 90000,
      preferredPayloadType: 101,
      parameters: {
        'level-asymmetry-allowed': 1,
        'packetization-mode': 1,
        'profile-level-id': '42e01f', // v2: 与 router/Host 对齐 (encoder-backend-codec-config T7)
      },
      rtcpFeedback: [],
    }, {
      kind: 'video',
      mimeType: 'video/VP9',
      clockRate: 90000,
      preferredPayloadType: 99,
      parameters: {},
      rtcpFeedback: [],
    }, {
      kind: 'video',
      mimeType: 'video/AV1',
      clockRate: 90000,
      preferredPayloadType: 97,
      parameters: {},
      rtcpFeedback: [],
    }],
    // v3 (sfu-negotiation-completion T4): 声明 transport-cc/abs-capture-time —
    // mediasoup 端据此在输出 RTP 上附加扩展 → 浏览器生成 transport-cc feedback
    // → mediasoup 转发 → host BWE 自适应（BWE 闭环第三段, 与 host T2 对称）。
    headerExtensions: [{
      kind: 'video',
      uri: 'http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01',
      preferredId: 3,
      preferredEncrypt: false,
      direction: 'sendrecv',
    }, {
      kind: 'video',
      uri: 'http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time',
      preferredId: 5,
      preferredEncrypt: false,
      direction: 'sendrecv',
    }],
  };
}
interface IceParams {
  username_fragment: string;
  password: string;
}

interface DtlsParams {
  fingerprints: { algorithm: string; value: string }[];
  role: string;
}

// PIT-56: server 的 IceCandidate 是字段格式 (ip/port/protocol/foundation/priority/candidate_type)，非 SDP 字符串
interface IceCandidate {
  ip: string;
  port: number;
  protocol: string;
  foundation: string;
  priority: number;
  candidate_type?: string;
}

interface TransportCreated {
  transport_id: string;
  ice_parameters: IceParams;
  dtls_parameters: DtlsParams;
  ice_candidates?: IceCandidate[];
}

type StreamCallback = (stream: MediaStream) => void;
type StatusCallback = (status: 'connecting' | 'connected' | 'playing' | 'stalled' | 'disconnected' | 'error') => void;
type MetricsCallback = (metrics: StreamMetrics) => void;
/// F2: 连续零增长 tick 数（2s/tick）判「源离线」——≈6s，保守于 fps 最慢流且不早于任何信令自愈。
const STALL_TICKS = 3;
/// W2: 首帧轮次 watchdog——30s 无 ontrack 计一轮失败，restartStream 最多 3 轮，耗尽→「源离线」等待。
const PLAY_WATCHDOG_MS = 30000;
const PLAY_ROUNDS_MAX = 3;

/// W4: 错误码分流——auth/授权族=终态（红牌唯一合法源）；4031（权限可热改，C33）、
/// 5000（SFU 内部失败，server 重启窗口典型码）、5001（网关上游切换，host 侧发射经转发）=可重试。
export function classifySfuError(code: number): 'terminal' | 'retry' {
  switch (code) {
    case 4000: case 4001: case 4002: case 4003: case 4010: case 4011:
      return 'terminal';
    default:
      return 'retry';
  }
}
/// W1: 指数退避序列 1s→封顶（默认 30s）纯函数；jitter 由调用方外置（可断言）。
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
  // v2 (web-stream-stats T4): 编解码诊断 — 浏览器侧 + Host EncoderStatus 上报合并
  decoderImplementation?: string;   // 浏览器解码器（inbound-rtp, 真实 Chrome 有; headless 缺失）
  decoderCodec?: string;            // 降级: getStats codec 报告 mimeType（浏览器实际解码格式）
  codec?: string;                   // 编码模式（Host EncoderStatus, e.g. video/H264）
  encoderBackend?: string;          // Host backend 请求值（auto/software/hardware...）
  encoderImplementation?: string;   // Host 实际编码器（get_stats, e.g. OpenH264/libvpx）
  hostFps?: number;                 // Host outbound fps
  hostResolution?: string;          // Host 编码分辨率
  avgEncodeMs?: number;             // v3 (encode-time-stats T4): Host 平均每帧编码耗时（ms/帧）
}

export class SfuConsumerClient {
  private ws: WebSocket | null = null;
  private closed = false;  // PIT-50: close() 后禁止重连（StrictMode 双挂载竞争）
  private pc: RTCPeerConnection | null = null;
  // v2 (web-stream-stats 修复): 双数据源合并累加器 — getStats 与 encoder_status 交替
  // 覆盖导致面板闪烁（一会数值一会 "-"）; 统一合并后回调
  private mergedMetrics: StreamMetrics | null = null;
  // 码率增量计算: 累计 bytesReceived 当瞬时值是单位错误根因
  private lastBytes = 0;
  private mutedTicks = 0; // H1: muted 连续 tick 计数（2s tick）
  // F2: 媒体新鲜度 watchdog（主流双保险第二柱——信令全丢也不假 LIVE）
  private playingSeen = false; // 首次 ontrack 后启用
  private stallTicks = 0;
  private stalled = false;
  // W1/W2: 韧性状态机字段
  private playTimer: ReturnType<typeof setTimeout> | null = null;
  private playRounds = 0;
  private waitingForProducer = false;
  private reconnecting = false;
  private lastTs = 0;
  private onTrack: StreamCallback;
  private onStatus: StatusCallback;
  private onMetrics: MetricsCallback;
  private transportId: string | null = null;
  /** SFU 模式标志: startPlay 发出 create_web_rtc_transport 即置位 —
      P2P 房间的 SDP/ICE 全房间广播（host 侧协商），SFU 流程收到即无关，一律忽略 */
  private sfuMode = false;
  // PIT-65: 每连接唯一 SFU peer_id — 多网页同 peer_id 导致 SfuManager recv_transport 互相覆盖
  private sfuPeerId: string;
  private transportResolver: ((params: TransportCreated) => void) | null = null;
  private pendingProducer: any = null;
  private pendingSdp: any = null;
  /** H1: producer_closed 自愈重入守卫（广播风暴/连续消息只 restart 一次）。 */
  private restarting = false;
  private metricsTimer: ReturnType<typeof setInterval> | null = null;
  // PIT-76: 首帧/渲染时间戳观测 — 从 startPlay 起计时，各节点打 [T+nms]
  private t0 = 0;
  private logT(msg: string): void {
    const t = performance.now();
    console.log(`[T+${Math.round(t - this.t0)}ms] ${msg}`);
  }


  constructor(
    private serverUrl: string,
    private roomId: string,
    private token: string,
    callbacks: {
      onTrack: StreamCallback;
      onStatus: StatusCallback;
      onMetrics: MetricsCallback;
    },
  ) {
    this.onTrack = callbacks.onTrack;
    this.onStatus = callbacks.onStatus;
    this.onMetrics = callbacks.onMetrics;
    // PIT-65: 每连接唯一 SFU peer_id (多网页同 peer_id → SfuManager transport 覆盖)
    this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`;
  }

  async connect(): Promise<void> {
    this.closed = false;  // PIT-50: 每次 connect 重置关闭标志
    this.onStatus('connecting');

    // W1: 建连前摘除旧 socket 与其 onclose 重连语义——防重连成功后旧 socket 又触发 reconnect 振荡。
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

    // Auth: JWT 经 sec-websocket-protocol 子协议（RFC 6455 token 禁止空格——不能带 "Bearer " 前缀）
    // PIT-49: 浏览器子协议 = 纯 JWT；server 解析时兼容 "Bearer " 前缀
    this.ws = new WebSocket(wsUrl, this.token ? [this.token] : []);

    // Auth: PSK fallback（无 token 时发明文 PSK；有 JWT 子协议则不发）
    const psk = this.token ? null : 'mediaservo-dev';
    const authPromise = new Promise<void>((resolve, reject) => {
      this.ws!.onopen = () => {
        if (psk) this.ws!.send(psk);
      };
      this.ws!.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.code === 0 || msg.type === 'error' && msg.code === 0) {
            this.onStatus('connected');
            resolve();
          } else if (typeof msg.code === 'number' && msg.code !== 0) {
            // W4: 握手期错误——auth/授权族=终态（重试无义）；5000/5001 族=继续等 ack（Auth timeout 10s 兜底重连）
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
    }));

    // Reconnect on WS close
    this.ws.onclose = () => {
      if (this.closed) return;  // PIT-50: close() 后不重连
      this.onStatus('disconnected');
      this.stopMetrics();
      this.reconnect();
    };
  }
  async startPlay(): Promise<void> {
    this.sfuMode = true; // 进入 SFU 流程: 之后到达的 sdp 广播一律忽略
    if (!this.ws) throw new Error('Not connected');

    // Create RTCPeerConnection upfront (shared for SFU and P2P)
    this.t0 = performance.now();
    this.logT('startPlay: 创建 RTCPeerConnection');
    this.pc = new RTCPeerConnection({
      iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
    });
    (window as any).__sfuPc = this.pc; // PIT-64 观测: 暴露 pc 供 getStats 查询
    this.pc.ontrack = (event) => { this.logT('ONTRACK fired (track=' + event.track?.kind + ')'); console.log('SfuClient: ONTRACK fired, streams=', event.streams.length, 'track=', event.track?.kind); this.clearPlayWatchdog(); this.playRounds = 0; this.waitingForProducer = false; this.playingSeen = true; this.stalled = false; this.stallTicks = 0; this.onTrack(event.streams[0]); this.onStatus('playing'); this.startMetrics(); };
    let iceEver = false; // ICE 曾连通标记——拆分"初次建联"与"已连通后断开"的语义
    this.pc.oniceconnectionstatechange = () => {
      console.log('SfuClient: iceConnectionState =', this.pc?.iceConnectionState); // PIT-56 观测
      this.logT('iceConnectionState = ' + this.pc?.iceConnectionState);
      const st = this.pc?.iceConnectionState;
      if (st === 'connected' || st === 'completed') iceEver = true;
      if (st === 'failed') {
        // 从未连通 = 真·无法建立（error 语义）；已连通后 = 流中断（disconnected 语义）
        this.onStatus(iceEver ? 'disconnected' : 'error');
        this.stopMetrics();
      } else if (st === 'disconnected' && iceEver) {
        // 建联完成前的瞬态 disconnected（候选对收敛中）忽略——30s 建联 watchdog 兜底
        this.onStatus('disconnected'); this.stopMetrics();
      }
    };
    this.pc.onicecandidate = (event) => {
      console.log('SfuClient: local candidate', event.candidate?.candidate); // PIT-56 观测
      if (event.candidate) this.ws?.send(JSON.stringify({ type: 'rtc_ice_candidate', room_id: this.roomId, target: null, candidate: event.candidate.candidate, sdp_mid: event.candidate.sdpMid, sdp_mline_index: event.candidate.sdpMLineIndex }));
    };
    this.pc.addTransceiver('video', { direction: 'recvonly' });
    this.pc.addTransceiver('audio', { direction: 'recvonly' });

    // Try SFU (mediasoup) with 3s timeout. Fall back to P2P if no response.
    // Set resolver BEFORE sending to avoid race condition
    const sfuPromise = new Promise<TransportCreated | null>(r => { this.transportResolver = r; });
    this.logT('发送 create_web_rtc_transport'); console.log("SfuClient: sending create_web_rtc_transport"); this.ws.send(JSON.stringify({ type: "create_web_rtc_transport", room_id: this.roomId, peer_id: this.sfuPeerId, direction: 'recv' }));
    this.armPlayWatchdog(); // W2: 首帧轮次截止
    const sfuResult = await Promise.race([
      sfuPromise,
      new Promise<null>(r => setTimeout(() => r(null), 3000)),
    ]);

    if (sfuResult) {
      this.logT('收到 web_rtc_transport_created'); console.log('SfuClient: SFU transport created, building SDP...');
      this.transportId = sfuResult.transport_id;
      // pending producer will be processed after connect_web_rtc_transport succeeds
      this.logT('开始 setRemoteDescription'); console.log('SfuClient: setting remote description...');
      const offerSdp = this.buildRemoteSdp(sfuResult.ice_parameters, sfuResult.dtls_parameters, sfuResult.ice_candidates ?? []);
      try {
        console.log('SfuClient: offer SDP:\n' + offerSdp); // PIT-56 观测
        await this.pc.setRemoteDescription({ type: 'offer', sdp: offerSdp });
        this.logT('setRemoteDescription 完成'); console.log('SfuClient: remote description set OK');
        const answer = await this.pc.createAnswer();
        this.logT('createAnswer 完成'); console.log('SfuClient: answer created');
        console.log('SfuClient: answer SDP:\n' + answer.sdp); // PIT-56 观测
        await this.pc.setLocalDescription(answer);
        this.logT('setLocalDescription 完成, 发送 connect_web_rtc_transport'); console.log('SfuClient: local description set, sending connect_web_rtc_transport');
        // PIT-56: connect 的 fingerprints 必须是浏览器本地证书指纹 (从 answer SDP 提取),
        // 传 sfuResult 的 (mediasoup 指纹) → DTLS fingerprint mismatch → 无 SRTP → Consumer 不转发
        const localFp = (answer.sdp ?? '').match(/a=fingerprint:(\S+) (\S+)/);
        this.ws.send(JSON.stringify({ type: 'connect_web_rtc_transport', room_id: this.roomId, peer_id: this.sfuPeerId, transport_id: sfuResult.transport_id, dtls_parameters: { fingerprints: localFp ? [{ algorithm: localFp[1], value: localFp[2] }] : sfuResult.dtls_parameters.fingerprints, role: "client" }, sdp: answer.sdp }));
      } catch (e) {
        console.error('SfuClient: SDP negotiation failed:', e);
      }
    } else {
      console.log("SfuClient: SFU timeout, falling back to P2P");
      if (this.pendingSdp) {
        console.log("SfuClient: replaying pending SDP");
        this.handleMessage(JSON.stringify(this.pendingSdp));
      } else {
        void this.playDeadlineHit(); // W3: 无 P2P 素材且 SFU 无响应 → 立即计轮次失败，不空等 30s
      }
    }
  }
  // WS message handler — routed from connect() onmessage
  /** v2: 合并式 metrics 上报 — 部分字段只覆盖, 不重置其他字段（闪烁修复） */
  private emitMetrics(partial: Partial<StreamMetrics>): void {
    this.mergedMetrics = { ...(this.mergedMetrics ?? { rtt: 0, packetLoss: 0, fps: 0, bitrate: 0, jitter: 0, resolution: '' }), ...partial };
    this.onMetrics(this.mergedMetrics);
  }

  handleMessage(data: string): void {
    try {
      const msg = JSON.parse(data);
      console.log('SfuClient: received message type:', msg.type);

      if (msg.type === 'web_rtc_transport_created' && this.transportResolver) {
        console.log('SfuClient: transport msg keys:', Object.keys(msg).join(','), 'cands=', JSON.stringify(msg.ice_candidates)); // PIT-56 观测
        console.log('SfuClient: transport created, id:', msg.transport_id);
        this.transportResolver({
          transport_id: msg.transport_id,
          ice_parameters: msg.ice_parameters,
          dtls_parameters: msg.dtls_parameters,
          ice_candidates: msg.ice_candidates ?? [], // PIT-56: 必须传给 buildRemoteSdp
        });
        this.transportResolver = null;
      } else if (msg.type === 'new_producer') {
        // W2: 「源离线等待」态收到 producer 信号 → 唤醒重开轮次预算（全量重走协商，含 late-join）。
        if (this.waitingForProducer) {
          this.waitingForProducer = false;
          this.playRounds = 0;
          this.logT('new_producer → 唤醒源离线等待');
          void this.restartStream();
          return;
        }
        if (this.transportId) {
          console.log('SfuClient: consuming producer', msg.producer_id);
          const rtpCaps = videoRtpCapabilities();
          this.ws?.send(JSON.stringify({
            type: 'consume', room_id: this.roomId, peer_id: this.sfuPeerId, transport_id: this.transportId,
            producer_id: msg.producer_id, kind: msg.kind, rtp_capabilities: rtpCaps,
          }));
        } else {
          console.log('SfuClient: new_producer before transport, queuing');
          this.pendingProducer = msg;
        }
      } else if (msg.type === 'producer_closed') {
        // H1: host 断开/重启 → server 广播 producer 死亡。免刷新自愈：拆媒体面
        // 重跑 startPlay（逻辑刷新），late-join/新 new_producer 广播驱动重订阅。
        if ((this.sfuMode || this.waitingForProducer) && !this.closed && !this.restarting) {
          this.logT('producer_closed → 自动重订阅（免刷新自愈）');
          void this.restartStream();
        }
      } else if (msg.type === 'encoder_status') {
        // v2 (web-stream-stats T4): Host 编码状态（room 广播）→ 合并进 metrics（不覆盖浏览器字段）
        this.emitMetrics({
          codec: msg.codec,
          encoderBackend: msg.encoder_backend,
          encoderImplementation: msg.encoder_implementation ?? undefined,
          hostFps: msg.frames_per_second,
          hostResolution: msg.frame_width && msg.frame_height ? `${msg.frame_width}x${msg.frame_height}` : undefined,
          avgEncodeMs: msg.avg_encode_ms ?? undefined,
        });
      } else if (msg.type === 'consumed') {
        this.logT('consumed (consumer 创建成功, 等待 RTP)');
        // ponytail: producer consumed, stream arrives via ontrack
      } else if (msg.type === 'error' && msg.code === 0) {
        this.logT('transport_connected'); console.log('SfuClient: transport_connected (code: 0)');
        if (this.pendingProducer && this.transportId) {
          this.logT('consuming pending producer ' + this.pendingProducer.producer_id); console.log('SfuClient: consuming pending producer', this.pendingProducer.producer_id);
          // PIT-55: rtp_capabilities 需完整 codec 字段, 见 videoRtpCapabilities()
          const rtpCaps = videoRtpCapabilities();
          this.ws?.send(JSON.stringify({
            type: 'consume', room_id: this.roomId, peer_id: this.sfuPeerId, transport_id: this.transportId,
            producer_id: this.pendingProducer.producer_id, kind: this.pendingProducer.kind, rtp_capabilities: rtpCaps,
          }));
        }
      } else if (msg.type === 'error') {
        // W4 (C16 客户端合规): server 失败不得静默——分类驱动状态机。
        const code = Number(msg.code) || 0;
        console.log('SfuClient: error', code, msg.message);
        if (classifySfuError(code) === 'terminal') {
          this.clearPlayWatchdog();
          this.onStatus('error');
        } else if (!this.playingSeen && !this.waitingForProducer && !this.restarting) {
          this.logT(`可重试错误 ${code} → 轮次推进`);
          void this.playDeadlineHit();
        }
      } else if (msg.type === "sdp") {
        console.log("SfuClient: SDP received");
        // P2P 房间广播过滤: vehicle 房间是 P2P 类型, SDP/ICE 全房间广播 —
        // host 侧（controller/emergency/vision）的协商 SDP 会到达浏览器。
        // SFU 模式（transportId 已建）的 consume 是消息驱动, 不需要任何 SDP 消息 —
        // 收到即无关广播, 忽略（否则把别人的 offer 当自己的协商 → 状态错乱）。
        if (this.sfuMode) {
          console.log("SfuClient: SDP ignored (SFU mode, room broadcast)");
          return;
        }
        if (!this.pc) { this.pendingSdp = msg; return; }
        // P2P mode: handle host's SDP offer → create answer
        try {
          const sdp = typeof msg.sdp === 'string' ? JSON.parse(msg.sdp) : msg.sdp;
          if (sdp.type === 'offer' && this.pc) {
            this.pc.setRemoteDescription(sdp).then(async () => {
              if (!this.pc) return;
              const answer = await this.pc.createAnswer();
              await this.pc.setLocalDescription(answer);
              this.ws?.send(JSON.stringify({ type: 'sdp', room_id: this.roomId, target: null, sdp: JSON.stringify(answer) }));
            }).catch((err) => console.warn('SfuClient: SDP setRemoteDescription failed', err));
          }
        } catch (err) {
          console.warn('SfuClient: SDP handling failed', err);
        }
      } else if ((msg.type === 'rtc_ice_candidate' || msg.type === 'r_t_c_ice_candidate') && this.pc) {
        // PIT-106 (I2 review): server 中继重序列化为规范名 r_t_c_ice_candidate（serde snake_case）
        // — 两 tag 都收，兼容旧 server 与新 alias 两种 wire。
        if (this.sfuMode && this.pc && this.pc.remoteDescription === null) {
          return; // 协商未完成前的房间广播 candidate，忽略
        }
        console.log('SfuClient: ICE candidate received', msg.candidate);
        this.pc.addIceCandidate({
          candidate: msg.candidate,
          sdpMid: msg.sdp_mid ?? null,
          sdpMLineIndex: msg.sdp_mline_index ?? null,
        }).catch((e) => console.warn('SfuClient: addIceCandidate failed', e));
      }
    } catch (err) {
      console.warn('SfuClient: message handling failed', err);
    }
  }

  // Build a server-side SDP offer from mediasoup ICE/DTLS parameters.
  // The browser answers this offer to establish the server-offer transport.
  private buildRemoteSdp(ice: IceParams, dtls: DtlsParams, candidates: IceCandidate[]): string {
    const fp = dtls.fingerprints[0];
    // PIT-56: mediasoup ICE-Lite 候选必须嵌入 offer 的 m= 段（无候选 → 浏览器 ICE 无对端地址，永不发起）
    // 转为 SDP candidate 行 (candidate 必须在 m= 段内 — PIT-46 同教训)
    const toCandidateLine = (c: IceCandidate) =>
      `a=candidate:${c.foundation} 1 ${c.protocol.toUpperCase()} ${c.priority} ${c.ip} ${c.port} typ ${c.candidate_type ?? 'host'}`;
    const videoCandidates = candidates.map(toCandidateLine).join('\r\n');
    const audioCandidates = '';
    return [
      'v=0',
      'o=- 0 0 IN IP4 0.0.0.0',
      's=-',
      't=0 0',
      'a=group:BUNDLE video audio',
      'a=ice-lite',
      `a=ice-ufrag:${ice.username_fragment}`,
      `a=ice-pwd:${ice.password}`,
      `a=fingerprint:${fp.algorithm.toLowerCase()} ${fp.value}`,
      'a=setup:passive', // PIT-56: offer setup 决定 answerer 角色 — passive → 浏览器 active (ClientHello 发起方)；mediasoup 是 DTLS server 等 ClientHello (Host 侧 actpass 同理)
      // Video: VP8 96 + H264 101 同时请求（producer codec 由 Host 配置决定, v2:
      // offer codec 必须匹配 consume codec, 否则浏览器不接收 RTP → 无视频）
      'm=video 7 UDP/TLS/RTP/SAVPF 96 101 99 97',
      'c=IN IP4 127.0.0.1',
      'a=rtcp-mux',
      'a=mid:video',
      // v3 (sfu-negotiation-completion T4): transport-cc + abs-capture-time extmap —
      // 浏览器收流需声明 transport-cc 才会生成 feedback → mediasoup 转发 → host BWE
      // 自适应（BWE 闭环另一端, 与 host 侧 T1 对称）。
      'a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01',
      'a=extmap:5 http://www.webrtc.org/experiments/rtp-hdrext/abs-capture-time',
      'a=sendonly', // PIT-56: offer 描述 mediasoup (发送方) — recvonly+浏览器recvonly → 协商 inactive → 无媒体轨
      'a=rtpmap:96 VP8/90000',
      'a=rtcp-fb:96 nack',
      'a=rtcp-fb:96 nack pli',
      'a=rtpmap:101 H264/90000',
      'a=fmtp:101 profile-level-id=42e01f;packetization-mode=1',
      'a=rtcp-fb:101 nack',
      'a=rtcp-fb:101 nack pli',
      'a=rtpmap:99 VP9/90000',
      'a=rtcp-fb:99 nack',
      'a=rtcp-fb:99 nack pli',
      'a=rtpmap:97 AV1/90000',
      'a=rtcp-fb:97 nack',
      'a=rtcp-fb:97 nack pli',
      ...(videoCandidates ? [videoCandidates] : []),
      'a=end-of-candidates',
      // Audio: Opus
      'm=audio 7 UDP/TLS/RTP/SAVPF 111',
      'c=IN IP4 127.0.0.1',
      'a=rtcp-mux',
      'a=mid:audio',
      'a=rtpmap:111 opus/48000/2',
      'a=fmtp:111 minptime=10;useinbandfec=1',
      ...(audioCandidates ? [audioCandidates] : []),
      'a=end-of-candidates',
      '',
    ].join('\r\n');
  }

  // startMetrics polls RTCPeerConnection.getStats() every 2s
  private startMetrics(): void {
    this.stopMetrics();
    this.metricsTimer = setInterval(async () => {
      if (!this.pc) return;
      // H1 兑底观测: producer 中途死且 ProducerClosed 不可达（H3 worker 通知静默）→ muted 持续 10s 告警
      const vt = this.pc.getTransceivers().find(t => t.receiver.track?.kind === 'video');
      if (vt?.receiver.track.muted) {
        this.mutedTicks++;
        if (this.mutedTicks === 5) console.warn('SfuClient: video track muted ≥10s — producer 可能已死且无 ProducerClosed 通知');
      } else {
        this.mutedTicks = 0;
      }
      try {
        const stats = await this.pc.getStats();
        let rtt = 0, packetsLost = 0, packetsReceived = 0, fps = 0, bitrate = 0, jitter = 0;
        let width = 0, height = 0, decoderImpl: string | undefined, decoderCodec: string | undefined;
        let growing = false; // F2: 本 tick 字节增量即媒体新鲜信号

        stats.forEach((report) => {
          if (report.type === 'candidate-pair' && report.state === 'succeeded') {
            rtt = Math.round((report as any).currentRoundTripTime * 1000) || 0;
          }
          // v2 (解码器修复): headless shell inbound-rtp 无 decoderImplementation 字段 →
          // 降级用 codec 报告 mimeType（浏览器实际解码格式）
          if (report.type === 'codec' && (report as any).mimeType?.startsWith('video/')) {
            decoderCodec = (report as any).mimeType;
          }
          if (report.type === 'inbound-rtp' && report.kind === 'video') {
            packetsLost = (report as any).packetsLost || 0;
            packetsReceived = (report as any).packetsReceived || 0;
            fps = (report as any).framesPerSecond || 0;
            // v2 (单位修复): 码率 = 字节增量/时间窗（累计 bytesReceived 当瞬时值 → 数字虚增）
            const bytes = (report as any).bytesReceived || 0;
            const now = performance.now();
            if (this.lastTs > 0 && bytes >= this.lastBytes) {
              const elapsed = (now - this.lastTs) / 1000;
              if (elapsed > 0) bitrate = Math.round(((bytes - this.lastBytes) * 8) / elapsed / 1000); // kbps
            }
            if (bytes > this.lastBytes) growing = true;
            this.lastBytes = bytes;
            this.lastTs = now;
            jitter = Math.round(((report as any).jitter || 0) * 1000);
            width = (report as any).frameWidth || 0;
            height = (report as any).frameHeight || 0;
            decoderImpl = (report as any).decoderImplementation || undefined; // v2 T4
          }
        });

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
        // F2: 新鲜度判定——连续 STALL_TICKS 次零增长 → 'stalled'（源离线）；恢复增长 → 'playing'。
        if (this.playingSeen) {
          if (growing) {
            this.stallTicks = 0;
            if (this.stalled) { this.stalled = false; this.logT('媒体恢复增长 → playing'); this.onStatus('playing'); }
          } else {
            this.stallTicks++;
            if (!this.stalled && this.stallTicks >= STALL_TICKS) {
              this.stalled = true;
              this.logT(`媒体 ${STALL_TICKS * 2}s 零增长 → 源离线(stalled)`);
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

  /** W2: 首帧 watchdog 装填/清除 + 轮次推进（超时/可重试错误共用入口）。 */
  private armPlayWatchdog(): void {
    this.clearPlayWatchdog();
    this.playTimer = setTimeout(() => { void this.playDeadlineHit(); }, PLAY_WATCHDOG_MS);
  }
  private clearPlayWatchdog(): void {
    if (this.playTimer) { clearTimeout(this.playTimer); this.playTimer = null; }
  }
  /** W2: 轮次耗尽→「源离线(等待流)」：补发一次 room_join 拿 late-join 回放
   *  （堵住“producer 恰在轮次窗口内上线错过广播”的死锁），后续 new_producer/producer_closed 唤醒。 */
  private async playDeadlineHit(): Promise<void> {
    if (this.closed || this.playingSeen || this.restarting || this.waitingForProducer) return;
    this.playRounds++;
    if (this.playRounds >= PLAY_ROUNDS_MAX) {
      this.waitingForProducer = true;
      this.ws?.send(JSON.stringify({ type: 'room_join', room_id: this.roomId, peer_role: 'consumer' }));
      this.logT(`连续 ${PLAY_ROUNDS_MAX} 轮无首帧 → 源离线（等待流唤醒）`);
      this.onStatus('stalled');
      return;
    }
    this.logT(`首帧超时（轮次 ${this.playRounds}/${PLAY_ROUNDS_MAX}）→ restartStream`);
    await this.restartStream();
  }

  private stopMetrics(): void {
    if (this.metricsTimer) {
      clearInterval(this.metricsTimer);
      this.metricsTimer = null;
    }
  }

  /**
   * W1: 无限退避重连——多 host 压测下 server 崩溃-复活（oxmgr 退避可达分钟级）禁止永久红牌。
   *  出口仅两个：组件卸载（closed）/ auth 终态。*/
  async reconnect(): Promise<void> {
    if (this.reconnecting) return; // 多路 onclose/重试并发防双循环
    this.reconnecting = true;
    try {
      let attempt = 0;
      while (!this.closed) {
        attempt++;
        const delay = nextBackoff(attempt) * (0.75 + Math.random() * 0.5); // jitter：防 server 复活瞬间 8 tile 对齐惊群
        console.log(`SfuClient: reconnecting (attempt ${attempt}, ~${Math.round(delay)}ms)...`);
        await new Promise(r => setTimeout(r, delay));
        if (this.closed) return;
        this.onStatus('connecting');
        try {
          await this.connectAndDrive();
          return;
        } catch (err) {
          const m = String((err as Error)?.message ?? err);
          if (m.startsWith('sfu-terminal:')) {
            console.error('SfuClient: auth 终态，停止重试:', m);
            this.onStatus('error');
            return;
          }
          console.warn(`SfuClient: reconnect attempt ${attempt} failed (keep retrying):`, m);
          // connect 失败时若 play-watchdog 未挂（无首帧在途）→ 重启轮次引擎，防“无限 connecting 无退出”死锁
          if (!this.playTimer && !this.restarting) void this.playDeadlineHit();
          continue;
        }
      }
    } finally {
      this.reconnecting = false;
    }
  }

  /** W1: 直连重跑——旧 socket 振荡已由 connect() 内「摘 handlers + close」消除；server 宕机时
   *  connect 毫秒级快拒（onerror→WS error），半死时 Auth timeout 10s，退避循环全吸收。
   *  （探测 socket 方案已删：压测实证额外握手面只引入新失败模式，无增益。） */
  private async connectAndDrive(): Promise<void> {
    await this.connect();
    if (this.closed) return;
    console.log('SfuClient: reconnected successfully');
    if (this.sfuMode) void this.restartStream();
    else void this.startPlay();
  }

  /**
   * H1: producer_closed 自愈 — 拆 pc/transport 后重跑完整 startPlay（等价页面刷新，
   * 但保留用户视角无感）。重发 room_join 触发 server late-join 回放 existing producers
   * （否则 host 先于本端完成 re-produce 时无 new_producer 广播可收）。
   */
  private async restartStream(): Promise<void> {
    this.restarting = true;
    try {
      this.stopMetrics();
      this.pc?.close();
      this.pc = null;
      this.transportId = null;
      this.pendingProducer = null;
      // PIT-65: 新 peer_id——旧 SfuPeer 残留随 server 端 producer 死亡已失效。
      this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`;
      this.onStatus('connecting');
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        await this.connect(); // WS 也断了（罕见）→ 先全量重连
      }
      this.ws?.send(JSON.stringify({
        type: 'room_join', room_id: this.roomId, peer_role: 'consumer',
      }));
      this.playingSeen = false; this.stalled = false; this.stallTicks = 0; // F2: 重置 watchdog 至新 ontrack
      this.waitingForProducer = false; // W2: 主动重走即脱离等待态
      await this.startPlay();
    } catch (err) {
      const m = String((err as Error)?.message ?? err);
      if (m.startsWith('sfu-terminal:')) {
        console.error('SfuClient: restartStream auth 终态:', m);
        this.onStatus('error');
      } else {
        console.warn('SfuClient: restartStream failed（推进轮次）:', m);
        this.onStatus('connecting');
        // W2: 失败也烧轮次——finally 清 restarting 后执行，防事件丢失式死锁
        setTimeout(() => { void this.playDeadlineHit(); }, 1000);
      }
    } finally {
      this.restarting = false;
    }
  }

  close(): void {
    this.closed = true;  // PIT-50: 先设标志防 onclose 重连
    if (this.playTimer) { clearTimeout(this.playTimer); this.playTimer = null; } // W2: 卸载即拆 watchdog（防幽灵轮次）
    this.stopMetrics();
    this.pc?.close();
    this.pc = null;
    this.ws?.close();
    this.ws = null;
    this.transportId = null;
    this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`; // PIT-65
    this.transportResolver = null;
  }
}
