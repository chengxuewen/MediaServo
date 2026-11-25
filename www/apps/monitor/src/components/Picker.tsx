import { useRef, useState } from 'react';
import { Modal } from '@mediaservo/react';
import type { SfuStreamStatRow } from '../api/client';
import './Picker.css';

/**
 * V9b (K12=A): the add-stream picker carries the device/stream list (grouped by room with
 * a per-room Play All) behind "＋ Select streams" — the left rail now shows only
 * selected/on-wall cards (prototype IA). Keeps the accessible shuttle: role=tree +
 * arrow/Home/End/Enter keyboard + search + chosen column + Apply. Reuses the
 * .st-play-all / .st-device-name classes so the e2e selectors stay meaningful.
 */
export default function Picker({ producers, playing, onClose, onAdd, onPlay }: {
  producers: SfuStreamStatRow[];
  playing: string[];
  onClose: () => void;
  onAdd: (ids: string[]) => void;
  onPlay: (room: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [focusIdx, setFocusIdx] = useState(0);

  const q = query.trim().toLowerCase();
  const avail = producers.filter((p) => !playing.includes(p.id)
    && (!q || `${p.room} ${p.peer_id} ${p.kind}`.toLowerCase().includes(q)));
  const rooms = [...new Set(avail.map((p) => p.room))];
  const toggle = (id: string) => setChosen((prev) => { const n = new Set(prev); n.has(id) ? n.delete(id) : n.add(id); return n; });

  const availRef = useRef<HTMLDivElement>(null);
  const moveFocus = (next: number) => { setFocusIdx(next); availRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]')[next]?.focus(); };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); moveFocus(Math.min(avail.length - 1, focusIdx + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); moveFocus(Math.max(0, focusIdx - 1)); }
    else if (e.key === 'Home') { e.preventDefault(); moveFocus(0); }
    else if (e.key === 'End') { e.preventDefault(); moveFocus(Math.max(0, avail.length - 1)); }
    else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (avail[focusIdx]) toggle(avail[focusIdx].id); }
  };

  let flat = 0;
  return (
    <Modal open title="Select streams" onClose={onClose} className="picker-modal"
      actions={<>
        <button className="picker-cancel" onClick={onClose}>Cancel</button>
        <button className="picker-add" disabled={chosen.size === 0} onClick={() => { onAdd([...chosen]); onClose(); }}>Add to wall ({chosen.size})</button>
      </>}>
      <input className="picker-search" type="search" placeholder="Search devices / streams…"
        value={query} onChange={(e) => { setQuery(e.target.value); setFocusIdx(0); }} aria-label="Search streams" />
      <div className="picker-cols">
        <div ref={availRef} className="picker-avail" role="tree" aria-label="Available streams" aria-multiselectable="true" tabIndex={0} onKeyDown={onKey}>
          {avail.length === 0 && <div className="picker-empty">No available streams match.</div>}
          {rooms.map((room) => (
            <div className="st-group" key={room}>
              <div className="st-device-row">
                <span className="st-device-name">{room}</span>
                <button className="st-play-all" title="Play all streams of this device" onClick={() => { onPlay(room); onClose(); }}>Play All</button>
              </div>
              {avail.filter((p) => p.room === room).map((p) => {
                const i = flat++;
                return (
                  <div key={p.id} role="treeitem" aria-selected={chosen.has(p.id)} tabIndex={i === focusIdx ? 0 : -1}
                    className={`picker-item${chosen.has(p.id) ? ' chosen' : ''}${i === focusIdx ? ' focused' : ''}`}
                    onClick={() => toggle(p.id)}>{p.kind} · {p.peer_id}</div>
                );
              })}
            </div>
          ))}
        </div>
        <div className="picker-chosen" aria-label="Chosen streams">
          <div className="picker-chosen-head">Chosen ({chosen.size})</div>
          {[...chosen].map((id) => { const p = producers.find((x) => x.id === id); return p ? <div key={id} className="picker-chosen-item">{p.room} · {p.kind}</div> : null; })}
        </div>
      </div>
    </Modal>
  );
}