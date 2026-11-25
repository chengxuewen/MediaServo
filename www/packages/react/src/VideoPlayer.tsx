import { useRef, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { SfuConsumerClient, type StreamMetrics } from '@mediaservo/client';
import { Modal } from './Modal';
import './VideoPlayer.css';
import { X, VolumeX, Volume2, Pause, Maximize, Zap, Package, Clapperboard } from 'lucide-react';

// P3 slice ③a (admin-prototype-restyle): extracted verbatim from apps/admin
// components/VideoPlayer.tsx so admin + monitor share ONE player (reuse-first).
// The component was already decoupled from admin (SDK + lucide + CSS only); the
// react package already ships portal/DOM components (Modal, PIT-269), so D4
// headless-friendly is not a blocker. Behavior unchanged.

export interface VideoPlayerProps {
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
  /** V1/K9 (additive): NVR tile chrome — radius0/1px border/letterbox, hover top-right
   *  actions, LIVE badge, mono stats. Default off; admin unaffected. */
  nvr?: boolean;
  /** V1/K9: wall-internal single-stream zoom (nvr tile action). */
  onZoom?: () => void;
  /** V1/K9: remove this stream from the wall (nvr tile action). */
  onRemove?: () => void;
  /** V6 (I3): toggle 2x2 span (elastic layouts only); size-b action. */
  onSpan?: () => void;
  spanned?: boolean;
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

export default function VideoPlayer({ roomId, serverUrl, token, streamLabel, onClose, variant = 'modal', onExpand, nvr = false, onZoom, onRemove, onSpan, spanned = false }: VideoPlayerProps) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const clientRef = useRef<SfuConsumerClient | null>(null);
  const controlsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // S1 fix: first-frame poll + 30s cap timeout tracked so unmount cleans them up
  // (cap16 frequent tile open/close otherwise leaks timers up to 30s).
  const firstFramePollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const firstFrameCapRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [status, setStatus] = useState<ConnectionStatus>('connecting');
  const [metrics, setMetrics] = useState<StreamMetrics | null>(null);
  const [showStats, setShowStats] = useState(false);
  const [showControls, setShowControls] = useState(false);
  // C1: audio-monitor mute is a real control (video starts muted for autoplay).
  const [muted, setMuted] = useState(true);
  const rootRef = useRef<HTMLDivElement>(null);
  // V6 (I4): region drag-box zoom on the nvr tile; double-click resets.
  const bodyRef = useRef<HTMLDivElement>(null);
  const rzStart = useRef<{ x: number; y: number } | null>(null);
  const [rz, setRz] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const [rzView, setRzView] = useState<{ scale: number; ox: number; oy: number } | null>(null);
  const rzDown = (e: React.PointerEvent) => {
    if (!nvr || !isTile) return;
    // don't capture pointer on tile-action buttons (would swallow their click)
    if ((e.target as HTMLElement).closest('button')) return;
    const r = bodyRef.current!.getBoundingClientRect();
    rzStart.current = { x: e.clientX - r.left, y: e.clientY - r.top };
    setRz({ x: rzStart.current.x, y: rzStart.current.y, w: 0, h: 0 });
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const rzMove = (e: React.PointerEvent) => {
    if (!rzStart.current) return;
    const r = bodyRef.current!.getBoundingClientRect();
    const cx = e.clientX - r.left, cy = e.clientY - r.top;
    setRz({ x: Math.min(cx, rzStart.current.x), y: Math.min(cy, rzStart.current.y), w: Math.abs(cx - rzStart.current.x), h: Math.abs(cy - rzStart.current.y) });
  };
  const rzUp = () => {
    if (rz && rz.w >= 24 && rz.h >= 24 && bodyRef.current) {
      const r = bodyRef.current.getBoundingClientRect();
      const scale = Math.min(8, Math.max(r.width / rz.w, r.height / rz.h));
      setRzView({ scale, ox: rz.x + rz.w / 2, oy: rz.y + rz.h / 2 });
    }
    setRz(null); rzStart.current = null;
  };
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
          if (firstFramePollRef.current) clearInterval(firstFramePollRef.current);
          if (firstFrameCapRef.current) clearTimeout(firstFrameCapRef.current);
          firstFramePollRef.current = setInterval(() => {
            const v = videoRef.current;
            if (v && v.videoWidth > 0) {
              logT('first frame rendered, videoWidth=' + v.videoWidth + 'x' + v.videoHeight);
              console.log(`[mount→first frame] ${Math.round(performance.now() - t0)}ms`);
              if (firstFramePollRef.current) clearInterval(firstFramePollRef.current);
              if (firstFrameCapRef.current) clearTimeout(firstFrameCapRef.current);
            }
          }, 100);
          firstFrameCapRef.current = setTimeout(() => {
            if (firstFramePollRef.current) clearInterval(firstFramePollRef.current);
          }, 30000); // 30s cap
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
      if (firstFramePollRef.current) clearInterval(firstFramePollRef.current);
      if (firstFrameCapRef.current) clearTimeout(firstFrameCapRef.current);
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
  const playerInner = (
      <div
        ref={rootRef}
        className={`video-player ${isDisconnected ? 'disconnected' : ''}${isTile ? ' tile' : ''}${nvr && isTile ? ' nvr' : ''}`}
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={isTile ? (nvr ? () => setRzView(null) : onExpand) : undefined}
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
        <div className="vp-body" ref={bodyRef} onPointerDown={rzDown} onPointerMove={rzMove} onPointerUp={rzUp}>
          <video ref={videoRef} autoPlay playsInline muted style={rzView ? { transform: `scale(${rzView.scale})`, transformOrigin: `${rzView.ox}px ${rzView.oy}px` } : undefined} />
          {rz && <span className="zoom-rect" style={{ left: rz.x, top: rz.y, width: rz.w, height: rz.h }} />}
          {status === 'connecting' && <div className="vp-status-msg">Connecting…</div>}
          {isTile && (status === 'connected' || status === 'playing' || status === 'stalled') && (
            <MiniStatsCard status={status} metrics={metrics} onExpand={() => setShowStats(true)} />
          )}
          {nvr && isTile && (
            <div className="tile-actions">
              {onSpan && <button className={`tile-btn${spanned ? ' on' : ''}`} title="Toggle 2x2 span" aria-label="Toggle 2x2 span" aria-pressed={spanned} onClick={(e) => { e.stopPropagation(); onSpan(); }}><Maximize size={12} /></button>}
              {onZoom && <button className="tile-btn" title="Zoom this stream" aria-label="Zoom this stream" onClick={(e) => { e.stopPropagation(); onZoom(); }}><Maximize size={12} /></button>}
              {onRemove && <button className="tile-btn tile-del" title="Remove from wall" aria-label="Remove from wall" onClick={(e) => { e.stopPropagation(); onRemove(); }}><X size={12} /></button>}
            </div>
          )}
          {nvr && isTile && (
            <span className={`tile-live${status === 'playing' ? ' live' : ' off'}`}>{status === 'playing' ? '■ LIVE' : '□ OFF'}</span>
          )}
          {isDisconnected && <DisconnectedOverlay status={status} />}
        </div>

        {/* Controls (hover) */}
        {showControls && !isDisconnected && (
          <div className="vp-controls">
            <button title={muted ? 'Unmute audio monitor' : 'Mute audio monitor'} aria-pressed={!muted} onClick={() => { const v = videoRef.current; if (v) { v.muted = !v.muted; setMuted(v.muted); } }}>{muted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>
            <button title="Live stream — pause unavailable" aria-disabled="true" className="vp-ctrl-disabled"><Pause size={16} /></button>
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
  );

  // H3: modal variant uses the kit Modal (role=dialog / Esc / focus trap / LIFO
  // restore, PIT-269) styled cinema-wide; tile variant stays a grid cell.
  return (
    <>
      {isTile ? (
        <div className="video-player-overlay tile">{playerInner}</div>
      ) : (
        <Modal open title={streamLabel ? `${roomId} · ${streamLabel}` : roomId} onClose={onClose} className="vp-modal-wide">{playerInner}</Modal>
      )}
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
            <div><label>Chroma</label><span>—</span></div>
            <div><label>HDR</label><span>—</span></div>
          </div>
          <h5>System performance</h5>
          <div className="stats-grid">
            <div><label>Host FPS</label><span>{metrics.hostFps ? `${metrics.hostFps}fps` : '—'}</span></div>
            <div><label>Host resolution</label><span>{metrics.hostResolution || '—'}</span></div>
            <div><label>Avg encode time</label><span>{metrics.avgEncodeMs != null ? `${metrics.avgEncodeMs.toFixed(1)}ms/frame` : '—'}</span></div>
            <div><label>CPU/GPU</label><span>—</span></div>
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