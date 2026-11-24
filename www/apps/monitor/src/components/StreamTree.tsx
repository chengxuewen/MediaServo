import { useState } from 'react';
import { StatusGlyph } from '@mediaservo/react';
import type { SfuStreamStatRow } from '../api/client';
import './StreamTree.css';
import { ChevronRight, Video, Mic, Play, Maximize } from 'lucide-react';

export interface StreamSelection { room: string; producerId: string; kind: string; }

// M5 (O4): per-producer bitrate on the tree row, from the stats list (bits/s).
function fmtBitrate(bps: number | null): string {
  if (bps == null) return '—';
  return bps >= 1_000_000 ? `${(bps / 1_000_000).toFixed(1)}M` : `${Math.round(bps / 1000)}K`;
}

/**
 * Device stream tree (P3 slice ③b, B5 corrected contract): live producers from
 * /sfu/stats grouped by device room, default-expanded (B5 board default).
 * Multi-select checkboxes feed the wall (cap 16); per-room Play All / Watch
 * Selected; per-stream Preview opens a large modal window. Addressing is device
 * room (+ stream label when a registered device exposes one); a PSK host room is
 * single-producer so consuming the room = that one stream.
 */
export default function StreamTree({ producers, loading, error, selected, onToggleSelect, onToggleAll, onPlay, onPreview, onRetry }: {
  producers: SfuStreamStatRow[];
  loading: boolean;
  error: string | null;
  selected: Set<string>;
  onToggleSelect: (producerId: string) => void;
  onToggleAll: (room: string, checked: boolean) => void;
  onPlay: (room: string) => void;
  onPreview: (sel: StreamSelection) => void;
  onRetry: () => void;
}) {
  // default-expanded: track collapsed rooms instead (empty = all open, B5 board default)
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  if (loading) return <div className="st-state">Loading streams…</div>;
  // M1: error with no data → error state + Retry; error with stale data → keep the
  // tree and show a stale banner + Retry (do not discard the last known streams).
  if (error && producers.length === 0) return <div className="st-state st-error">{error} <button className="st-retry" onClick={onRetry}>Retry</button></div>;

  const rooms = new Map<string, SfuStreamStatRow[]>();
  for (const p of producers) {
    const list = rooms.get(p.room) ?? [];
    list.push(p);
    rooms.set(p.room, list);
  }

  if (rooms.size === 0) {
    return <div className="st-state">No live streams. A producing host (device room) appears here once it pushes media.</div>;
  }

  const toggle = (room: string) => setCollapsed((prev) => {
    const next = new Set(prev);
    next.has(room) ? next.delete(room) : next.add(room);
    return next;
  });

  return (
    <div className="stream-tree">
      {error && <div className="st-stale" role="alert">Showing last known streams — refresh failed. <button className="st-retry" onClick={onRetry}>Retry</button></div>}
      {[...rooms.entries()].map(([room, list]) => {
        const open = !collapsed.has(room);
        const allSelected = list.length > 0 && list.every((p) => selected.has(p.id));
        const anySelected = list.some((p) => selected.has(p.id));
        return (
          <div className="st-group" key={room}>
            <div className="st-device-row">
              <button className="st-device" onClick={() => toggle(room)} aria-expanded={open}>
                <span className={`st-chevron${open ? ' open' : ''}`}><ChevronRight size={12} /></span>
                <span className="st-device-name">{room}</span>
                <span className="st-count">{list.length} stream{list.length === 1 ? '' : 's'}</span>
              </button>
              <button className="st-play-all" onClick={() => onPlay(room)} title={anySelected ? 'Play the selected streams' : 'Play all streams of this device'}>
                <Play size={12} /> {anySelected ? 'Watch Selected' : 'Play All'}
              </button>
              <label className="st-select-all">
                <input type="checkbox" checked={allSelected} onChange={(e) => onToggleAll(room, e.target.checked)} title="Select all streams" />
                <span>All</span>
              </label>
            </div>
            {open && (
              <div className="st-streams">
                {list.map((p) => {
                  const isSel = selected.has(p.id);
                  return (
                    <div key={p.id} className={`st-stream${isSel ? ' active' : ''}`}>
                      <label className="st-check">
                        <input type="checkbox" checked={isSel} onChange={() => onToggleSelect(p.id)} title="Select for the wall" />
                      </label>
                      <span className="st-kind">{p.kind === 'audio' ? <Mic size={14} /> : <Video size={14} />}</span>
                      <span className="st-stream-name">{p.kind} · {p.peer_id}</span>
                      <StatusGlyph tone={p.score >= 8 ? 'ok' : p.score >= 4 ? 'warn' : 'danger'} shape="dot" label={`score ${p.score}`} />
                      <span className="st-score">score {p.score}</span>
                      <span className="st-bitrate">{fmtBitrate(p.available_incoming_bitrate)}</span>
                      <button className="st-preview" onClick={() => onPreview({ room: p.room, producerId: p.id, kind: p.kind })} title="Preview in a large window">
                        <Maximize size={12} /> Preview
                      </button>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}