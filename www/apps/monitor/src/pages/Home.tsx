import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { VideoPlayer, Confirm } from '@mediaservo/react';
import { getToken } from '@mediaservo/client';
import StreamTree, { type StreamSelection } from '../components/StreamTree';
import Picker from '../components/Picker';
import AlertPanel from '../components/AlertPanel';
import { useStreams } from '../hooks/useStreams';
import { useAdminWS } from '../hooks/useAdminWS';
import { useAuth } from '../hooks/useAuth';
import Timeline from '../components/Timeline';
import './Home.css';

// Playback cap (browser soft-decode guard): 16 = a 4x4 grid scale (B5 contract).
const MAX_PLAYING = 16;
const PLAY_ALL_CONFIRM = 8;
const DROP_GRACE_MS = 6000;
const REGION_KEY = 'mediaservo_monitor_regions';
// B2 (R1): named-layout persistence uses a NEW key; migrate the legacy cols key.
const LAYOUT_KEY = 'mediaservo_monitor_layout';
const LEGACY_COLS_KEY = 'mediaservo_monitor_cols';
// B2 min-tile width (prototype §2g): narrow screens horizontal-scroll, never shrink tiles to nothing.
const MIN_TILE_PX = 230;

interface Layout { id: string; name: string; cols: number; slots: number; areas?: string[]; }
const sym = (id: string, n: number): Layout => { const c = Math.round(Math.sqrt(n)); return { id, name: id, cols: c, slots: n }; };
// K4=A R6 精简集: symmetric 1/4/9/16 + asymmetric 1+3/1+5/1+7 (25/36/custom deferred D3).
const LAYOUTS: Layout[] = [
  sym('1', 1), sym('4', 4), sym('9', 9), sym('16', 16),
  { id: '1+3', name: '1 + 3', cols: 3, slots: 4, areas: ['big big s1', 'big big s2', 'big big s3'] },
  { id: '1+5', name: '1 + 5', cols: 4, slots: 6, areas: ['big big s1 s2', 'big big s3 s4', 'big big s5 .'] },
  { id: '1+7', name: '1 + 7', cols: 4, slots: 8, areas: ['big big s1 s2', 'big big s3 s4', 'big big s5 s6', 'big big s7 .'] },
];
const autoTemplateId = (n: number) => (n <= 1 ? '1' : n <= 4 ? '4' : n <= 6 ? '1+5' : n <= 9 ? '9' : '16');
const findLayout = (id: string) => LAYOUTS.find((l) => l.id === id) ?? LAYOUTS[1];
function loadLayout(): string {
  try {
    const v = localStorage.getItem(LAYOUT_KEY);
    if (v && (v === 'auto' || LAYOUTS.some((l) => l.id === v))) return v;
    const c = Number(localStorage.getItem(LEGACY_COLS_KEY));
    const map: Record<number, string> = { 1: '1', 2: '4', 3: '9', 4: '16' };
    if (map[c]) { localStorage.setItem(LAYOUT_KEY, map[c]); return map[c]; }
  } catch { /* silent degrade */ }
  return 'auto';
}
function loadRegions() {
  try {
    const v = JSON.parse(localStorage.getItem(REGION_KEY) || '{}');
    return { left: v.left !== false, right: v.right !== false, bottom: v.bottom !== false };
  } catch { return { left: true, right: true, bottom: true }; }
}
function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

/**
 * Monitor realtime face — B1 three-panel + B2 named layouts (monitor-prototype-parity).
 * Left stream rail | center canvas (wall) | right alerts placeholder (B4). Bottom Event feed.
 * B2: R6 layout set (1/4/9/16 + 1+3/1+5/1+7) + Auto-by-count via a single dropdown,
 * min-tile horizontal scroll, pager + carousel (default off), single-tile zoom (K5).
 */
export default function Home() {
  const { username, role, canMonitor } = useAuth();
  const { producers, loading, error, refetch } = useStreams();
  const onWsEvent = useCallback(() => { refetch(); }, [refetch]);
  useAdminWS(onWsEvent);

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [playing, setPlaying] = useState<string[]>([]);
  const [modal, setModal] = useState<StreamSelection | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [playHint, setPlayHint] = useState('');
  const [pendingPlay, setPendingPlay] = useState<{ room: string; ids: string[] } | null>(null);

  // B2 layout / pagination / carousel / zoom.
  const [layoutId, setLayoutId] = useState(loadLayout);
  const [page, setPage] = useState(0);
  const [carousel, setCarousel] = useState(false);
  const [zoomedId, setZoomedId] = useState<string | null>(null);
  const applyLayout = (id: string) => {
    setLayoutId(id); setPage(0); setZoomedId(null);
    try { localStorage.setItem(LAYOUT_KEY, id); } catch { /* silent degrade */ }
  };

  const [regions, setRegions] = useState(loadRegions);
  const toggleRegion = (k: 'left' | 'right' | 'bottom') => setRegions((prev) => {
    const next = { ...prev, [k]: !prev[k] };
    try { localStorage.setItem(REGION_KEY, JSON.stringify(next)); } catch { /* silent degrade */ }
    return next;
  });

  const [clock, setClock] = useState(() => new Date());
  useEffect(() => { const t = setInterval(() => setClock(new Date()), 1000); return () => clearInterval(t); }, []);

  const byId = new Map(producers.map((p) => [p.id, p]));
  const degraded = producers.filter((p) => p.score < 8).length;

  // B2 effective layout + pagination.
  const layout = layoutId === 'auto' ? findLayout(autoTemplateId(playing.length)) : findLayout(layoutId);
  const pages = Math.max(1, Math.ceil(playing.length / layout.slots));
  const curPage = Math.min(page, pages - 1);
  const visible = zoomedId ? [zoomedId] : playing.slice(curPage * layout.slots, curPage * layout.slots + layout.slots);
  useEffect(() => {
    if (!carousel || pages <= 1 || zoomedId) return;
    const t = setInterval(() => setPage((p) => (p + 1) % pages), 5000);
    return () => clearInterval(t);
  }, [carousel, pages, zoomedId]);

  const gridStyle: CSSProperties = zoomedId
    ? { gridTemplateColumns: '1fr' }
    : layout.areas
      ? { gridTemplateColumns: `repeat(${layout.cols}, minmax(${MIN_TILE_PX}px, 1fr))`, gridTemplateAreas: layout.areas.map((r) => `"${r}"`).join(' ') }
      : { gridTemplateColumns: `repeat(${layout.cols}, minmax(${MIN_TILE_PX}px, 1fr))` };
  const cellStyle = (idx: number): CSSProperties => (layout.areas && !zoomedId ? { gridArea: idx === 0 ? 'big' : `s${idx}` } : {});

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
    setZoomedId((prev) => (prev === id ? null : prev));
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
            <button className="open-picker" onClick={() => setPickerOpen(true)}>＋ Select streams</button>
            <StreamTree
              producers={producers}
              loading={loading}
              error={error}
              selected={selected}
              onToggleSelect={toggleSelect}
              onToggleAll={toggleAll}
              onPlay={playRoom}
              onPreview={setModal}
              onRetry={refetch}
            />
          </div>
        </aside>

        <div className="canvas">
          <div className="wall-toolbar">
                <label className="wt-label" htmlFor="layout-menu">Layout</label>
                <select id="layout-menu" className="layout-menu" value={layoutId} onChange={(e) => applyLayout(e.target.value)}>
                  <option value="auto">Auto · by count</option>
                  {LAYOUTS.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
                </select>
                {zoomedId && <button className="wt-exit-zoom" onClick={() => setZoomedId(null)}>Exit zoom</button>}
                {pages > 1 && !zoomedId && (
                  <div className="wt-pager">
                    <button onClick={() => setPage((p) => Math.max(0, p - 1))} disabled={curPage === 0} aria-label="Previous page">‹</button>
                    <span className="wt-page">{curPage + 1}/{pages}</span>
                    <button onClick={() => setPage((p) => Math.min(pages - 1, p + 1))} disabled={curPage >= pages - 1} aria-label="Next page">›</button>
                    <button className={`wt-carousel${carousel ? ' active' : ''}`} aria-pressed={carousel} onClick={() => setCarousel((c) => !c)}>Carousel</button>
                  </div>
                )}
            {playHint && <span className="wt-hint" role="status">{playHint}</span>}
          </div>
          {playing.length > 0 ? (
            <div className="wall-grid" style={gridStyle}>
                {visible.map((id, idx) => {
                  const p = byId.get(id);
                  if (!p) return (
                    <div className="wall-offline" key={id} style={cellStyle(idx)}>
                      <span className="wo-title">SIGNAL LOST</span>
                      <span className="wo-reason">Producer left — stream ended or device offline</span>
                      <button className="wo-close" onClick={() => closeTile(id)}>Close</button>
                    </div>
                  );
                  return (
                    <div className="wall-cell" key={id} style={cellStyle(idx)}>
                      <VideoPlayer
                        roomId={p.room}
                        serverUrl={wsServerUrl()}
                        token={getToken() ?? ''}
                        variant="tile"
                        onExpand={() => setModal({ room: p.room, producerId: p.id, kind: p.kind })}
                        onClose={() => closeTile(id)}
                      />
                      {!zoomedId && <button className="wc-zoom" onClick={() => setZoomedId(id)} title="Zoom this stream" aria-label="Zoom this stream">⤢</button>}
                    </div>
                  );
                })}
            </div>
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
            <AlertPanel producers={producers} onPlayRoom={playRoom} />
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

      {pickerOpen && (
        <Picker producers={producers} playing={playing} onClose={() => setPickerOpen(false)} onAdd={addPlaying} />
      )}
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