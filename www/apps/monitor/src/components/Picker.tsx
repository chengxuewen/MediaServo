import { useState } from 'react';
import { Modal } from '@mediaservo/react';
import type { SfuStreamStatRow } from '../api/client';
import './Picker.css';

/**
 * Add-stream shuttle picker (B3, B5 corrected contract): a modal with a searchable
 * available list (role=tree + arrow-key navigation + Enter/Space toggle) and a chosen
 * column. Apply adds the chosen producers to the wall. The left rail keeps the
 * selected-stream face; this picker is the add-stream affordance (O2/N3/R8 split).
 * ARIA tree lives in the kit Modal portal (dialog/Esc/focus-trap inherited).
 */
export default function Picker({ producers, playing, onClose, onAdd }: {
  producers: SfuStreamStatRow[];
  playing: string[];
  onClose: () => void;
  onAdd: (ids: string[]) => void;
}) {
  const [query, setQuery] = useState('');
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [focusIdx, setFocusIdx] = useState(0);

  const q = query.trim().toLowerCase();
  const avail = producers.filter((p) => !playing.includes(p.id)
    && (!q || `${p.room} ${p.peer_id} ${p.kind}`.toLowerCase().includes(q)));

  const toggle = (id: string) => setChosen((prev) => {
    const next = new Set(prev);
    next.has(id) ? next.delete(id) : next.add(id);
    return next;
  });

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); setFocusIdx((i) => Math.min(avail.length - 1, i + 1)); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setFocusIdx((i) => Math.max(0, i - 1)); }
    else if (e.key === 'Home') { e.preventDefault(); setFocusIdx(0); }
    else if (e.key === 'End') { e.preventDefault(); setFocusIdx(Math.max(0, avail.length - 1)); }
    else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (avail[focusIdx]) toggle(avail[focusIdx].id); }
  };

  return (
    <Modal
      open
      title="Select streams"
      onClose={onClose}
      className="picker-modal"
      actions={(
        <>
          <button className="picker-cancel" onClick={onClose}>Cancel</button>
          <button className="picker-add" disabled={chosen.size === 0} onClick={() => { onAdd([...chosen]); onClose(); }}>
            Add to wall ({chosen.size})
          </button>
        </>
      )}
    >
      <input
        className="picker-search"
        type="search"
        placeholder="Search streams…"
        value={query}
        onChange={(e) => { setQuery(e.target.value); setFocusIdx(0); }}
        aria-label="Search streams"
      />
      <div className="picker-cols">
        <div className="picker-avail" role="tree" aria-label="Available streams" tabIndex={0} onKeyDown={onKey}>
          {avail.length === 0 && <div className="picker-empty">No available streams match.</div>}
          {avail.map((p, i) => (
            <div
              key={p.id}
              role="treeitem"
              aria-selected={chosen.has(p.id)}
              tabIndex={i === focusIdx ? 0 : -1}
              className={`picker-item${chosen.has(p.id) ? ' chosen' : ''}${i === focusIdx ? ' focused' : ''}`}
              onClick={() => toggle(p.id)}
            >
              {p.room} · {p.kind} · {p.peer_id}
            </div>
          ))}
        </div>
        <div className="picker-chosen" aria-label="Chosen streams">
          <div className="picker-chosen-head">Chosen ({chosen.size})</div>
          {[...chosen].map((id) => {
            const p = producers.find((x) => x.id === id);
            return p ? <div key={id} className="picker-chosen-item">{p.room} · {p.kind}</div> : null;
          })}
        </div>
      </div>
    </Modal>
  );
}