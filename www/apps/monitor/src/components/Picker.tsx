import { useMemo, useRef, useState } from 'react';
import { Modal } from '@mediaservo/react';
import type { SfuStreamStatRow } from '../api/client';
import './Picker.css';

/**
 * V10 (prototype parity): the add-stream dialog mirrors portal-unified.html's
 * "Select streams" — two panes (Device / Stream with status glyphs) ↔
 * (Selected + order + up/down/remove), a swap glyph, per-pane search, footer
 * "<n> selected · Cancel · Apply". Keeps the B3 a11y contract: role=tree +
 * arrow/Home/End/Enter keyboard + search; Play All per device still plays that
 * room immediately (e2e flow.playAllRoom).
 */
type Status = 'online' | 'degraded' | 'offline';
const statusOf = (score: number | null | undefined): Status => (score == null ? 'offline' : score >= 8 ? 'online' : score > 0 ? 'degraded' : 'offline');

export default function Picker({ producers, playing, onClose, onAdd, onPlay }: {
  producers: SfuStreamStatRow[];
  playing: string[];
  onClose: () => void;
  onAdd: (ids: string[]) => void;
  onPlay: (room: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [selQuery, setSelQuery] = useState('');
  const [chosen, setChosen] = useState<string[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [focusIdx, setFocusIdx] = useState(0);

  const q = query.trim().toLowerCase();
  const playable = producers.filter((p) => !playing.includes(p.id));
  const rooms = useMemo(() => [...new Set(playable.map((p) => p.room))]
    .filter((r) => !q || playable.some((p) => p.room === r && `${p.room} ${p.peer_id} ${p.kind}`.toLowerCase().includes(q))), [playable, q]);
  const byId = useMemo(() => new Map(producers.map((p) => [p.id, p])), [producers]);
  const roomStreams = (room: string) => playable.filter((p) => p.room === room);
  const roomStatus = (room: string): Status => {
    const list = roomStreams(room);
    if (list.some((p) => statusOf(p.score) === 'offline')) return 'offline';
    if (list.some((p) => statusOf(p.score) === 'degraded')) return 'degraded';
    return 'online';
  };
  const allChosen = (room: string) => { const ids = roomStreams(room).map((p) => p.id); return ids.length > 0 && ids.every((i) => chosen.includes(i)); };

  const toggleRoom = (room: string) => {
    const ids = roomStreams(room).map((p) => p.id);
    setChosen((prev) => (ids.every((i) => prev.includes(i)) ? prev.filter((i) => !ids.includes(i)) : [...prev, ...ids.filter((i) => !prev.includes(i))]));
  };
  const toggleExpand = (room: string) => setExpanded((prev) => { const n = new Set(prev); n.has(room) ? n.delete(room) : n.add(room); return n; });
  const toggleStream = (id: string) => setChosen((prev) => (prev.includes(id) ? prev.filter((i) => i !== id) : [...prev, id]));
  const move = (idx: number, dir: -1 | 1) => setChosen((prev) => { const n = [...prev]; const t = idx + dir; if (t < 0 || t >= n.length) return prev; [n[idx], n[t]] = [n[t], n[idx]]; return n; });
  const remove = (id: string) => setChosen((prev) => prev.filter((i) => i !== id));

  const availRef = useRef<HTMLDivElement>(null);
  const moveFocus = (next: number) => { setFocusIdx(next); availRef.current?.querySelectorAll<HTMLElement>('[role="treeitem"]')[next]?.focus(); };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); moveFocus(Math.min(rooms.length - 1, focusIdx + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); moveFocus(Math.max(0, focusIdx - 1)); }
    else if (e.key === 'Home') { e.preventDefault(); moveFocus(0); }
    else if (e.key === 'End') { e.preventDefault(); moveFocus(Math.max(0, rooms.length - 1)); }
    else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (rooms[focusIdx]) toggleRoom(rooms[focusIdx]); }
  };

  const chosenView = chosen
    .map((id, order) => ({ id, order, p: byId.get(id) }))
    .filter((x) => x.p && (!selQuery.trim() || `${x.p.room} ${x.p.kind} ${x.p.peer_id}`.toLowerCase().includes(selQuery.trim().toLowerCase())));

  let flat = 0;
  return (
    <Modal open title="Select streams" onClose={onClose} className="picker-modal"
      actions={<>
        <span className="picker-foot-count">{chosen.length} selected</span>
        <button className="picker-cancel" onClick={onClose}>Cancel</button>
        <button className="picker-add" disabled={chosen.length === 0} onClick={() => { onAdd(chosen); onClose(); }}>Apply</button>
      </>}>
      <div className="picker-cols">
        <div className="picker-pane">
          <div className="picker-pane-head">
            <span>Device / Stream</span>
            <span className="picker-head-right">
              <button className="picker-playall" onClick={() => { const all = rooms.flatMap((r) => roomStreams(r).map((p) => p.id)); setChosen((prev) => [...prev, ...all.filter((i) => !prev.includes(i))]); }}>Play All</button>
              <span>Status</span>
            </span>
          </div>
          <input className="picker-search" type="search" placeholder="Search device / stream…" value={query}
            onChange={(e) => { setQuery(e.target.value); setFocusIdx(0); }} aria-label="Search streams" />
          <div ref={availRef} className="picker-avail" role="tree" aria-label="Available streams" aria-multiselectable="true" tabIndex={0} onKeyDown={onKey}>
            {rooms.length === 0 && <div className="picker-empty">No available streams match.</div>}
            {rooms.map((room) => {
              const list = roomStreams(room); const st = roomStatus(room); const open = expanded.has(room); const i = flat++;
              return (
                <div key={room} className="st-group">
                  <div role="treeitem" aria-selected={allChosen(room)} aria-expanded={open} tabIndex={i === focusIdx ? 0 : -1}
                    className={`picker-dev${allChosen(room) ? ' chosen' : ''}${i === focusIdx ? ' focused' : ''}`}
                    onClick={() => toggleRoom(room)}>
                    <button className="pick-caret" aria-label={open ? 'Collapse' : 'Expand'} aria-expanded={open}
                      onClick={(e) => { e.stopPropagation(); toggleExpand(room); }}>{open ? '▾' : '▸'}</button>
                    <input type="checkbox" readOnly checked={allChosen(room)} tabIndex={-1} aria-hidden="true" />
                    <span className="st-device-name">{room}</span>
                    <span className="picker-dev-count">{list.length}/{list.length}</span>
                    <span className={`pick-status ${st}`}><span className="pick-dot" aria-hidden="true" />{st}</span>
                    <button className="st-play-all" title="Play all streams of this device" onClick={(e) => { e.stopPropagation(); onPlay(room); onClose(); }}>Play</button>
                  </div>
                  {open && (
                    <div className="picker-dev-streams">
                      {list.map((p) => {
                        const on = chosen.includes(p.id); const s = statusOf(p.score);
                        return (
                          <div key={p.id} className={`picker-child${on ? ' chosen' : ''}`} onClick={() => toggleStream(p.id)}>
                            <input type="checkbox" readOnly checked={on} tabIndex={-1} aria-hidden="true" />
                            <span className="picker-child-name">{p.kind} · {p.peer_id}</span>
                            <span className={`pick-status ${s}`}><span className="pick-dot" aria-hidden="true" />{s}</span>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>

        <div className="picker-swap" aria-hidden="true">⇄</div>

        <div className="picker-pane">
          <div className="picker-pane-head"><span>Selected ({chosen.length})</span><span>Order</span></div>
          <input className="picker-search" type="search" placeholder={`Search ${chosen.length} selected`} value={selQuery}
            onChange={(e) => setSelQuery(e.target.value)} aria-label="Search selected" />
          <div className="picker-chosen" aria-label="Chosen streams">
            {chosenView.length === 0 && <div className="picker-empty">Nothing selected yet.</div>}
            {chosenView.map(({ id, order, p }) => p && (
              <div key={id} className="picker-chosen-item">
                <span className="pc-order">{order + 1}</span>
                <span className="pc-name">{p.room} · <span className="pc-sub">{p.kind}</span></span>
                <span className={`pick-status ${statusOf(p.score)}`}><span className="pick-dot" aria-hidden="true" />{statusOf(p.score)}</span>
                <span className="pc-acts">
                  <button onClick={() => move(chosen.indexOf(id), -1)} aria-label="Move up" disabled={order === 0}>↑</button>
                  <button onClick={() => move(chosen.indexOf(id), 1)} aria-label="Move down" disabled={order === chosen.length - 1}>↓</button>
                  <button onClick={() => remove(id)} aria-label="Remove">−</button>
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </Modal>
  );
}