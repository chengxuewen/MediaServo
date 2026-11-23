import { useState } from 'react';
import { StatusGlyph } from '@mediaservo/react';
import type { SfuStreamStatRow } from '../api/client';
import './StreamTree.css';
import { ChevronRight, Video, Mic, Play } from 'lucide-react';

export interface StreamSelection { room: string; producerId: string; kind: string; }

/**
 * Device stream tree (P3 slice ③a, B5 corrected contract): live producers from
 * /sfu/stats grouped by device room. Rooms default-expanded (B5 board default).
 * Select a stream → single-stream playback. Multi-stream wall (cap 16) + Play All
 * land in slice ③b; label-based single-stream isolation for registered devices too.
 */
export default function StreamTree({ producers, loading, error, selected, onSelect }: {
  producers: SfuStreamStatRow[];
  loading: boolean;
  error: string | null;
  selected: StreamSelection | null;
  onSelect: (sel: StreamSelection) => void;
}) {
  // default-expanded: track collapsed rooms instead (empty = all open, B5 board default)
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  if (loading) return <div className="st-state">Loading streams…</div>;
  if (error) return <div className="st-state st-error">{error}</div>;

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
      {[...rooms.entries()].map(([room, list]) => {
        const open = !collapsed.has(room);
        return (
          <div className="st-group" key={room}>
            <button className="st-device" onClick={() => toggle(room)} aria-expanded={open}>
              <span className={`st-chevron${open ? ' open' : ''}`}><ChevronRight size={12} /></span>
              <span className="st-device-name">{room}</span>
              <span className="st-count">{list.length} stream{list.length === 1 ? '' : 's'}</span>
            </button>
            {open && (
              <div className="st-streams">
                {list.map((p) => {
                  const active = selected?.producerId === p.id;
                  return (
                    <button
                      key={p.id}
                      className={`st-stream${active ? ' active' : ''}`}
                      onClick={() => onSelect({ room: p.room, producerId: p.id, kind: p.kind })}
                      title={`producer ${p.id}`}
                    >
                      <span className="st-kind">{p.kind === 'audio' ? <Mic size={14} /> : <Video size={14} />}</span>
                      <span className="st-stream-name">{p.kind} · {p.peer_id}</span>
                      <StatusGlyph tone={p.score >= 8 ? 'ok' : p.score >= 4 ? 'warn' : 'danger'} shape="dot" label={`score ${p.score}`} />
                      <span className="st-score">score {p.score}</span>
                      <span className="st-play"><Play size={12} /> Play</span>
                    </button>
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