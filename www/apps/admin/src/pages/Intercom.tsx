import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { FloorBoard } from '@mediaservo/talk-ui';
import { listManagedRooms } from '../api/client';
import Rooms from './Rooms';
import Evidence from './Evidence';

/**
 * Intercom board page (talk-app T5): one row per designated talk room --
 * hue+shape state, holder, mode, queue, guarded Request/Release.
 * Disconnected rooms render UNKNOWN, never idle (K discipline).
 *
 * Room list source: managed-rooms kind=talk (admin + dispatcher readable).
 * The channel session (level 2) lands with batches T11/T12.
 */
export default function Intercom() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<'board'|'rooms'|'evidence'>('board');
  const [rooms, setRooms] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    listManagedRooms()
      .then((r) => {
        if (!alive) return;
        setRooms(r.rooms.filter((x) => x.kind === 'talk').map((x) => x.id));
      })
      .catch((e: unknown) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
    };
  }, []);

  return (
    <section>
      <h1>Intercom</h1>
      {error && <p role="alert">Failed to load the room list: {error}</p>}
      {rooms === null && !error && <p role="status">Loading rooms...</p>}
      <div className="hub-tabs" role="tablist" aria-label="Intercom">
        {(['board','rooms','evidence'] as const).map((t) => (
          <button key={t} role="tab" aria-selected={tab===t}
            className={tab===t ? 'hub-tab active' : 'hub-tab'}
            onClick={() => setTab(t)}>
            {t === 'board' ? 'Board' : t === 'rooms' ? 'Rooms' : 'Evidence'}
          </button>
        ))}
      </div>
      {tab === 'rooms' && <Rooms />}
      {tab === 'evidence' && <Evidence />}
      {tab === 'board' && rooms !== null && !error && (
        <FloorBoard rooms={rooms} onOpenRoom={(id) => navigate(`/intercom/${id}`)} />
      )}
    </section>
  );
}
