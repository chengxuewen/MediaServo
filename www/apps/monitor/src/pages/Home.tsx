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
// B2 fix: a playing producer that vanishes from /sfu/stats (device offline/restart,
// uuid churn) is reconciled out of the wall after this grace (~3 poll cycles), so
// dead ids cannot permanently occupy cap16 slots. Transient stats gaps < grace keep
// the tile (shown as a closable offline placeholder meanwhile).
const DROP_GRACE_MS = 6000;

function wsServerUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}`;
}

/**
 * Monitor realtime face — slice ③b: device stream tree + multi-stream wall.
 * Tiles reuse the shared VideoPlayer (packages/react, tile variant) consuming via
 * SfuConsumerClient; addressing is device room (+ stream label when a registered
 * device exposes one). A PSK host room is single-producer → consuming the room =
 * that one stream; per-producer isolation inside a multi-producer room needs
 * registered-device stream labels (cross-ref /rooms), a later refinement.
 * Realtime timeline below. Recording stays deferred (RECORDING_READY, §8).
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
    try { localStorage.setItem(PLAY_COLS_KEY, String(n)); } catch { /* silent degrade (private mode) */ }
  };

  const byId = new Map(producers.map((p) => [p.id, p]));

  // B2 fix: reconcile dead playing ids (producer gone from /sfu/stats) after a grace
  // period so a device offline/restart cannot permanently fill the cap with dead ids.
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
    // B1 fix: per-room selection (matches the button's per-room anySelected label),
    // not the global selected.size — selecting in room A must not turn room B's
    // "Play All" into a silent no-op.
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
      <div className="mh-head">
        <h1>Monitor</h1>
        <p role="note">
          Wall (cap {MAX_PLAYING}) — signed in as {username ?? 'unknown'}{role ? ` [${role}]` : ''}. {canMonitor ? '' : 'Wall access requires admin/dispatcher. '}Realtime timeline below. Per-producer isolation within a multi-producer room needs registered-device stream labels; PSK host rooms are single-producer.
        </p>
      </div>
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
      {playing.length > 0 && (
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
              // B2 fix: a vanished producer renders a closable offline placeholder
              // (not a blank null) until the grace reconcile drops it — the cap slot
              // stays recoverable without a page refresh.
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
      <Timeline />
    </section>
  );
}