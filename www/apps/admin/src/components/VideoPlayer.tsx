import { useRef, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { SfuConsumerClient, type StreamMetrics } from '../sfu/sfu-client';
import './VideoPlayer.css';
import { X, VolumeX, Pause, Maximize, Zap, Package, Clapperboard } from 'lucide-react';

interface Props {
  roomId: string;
  serverUrl: string;
  token: string;
  onClose: () => void;
  /** modal=全屏浮层（默认）；tile=网格单元（多流并排，multi-stream P3） */
  variant?: 'modal' | 'tile';
  /** tile 双击画面→外部打开大窗（产品方案②，2026-09-02） */
  onExpand?: () => void;
}

type ConnectionStatus = 'connecting' | 'connected' | 'playing' | 'disconnected' | 'error';

const STATUS_COLORS: Record<ConnectionStatus, string> = {
  connecting: '#f39c12',
  connected: '#27ae60',
  playing: '#27ae60',
  disconnected: '#e74c3c',
  error: '#e74c3c',
};

export default function VideoPlayer({ roomId, serverUrl, token, onClose, variant = 'modal', onExpand }: Props) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const clientRef = useRef<SfuConsumerClient | null>(null);
  const controlsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('connecting');
  const [metrics, setMetrics] = useState<StreamMetrics | null>(null);
  const [showStats, setShowStats] = useState(false);
  const [showControls, setShowControls] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const [popPos, setPopPos] = useState<{ left: number; top: number } | null>(null);

  // 详情浮层（portal→body，浮在卡旁，与 tile 尺寸解耦——产品方案① 2026-09-02）：
  // 锚点=播放器矩形右侧，无空间翻左；点外部/ESC 收起；resize/scroll 跟随。
  useEffect(() => {
    if (!showStats) return;
    const place = () => {
      const a = rootRef.current?.getBoundingClientRect();
      if (!a) return;
      const W = 260, H = popRef.current?.offsetHeight ?? 180;
      let left = a.right + 8;
      if (left + W > window.innerWidth - 8) left = a.left - W - 8;
      if (left < 8) left = Math.max(8, window.innerWidth - W - 8);
      const top = Math.min(Math.max(8, a.top + 8), window.innerHeight - H - 8);
      setPopPos({ left, top });
    };
    place();
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (popRef.current?.contains(t) || rootRef.current?.contains(t)) return;
      setShowStats(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setShowStats(false); };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [showStats]);

  useEffect(() => {
    // PIT-76: 首帧渲染时间观测
    const t0 = performance.now();
    const logT = (msg: string) => console.log(`[T+${Math.round(performance.now() - t0)}ms] [VideoPlayer] ${msg}`);
    const client = new SfuConsumerClient(serverUrl, roomId, token, {
      onTrack: (stream) => {
        logT('onTrack 收到 stream');
        if (videoRef.current) {
          videoRef.current.srcObject = stream;
          videoRef.current.play().catch(() => {});
          // 首帧检测: loadedmetadata 触发时 videoWidth > 0
          videoRef.current.onloadedmetadata = () => logT('video loadedmetadata (videoWidth=' + videoRef.current?.videoWidth + ')');
          videoRef.current.onplaying = () => logT('video onplaying');
          // 轮询 videoWidth 确认首帧实际渲染（计时内聚组件——挂载→首帧，
          // 避免多路时 window.__playT0 串路，multi-stream P3）
          const poll = setInterval(() => {
            const v = videoRef.current;
            if (v && v.videoWidth > 0) {
              logT('首帧渲染确认 videoWidth=' + v.videoWidth + 'x' + v.videoHeight);
              console.log(`[挂载→首帧] ${Math.round(performance.now() - t0)}ms`);
              clearInterval(poll);
            }
          }, 100);
          setTimeout(() => clearInterval(poll), 30000); // 30s 上限
        }
      },
      onStatus: (s) => { logT('status = ' + s); setStatus(s); },
      onMetrics: setMetrics,
    });

    clientRef.current = client;
    logT('connect() 调用');
    client.connect().then(() => client.startPlay()).catch(() => setStatus('error'));

    return () => {
      client.close();
      clientRef.current = null;
      if (controlsTimerRef.current) clearTimeout(controlsTimerRef.current);
    };
  }, [roomId, serverUrl, token]);

  const handleMouseMove = () => {
    setShowControls(true);
    if (controlsTimerRef.current) clearTimeout(controlsTimerRef.current);
    controlsTimerRef.current = setTimeout(() => setShowControls(false), 2000);
  };

  const statusColor = STATUS_COLORS[status];
  const isDisconnected = status === 'disconnected' || status === 'error';

  const isTile = variant === 'tile';
  return (
    <>
    <div
      className={`video-player-overlay${isTile ? ' tile' : ''}`}
      onClick={isTile ? undefined : onClose}
    >
      <div
        ref={rootRef}
        className={`video-player ${isDisconnected ? 'disconnected' : ''}${isTile ? ' tile' : ''}`}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={isTile ? onExpand : undefined}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => setShowControls(false)}
      >
        {/* Top bar */}
        <div className="vp-top-bar" style={{ background: isDisconnected ? '#c0392b' : '#2c3e50' }}>
          <span className="vp-status-dot" style={{ background: statusColor }} />
          <span className="vp-title">{roomId}</span>
          {!isTile && metrics && !isDisconnected && (
            <span className="vp-bitrate">{metrics.resolution} · {Math.round(metrics.bitrate)}Kbps</span>
          )}
          <button className="vp-close" onClick={onClose}><X size={14} /></button>
        </div>

        {/* Video */}
        <div className="vp-body">
          <video ref={videoRef} autoPlay playsInline muted />
          {status === 'connecting' && <div className="vp-status-msg">Connecting...</div>}
          {isTile && (status === 'connected' || status === 'playing') && (
            <MiniStatsCard status={status} metrics={metrics} onExpand={() => setShowStats(true)} />
          )}
          {isDisconnected && <DisconnectedOverlay status={status} />}
        </div>

        {/* Controls (hover) */}
        {showControls && !isDisconnected && (
          <div className="vp-controls">
            <button title="Mute"><VolumeX size={16} /></button>
            <button title="Pause"><Pause size={16} /></button>
            <button title="Fullscreen" onClick={() => videoRef.current?.requestFullscreen()}><Maximize size={16} /></button>
          </div>
        )}

        {/* Metrics bar — modal-only（tile 速览入口已并入左上 mini 卡；断联展示统一走遮罩） */}
        {!isTile && metrics && !isDisconnected && (
          <div className="vp-metrics-bar" onClick={() => setShowStats(!showStats)}>
            <span><Zap size={12} /> {metrics.rtt}ms</span>
            <span><Package size={12} /> {metrics.packetLoss}%</span>
            <span><Clapperboard size={12} /> {metrics.fps}fps</span>
          </div>
        )}

      </div>
    </div>
      {showStats && metrics && createPortal(
        <div
          ref={popRef}
          className="vp-stats-pop"
          style={popPos ? { left: popPos.left, top: popPos.top } : { left: -9999, top: -9999 }}
          onClick={(e) => e.stopPropagation()}
        >
          <h4>Stream Details · {roomId}</h4>
          <h5>编解码器</h5>
          <div className="stats-grid">
            <div><label>编码</label><span>{metrics.encoderImplementation ? (metrics.encoderImplementation.toLowerCase().includes('libvpx') || metrics.encoderImplementation.toLowerCase().includes('openh264') || metrics.encoderImplementation.toLowerCase().includes('libaom') ? '软编' : '硬编') : (metrics.encoderBackend === 'software' ? '软编' : metrics.encoderBackend && metrics.encoderBackend !== 'auto' ? '硬编' : '未知')}</span></div>
            <div><label>实际编码器</label><span>{metrics.encoderImplementation || metrics.encoderBackend || '—'}</span></div>
            <div><label>编码模式</label><span>{metrics.codec ? metrics.codec.replace('video/', '') : '—'}</span></div>
            <div><label>解码器</label><span>{metrics.decoderImplementation || metrics.decoderCodec || '—'}</span></div>
            <div><label>色度采样</label><span>4:2:0</span></div>
            <div><label>HDR</label><span>已关闭</span></div>
          </div>
          <h5>系统性能</h5>
          <div className="stats-grid">
            <div><label>Host 帧率</label><span>{metrics.hostFps ? `${metrics.hostFps}fps` : '—'}</span></div>
            <div><label>Host 分辨率</label><span>{metrics.hostResolution || '—'}</span></div>
            <div><label>平均编码耗时</label><span>{metrics.avgEncodeMs != null ? `${metrics.avgEncodeMs.toFixed(1)}ms/帧` : '—'}</span></div>
            <div><label>CPU/GPU</label><span>待 P3</span></div>
          </div>
          <button className="vp-stats-close" onClick={() => setShowStats(false)}><X size={14} /></button>
        </div>, document.body)}
    </>
  );
}

function MiniStatsCard({ status, metrics, onExpand }: {
  status: ConnectionStatus; metrics: StreamMetrics | null; onExpand: () => void;
}) {
  // T1-a 消解：connected 即渲染，不等首个 getStats tick（2s）；metrics null → 占位。
  // 产品精化（2026-09-01 采纳）：核心六指标 2列×3行常驻，编解码/系统详情走大面板二次点击。
  const fmtKbps = (k: number) => (k >= 1000 ? `${(k / 1000).toFixed(1)}M` : `${Math.round(k)}K`);
  const live = status === 'playing';
  const cells: Array<[string, string]> | null = metrics && live
    ? [
        ['帧率', `${metrics.fps}fps`],
        ['分辨率', metrics.resolution === 'unknown' ? '—' : metrics.resolution],
        ['码率', fmtKbps(metrics.bitrate)],
        ['抖动', `${metrics.jitter}ms`],
        ['延时', `${metrics.rtt}ms`],
        ['丢包', `${metrics.packetLoss}%`],
      ]
    : null;
  return (
    <div className="vp-mini-stats" onClick={(e) => { e.stopPropagation(); onExpand(); }} title="点击展开详细统计">
      <div className="ms-head">
        <span className={`ms-dot ms-${status}`} />
        <span>{live ? 'LIVE' : '…'}</span>
        {!cells && <span className="ms-vals">等待数据…</span>}
      </div>
      {cells && (
        <div className="ms-grid">
          {cells.map(([k, v]) => (
            <div key={k} className="ms-cell"><span className="ms-k">{k}</span><span className="ms-v">{v}</span></div>
          ))}
        </div>
      )}
    </div>
  );
}

function DisconnectedOverlay({ status }: { status: ConnectionStatus }) {
  // 双态语义（T7）：error=建联失败 / disconnected=流中断——判障一眼分流。
  const failed = status === 'error';
  return (
    <div className="vp-discon-overlay">
      <div className="vp-discon-card">
        <p className="vp-discon-title">{failed ? '连接失败' : '连接已断开'}</p>
        <p className="vp-discon-sub">{failed ? '无法建立 WebRTC 连接' : '视频流已中断'}</p>
      </div>
    </div>
  );
}
