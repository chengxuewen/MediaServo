import { useCallback, useEffect, useRef, useState } from 'react';
import { VideoPlayer, Confirm } from '@mediaservo/react';
import { getToken } from '@mediaservo/client';
import StreamTree, { type StreamSelection } from '../components/StreamTree';
import { useStreams } from '../hooks/useStreams';
import { useAdminWS } from '../hooks/useAdminWS';
import { useAuth } from '../hooks/useAuth';
import Timeline from '../components/Timeline';
import './Home.css';

// Playback cap (browser soft-decode guard): 16 = a 4x4 grid scale (B5 contract).
const MAX_PLAYING = 16;
// Column control: default 3, max 4, persisted with dirty-value defense.
const PLAY_COLS_KEY = 'mediaservo_monitor_cols';
const MAX_COLS = 4;
const DEFAULT_COLS = 3;
// B5 micro-decision: Play All above 8 streams asks for confirmation.
const PLAY_ALL_CONFIRM = 8;
// B2 fix: a playing producer that vanishes from /sfu/stats is reconciled out of
// the wall after this grace (~3 poll cycles) so dead ids cannot fill cap16.
const DROP_GRACE_MS = 6000;
// B1: three-panel region collapse memory (left stream rail / right alerts / bottom feed).
const REGION_KEY = 'mediaservo_monitor_regions';

function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

function loadRegions(): { left: boolean; right: boolean; bottom: boolean } {
  try {
    const v = JSON.parse(localStorage.getItem(REGION_KEY) || '{}');
    return { left: v.left !== false, right: v.right !== false, bottom: v.bottom !== false };
  } catch { return { left: true, right: true, bottom: true }; }
}

/**
 * Monitor realtime face — B1 three-panel progressive disclosure (monitor-prototype-parity).
 * Left stream rail (selected-stream list) | center canvas (wall, always visible) |
 * right alerts placeholder (B4 fills it). Bottom collapsible Event feed (audit ring).
 * Region collapse is remembered; narrow screens (<1200) stack the panels. healthChip
 * surfaces degraded streams; a LIVE clock marks the realtime face. Playback stays the
 * shared VideoPlayer tile; Recording/playback scrubber deferred (RECORDING_READY=false).
 */
export default function Home() {
  const { username, role, canMonitor } = useAuth();
  const { producers, loading, error, refetch } = useStreams();
  const onWsEvent = useCallback(() => { refetch(); }, [refetch]);
  useAdminWS(onWsEvent);

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [playing, setPlaying] = useState<string[]>([]);
  const [modal, setModal] = useState<StreamSelection | null>(null);
  const [playHint, setPlayHint] = useState('');
  const [pendingPlay, setPendingPlay] = useState<{ room: string; ids: string[] } | null>(null);
  const [cols, setCols] = useState<number>(() => {
    let v: number;
    try { v = Number(localStorage.getItem(PLAY_COLS_KEY)); } catch { v = NaN; }
    return Number.isInteger(v) && v >= 1 && v <= MAX_COLS ? v : DEFAULT_COLS;
  });
  const applyCols = (n: number) => {
    setCols(n);
    try { localStorage.setItem(PLAY_COLS_KEY, String(n)); } catch { /* silent degrade */ }
  };

  // B1 region collapse (memory). Handles persist so a collapsed region can reopen.
  const [regions, setRegions] = useState(loadRegions);
  const toggleRegion = (k: 'left' | 'right' | 'bottom') => setRegions((prev) => {
    const next = { ...prev, [k]: !prev[k] };
    try { localStorage.setItem(REGION_KEY, JSON.stringify(next)); } catch { /* silent degrade */ }
    return next;
  });

  // B1 LIVE clock (realtime face marker).
  const [clock, setClock] = useState(() => new Date());
  useEffect(() => { const t = setInterval(() => setClock(new Date()), 1000); return () => clearInterval(t); }, []);

  const byId = new Map(producers.map((p) => [p.id, p]));
  // B1 healthChip: degraded = low transport score (stats only lists live producers).
  const degraded = producers.filter((p) => p.score < 8).length;

  // B2 fix: reconcile dead playing ids after a grace period.
  const missingSince = useRef<Map<string, number>>(new Map());
  useEffect(() => {
    const live = new Set(producers.map((p) => p.id));
    const now = Date.now();
    const missing = missingSince.current;
    for (const id of [...missing.keys()]) if (live.has(id)) missing.delete(id);
    for (const id of playing) if (!live.has(id) && !missing.has(id)) missing.set(id, now);
    const dead = playing.filter((id) => {
      const t = missing.get(id);
      return t !== undefined && now - t >= DROP_GRACE_MS;
    });
    if (dead.length > 0) {
      setPlaying((prev) => prev.filter((id) => !dead.includes(id)));
      for (const id of dead) missing.delete(id);
    }
  }, [producers, playing]);

  const toggleSelect = (id: string) => setSelected((prev) => {
    const next = new Set(prev);
    next.has(id) ? next.delete(id) : next.add(id);
    return next;
  });
  const toggleAll = (room: string, checked: boolean) => setSelected((prev) => {
    const next = new Set(prev);
    for (const p of producers) if (p.room === room) { checked ? next.add(p.id) : next.delete(p.id); }
    return next;
  });

  const addPlaying = (ids: string[]) => {
    const fresh = ids.filter((id) => !playing.includes(id));
    const merged = [...playing, ...fresh].slice(0, MAX_PLAYING);
    const dropped = fresh.length - (merged.length - playing.length);
    setPlaying(merged);
    setPlayHint(dropped > 0 ? `Playback cap of ${MAX_PLAYING} reached; skipped ${dropped} (close tiles to play more)` : '');
  };

  const playRoom = (room: string) => {
    const roomProducers = producers.filter((p) => p.room === room);
    const roomSelected = roomProducers.some((p) => selected.has(p.id));
    const ids = roomSelected
      ? roomProducers.filter((p) => selected.has(p.id)).map((p) => p.id)
      : roomProducers.map((p) => p.id);
    if (ids.length === 0) return;
    if (ids.length > PLAY_ALL_CONFIRM) { setPendingPlay({ room, ids }); return; }
    addPlaying(ids);
  };

  const closeTile = (id: string) => {
    setPlaying((prev) => prev.filter((x) => x !== id));
    setModal((prev) => (prev?.producerId === id ? null : prev));
  };

  return (
    <section className="monitor-home">
      <div className="mh-bar">
        <h1>Monitor</h1>
        <span className={`health-chip${degraded > 0 ? ' warn' : ''}`} role="status">
          {degraded > 0 ? `${degraded} degraded` : 'Healthy'}
        </span>
        <span className="live-clock"><span className="live-dot" aria-hidden="true" />LIVE {clock.toLocaleTimeString('en-GB', { hour12: false })}</span>
        <div className="region-toggles">
          <button aria-pressed={regions.left} onClick={() => toggleRegion('left')}>Streams</button>
          <button aria-pressed={regions.right} onClick={() => toggleRegion('right')}>Alerts</button>
          <button aria-pressed={regions.bottom} onClick={() => toggleRegion('bottom')}>Events</button>
        </div>
      </div>
      <p className="mh-note" role="note">
        Signed in as {username ?? 'unknown'}{role ? ` [${role}]` : ''}. {canMonitor ? '' : 'Wall access requires admin/dispatcher. '}Cap {MAX_PLAYING}; PSK host rooms are single-producer. Alert panel lands in B4; playback scrubber/Recording deferred.
      </p>

      <div className="monitor-three-panel">
        <aside className={`stream-rail${regions.left ? '' : ' collapsed'}`}>
          <div className="rail-head">
            <span className="rail-title">Selected streams</span>
            <button className="region-collapse" onClick={() => toggleRegion('left')} aria-label={regions.left ? 'Hide left panel' : 'Show left panel'}>{regions.left ? '‹' : '›'}</button>
          </div>
          <div className="rail-body">
            <StreamTree
              producers={producers}
              loading={loading}
              error={error}
              selected={selected}
              onToggleSelect={toggleSelect}
              onToggleAll={toggleAll}
              onPlay={playRoom}
              onPreview={setModal}
            />
          </div>
        </aside>

        <div className="canvas">
          {playing.length > 0 ? (
            <>
              <div className="wall-toolbar">
                <span className="wt-label">Columns</span>
                {[1, 2, 3, 4].map((n) => (
                  <button key={n} className={n === cols ? 'wt-btn active' : 'wt-btn'} aria-pressed={n === cols} onClick={() => applyCols(n)}>{n}</button>
                ))}
                {playHint && <span className="wt-hint" role="status">{playHint}</span>}
              </div>
              <div className="wall-grid" style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}>
                {playing.map((id) => {
                  const p = byId.get(id);
                  if (!p) return (
                    <div className="wall-offline" key={id}>
                      <span className="wo-title">SIGNAL LOST</span>
                      <span className="wo-reason">Producer left — stream ended or device offline</span>
                      <button className="wo-close" onClick={() => closeTile(id)}>Close</button>
                    </div>
                  );
                  return (
                    <VideoPlayer
                      key={id}
                      roomId={p.room}
                      serverUrl={wsServerUrl()}
                      token={getToken() ?? ''}
                      variant="tile"
                      onExpand={() => setModal({ room: p.room, producerId: p.id, kind: p.kind })}
                      onClose={() => closeTile(id)}
                    />
                  );
                })}
              </div>
            </>
          ) : (
            <div className="canvas-empty">No streams on the wall yet. Pick streams in the left rail and Play All / Watch Selected.</div>
          )}
        </div>

        <aside className={`mon-alerts${regions.right ? '' : ' collapsed'}`}>
          <div className="rail-head">
            <span className="rail-title">Alerts</span>
            <button className="region-collapse" onClick={() => toggleRegion('right')} aria-label={regions.right ? 'Hide right panel' : 'Show right panel'}>{regions.right ? '›' : '‹'}</button>
          </div>
          <div className="rail-body">
            <div className="alerts-placeholder">Alert panel (filter / unread / click-to-wall) lands in B4. Degraded streams surface in the health chip above.</div>
          </div>
        </aside>
      </div>

      <div className={`bottom-zone${regions.bottom ? '' : ' collapsed'}`}>
        <div className="rail-head">
          <span className="rail-title">Event feed</span>
          <button className="region-collapse" onClick={() => toggleRegion('bottom')} aria-label={regions.bottom ? 'Hide bottom panel' : 'Show bottom panel'}>{regions.bottom ? '⌄' : '⌃'}</button>
        </div>
        <div className="rail-body">
          <Timeline />
        </div>
      </div>

      {modal && (
        <VideoPlayer
          key={`modal-${modal.producerId}`}
          roomId={modal.room}
          serverUrl={wsServerUrl()}
          token={getToken() ?? ''}
          variant="modal"
          onClose={() => setModal(null)}
        />
      )}
      <Confirm
        open={pendingPlay !== null}
        title="Play all streams?"
        message={`This subscribes to ${pendingPlay?.ids.length ?? 0} streams in ${pendingPlay?.room ?? ''} (playback cap ${MAX_PLAYING}). Continue?`}
        confirmLabel="Play"
        cancelLabel="Cancel"
        onConfirm={() => { if (pendingPlay) addPlaying(pendingPlay.ids); setPendingPlay(null); }}
        onCancel={() => setPendingPlay(null)}
      />
    </section>
  );
}