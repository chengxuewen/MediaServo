// SFU Consumer Client — mediasoup-client 内核（P1/D270-R2 换心）
// Flow: get_router_rtp_capabilities → Device.load → create_web_rtc_transport(recv)
//       → device.createRecvTransport(opts) → 'connect'→connect_web_rtc_transport(dtls)
//       → new_producer → consume(rtp_capabilities=device.rtpCapabilities) → consumed
//       → transport.consume → track。
// 韧性层（W1-W5/F2/H1/H3/V2）与旧实现逐段同构——SDP/ORTC 手拼逻辑（拼 offer/硬编码 PT/
// videoRtpCapabilities，PIT-173/PIT-55/PIT-56 债群）由 mediasoup-client 官方实现接管（C18）。
// P2P 回退移除：全 SFU 决策（2026-08-25，server 房间统一 DeviceStream、SDP/ICE 中继丢弃）
// 使旧 P2P 通路本就不通——sdp/ice 广播静默忽略（噪音），协商失败走轮次引擎。

import { Device } from 'mediasoup-client';
import type { Consumer, Transport as MsTransport } from 'mediasoup-client/types';
import type { DtlsParameters as MsDtlsParameters, RtpParameters } from 'mediasoup-client/types';

// ── server wire（snake_case SignalingMessage）↔ mediasoup-client（camel）映射纯函数 ──
// 单点收敛（design §3）：vitest 直接回放 P1.1 新消息 wire 形。

interface WireIceParams { username_fragment: string; password: string }
interface WireDtlsParams { fingerprints: { algorithm: string; value: string }[]; role: string }
// 服务端 IceCandidate 结构体 rename_all=camelCase（candidateType）；兼容手写 candidate_type。
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
  // iceLite 恒真 = mediasoup server 传输的设计不变量（worker 侧 ICE-Lite）；server wire 的
  // IceParameters 不带该旗，handler 据 iceParameters.iceLite 在 remote SDP 输出 a=ice-lite——
  // 缺失则浏览器按 full-ICE 对待对端，STUN 检查方向/时机错位（A/B 抓包定位：req 95/resp 0）。
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
/// F2: 连续零增长 tick 数（2s/tick）判「源离线」——≈6s，保守于 fps 最慢流且不早于任何信令自愈。
const STALL_TICKS = 3;
/// W2: 首帧轮次 watchdog——30s 无首帧计一轮失败，restartStream 最多 3 轮，耗尽→「源离线」等待。
const PLAY_WATCHDOG_MS = 30000;
const PLAY_ROUNDS_MAX = 3;
/// PIT-76 观测：logT 默认静默（T1.2 调试输出清账），localStorage 开关 `mediaservo_sfu_debug=1` 还原。
const SFU_DEBUG = typeof localStorage !== 'undefined' && localStorage.getItem('mediaservo_sfu_debug') === '1';

/// W4: 错误码分流——auth/授权族=终态（红牌唯一合法源）；4031（权限可热改，C33）、
/// 5000（SFU 内部失败，server 重启窗口典型码）、5001（网关上游切换，host 侧发射经转发）=可重试。
/// P1/T1.2 有意变更（design §1）：4012（控制 DC 拒权，F8 role 门显式拒）入 terminal 族。
export function classifySfuError(code: number): 'terminal' | 'retry' {
  switch (code) {
    case 4000: case 4001: case 4002: case 4003: case 4010: case 4011: case 4012: case 4101:
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
  // P1 换心：媒体面归 mediasoup-client（Transport 自带 getStats/connectionstatechange）。
  private device: Device | null = null;
  private transport: MsTransport | null = null;
  private consumer: Consumer | null = null;
  // v2 (web-stream-stats 修复): 双数据源合并累加器 — getStats 与 encoder_status 交替
  // 覆盖导致面板闪烁（一会数值一会 "-"）; 统一合并后回调
  private mergedMetrics: StreamMetrics | null = null;
  // 码率增量计算: 累计 bytesReceived 当瞬时值是单位错误根因
  private lastBytes = 0;
  private mutedTicks = 0; // H1: muted 连续 tick 计数（2s tick）
  // F2: 媒体新鲜度 watchdog（主流双保险第二柱——信令全丢也不假 LIVE）
  private playingSeen = false;
  /** S0: 与 server 协商谈成的方言版本（room_joined.protocol；缺省 = 1 旧 server）。 */
  private negotiatedProtocol = 1;

  /** S0 协商结果的公开读面（控制 DC 能力位/S0.5 resume 门消费）。 */
  get negotiated(): number {
    return this.negotiatedProtocol;
  }
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
  /** SFU 模式标志: startPlay 发出 create_web_rtc_transport 即置位——
      房间内历史 P2P 协商广播（sdp/rtc_ice_candidate）一律静默忽略。 */
  private sfuMode = false;
  // PIT-65: 每连接唯一 SFU peer_id — 多网页同 peer_id 导致 SfuManager recv_transport 互相覆盖
  private sfuPeerId: string;
  private transportResolver: ((params: TransportCreated) => void) | null = null;
  private pendingProducer: { producer_id: string; kind: string } | null = null;
  // P1 换心 RPC 单发旁路（handleMessage 优先消费；一次一个在途请求——协商本就串行）：
  private capsResolver: ((caps: unknown) => void) | null = null;
  private consumeResolver: ((msg: { consumer_id: string; producer_id: string; kind: string; rtp_parameters: unknown }) => void) | null = null;
  /** H1: producer_closed 自愈重入守卫（广播风暴/连续消息只 restart 一次）。 */
  private restarting = false;
  private metricsTimer: ReturnType<typeof setInterval> | null = null;
  // PIT-76: 首帧/渲染时间戳观测 — 从 startPlay 起计时（SFU_DEBUG 开关，默认静默）
  private t0 = 0;
  private logT(msg: string): void {
    if (!SFU_DEBUG) return;
    const t = performance.now();
    console.info(`[T+${Math.round(t - this.t0)}ms] ${msg}`);
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
      protocol: 3, // S0.5: 声明本端方言上限（v3 = resume 域；server 取 min 谈成）
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
    this.sfuMode = true; // 进入 SFU 流程: 之后到达的 sdp/ice 广播一律忽略
    if (!this.ws) throw new Error('Not connected');

    this.t0 = performance.now();
    this.logT('startPlay: Device.load(routerRtpCapabilities)');

    // 换心第一步：P1.1 server 面拉 router caps → Device.load（官方协商序 C18；
    // 旧 videoRtpCapabilities 硬编码 PT 面自此消亡）。失败与协商失败同路径→轮次引擎。
    this.device = new Device();
    try {
      const caps = await this.rpcRouterRtpCapabilities();
      await this.device.load({ routerRtpCapabilities: caps as never });
    } catch (err) {
      this.device = null;
      const m = String((err as Error)?.message ?? err);
      if (m.startsWith('sfu-terminal:')) { this.device = null; throw err; }
      this.logT(`Device.load 失败 → 轮次推进: ${m}`);
      void this.playDeadlineHit();
      return;
    }

    // recv transport：server create → 选项映射 → device.createRecvTransport。
    // resolver 先挂后发（旧竞态教训）；3s 无响应=轮次失败（P2P 回退已随全 SFU 现实移除）。
    const sfuPromise = new Promise<TransportCreated | null>((r) => { this.transportResolver = r; });
    this.logT('发送 create_web_rtc_transport');
    this.ws.send(JSON.stringify({ type: 'create_web_rtc_transport', room_id: this.roomId, peer_id: this.sfuPeerId, direction: 'recv' }));
    this.armPlayWatchdog(); // W2: 首帧轮次截止
    const sfuResult = await Promise.race([
      sfuPromise,
      new Promise<null>((r) => setTimeout(() => r(null), 3000)),
    ]);
    if (!sfuResult) {
      this.transportResolver = null;
      this.logT('SFU create 无响应 → 轮次推进');
      void this.playDeadlineHit();
      return;
    }

    this.logT('收到 web_rtc_transport_created, id: ' + sfuResult.transport_id);
    this.transportId = sfuResult.transport_id;
    const transport = this.device.createRecvTransport({
      id: sfuResult.transport_id,
      iceParameters: mapIceParameters(sfuResult.ice_parameters),
      iceCandidates: mapIceCandidates(sfuResult.ice_candidates ?? []) as never,
      dtlsParameters: mapDtlsParameters(sfuResult.dtls_parameters),
      // sctpParameters 省略——P1 无浏览器 SFU-DC（控制面随 P2 host-controller 迁移接入）。
    });
    this.transport = transport;
    // PIT-64 观测面（headless 调试）：debug 开关下暴露 transport。
    if (SFU_DEBUG) ((window as unknown as { __sfuTransports?: MsTransport[] }).__sfuTransports ??= []).push(transport);

    // 'connect' 事件 = mediasoup-client 生成本地 DTLS 指纹（旧手拼 answer 提取指纹的 PIT-56 债群消亡）。
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
    // ICE/DTLS 汇聚状态（旧 pc.oniceconnectionstatechange 语义等价迁移）
    let iceEver = false; // 曾连通标记——区分"初次建联失败"与"已连通后断开"语义
    transport.on('connectionstatechange', (state) => {
      this.logT(`connectionstatechange = ${state}`);
      if (state === 'connected') iceEver = true;
      if (state === 'failed') {
        this.onStatus(iceEver ? 'disconnected' : 'error');
        this.stopMetrics();
      } else if (state === 'disconnected' && iceEver) {
        // 建联完成前的瞬态忽略——30s 建联 watchdog 兜底
        this.onStatus('disconnected');
        this.stopMetrics();
      }
    });

    // transport 就绪：排队 producer（transport 前到达）即刻消费；后续 new_producer 走 handleMessage。
    if (this.pendingProducer) {
      const p = this.pendingProducer;
      this.pendingProducer = null;
      void this.consumeProducer(p.producer_id, p.kind);
    }
  }

  /** get_router_rtp_capabilities → router_rtp_capabilities 单发旁路（handleMessage 路由）。 */
  private rpcRouterRtpCapabilities(): Promise<unknown> {
    const ws = this.ws;
    if (!ws) return Promise.reject(new Error('Not connected'));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.capsResolver = null; reject(new Error('router caps timeout')); }, 5000);
      this.capsResolver = (caps) => { clearTimeout(timer); this.capsResolver = null; resolve(caps); };
      ws.send(JSON.stringify({ type: 'get_router_rtp_capabilities', room_id: this.roomId }));
    });
  }

  /** consume 请求+consumed 响应（server Consumer 落参）→ 本地 transport.consume 落轨
   *  （协商完成点 = 旧 ontrack 语义锚）。rtp_capabilities 用 Device 协商结果——
   *  旧 PIT-55 完整 codec 字段手拼债随换心消亡。 */
  private async consumeProducer(producerId: string, kind: string): Promise<void> {
    const transport = this.transport;
    const device = this.device;
    const ws = this.ws;
    if (!transport || !device || !ws) return;
    this.logT(`发送 consume（producer=${producerId}）`);
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
      this.logT('consume 超时 → 轮次推进');
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

  /** v2: 合并式 metrics 上报 — 部分字段只覆盖, 不重置其他字段（闪烁修复） */
  private emitMetrics(partial: Partial<StreamMetrics>): void {
    this.mergedMetrics = { ...(this.mergedMetrics ?? { rtt: 0, packetLoss: 0, fps: 0, bitrate: 0, jitter: 0, resolution: '' }), ...partial };
    this.onMetrics(this.mergedMetrics);
  }

  handleMessage(data: string): void {
    try {
      const msg = JSON.parse(data);

      // S0: 协商结果落存（缺 protocol = v1）——控制 DC/(未来)resume 门控读点。
      if (msg.type === 'room_joined' && typeof msg.protocol === 'number') this.negotiatedProtocol = msg.protocol;

      // RPC 单发旁路优先（caps/consumed）——消费后即摘。
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
        // W2: 「源离线等待」态收到 producer 信号 → 唤醒重开轮次预算（全量重走协商，含 late-join）。
        if (this.waitingForProducer) {
          this.waitingForProducer = false;
          this.playRounds = 0;
          this.logT('new_producer → 唤醒源离线等待');
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
        // S4′: data producer（遥控 DC 域）死亡与视频流无关——不触发媒体面 restart，
        // 否则每次舱端/车端 DC 翻转全 dashboard 假重启。
        if (msg.kind === 'data') {
          this.logT('producer_closed(data) — 媒体面无涉，忽略');
        } else if ((this.sfuMode || this.waitingForProducer) && !this.closed && !this.restarting) {
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
      } else if (msg.type === 'error' && msg.code === 0) {
        // transport_connected / RPC ack——协商节拍由 consumed/track 驱动，此处仅观测。
        this.logT('ack code:0 ' + msg.message);
      } else if (msg.type === 'error') {
        // W4 (C16 客户端合规): server 失败不得静默——分类驱动状态机。
        const code = Number(msg.code) || 0;
        console.warn('SfuClient: error', code, msg.message);
        if (classifySfuError(code) === 'terminal') {
          this.clearPlayWatchdog();
          this.onStatus('error');
        } else if (!this.playingSeen && !this.waitingForProducer && !this.restarting) {
          this.logT(`可重试错误 ${code} → 轮次推进`);
          void this.playDeadlineHit();
        }
      } else if (msg.type === 'sdp' || msg.type === 'rtc_ice_candidate' || msg.type === 'r_t_c_ice_candidate') {
        // 全 SFU 决策（2026-08-25）后 server 丢弃 P2P 中继——房间内历史协商广播静默忽略（噪音）。
      }
    } catch (err) {
      console.warn('SfuClient: message handling failed', err);
    }
  }

  // startMetrics polls Transport.getStats()（= underlying RTCPeerConnection 原始报告）every 2s
  private startMetrics(): void {
    this.stopMetrics();
    this.metricsTimer = setInterval(async () => {
      const transport = this.transport;
      if (!transport) return;
      // H1 兜底观测: producer 中途死且 ProducerClosed 不可达（H3 worker 通知静默）→ muted 持续 10s 告警
      const track = this.consumer?.track;
      if (track?.muted) {
        this.mutedTicks++;
        if (this.mutedTicks === 5) console.warn('SfuClient: video track muted ≥10s — producer 可能已死且无 ProducerClosed 通知');
      } else {
        this.mutedTicks = 0;
      }
      try {
        const stats = await transport.getStats();
        let rtt = 0, packetsLost = 0, packetsReceived = 0, fps = 0, bitrate = 0, jitter = 0;
        let width = 0, height = 0, decoderImpl: string | undefined, decoderCodec: string | undefined;
        let growing = false; // F2: 本 tick 字节增量即媒体新鲜信号

        stats.forEach((report) => {
          if (report.type === 'candidate-pair' && report.state === 'succeeded') {
            rtt = Math.round((report as unknown as { currentRoundTripTime?: number }).currentRoundTripTime ?? 0 * 1000) || 0;
          }
          // v2 (解码器修复): headless shell inbound-rtp 无 decoderImplementation 字段 →
          // 降级用 codec 报告 mimeType（浏览器实际解码格式）
          if (report.type === 'codec') {
            const mime = (report as unknown as { mimeType?: string }).mimeType;
            if (mime?.startsWith('video/')) decoderCodec = mime;
          }
          if (report.type === 'inbound-rtp' && report.kind === 'video') {
            const r = report as RTCInboundRtpStreamStats & { decoderImplementation?: string };
            packetsLost = r.packetsLost || 0;
            packetsReceived = r.packetsReceived || 0;
            fps = r.framesPerSecond || 0;
            // v2 (单位修复): 码率 = 字节增量/时间窗（累计 bytesReceived 当瞬时值 → 数字虚增）
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
            decoderImpl = r.decoderImplementation || undefined; // v2 T4
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
   *  （堵住"producer 恰在轮次窗口内上线错过广播"的死锁），后续 new_producer/producer_closed 唤醒。 */
  private async playDeadlineHit(): Promise<void> {
    if (this.closed || this.playingSeen || this.restarting || this.waitingForProducer) return;
    this.playRounds++;
    if (this.playRounds >= PLAY_ROUNDS_MAX) {
      this.waitingForProducer = true;
      this.ws?.send(JSON.stringify({ type: 'room_join', room_id: this.roomId, peer_role: 'consumer', protocol: 3 }));
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
            console.error('SfuClient: auth 终态，停止重试:', m);
            this.onStatus('error');
            return;
          }
          console.warn(`SfuClient: reconnect attempt ${attempt} failed (keep retrying):`, m);
          // connect 失败时若 play-watchdog 未挂（无首帧在途）→ 重启轮次引擎，防"无限 connecting 无退出"死锁
          if (!this.playTimer && !this.restarting) void this.playDeadlineHit();
          continue;
        }
      }
    } finally {
      this.reconnecting = false;
    }
  }

  /** W1: 直连重跑——旧 socket 振荡已由 connect() 内「摘 handlers + close」消除；server 宕机时
   *  connect 毫秒级快拒（onerror→WS error），半死时 Auth timeout 10s，退避循环全吸收。 */
  private async connectAndDrive(): Promise<void> {
    await this.connect();
    if (this.closed) return;
    this.logT('reconnected successfully');
    if (this.sfuMode) void this.restartStream();
    else void this.startPlay();
  }

  /**
   * H1: producer_closed 自愈 — 拆媒体面（transport/device 全弃重造）后重跑完整 startPlay
   * （等价页面刷新，但保留用户视角无感）。重发 room_join 触发 server late-join 回放 existing
   * producers（否则 host 先于本端完成 re-produce 时无 new_producer 广播可收）。
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
      // PIT-65: 新 peer_id——旧 SfuPeer 残留随 server 端 producer 死亡已失效。
      this.sfuPeerId = `${this.roomId}-consumer-${Math.random().toString(36).slice(2, 8)}`;
      this.onStatus('connecting');
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        await this.connect(); // WS 也断了（罕见）→ 先全量重连
      }
      this.ws?.send(JSON.stringify({
        type: 'room_join', room_id: this.roomId, peer_role: 'consumer', protocol: 3,
      }));
      this.playingSeen = false; this.stalled = false; this.stallTicks = 0; // F2: 重置 watchdog 至新首帧
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
 * P1/T1.2 上行最小面（会议麦根，D270-R2 验收「会议」的 SDK 前置）：
 * send transport + produce(track)。server 门 = audio-* 房账号豁免（D-H11 修订/P1.1 已落）。
 * 控制面 data-produce 随 P2 host-controller SFU-DC 迁移接入（design §4）——本类不掺。
 */
export class SfuMicProducer {
  private ws: WebSocket | null = null;
  private transport: MsTransport | null = null;
  /** 响应类型键 → 一次性 resolver（auth/join/caps/transport/produced 串行流程够用）。 */
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
    // error code:0 = ack（transport_connected/produced 例外——produced 是独立 type）；终态错误打断在途 RPC。
    if (m.type === 'error' && Number(m.code) !== 0 && classifySfuError(Number(m.code)) === 'terminal') {
      for (const [, cb] of this.pending) { void cb; }
      this.pending.clear(); // 语义：调用方以 timeout/抛错感知（本类流程短，接受 timeout 形）
    }
    const type = String(m.type ?? '');
    const cb = this.pending.get(type);
    if (cb) { this.pending.delete(type); cb(m); }
  }

  /** 完整上行链：auth → join → caps → load → send transport → produce(track)。 */
  async produceAudio(track: MediaStreamTrack): Promise<void> {
    const protocol = this.serverUrl.startsWith('wss:') ? 'wss:' : 'ws:';
    const host = this.serverUrl.replace(/^wss?:\/\//, '');
    const ws = new WebSocket(`${protocol}//${host}/ws`, this.token ? [this.token] : []);
    this.ws = ws;
    ws.onmessage = (ev) => {
      try { this.route(JSON.parse(String(ev.data))); } catch { /* 忽略非 JSON */ }
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

  /** 撤麦（幂等）：关 producer 所在 transport 与 WS——会议 P4a 若需细粒度改走 producer.pause()。 */
  close(): void {
    this.transport?.close();
    this.transport = null;
    if (this.ws) { this.ws.onmessage = null; this.ws.close(); this.ws = null; }
    this.pending.clear();
  }
}
