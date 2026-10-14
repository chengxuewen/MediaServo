import { useRef, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { SfuConsumerClient, type StreamMetrics } from '@mediaservo/client';
import './VideoPlayer.css';
import { X, VolumeX, Pause, Maximize, Zap, Package, Clapperboard } from 'lucide-react';

interface Props {
  roomId: string;
  serverUrl: string;
  token: string;
  /** producer-label (§12-1 device room + stream label): pass stream_id → only consume the producer with that label.
   *  Unset = legacy behavior (derived-room mode). */
  streamLabel?: string;
  onClose: () => void;
  /** modal = fullscreen overlay (default); tile = grid cell (multiple streams side by side, multi-stream P3) */
  variant?: 'modal' | 'tile';
  /** tile: double-click the video → open the large window externally (product option 2, 2026-09-02) */
  onExpand?: () => void;
}

type ConnectionStatus = 'connecting' | 'connected' | 'playing' | 'stalled' | 'disconnected' | 'error';

const STATUS_COLORS: Record<ConnectionStatus, string> = {
  connecting: '#f39c12',
  connected: '#27ae60',
  playing: '#27ae60',
  stalled: '#7f8c8d',
  disconnected: '#e74c3c',
  error: '#e74c3c',
};

export default function VideoPlayer({ roomId, serverUrl, token, streamLabel, onClose, variant = 'modal', onExpand }: Props) {
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

  // Detail popover (portal→body, floats beside the card, decoupled from tile size — product option 1, 2026-09-02):
  // anchored to the right of the player rect, flips left only if there is no space; closes on outside click/ESC; follows resize/scroll.
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
    // PIT-76: first-frame render timing observability
    const t0 = performance.now();
    const logT = (msg: string) => console.log(`[T+${Math.round(performance.now() - t0)}ms] [VideoPlayer] ${msg}`);
    const client = new SfuConsumerClient(serverUrl, roomId, token, {
      onTrack: (stream) => {
        logT('onTrack received stream');
        if (videoRef.current) {
          videoRef.current.srcObject = stream;
          videoRef.current.play().catch(() => {});
          // first-frame detection: videoWidth > 0 when loadedmetadata fires
          videoRef.current.onloadedmetadata = () => logT('video loadedmetadata (videoWidth=' + videoRef.current?.videoWidth + ')');
          videoRef.current.onplaying = () => logT('video onplaying');
          // poll videoWidth to confirm the first frame actually rendered (timing kept inside the component — mount→first frame,
          // avoiding window.__playT0 cross-talk when multiple streams play, multi-stream P3)
          const poll = setInterval(() => {
            const v = videoRef.current;
            if (v && v.videoWidth > 0) {
              logT('first frame rendered, videoWidth=' + v.videoWidth + 'x' + v.videoHeight);
              console.log(`[mount→first frame] ${Math.round(performance.now() - t0)}ms`);
              clearInterval(poll);
            }
          }, 100);
          setTimeout(() => clearInterval(poll), 30000); // 30s cap
        }
      },
      onStatus: (s) => { logT('status = ' + s); setStatus(s); },
      onMetrics: setMetrics,
    }, streamLabel ? { expectedLabel: streamLabel } : undefined);

    clientRef.current = client;
    logT('connect() called');
    // W1: unlimited backoff reconnect (connecting shows no red banner, except auth terminal); W2: first-frame round watchdog moved down into SfuClient.
    client.connect().then(() => client.startPlay()).catch(() => { void client.reconnect(); });

    return () => {
      client.close();
      clientRef.current = null;
      if (controlsTimerRef.current) clearTimeout(controlsTimerRef.current);
    };
  }, [roomId, serverUrl, token, streamLabel]);

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
          <span className="vp-title">{streamLabel ? `${roomId} · ${streamLabel}` : roomId}</span>
          {!isTile && metrics && !isDisconnected && (
            <span className="vp-bitrate">{metrics.resolution} · {Math.round(metrics.bitrate)}Kbps</span>
          )}
          <button className="vp-close" onClick={onClose}><X size={14} /></button>
        </div>

        {/* Video */}
        <div className="vp-body">
          <video ref={videoRef} autoPlay playsInline muted />
          {status === 'connecting' && <div className="vp-status-msg">Connecting…</div>}
          {isTile && (status === 'connected' || status === 'playing' || status === 'stalled') && (
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

        {/* Metrics bar — modal-only (the tile quick-view entry moved into the top-left mini card; disconnect display goes through the overlay) */}
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
          <h4>Stream Details · {streamLabel ? `${roomId} · ${streamLabel}` : roomId}</h4>
          <h5>Codecs</h5>
          <div className="stats-grid">
            <div><label>Encode</label><span>{metrics.encoderImplementation ? (metrics.encoderImplementation.toLowerCase().includes('libvpx') || metrics.encoderImplementation.toLowerCase().includes('openh264') || metrics.encoderImplementation.toLowerCase().includes('libaom') ? 'Software' : 'Hardware') : (metrics.encoderBackend === 'software' ? 'Software' : metrics.encoderBackend && metrics.encoderBackend !== 'auto' ? 'Hardware' : 'Unknown')}</span></div>
            <div><label>Actual encoder</label><span>{metrics.encoderImplementation || metrics.encoderBackend || '—'}</span></div>
            <div><label>Encode format</label><span>{metrics.codec ? metrics.codec.replace('video/', '') : '—'}</span></div>
            <div><label>Decoder</label><span>{metrics.decoderImplementation || metrics.decoderCodec || '—'}</span></div>
            <div><label>Chroma</label><span>4:2:0</span></div>
            <div><label>HDR</label><span>Off</span></div>
          </div>
          <h5>System performance</h5>
          <div className="stats-grid">
            <div><label>Host FPS</label><span>{metrics.hostFps ? `${metrics.hostFps}fps` : '—'}</span></div>
            <div><label>Host resolution</label><span>{metrics.hostResolution || '—'}</span></div>
            <div><label>Avg encode time</label><span>{metrics.avgEncodeMs != null ? `${metrics.avgEncodeMs.toFixed(1)}ms/frame` : '—'}</span></div>
            <div><label>CPU/GPU</label><span>Pending P3</span></div>
          </div>
          <button className="vp-stats-close" onClick={() => setShowStats(false)}><X size={14} /></button>
        </div>, document.body)}
    </>
  );
}

function MiniStatsCard({ status, metrics, onExpand }: {
  status: ConnectionStatus; metrics: StreamMetrics | null; onExpand: () => void;
}) {
  // T1-a resolution: render as soon as connected, do not wait for the first getStats tick (2s); metrics null → placeholders.
  // Product refinement (adopted 2026-09-01): the core six metrics stay resident in a 2x3 grid; codec/system details live in the large panel behind a second click.
  const fmtKbps = (k: number) => (k >= 1000 ? `${(k / 1000).toFixed(1)}M` : `${Math.round(k)}K`);
  const live = status === 'playing';
  const offline = status === 'stalled'; // F2: link alive but media not growing — not a red screen, degrade to a gray badge
  const cells: Array<[string, string]> | null = metrics && (live || offline)
    ? [
        ['FPS', `${metrics.fps}fps`],
        ['Resolution', metrics.resolution === 'unknown' ? '—' : metrics.resolution],
        ['Bitrate', fmtKbps(metrics.bitrate)],
        ['Jitter', `${metrics.jitter}ms`],
        ['Latency', `${metrics.rtt}ms`],
        ['Loss', `${metrics.packetLoss}%`],
      ]
    : null;
  return (
    <div className="vp-mini-stats" onClick={(e) => { e.stopPropagation(); onExpand(); }} title={status === 'stalled' ? 'Source offline (waiting for the stream to recover or media stopped updating; click for details)' : 'Click for detailed stats'}>
      <div className="ms-head">
        <span className={`ms-dot ms-${status}`} />
        <span>{live ? 'LIVE' : offline ? 'Source offline' : '…'}</span>
        {!cells && <span className="ms-vals">Waiting for data…</span>}
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
  // Dual-state semantics (T7): error = connection failed / disconnected = stream interrupted — splits troubleshooting at a glance.
  const failed = status === 'error';
  return (
    <div className="vp-discon-overlay">
      <div className="vp-discon-card">
        <p className="vp-discon-title">{failed ? 'Connection failed' : 'Connection lost'}</p>
        <p className="vp-discon-sub">{failed ? 'Unable to establish the WebRTC connection' : 'The video stream was interrupted'}</p>
      </div>
    </div>
  );
}
