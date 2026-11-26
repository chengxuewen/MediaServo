import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { VideoPlayer, Confirm } from '@mediaservo/react';
import { getToken } from '@mediaservo/client';
import type { StreamSelection } from '../components/StreamTree';
import Picker from '../components/Picker';
import AlertPanel from '../components/AlertPanel';
import { useStreams } from '../hooks/useStreams';
import { useAudit } from '../hooks/useAudit';
import { useAdminWS } from '../hooks/useAdminWS';
import { useAuth } from '../hooks/useAuth';
import PlaybackBar from '../components/PlaybackBar';
import { isDemo, DEMO_PRODUCERS, DEMO_PLAYING, DEMO_EVENTS, DEMO_OFFLINE_NAMES } from '../demo';
import { PanelLeft, Bell, Rows3 } from 'lucide-react';
import './Home.css';

// Playback cap (browser soft-decode guard): 16 = a 4x4 grid scale (B5 contract).
const MAX_PLAYING = 16;
const PLAY_ALL_CONFIRM = 8;
const DROP_GRACE_MS = 6000;
const REGION_KEY = 'mediaservo_monitor_regions';
// B2 (R1): named-layout persistence uses a NEW key; migrate the legacy cols key.
const LAYOUT_KEY = 'mediaservo_monitor_layout';
const LEGACY_COLS_KEY = 'mediaservo_monitor_cols';


interface Layout { id: string; name: string; cols: number; slots: number; areas?: string[]; elastic?: boolean; }
const sym = (id: string, n: number): Layout => { const c = Math.round(Math.sqrt(n)); return { id, name: id, cols: c, slots: n }; };
// V6 (K6): full LAYOUTS — R6 set + 25/36 + custom elastic (cap16 + empty-slot parity, K10).
const LAYOUTS: Layout[] = [
  sym('1', 1), sym('4', 4), sym('9', 9), sym('16', 16), sym('25', 25), sym('36', 36),
  { id: '1+3', name: '1 + 3', cols: 3, slots: 4, areas: ['big big s1', 'big big s2', 'big big s3'] },
  { id: '1+5', name: '1 + 5', cols: 4, slots: 6, areas: ['big big s1 s2', 'big big s3 s4', 'big big s5 .'] },
  { id: '1+7', name: '1 + 7', cols: 4, slots: 8, areas: ['big big s1 s2', 'big big s3 s4', 'big big s5 s6', 'big big s7 .'] },
  { id: 'custom', name: 'Custom', cols: 3, slots: 9, elastic: true },
];
const WEIGHTS_KEY = 'ms_col_weights';
const SPANS_KEY = 'ms_tile_spans';
function loadWeights(): number[] { try { const v = JSON.parse(localStorage.getItem(WEIGHTS_KEY) || 'null'); if (Array.isArray(v) && v.length === 3) return v; } catch { /* silent */ } return [1, 1, 1]; }
function loadSpans(): Record<string, boolean> { try { return JSON.parse(localStorage.getItem(SPANS_KEY) || '{}'); } catch { return {}; } }
// V8: with zero streams Auto still shows the prototype's featured 1+5 grid (dark empty
// slots), so the real default page is a layout, not an empty box.
const autoTemplateId = (n: number) => (n === 0 ? '1+5' : n <= 1 ? '1' : n <= 4 ? '4' : n <= 6 ? '1+5' : n <= 9 ? '9' : '16');
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
  // V3 (L5): narrow defaults to both drawers CLOSED (prototype), so they don't cover
  // the topbar; wide defaults open. Stored preference wins when present.
  const narrow = window.matchMedia('(max-width: 767px)').matches;
  // V27: on narrow screens a stored desktop state must NOT reopen both drawers at once
  // (they float over the wall and hide it). Narrow always starts closed; the stage-bar
  // toggles stay mutually exclusive.
  if (narrow) return { left: false, right: false, bottom: false };
  try {
    const raw = localStorage.getItem(REGION_KEY);
    if (raw) { const v = JSON.parse(raw); return { left: v.left !== false, right: v.right !== false, bottom: v.bottom !== false }; }
  } catch { /* fall through */ }
  // Prototype desktop: left rail + right alerts open; the PLAYBACK bar is OFF by default
  // and is opened from the stage-bar's top-left region toggle (V24).
  return { left: !narrow, right: !narrow, bottom: false };
}
function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

// V2 (I2): Synology-style layout gallery combo with keyboard nav (arrows/Home/End/Enter/Esc).
function LayoutGallery({ current, onSelect, onClose }: { current: string; onSelect: (id: string) => void; onClose: () => void }) {
  const items = ['auto', ...LAYOUTS.map((l) => l.id)];
  const [idx, setIdx] = useState(Math.max(0, items.indexOf(current)));
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => { (ref.current?.querySelectorAll('button')[idx] as HTMLElement | undefined)?.focus(); }, [idx]);
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowRight') { e.preventDefault(); setIdx((i) => Math.min(items.length - 1, i + 1)); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); setIdx((i) => Math.max(0, i - 1)); }
    else if (e.key === 'Home') { e.preventDefault(); setIdx(0); }
    else if (e.key === 'End') { e.preventDefault(); setIdx(items.length - 1); }
    else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSelect(items[idx]); }
    else if (e.key === 'Escape') { e.preventDefault(); onClose(); }
  };
  return (
    <div className="layout-gallery" role="listbox" aria-label="Layout gallery" ref={ref} onKeyDown={onKey}>
      {items.map((id, i) => {
        const l = id === 'auto' ? null : findLayout(id);
        return (
          <button key={id} role="option" aria-selected={id === current} className="lo-thumb" tabIndex={i === idx ? 0 : -1} onClick={() => onSelect(id)}>
            <span className="lo-grid" style={{ gridTemplateColumns: `repeat(${l ? l.cols : 2}, 1fr)` }}>
              {Array.from({ length: l ? l.slots : 4 }).map((_, c) => <span key={c} className={`lo-cell${l?.areas && c === 0 ? ' big' : ''}`} />)}
            </span>
            {l ? l.name : 'Auto'}
          </button>
        );
      })}
    </div>
  );
}

/**
 * Monitor realtime face — B1 three-panel + B2 named layouts (monitor-prototype-parity).
 * Left stream rail | center canvas (wall) | right alerts placeholder (B4). Bottom Event feed.
 * B2: R6 layout set (1/4/9/16 + 1+3/1+5/1+7) + Auto-by-count via a single dropdown,
 * min-tile horizontal scroll, pager + carousel (default off), single-tile zoom (K5).
 */
export default function Home() {
  const { canMonitor } = useAuth();
  // V8: opt-in labeled simulated preview (?demo=1) for like-for-like visual parity.
  const demo = isDemo();
  const real = useStreams();
  const producers = demo ? DEMO_PRODUCERS : real.producers;
  const error = demo ? null : real.error;
  const refetch = real.refetch;
  // M6: single audit poll + single admin WS owned here; Timeline/AlertPanel are
  // presentational (was 3 WS connections across components, now 1).
  const audit = useAudit();
  const events = demo ? DEMO_EVENTS : audit.events;
  const onWsEvent = useCallback(() => { refetch(); audit.refetch(); }, [refetch, audit.refetch]);
  useAdminWS(onWsEvent);

  const [playing, setPlaying] = useState<string[]>(demo ? DEMO_PLAYING : []);
  const [modal, setModal] = useState<StreamSelection | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  // V31: overlay drawers sit below the stage-bar (ruling B). The bar can wrap to two
  // rows, so its height is MEASURED and exposed as --bar-h instead of a fixed 60px.
  const barRef = useRef<HTMLDivElement>(null);
  const [barH, setBarH] = useState(52);
  useEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setBarH(Math.round(el.getBoundingClientRect().height)));
    ro.observe(el);
    setBarH(Math.round(el.getBoundingClientRect().height));
    return () => ro.disconnect();
  }, []);
  const [statsVisible, setStatsVisible] = useState(false); // V38: prototype default = hover-only; the toggle pins it on
  const [galleryOpen, setGalleryOpen] = useState(false);
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
    // V3 (L5): narrow overlay-drawer mutual exclusion — opening one side closes the other.
    if (window.matchMedia('(max-width: 767px)').matches) {
      if (k === 'left' && next.left) next.right = false;
      if (k === 'right' && next.right) next.left = false;
    }
    try { localStorage.setItem(REGION_KEY, JSON.stringify(next)); } catch { /* silent degrade */ }
    return next;
  });
  const [pinMsg, setPinMsg] = useState('');
  // V3 (I1): drag-reorder the on-wall list; drag onto the remove zone closes a tile.
  const reorderPlaying = (from: string, to: string) => {
    if (!from || from === to) return;
    setPlaying((prev) => { const a = [...prev]; const fi = a.indexOf(from); const ti = a.indexOf(to); if (fi < 0 || ti < 0) return prev; a.splice(fi, 1); a.splice(ti, 0, from); return a; });
  };
  // V9b: rail card -> wall slot reorder (drop onto a wall cell / empty slot).
  const dropToWall = (fromId: string, toIdx: number) => {
    if (!fromId) return;
    setPlaying((prev) => { if (!prev.includes(fromId)) return prev; const a = prev.filter((x) => x !== fromId); a.splice(Math.min(toIdx, a.length), 0, fromId); return a; });
  };

  const [clock, setClock] = useState(() => new Date());
  useEffect(() => { const t = setInterval(() => setClock(new Date()), 1000); return () => clearInterval(t); }, []);

  // V20: theme is now set in admin Settings (Appearance) — the monitor only APPLIES the
  // persisted choice (single source of truth, no theme button on the wall toolbar).
  useEffect(() => {
    try { document.documentElement.dataset.theme = localStorage.getItem('msrv_theme') || 'light'; } catch { /* silent */ }
  }, []);

  // V5 (I8): Esc hierarchical close — gallery -> zoom (modal/picker ride kit Modal LIFO).
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || modal || pickerOpen) return;
      if (galleryOpen) setGalleryOpen(false);
      else if (zoomedId) setZoomedId(null);
    };
    window.addEventListener('keydown', h);
    return () => window.removeEventListener('keydown', h);
  }, [galleryOpen, zoomedId, modal, pickerOpen]);

  const byId = new Map(producers.map((p) => [p.id, p]));
  const degraded = producers.filter((p) => p.score < 8).length;
  // K11/K13 (healthChip parity): offline = on-wall ids whose producer is gone; red when any issue, hidden when healthy (prototype).
  const offline = playing.filter((id) => !byId.has(id)).length;
  // V2 (I6): unread-style alert badge count from the audit ring (alert-worthy kinds).
  const ALERT_KINDS = ['device_offline', 'estop_audit', 'emergency_command', 'auth_failure', 'authorization_denied', 'stream_destroy', 'device_revoked'];
  const alertCount = events.filter((e) => ALERT_KINDS.includes(e.kind)).length;
  const focusDegraded = () => { const d = producers.find((p) => p.score < 8); if (d) { if (!playing.includes(d.id)) addPlaying([d.id]); setZoomedId(d.id); } };

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
  // S1: clamp raw page when the page count shrinks (closing tiles / layout change)
  // so Prev/Next stay responsive instead of walking a stale raw page down.
  useEffect(() => { setPage((p) => Math.min(p, pages - 1)); }, [pages]);

  // V6 (I3): elastic column weights + per-tile 2x2 span, persisted.
  const [colWeights, setColWeights] = useState(loadWeights);
  const [tileSpans, setTileSpans] = useState<Record<string, boolean>>(loadSpans);
  const toggleSpan = (id: string) => setTileSpans((prev) => { const n = { ...prev, [id]: !prev[id] }; try { localStorage.setItem(SPANS_KEY, JSON.stringify(n)); } catch { /* silent */ } return n; });
  const dragCol = (i: number) => (e: React.PointerEvent) => {
    e.preventDefault();
    const wrap = (e.currentTarget as HTMLElement).parentElement!;
    const r = wrap.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      const frac = Math.min(Math.max((ev.clientX - r.left) / r.width, 0.15), 0.85);
      setColWeights((prev) => {
        const total = prev.reduce((a, b) => a + b, 0);
        const before = prev.slice(0, i).reduce((a, b) => a + b, 0);
        const pair = prev[i] + prev[i + 1];
        const wi = Math.max(0.35, frac * total - before);
        const n = [...prev]; n[i] = wi; n[i + 1] = Math.max(0.35, pair - wi);
        try { localStorage.setItem(WEIGHTS_KEY, JSON.stringify(n)); } catch { /* silent */ }
        return n;
      });
    };
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up);
  };

  const gridStyle: CSSProperties = zoomedId
    ? { gridTemplateColumns: '1fr' }
    : layout.elastic
      ? { gridTemplateColumns: colWeights.map((w) => `${w}fr`).join(' ') }
      : layout.areas
        ? { gridTemplateColumns: `repeat(${layout.cols}, minmax(0, 1fr))`, gridTemplateAreas: layout.areas.map((r) => `"${r}"`).join(' ') }
        : { gridTemplateColumns: `repeat(${layout.cols}, minmax(0, 1fr))` };
  const cellStyle = (idx: number, id?: string): CSSProperties => {
    const s: CSSProperties = {};
    if (layout.areas && !zoomedId) s.gridArea = idx === 0 ? 'big' : `s${idx}`;
    if (layout.elastic && id && tileSpans[id] && !zoomedId) { s.gridColumn = 'span 2'; s.gridRow = 'span 2'; }
    return s;
  };
  const emptyCount = !zoomedId ? Math.max(0, layout.slots - visible.length) : 0;

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
      setZoomedId((prev) => (prev && dead.includes(prev) ? null : prev));
      for (const id of dead) missing.delete(id);
    }
  }, [producers, playing]);

  const addPlaying = (ids: string[]) => {
    const fresh = ids.filter((id) => !playing.includes(id));
    const merged = [...playing, ...fresh].slice(0, MAX_PLAYING);
    const dropped = fresh.length - (merged.length - playing.length);
    setPlaying(merged);
    setPlayHint(dropped > 0 ? `Playback cap of ${MAX_PLAYING} reached; skipped ${dropped} (close tiles to play more)` : '');
  };
  // V13: Apply in the picker SYNCHRONISES the wall to the checked set (adds new, removes
// unchecked), so reopening shows the current selection instead of losing it.
  const applyPlaying = (ids: string[]) => setPlaying(ids.slice(0, MAX_PLAYING));
  // V9c (K11=A): alert click focuses ONE camera — add its first producer to the wall if
  // absent, then zoom that single tile (prototype focusAlertCam).
  const focusRoom = (room: string) => {
    const p = producers.find((x) => x.room === room);
    if (!p) return;
    setPlaying((prev) => (prev.includes(p.id) ? prev : [...prev, p.id]));
    setZoomedId(p.id);
  };
  const playRoom = (room: string) => {
    const ids = producers.filter((p) => p.room === room).map((p) => p.id);
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
      {/* V19: top area compacted to the prototype's single toolbar row. The page title
          stays for a11y/SEO only (visually hidden); identity/notes/Demo moved into the
          toolbar (or removed) so the wall gets the height. */}
      <h1 className="sr-only">Monitor Center{canMonitor ? '' : ' (wall access requires admin/dispatcher)'}</h1>

      <div className="monitor-three-panel" style={{ ['--bar-h' as string]: `${barH}px` } as React.CSSProperties}>
        <aside className={`stream-rail${regions.left ? '' : ' collapsed'}`}>
          <div className="rail-head">
            <span className="rail-title">Selected streams</span>
            <button className="pin-btn" onClick={() => setPinMsg('Pinned — resize wider to dock')} title="Pin panel (dock when wide)" aria-label="Pin left panel">⤓</button>
            <button className="region-collapse" onClick={() => toggleRegion('left')} aria-label={regions.left ? 'Hide left panel' : 'Show left panel'}>{regions.left ? '‹' : '›'}</button>
          </div>
          <div className="rail-body">
            {error && producers.length === 0 && <div className="st-state st-error">{error} <button className="st-retry" onClick={refetch}>Retry</button></div>}
            {error && producers.length > 0 && <div className="st-stale" role="alert">Showing last known streams — refresh failed. <button className="st-retry" onClick={refetch}>Retry</button></div>}
            <button className="open-picker" onClick={() => setPickerOpen(true)}>＋ Select streams</button>
            <div className="rail-cards" aria-label="Selected streams (drag to reorder)">
              {playing.length === 0 && <div className="rail-empty">No streams selected — use “＋ Select streams”.</div>}
              {playing.map((id) => {
                const p = byId.get(id);
                const state = !p ? 'offline' : p.score < 8 ? 'degraded' : 'live';
                return (
                  <div key={id} className={`rail-card${state === 'live' ? ' active' : ''}`} draggable
                    onDragStart={(e) => e.dataTransfer.setData('text/plain', id)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={(e) => { e.preventDefault(); reorderPlaying(e.dataTransfer.getData('text/plain'), id); }}>
                    <span className={`rc-glyph ${state}`} aria-hidden="true" />
                    <span className="rc-main">
                      {/* V45: prototype rail card = "<device> · <stream>" ( Produce.label ) */}
                      <span className="rc-name" title={p ? `${p.room}${p.label ? ` · ${p.label}` : ''}` : (demo ? DEMO_OFFLINE_NAMES[id] ?? id : id)}>{p ? `${p.room}${p.label ? ` · ${p.label}` : ''}` : (demo ? DEMO_OFFLINE_NAMES[id] ?? id : id)}</span>
                      <span className="rc-sub">{p ? `${p.kind} · ${state}` : 'offline'}</span>
                    </span>
                    <button className="rc-del" onClick={() => closeTile(id)} aria-label={`Remove ${p?.room ?? id}`}>×</button>
                  </div>
                );
              })}
              <div className="rail-drop-remove" onDragOver={(e) => e.preventDefault()}
                onDrop={(e) => { e.preventDefault(); const id = e.dataTransfer.getData('text/plain'); if (id) closeTile(id); }}>
                Drop here to remove
              </div>
            </div>
            {pinMsg && <div className="rail-pinmsg" role="status">{pinMsg}</div>}
            <div className="rail-foot">{playing.length} streams selected</div>
          </div>
        </aside>

        <div className="canvas">
          <div className="wall-toolbar" ref={barRef}>
                <div className="region-toggles">
                  <button aria-pressed={regions.left} onClick={() => toggleRegion('left')} title="Toggle stream rail" aria-label="Toggle stream rail"><PanelLeft size={15} /></button>
                  <button aria-pressed={regions.right} onClick={() => toggleRegion('right')} title="Toggle alert panel" aria-label="Toggle alert panel">
                    <Bell size={15} />
                    {alertCount > 0 && <span className="al-badge" aria-label={`${alertCount} alerts`}>{alertCount}</span>}
                  </button>
                  <button aria-pressed={regions.bottom} onClick={() => toggleRegion('bottom')} title="Toggle playback bar" aria-label="Toggle playback bar"><Rows3 size={15} /></button>
                </div>
                <span className="wt-sep" aria-hidden="true" />
                <div className="layout-combo">
                  <button id="layout-menu" className="layout-menu" aria-haspopup="listbox" aria-expanded={galleryOpen} onClick={() => setGalleryOpen((o) => !o)}>
                    <span className="lm-label">Layout</span>
                    <span className="lm-value">{layoutId === 'auto' ? 'Auto' : findLayout(layoutId).name}</span>
                    <span className="lm-caret" aria-hidden="true">▾</span>
                  </button>
                  {galleryOpen && <LayoutGallery current={layoutId} onSelect={(id) => { applyLayout(id); setGalleryOpen(false); }} onClose={() => setGalleryOpen(false)} />}
                </div>
                <div className="wt-pager">
                  <button onClick={() => setPage((p) => Math.max(0, p - 1))} disabled={curPage === 0 || pages <= 1 || !!zoomedId} aria-label="Previous page">‹</button>
                  <span className="wt-page">{curPage + 1} / {pages}</span>
                  <button onClick={() => setPage((p) => Math.min(pages - 1, p + 1))} disabled={curPage >= pages - 1 || pages <= 1 || !!zoomedId} aria-label="Next page">›</button>
                  <button className={`wt-carousel${carousel ? ' active' : ''}`} aria-pressed={carousel} onClick={() => setCarousel((c) => !c)} title="Carousel" aria-label="Carousel">⟳</button>
                </div>
                {(degraded > 0 || offline > 0) && (
                  <button className="health-chip warn" role="status" onClick={focusDegraded} title="Focus first problem stream">
                    <span className="hc-ic" aria-hidden="true">⚠</span>
                    {[degraded > 0 ? `${degraded} degraded` : '', offline > 0 ? `${offline} offline` : ''].filter(Boolean).join(' · ')}
                  </button>
                )}
                {playHint && <span className="wt-hint" role="status">{playHint}</span>}
                <span className="live-chip" title="Live"><span className="live-dot" aria-hidden="true" />LIVE</span>
                <span className="live-clock">{clock.toLocaleTimeString('en-GB', { hour12: false })}</span>
                <span className="wt-spacer" />
                <button className="iconbtn" aria-disabled="true" title="Drift sync display — deferred (RECORDING_READY=false)" aria-label="Sync (deferred)">⇄</button>
                <button className={`global-stats-toggle iconbtn${statsVisible ? ' on' : ''}`} aria-pressed={statsVisible} onClick={() => setStatsVisible((s) => !s)} title="Stats" aria-label="Stats">ⓘ</button>
                {/* V20: no Demo / theme buttons on the wall toolbar (prototype right = sync/stats). */}
          </div>
          <div className="wall-wrap">
              <div className={`wall-grid${statsVisible ? ' stats-on' : ''}`} style={gridStyle}>
                {visible.map((id, idx) => {
                  const p = byId.get(id);
                  if (!p) return (
                    <div className="wall-offline" key={id} style={cellStyle(idx, id)}>
                      <span className="wo-title">SIGNAL LOST</span>
                      <span className="wo-reason">Producer left — stream ended or device offline</span>
                      <button className="wo-close" onClick={() => closeTile(id)}>Close</button>
                    </div>
                  );
                  return (
                    <div className="wall-cell" key={id} style={cellStyle(idx, id)}
                      onDragOver={(e) => e.preventDefault()}
                      onDrop={(e) => { e.preventDefault(); dropToWall(e.dataTransfer.getData('text/plain'), idx); }}>
                      <VideoPlayer
                        roomId={p.room}
                        streamLabel={p.label}
                        serverUrl={wsServerUrl()}
                        token={getToken() ?? ''}
                        variant="tile"
                        nvr
                        onZoom={() => setZoomedId((prev) => (prev === id ? null : id))}
                        zoomed={zoomedId === id}
                        onRemove={() => closeTile(id)}
                        onSpan={layout.elastic ? () => toggleSpan(id) : undefined}
                        spanned={!!tileSpans[id]}
                        simulated={demo}
                        degraded={p.score < 8}
                        idleText={demo ? `H.265 · 1080p · 25 fps · ${((p.available_incoming_bitrate ?? 0) / 1e6).toFixed(1)} Mbps` : undefined}
                        onExpand={() => setModal({ room: p.room, producerId: p.id, kind: p.kind })}
                        onClose={() => closeTile(id)}
                      />
                    </div>
                  );
                })}
                {Array.from({ length: emptyCount }).map((_, i) => (
                  <div className="tile-empty" key={`empty-${i}`} style={cellStyle(visible.length + i)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={(e) => { e.preventDefault(); dropToWall(e.dataTransfer.getData('text/plain'), visible.length + i); }}>
                    <svg className="te-ic" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></svg>
                    <span>Empty slot</span>
                  </div>
                ))}
              </div>
              {layout.elastic && !zoomedId && colWeights.slice(0, -1).map((_, i) => {
                const total = colWeights.reduce((a, b) => a + b, 0);
                const left = (colWeights.slice(0, i + 1).reduce((a, b) => a + b, 0) / total) * 100;
                return <span key={i} className="col-handle" style={{ left: `${left}%` }} onPointerDown={dragCol(i)} title="Drag to resize column" />;
              })}
          </div>
        </div>

        <aside className={`mon-alerts${regions.right ? '' : ' collapsed'}`}>
          <div className="rail-head">
            <span className="rail-title">Alerts</span>
            <button className="region-collapse" onClick={() => toggleRegion('right')} aria-label={regions.right ? 'Hide right panel' : 'Show right panel'}>{regions.right ? '›' : '‹'}</button>
          </div>
          <div className="rail-body">
            <AlertPanel events={events} producers={producers} onPlayRoom={playRoom} onFocusRoom={focusRoom} />
          </div>
        </aside>
      </div>

      {/* V24: no "Playback" header — the bar is shown/hidden only by the stage-bar
          region toggle (prototype has no header row above the timeline). */}
      <div className={`bottom-zone${regions.bottom ? '' : ' collapsed'}`}>
        <PlaybackBar events={events} demo={demo} />
      </div>

      {pickerOpen && (
        <Picker producers={producers} playing={playing} onClose={() => setPickerOpen(false)} onAdd={applyPlaying} onPlay={playRoom} />
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