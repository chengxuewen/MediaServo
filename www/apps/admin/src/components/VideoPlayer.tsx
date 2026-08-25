import { useRef, useEffect, useState } from 'react';
import { SfuConsumerClient, type StreamMetrics } from '../sfu/sfu-client';
import './VideoPlayer.css';

interface Props {
  roomId: string;
  serverUrl: string;
  token: string;
  onClose: () => void;
  /** modal=全屏浮层（默认）；tile=网格单元（多流并排，multi-stream P3） */
  variant?: 'modal' | 'tile';
}

type ConnectionStatus = 'connecting' | 'connected' | 'playing' | 'disconnected' | 'error';

const STATUS_COLORS: Record<ConnectionStatus, string> = {
  connecting: '#f39c12',
  connected: '#27ae60',
  playing: '#27ae60',
  disconnected: '#e74c3c',
  error: '#e74c3c',
};

export default function VideoPlayer({ roomId, serverUrl, token, onClose, variant = 'modal' }: Props) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const clientRef = useRef<SfuConsumerClient | null>(null);
  const controlsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('connecting');
  const [metrics, setMetrics] = useState<StreamMetrics | null>(null);
  const [showStats, setShowStats] = useState(false);
  const [showControls, setShowControls] = useState(false);

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
    <div
      className={`video-player-overlay${isTile ? ' tile' : ''}`}
      onClick={isTile ? undefined : onClose}
    >
      <div
        className={`video-player ${isDisconnected ? 'disconnected' : ''}${isTile ? ' tile' : ''}`}
        onClick={(e) => e.stopPropagation()}
        onMouseMove={handleMouseMove}
        onMouseLeave={() => setShowControls(false)}
      >
        {/* Top bar */}
        <div className="vp-top-bar" style={{ background: isDisconnected ? '#c0392b' : '#2c3e50' }}>
          <span className="vp-status-dot" style={{ background: statusColor }} />
          <span className="vp-title">{roomId}</span>
          {metrics && !isDisconnected && (
            <span className="vp-bitrate">{metrics.resolution} · {Math.round(metrics.bitrate)}Kbps</span>
          )}
          <button className="vp-close" onClick={onClose}>✕</button>
        </div>

        {/* Video */}
        <div className="vp-body">
          <video ref={videoRef} autoPlay playsInline muted />
          {status === 'connecting' && <div className="vp-status-msg">Connecting...</div>}
          {isDisconnected && <div className="vp-status-msg error">Signal Lost</div>}
        </div>

        {/* Controls (hover) */}
        {showControls && !isDisconnected && (
          <div className="vp-controls">
            <button title="Mute">🔇</button>
            <button title="Pause">⏸</button>
            <button title="Fullscreen" onClick={() => videoRef.current?.requestFullscreen()}>⛶</button>
          </div>
        )}

        {/* Metrics bar */}
        {metrics && !isDisconnected && (
          <div className="vp-metrics-bar" onClick={() => setShowStats(!showStats)}>
            <span>⚡{metrics.rtt}ms</span>
            <span>📦{metrics.packetLoss}%</span>
            <span>🎬{metrics.fps}fps</span>
          </div>
        )}
        {isDisconnected && (
          <div className="vp-metrics-bar disconnected">
            <span>⚠️ Connection Lost</span>
          </div>
        )}

        {/* Detail stats panel — v2 (web-stream-stats T5): ToDesk 风格分组 */}
        {showStats && metrics && (
          <div className="vp-stats-panel" onClick={(e) => e.stopPropagation()}>
            <h4>Stream Stats</h4>
            {/* 连接质量 */}
            <h5>连接质量</h5>
            <div className="stats-grid">
              <div><label>帧率</label><span>{metrics.fps}fps</span></div>
              <div><label>延时</label><span>{metrics.rtt}ms</span></div>
              <div><label>丢包</label><span>{metrics.packetLoss}%</span></div>
              <div><label>码率</label><span>{Math.round(metrics.bitrate)}Kbps</span></div>
              <div><label>抖动</label><span>{metrics.jitter}ms</span></div>
              <div><label>分辨率</label><span>{metrics.resolution}</span></div>
            </div>
            {/* 编解码器（Host EncoderStatus + 浏览器解码器） */}
            <h5>编解码器</h5>
            <div className="stats-grid">
              <div><label>编码</label><span>{metrics.encoderImplementation ? (metrics.encoderImplementation.toLowerCase().includes('libvpx') || metrics.encoderImplementation.toLowerCase().includes('openh264') || metrics.encoderImplementation.toLowerCase().includes('libaom') ? '软编' : '硬编') : (metrics.encoderBackend === 'software' ? '软编' : metrics.encoderBackend && metrics.encoderBackend !== 'auto' ? '硬编' : '未知')}</span></div>
              <div><label>实际编码器</label><span>{metrics.encoderImplementation || metrics.encoderBackend || '—'}</span></div>
              <div><label>编码模式</label><span>{metrics.codec ? metrics.codec.replace('video/', '') : '—'}</span></div>
              <div><label>解码器</label><span>{metrics.decoderImplementation || metrics.decoderCodec || '—'}</span></div>
              <div><label>色度采样</label><span>4:2:0</span></div>
              <div><label>HDR</label><span>已关闭</span></div>
            </div>
            {/* 系统性能（P3 占位） */}
            <h5>系统性能</h5>
            <div className="stats-grid">
              <div><label>Host 帧率</label><span>{metrics.hostFps ? `${metrics.hostFps}fps` : '—'}</span></div>
              <div><label>Host 分辨率</label><span>{metrics.hostResolution || '—'}</span></div>
              <div><label>平均编码耗时</label><span>{metrics.avgEncodeMs != null ? `${metrics.avgEncodeMs.toFixed(1)}ms/帧` : '—'}</span></div>
              <div><label>CPU/GPU</label><span>待 P3</span></div>
            </div>
            <button className="vp-stats-close" onClick={() => setShowStats(false)}>✕</button>
          </div>
        )}
      </div>
    </div>
  );
}
