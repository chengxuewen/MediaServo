// FloorBoard (talk-app T5): the level-1 board — one row per designated talk
// room: hue+shape state glyph (K: never hue alone), holder, mode, queue chip,
// guarded actions. Disconnected rooms render UNKNOWN, never idle (K discipline).
//
// Rendering only: state lives in BoardStore (framework-agnostic), styling in
// tokens (`@mediaservo/ui`) — headless-friendly (D4).
import type { BoardRoomSnapshot, FloorCommand } from '@mediaservo/client';
import { Button, StatusGlyph, type GlyphShape, type GlyphTone } from '@mediaservo/react';
import { useBoardStore } from './hooks';

export interface FloorBoardProps {
  /** Designated talk rooms (reference-stable — e.g. a state array). */
  rooms: string[];
  className?: string;
}

interface RowPhase {
  tone: GlyphTone;
  shape: GlyphShape;
  label: string;
}

/** Derive the row's display state from the connection + latest floor frame. */
function derive(s: BoardRoomSnapshot): {
  phase: RowPhase;
  holders: string | null;
  queue: number | null;
  mode: string | null;
} {
  if (s.stale || s.conn !== 'connected') {
    // K discipline: a down connection is UNKNOWN, never idle.
    return { phase: { tone: 'unknown', shape: 'triangle', label: 'unknown — reconnecting' }, holders: null, queue: null, mode: null };
  }
  const ev = s.lastEvent;
  if (!ev) return { phase: { tone: 'off', shape: 'ring', label: 'idle' }, holders: null, queue: null, mode: null };
  switch (ev.type) {
    case 'floor_state': {
      const mode = ev.snapshot.mode;
      const queue = ev.snapshot.queue?.length ?? null;
      if (ev.snapshot.grants.length) {
        const holders = ev.snapshot.grants.join(' + ');
        return { phase: { tone: 'ok', shape: 'dot', label: `held: ${holders}` }, holders, queue, mode };
      }
      if (queue) return { phase: { tone: 'warn', shape: 'seg', label: `queued (${queue})` }, holders: null, queue, mode };
      return { phase: { tone: 'off', shape: 'ring', label: 'idle' }, holders: null, queue, mode };
    }
    case 'floor_granted': {
      const holders = ev.grants.join(' + ');
      return { phase: { tone: 'ok', shape: 'dot', label: `held: ${holders}` }, holders, queue: null, mode: null };
    }
    case 'floor_taken':
      return { phase: { tone: 'ok', shape: 'dot', label: `held: ${ev.by}` }, holders: ev.by, queue: null, mode: null };
    case 'floor_queued':
      return { phase: { tone: 'warn', shape: 'seg', label: `queued (#${ev.position})` }, holders: null, queue: null, mode: null };
    case 'floor_idle':
      return { phase: { tone: 'off', shape: 'ring', label: 'idle' }, holders: null, queue: null, mode: null };
    case 'floor_denied':
      return { phase: { tone: 'danger', shape: 'triangle', label: `denied: ${ev.reason}` }, holders: null, queue: null, mode: null };
  }
}

function FloorRow({
  s,
  send,
}: {
  s: BoardRoomSnapshot;
  send: (roomId: string, cmd: FloorCommand) => void;
}) {
  const { phase, holders, queue, mode } = derive(s);
  const holding = Boolean(s.peerId && holders?.includes(s.peerId));
  const staleReason = 'connection down — state unknown';
  return (
    <div className="msr-talk-row" role="listitem">
      <StatusGlyph tone={phase.tone} shape={phase.shape} label={phase.label} />
      <div className="msr-talk-id">
        <span className="msr-talk-name">{s.roomId}</span>
        <span className="msr-talk-mode">{mode ?? '—'}</span>
      </div>
      <span className="msr-talk-state" aria-live="polite">
        {phase.label}
      </span>
      {queue !== null && <span className="msr-talk-queue">queue {queue}</span>}
      <span className="msr-talk-actions">
        <Button
          size="sm"
          variant="primary"
          disabled={s.stale || holding}
          disabledReason={s.stale ? staleReason : holding ? 'you already hold the floor' : undefined}
          onClick={() => send(s.roomId, { type: 'floor_request' })}
        >
          Request
        </Button>
        <Button
          size="sm"
          disabled={s.stale || !holding}
          disabledReason={s.stale ? staleReason : 'you are not holding the floor'}
          onClick={() => send(s.roomId, { type: 'floor_release' })}
        >
          Release
        </Button>
      </span>
    </div>
  );
}

export function FloorBoard({ rooms, className = '' }: FloorBoardProps) {
  const { snap, send } = useBoardStore(rooms);
  if (!rooms.length) {
    return (
      <div className={['msr-board-empty', className].filter(Boolean).join(' ')} role="status">
        No designated talk rooms — the floor board is empty.
      </div>
    );
  }
  return (
    <div className={['msr-board', className].filter(Boolean).join(' ')} role="list" aria-label="intercom rooms">
      {rooms.map((id) => {
        const s = snap.rooms[id];
        return s ? (
          <FloorRow key={id} s={s} send={send} />
        ) : (
          <FloorRow
            key={id}
            s={{ roomId: id, conn: 'connecting', stale: true, lastEvent: null, peerId: null }}
            send={send}
          />
        );
      })}
    </div>
  );
}
